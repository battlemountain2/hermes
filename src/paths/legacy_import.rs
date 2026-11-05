// SPDX-License-Identifier: GPL-3.0-or-later

//! Opt-in, local-only import. No caller may initialize stores until inspection or
//! an explicit decision succeeds. Originals and existing destinations are never
//! changed. Immutable journal generations precede every user-data write.
//!
//! Linux descriptor-anchored traversal rejects symlinks in *every* component;
//! keeping directory handles alive closes ancestor replacement races. This uses
//! the same /proc/self/fd facility as the thumbnail cache, without unsafe code.

use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

const NO_FOLLOW: i32 = 0x20000;
const DIRECTORY: i32 = 0x10000;
const NON_BLOCK: i32 = 0x800;
const TRANSACTION: &str = "legacy-import-v2";
const MAX_FILE: u64 = 16 * 1024 * 1024;
const MAX_TOTAL: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 128;
const MAX_JOURNAL: u64 = 128 * 1024;
const MAX_GENERATIONS: usize = 1024;
const CONFIG: &[&str] = &[
    "settings.toml",
    "trails.toml",
    "folder_views.json",
    "sidebar-order",
    "recent-folders.toml",
];
const DATA: &[&str] = &[
    "music-collections-v1.json",
    "ratings-v1.json",
    "saved-searches/queries-v1.json",
];
const STATE: &[&str] = &["music-queue-v1.json"];

#[derive(Debug)]
pub(crate) enum Error {
    Io(io::ErrorKind),
    UnsafePath,
    Limit,
    InvalidJournal,
    Changed,
    Busy,
    Development,
}

impl Error {
    // Never include filenames, journal contents, or OS error strings in UI/logs.
    pub(crate) fn message(&self) -> &'static str {
        match self {
            Self::Busy => "Another import is running. Wait for it to finish, then retry.",
            Self::UnsafePath => {
                "Import stopped at an unsafe path or symbolic link. Your files were preserved."
            }
            Self::Limit => {
                "Legacy data exceeds the bounded import limits. Your files were preserved."
            }
            Self::InvalidJournal => {
                "The local recovery record needs attention. Your files and backups were preserved."
            }
            Self::Changed => {
                "Import data changed or failed verification. Your files and backups were preserved."
            }
            Self::Development => "Legacy import is disabled in development mode.",
            Self::Io(io::ErrorKind::PermissionDenied) => {
                "Import needs access to its local data directories. Your files were preserved."
            }
            Self::Io(_) => {
                "Import could not finish. Fix the local storage problem, then retry. Your files and backups were preserved."
            }
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub(crate) struct Roots {
    pub(crate) source: [PathBuf; 3],
    pub(crate) target: [PathBuf; 3],
    pub(crate) development: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Ready,
    Consent { recovering: bool },
}

#[derive(Clone, Copy)]
pub(crate) enum Choice {
    Import,
    KeepHermes,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    root: usize,
    relative: PathBuf,
    bytes: u64,
    sha256: String,
    state: EntryState,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
enum EntryState {
    Pending,
    BackedUp,
    Published,
    Preserved,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    completed: bool,
    keep_hermes: bool,
    entries: Vec<Entry>,
}

// Decode old records only for completion, never execute their stored paths.
#[derive(Deserialize)]
struct OldJournal {
    version: u32,
    completed: bool,
}

struct Directory(File);

impl Directory {
    fn open(path: &Path, create: bool) -> Result<Self> {
        if !cfg!(target_os = "linux") || !path.is_absolute() {
            return Err(Error::UnsafePath);
        }
        let mut directory = Self(File::open("/")?);
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(name) => directory = directory.child(name.as_ref(), create)?,
                _ => return Err(Error::UnsafePath),
            }
        }
        Ok(directory)
    }

    fn path(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.0.as_raw_fd()))
    }

    fn child(&self, name: &Path, create: bool) -> Result<Self> {
        if name.components().count() != 1
            || !matches!(name.components().next(), Some(Component::Normal(_)))
        {
            return Err(Error::UnsafePath);
        }
        let path = self.path().join(name);
        if create {
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => self.0.sync_all()?,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        // O_NOFOLLOW and O_DIRECTORY apply together to each single component.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | DIRECTORY | NON_BLOCK)
            .open(&path)
            .map_err(|error| match error.raw_os_error() {
                Some(20 | 40) => Error::UnsafePath, // ENOTDIR / ELOOP
                _ => error.into(),
            })?;
        Ok(Self(file))
    }

    fn private(&self) -> Result<()> {
        let metadata = self.0.metadata()?;
        let uid = gtk::gio::Credentials::new()
            .unix_user()
            .map_err(|_| Error::UnsafePath)?;
        if metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            return Err(Error::UnsafePath);
        }
        Ok(())
    }

    fn file(&self, name: &Path) -> Result<File> {
        let path = self.path().join(name);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(Error::UnsafePath);
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | NON_BLOCK)
            .open(&path)?;
        let actual = file.metadata()?;
        if !actual.is_file() || actual.dev() != metadata.dev() || actual.ino() != metadata.ino() {
            return Err(Error::UnsafePath);
        }
        Ok(file)
    }
}

fn parent(path: &Path, create: bool) -> Result<(Directory, &Path)> {
    let name = path.file_name().ok_or(Error::UnsafePath)?;
    let directory = Directory::open(path.parent().ok_or(Error::UnsafePath)?, create)?;
    Ok((directory, Path::new(name)))
}

fn missing(error: &Error) -> bool {
    matches!(error, Error::Io(io::ErrorKind::NotFound))
}

fn read_bounded(mut file: File, limit: u64) -> Result<Vec<u8>> {
    if file.metadata()?.len() > limit {
        return Err(Error::Limit);
    }
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Limit);
    }
    Ok(bytes)
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let (directory, name) = parent(path, false)?;
    read_bounded(directory.file(name)?, limit)
}

fn checksum(bytes: &[u8]) -> Result<String> {
    gtk::glib::compute_checksum_for_data(gtk::glib::ChecksumType::Sha256, bytes)
        .map(|value| value.to_string())
        .ok_or(Error::Changed)
}

fn verified(bytes: &[u8], entry: &Entry) -> Result<()> {
    if bytes.len() as u64 != entry.bytes || checksum(bytes)? != entry.sha256 {
        return Err(Error::Changed);
    }
    Ok(())
}

// Every staging directory is exclusively reserved and private. Never reuse or
// clean up someone else's predictable temporary name (including the v1 name).
struct Stage {
    parent: Directory,
    name: PathBuf,
    directory: Directory,
}

impl Stage {
    fn new(parent: &Directory) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..32 {
            let name = PathBuf::from(format!(
                ".import-stage-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::DirBuilder::new()
                .mode(0o700)
                .create(parent.path().join(&name))
            {
                Ok(()) => {
                    let directory = parent.child(&name, false)?;
                    directory.private()?;
                    return Ok(Self {
                        parent: Directory(parent.0.try_clone()?),
                        name,
                        directory,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::Busy)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.directory.path().join("content"));
        let _ = fs::remove_dir(self.parent.path().join(&self.name));
    }
}

// Hard-link publication is atomic and fails if *anything* already occupies the
// destination, even if it appeared after inspection. Stage on that filesystem.
fn publish(directory: &Directory, name: &Path, bytes: &[u8]) -> Result<()> {
    let stage = Stage::new(directory)?;
    let staged = stage.directory.path().join("content");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(&staged)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if read_bounded(stage.directory.file(Path::new("content"))?, MAX_FILE)? != bytes {
        return Err(Error::Changed);
    }
    fs::hard_link(&staged, directory.path().join(name))?;
    directory.0.sync_all()?;
    Ok(())
}

impl Roots {
    pub(crate) fn current() -> Self {
        Self {
            source: [
                super::legacy_config_dir(),
                super::legacy_data_dir(),
                super::legacy_state_dir(),
            ],
            target: [super::config_dir(), super::data_dir(), super::state_dir()],
            development: super::is_development(),
        }
    }

    fn transaction(&self) -> Result<Directory> {
        let directory =
            Directory::open(&self.target[2], false)?.child(Path::new(TRANSACTION), false)?;
        directory.private()?;
        Ok(directory)
    }

    fn old_journal(&self) -> Result<Option<(OldJournal, Vec<u8>)>> {
        let bytes = match read(&self.target[2].join("migration-v1.json"), MAX_JOURNAL) {
            Ok(bytes) => bytes,
            Err(error) if missing(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        let journal: OldJournal =
            serde_json::from_slice(&bytes).map_err(|_| Error::InvalidJournal)?;
        if journal.version != 1 {
            return Err(Error::InvalidJournal);
        }
        Ok(Some((journal, bytes)))
    }

    pub(crate) fn inspect(&self) -> Result<Status> {
        // Do not even inspect the shared legacy source in development mode.
        if self.development {
            return Ok(Status::Ready);
        }
        match self.transaction() {
            Ok(directory) => {
                return Ok(match load(&directory)? {
                    Some((_, journal)) if journal.completed => Status::Ready,
                    _ => Status::Consent { recovering: true },
                });
            }
            Err(error) if missing(&error) => {}
            Err(error) => return Err(error),
        }
        if let Some((old, _)) = self.old_journal()? {
            return Ok(if old.completed {
                Status::Ready
            } else {
                Status::Consent { recovering: true }
            });
        }
        Ok(if self.plan()?.is_empty() {
            Status::Ready
        } else {
            Status::Consent { recovering: false }
        })
    }

    fn plan(&self) -> Result<Vec<Entry>> {
        let mut candidates = Vec::new();
        for (root, names) in [CONFIG, DATA, STATE].iter().enumerate() {
            for name in *names {
                candidates.push((root, PathBuf::from(name)));
            }
        }
        let themes = self.source[0].join("themes");
        match Directory::open(&themes, false) {
            Ok(directory) => {
                for (count, item) in fs::read_dir(directory.path())?.enumerate() {
                    if count >= MAX_FILES {
                        return Err(Error::Limit);
                    }
                    let name = item?.file_name();
                    if Path::new(&name)
                        .extension()
                        .is_some_and(|extension| extension == "toml")
                    {
                        candidates.push((0, PathBuf::from("themes").join(name)));
                    }
                }
            }
            Err(error) if missing(&error) => {}
            Err(error) => return Err(error),
        }
        candidates.sort();
        let mut entries = Vec::new();
        let mut total = 0;
        for (root, relative) in candidates {
            let bytes = match read(&self.source[root].join(&relative), MAX_FILE) {
                Ok(bytes) => bytes,
                Err(error) if missing(&error) => continue,
                Err(error) => return Err(error),
            };
            total += bytes.len() as u64;
            if total > MAX_TOTAL || entries.len() >= MAX_FILES {
                return Err(Error::Limit);
            }
            entries.push(Entry {
                root,
                relative,
                bytes: bytes.len() as u64,
                sha256: checksum(&bytes)?,
                state: EntryState::Pending,
            });
        }
        Ok(entries)
    }

    pub(crate) fn decide(&self, choice: Choice) -> Result<()> {
        if self.development {
            return Err(Error::Development);
        }
        let state = Directory::open(&self.target[2], true)?;
        let directory = state.child(Path::new(TRANSACTION), true)?;
        directory.private()?;
        // A persistent lock inode, never removed. Kernel releases it on crashes.
        directory.0.try_lock().map_err(|_| Error::Busy)?;
        let loaded = load(&directory)?;
        let (mut generation, mut journal) = match loaded {
            Some((generation, journal)) => (generation + 1, journal),
            None => {
                let journal = Journal {
                    version: 2,
                    completed: false,
                    keep_hermes: false,
                    entries: if matches!(choice, Choice::Import) {
                        self.plan()?
                    } else {
                        Vec::new()
                    },
                };
                save(&directory, 0, &journal)?;
                (1, journal)
            }
        };
        if journal.completed {
            return Ok(());
        }
        if matches!(choice, Choice::KeepHermes) {
            journal.keep_hermes = true;
            journal.completed = true;
            return save(&directory, generation, &journal);
        }
        if let Some((_, bytes)) = self.old_journal()? {
            // Preserve the old journal verbatim, but never trust its paths.
            match publish(&directory, Path::new("previous-journal.json"), &bytes) {
                Ok(()) => {}
                Err(Error::Io(io::ErrorKind::AlreadyExists)) => {
                    if read_bounded(
                        directory.file(Path::new("previous-journal.json"))?,
                        MAX_JOURNAL,
                    )? != bytes
                    {
                        return Err(Error::Changed);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        let backups = directory.child(Path::new("backups"), true)?;
        backups.private()?;
        for index in 0..journal.entries.len() {
            if matches!(
                journal.entries[index].state,
                EntryState::Published | EntryState::Preserved
            ) {
                continue;
            }
            let entry = &journal.entries[index];
            let destination = self.target[entry.root].join(&entry.relative);
            let (target, name) = parent(&destination, true)?;
            match fs::symlink_metadata(target.path().join(name)) {
                Ok(metadata) => {
                    if !metadata.is_file() || metadata.file_type().is_symlink() {
                        return Err(Error::UnsafePath);
                    }
                    journal.entries[index].state = EntryState::Preserved;
                    save(&directory, generation, &journal)?;
                    generation += 1;
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let backup_name = PathBuf::from(format!("{index:03}.backup"));
            let bytes = match backups.file(&backup_name) {
                Ok(file) => {
                    let bytes = read_bounded(file, MAX_FILE)?;
                    verified(&bytes, entry)?;
                    bytes
                }
                Err(error) if missing(&error) && entry.state == EntryState::Pending => {
                    let bytes = read(&self.source[entry.root].join(&entry.relative), MAX_FILE)?;
                    verified(&bytes, entry)?;
                    publish(&backups, &backup_name, &bytes)?;
                    bytes
                }
                Err(error) => return Err(error),
            };
            // Backup is durable and checksum-validated before this checkpoint;
            // retry can use it even if the original disappears after a crash.
            if journal.entries[index].state == EntryState::Pending {
                journal.entries[index].state = EntryState::BackedUp;
                save(&directory, generation, &journal)?;
                generation += 1;
            }
            match publish(&target, name, &bytes) {
                Ok(()) => journal.entries[index].state = EntryState::Published,
                Err(Error::Io(io::ErrorKind::AlreadyExists)) => {
                    // Re-check the raced-in entry; never follow a symlink.
                    target.file(name)?;
                    journal.entries[index].state = EntryState::Preserved;
                }
                Err(error) => return Err(error),
            }
            save(&directory, generation, &journal)?;
            generation += 1;
        }
        journal.completed = true;
        save(&directory, generation, &journal)
    }
}

fn allowed(entry: &Entry) -> bool {
    if entry.root >= 3
        || !entry
            .relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return false;
    }
    let names = [CONFIG, DATA, STATE][entry.root];
    names.iter().any(|name| entry.relative == Path::new(name))
        || (entry.root == 0
            && entry.relative.components().count() == 2
            && entry.relative.starts_with("themes")
            && entry
                .relative
                .extension()
                .is_some_and(|extension| extension == "toml"))
}

fn validate(journal: &Journal) -> Result<()> {
    let mut keys = HashSet::new();
    let mut total = 0_u64;
    if journal.version != 2 || journal.entries.len() > MAX_FILES {
        return Err(Error::InvalidJournal);
    }
    for entry in &journal.entries {
        total = total.saturating_add(entry.bytes);
        if !allowed(entry)
            || !keys.insert((entry.root, &entry.relative))
            || entry.bytes > MAX_FILE
            || total > MAX_TOTAL
            || entry.sha256.len() != 64
            || !entry.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || (journal.completed
                && !journal.keep_hermes
                && !matches!(entry.state, EntryState::Published | EntryState::Preserved))
        {
            return Err(Error::InvalidJournal);
        }
    }
    Ok(())
}

fn load(directory: &Directory) -> Result<Option<(usize, Journal)>> {
    let mut latest = None;
    for (count, item) in fs::read_dir(directory.path())?.enumerate() {
        if count >= MAX_GENERATIONS * 2 {
            return Err(Error::Limit);
        }
        let name = item?.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(number) = name
            .strip_prefix("journal-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let number: usize = number.parse().map_err(|_| Error::InvalidJournal)?;
        if number >= MAX_GENERATIONS || name != format!("journal-{number:04}.json") {
            return Err(Error::InvalidJournal);
        }
        latest = Some(latest.map_or(number, |previous: usize| previous.max(number)));
    }
    latest
        .map(|generation| {
            let name = PathBuf::from(format!("journal-{generation:04}.json"));
            let bytes = read_bounded(directory.file(&name)?, MAX_JOURNAL)?;
            let journal = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidJournal)?;
            validate(&journal)?;
            Ok((generation, journal))
        })
        .transpose()
}

fn save(directory: &Directory, generation: usize, journal: &Journal) -> Result<()> {
    validate(journal)?;
    if generation >= MAX_GENERATIONS {
        return Err(Error::Limit);
    }
    let bytes = serde_json::to_vec(journal).map_err(|_| Error::InvalidJournal)?;
    if bytes.len() as u64 > MAX_JOURNAL {
        return Err(Error::Limit);
    }
    publish(
        directory,
        Path::new(&format!("journal-{generation:04}.json")),
        &bytes,
    )
}

#[cfg(test)]
mod tests;
