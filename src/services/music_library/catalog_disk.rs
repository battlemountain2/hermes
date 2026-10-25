use super::{Entry, FileIdentity};
use crate::{model::MusicTags, sandbox::Cancellation};
use rusqlite::{
    Connection, OpenFlags, TransactionBehavior, config::DbConfig, limits::Limit, params,
};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    mem::size_of,
    os::{
        fd::AsRawFd,
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const VERSION: i64 = 1;
const MAX_DATABASE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ROW_BYTES: usize = 16 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_NOTICE_BYTES: usize = 4096;
const NO_FOLLOW: i32 = 0x8000;
const DIRECTORY: i32 = 0x4000;
const NON_BLOCK: i32 = 0x800;
const META_SCHEMA: &str = "CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), root BLOB NOT NULL, root_identity BLOB NOT NULL, notice TEXT) STRICT";
const ENTRY_SCHEMA: &str = "CREATE TABLE entries (sequence INTEGER PRIMARY KEY, path BLOB NOT NULL, directory INTEGER NOT NULL CHECK (directory IN (0, 1)), modified INTEGER, identity BLOB NOT NULL, title TEXT, artist TEXT, album_artist TEXT, album TEXT, disc INTEGER, track INTEGER, tagged INTEGER NOT NULL CHECK (tagged IN (0, 1))) STRICT";

pub(super) fn path(root: &Path) -> PathBuf {
    let digest = gio::glib::compute_checksum_for_data(
        gio::glib::ChecksumType::Sha256,
        root.as_os_str().as_bytes(),
    )
    .expect("SHA-256 is available in GLib");
    gio::glib::user_cache_dir()
        .join("strata/search-catalog")
        .join(format!("general-{digest}.sqlite"))
}

fn cancelled(cancellation: &Cancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Search catalog operation cancelled".into())
    } else {
        Ok(())
    }
}

fn error(error: impl std::fmt::Display) -> String {
    format!("Search catalog: {error}")
}

struct DirectoryHandle(File);

impl DirectoryHandle {
    fn at(parent: &Self, name: &OsStr) -> Result<Self, String> {
        OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | DIRECTORY | NON_BLOCK)
            .open(parent.pinned().join(name))
            .map(Self)
            .map_err(error)
    }

    fn open(path: &Path, create: bool) -> Result<Self, String> {
        if !path.is_absolute() || path.as_os_str().as_bytes().contains(&0) {
            return Err("Search catalog requires an absolute directory".into());
        }
        let mut directory = Self(File::open("/").map_err(error)?);
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    if create {
                        match fs::DirBuilder::new()
                            .mode(0o700)
                            .create(directory.pinned().join(name))
                        {
                            Ok(()) => {}
                            Err(failure) if failure.kind() == std::io::ErrorKind::AlreadyExists => {
                            }
                            Err(failure) => return Err(error(failure)),
                        }
                    }
                    directory = Self::at(&directory, name)?;
                }
                _ => return Err("Search catalog directory contains a path escape".into()),
            }
        }
        Ok(directory)
    }

    fn pinned(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.0.as_raw_fd()))
    }

    fn private(&self) -> Result<(), String> {
        let metadata = self.0.metadata().map_err(error)?;
        if metadata.uid() != gio::Credentials::new().unix_user().map_err(error)?
            || metadata.mode() & 0o7777 != 0o700
        {
            return Err(
                "Search catalog directory must be owned by this user with mode 0700".into(),
            );
        }
        Ok(())
    }
}

fn private_file(file: &File) -> Result<fs::Metadata, String> {
    let metadata = file.metadata().map_err(error)?;
    if !metadata.is_file()
        || metadata.uid() != gio::Credentials::new().unix_user().map_err(error)?
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
        || metadata.len() > MAX_DATABASE_BYTES
    {
        return Err("Search catalog must be a private regular file of at most 64 MiB".into());
    }
    Ok(metadata)
}

struct Stage {
    directory: DirectoryHandle,
    parent: DirectoryHandle,
    name: OsString,
}

impl Stage {
    fn new(parent: &DirectoryHandle) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = OsString::from(format!(
            ".snapshot-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(parent.pinned().join(&name))
            .map_err(error)?;
        let stage = Self {
            directory: DirectoryHandle::at(parent, &name)?,
            parent: DirectoryHandle(parent.0.try_clone().map_err(error)?),
            name,
        };
        stage.directory.private()?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(NO_FOLLOW | NON_BLOCK)
            .open(stage.file())
            .map_err(error)?;
        private_file(&file)?;
        Ok(stage)
    }

    fn file(&self) -> PathBuf {
        self.directory.pinned().join("snapshot.sqlite")
    }

    fn copy(&self, mut source: File, cancellation: &Cancellation) -> Result<(), String> {
        let original = FileIdentity::from_metadata(&private_file(&source)?);
        let mut output = OpenOptions::new()
            .write(true)
            .custom_flags(NO_FOLLOW | NON_BLOCK)
            .open(self.file())
            .map_err(error)?;
        let mut buffer = [0; 64 * 1024];
        let mut total = 0u64;
        loop {
            cancelled(cancellation)?;
            let count = source.read(&mut buffer).map_err(error)?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .ok_or("Search catalog size overflow")?;
            if total > MAX_DATABASE_BYTES {
                return Err("Search catalog exceeds 64 MiB".into());
            }
            output.write_all(&buffer[..count]).map_err(error)?;
        }
        if FileIdentity::from_metadata(&private_file(&source)?) != original {
            return Err("Search catalog changed while being read".into());
        }
        Ok(())
    }

    fn connect(&self, writable: bool) -> Result<Connection, String> {
        let actual = fs::read_link(self.directory.pinned())
            .map_err(error)?
            .join("snapshot.sqlite");
        let flags = if writable {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = Connection::open_with_flags(&actual, flags).map_err(error)?;
        let opened = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | NON_BLOCK)
            .open(&actual)
            .map_err(error)?;
        let pinned = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | NON_BLOCK)
            .open(self.file())
            .map_err(error)?;
        if FileIdentity::from_metadata(&private_file(&opened)?)
            != FileIdentity::from_metadata(&private_file(&pinned)?)
        {
            return Err("Search catalog staging directory moved".into());
        }
        configure(&connection, writable)?;
        Ok(connection)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        for name in [
            "snapshot.sqlite",
            "snapshot.sqlite-journal",
            "snapshot.sqlite-wal",
            "snapshot.sqlite-shm",
        ] {
            let _removed = fs::remove_file(self.directory.pinned().join(name));
        }
        let _removed = fs::remove_dir(self.parent.pinned().join(&self.name));
    }
}

fn configure(connection: &Connection, writable: bool) -> Result<(), String> {
    for (limit, value) in [
        (Limit::SQLITE_LIMIT_LENGTH, MAX_ROW_BYTES as i32),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, MAX_ROW_BYTES as i32),
        (Limit::SQLITE_LIMIT_COLUMN, 16),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 32),
        (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 2),
        (Limit::SQLITE_LIMIT_VDBE_OP, 10_000),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
        (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 16),
        (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 0),
        (Limit::SQLITE_LIMIT_WORKER_THREADS, 0),
    ] {
        connection.set_limit(limit, value).map_err(error)?;
    }
    for (setting, enabled) in [
        (DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true),
        (DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE, false),
    ] {
        connection.set_db_config(setting, enabled).map_err(error)?;
    }
    connection
        .busy_timeout(Duration::from_millis(50))
        .map_err(error)?;
    connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA cache_size=-2048; PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY; PRAGMA cell_size_check=ON;").map_err(error)?;
    if writable {
        connection.execute_batch("PRAGMA page_size=4096; PRAGMA max_page_count=16384; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;").map_err(error)?;
    } else {
        connection
            .execute_batch("PRAGMA query_only=ON;")
            .map_err(error)?;
    }
    Ok(())
}

fn identity_bytes(identity: &FileIdentity) -> Vec<u8> {
    [
        identity.device.to_le_bytes(),
        identity.inode.to_le_bytes(),
        identity.size.to_le_bytes(),
        identity.modified.0.to_le_bytes(),
        identity.modified.1.to_le_bytes(),
        identity.changed.0.to_le_bytes(),
        identity.changed.1.to_le_bytes(),
    ]
    .concat()
}

fn decode_identity(bytes: &[u8]) -> Result<FileIdentity, String> {
    if bytes.len() != 56 {
        return Err("Invalid search catalog identity".into());
    }
    let mut words = [[0; 8]; 7];
    for (word, bytes) in words.iter_mut().zip(bytes.chunks_exact(8)) {
        word.copy_from_slice(bytes);
    }
    let identity = FileIdentity {
        device: u64::from_le_bytes(words[0]),
        inode: u64::from_le_bytes(words[1]),
        size: u64::from_le_bytes(words[2]),
        modified: (i64::from_le_bytes(words[3]), i64::from_le_bytes(words[4])),
        changed: (i64::from_le_bytes(words[5]), i64::from_le_bytes(words[6])),
    };
    if !(0..1_000_000_000).contains(&identity.modified.1)
        || !(0..1_000_000_000).contains(&identity.changed.1)
    {
        return Err("Invalid search catalog identity timestamp".into());
    }
    Ok(identity)
}

struct Sources {
    root: PathBuf,
    directory: DirectoryHandle,
    identity: FileIdentity,
}

impl Sources {
    fn new(root: &Path) -> Result<Self, String> {
        if root.as_os_str().len() > MAX_PATH_BYTES {
            return Err("Search catalog root path is too long".into());
        }
        let directory = DirectoryHandle::open(root, false)?;
        let identity = FileIdentity::from_metadata(&directory.0.metadata().map_err(error)?);
        Ok(Self {
            root: root.to_owned(),
            directory,
            identity,
        })
    }

    fn relative<'path>(&self, path: &'path Path) -> Result<&'path Path, String> {
        let bytes = path.as_os_str().as_bytes();
        if !path.is_absolute() || bytes.len() > MAX_PATH_BYTES || bytes.contains(&0) {
            return Err("Invalid search catalog source path".into());
        }
        let relative = path.strip_prefix(&self.root).map_err(error)?;
        if relative.as_os_str().is_empty()
            || relative
                .as_os_str()
                .as_bytes()
                .split(|byte| *byte == b'/')
                .any(|part| part.is_empty() || part.starts_with(b"."))
            || relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err("Search catalog source is hidden or escapes its root".into());
        }
        Ok(relative)
    }

    fn fresh(&self, entry: &Entry) -> Result<bool, String> {
        let relative = self.relative(&entry.path)?;
        let mut current = None;
        let mut parts = relative.components().peekable();
        while let Some(Component::Normal(name)) = parts.next() {
            let parent = current.as_ref().unwrap_or(&self.directory);
            if parts.peek().is_some() {
                current = match DirectoryHandle::at(parent, name) {
                    Ok(directory) => Some(directory),
                    Err(_) => return Ok(false),
                };
            } else {
                let candidate = parent.pinned().join(name);
                let metadata = match fs::symlink_metadata(&candidate) {
                    Ok(metadata) if metadata.is_file() || metadata.is_dir() => metadata,
                    _ => return Ok(false),
                };
                return Ok((metadata.is_file() || metadata.is_dir())
                    && metadata.is_dir() == entry.is_directory
                    && entry.identity.as_ref() == Some(&FileIdentity::from_metadata(&metadata)));
            }
        }
        Ok(false)
    }

    fn root_unchanged(&self) -> Result<(), String> {
        let current = DirectoryHandle::open(&self.root, false)?;
        let identity = FileIdentity::from_metadata(&current.0.metadata().map_err(error)?);
        if identity.device != self.identity.device || identity.inode != self.identity.inode {
            return Err("Search catalog root changed".into());
        }
        Ok(())
    }
}

fn metadata(
    connection: &Connection,
    sources: &Sources,
    require_identity: bool,
) -> Result<Option<String>, String> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(error)?;
    if version > VERSION {
        return Err("Search catalog has a newer version; preserving it unchanged".into());
    }
    if version != VERSION {
        return Err("Unsupported search catalog version".into());
    }
    let mut statement = connection
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema LIMIT 3")
        .map_err(error)?;
    let mut rows = statement.query([]).map_err(error)?;
    let mut seen = 0;
    while let Some(row) = rows.next().map_err(error)? {
        let kind: String = row.get(0).map_err(error)?;
        let name: String = row.get(1).map_err(error)?;
        let table: String = row.get(2).map_err(error)?;
        let sql: String = row.get(3).map_err(error)?;
        let expected = match name.as_str() {
            "metadata" => META_SCHEMA,
            "entries" => ENTRY_SCHEMA,
            _ => return Err("Unexpected search catalog schema".into()),
        };
        if kind != "table" || name != table || sql != expected {
            return Err("Unexpected search catalog schema".into());
        }
        seen += 1;
    }
    if seen != 2 {
        return Err("Incomplete search catalog schema".into());
    }
    let (root, identity, notice): (Vec<u8>, Vec<u8>, Option<String>) = connection
        .query_row(
            "SELECT root, root_identity, notice FROM metadata WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(error)?;
    let identity = decode_identity(&identity)?;
    if root != sources.root.as_os_str().as_bytes()
        || (require_identity
            && (identity.device != sources.identity.device
                || identity.inode != sources.identity.inode))
    {
        return Err("Search catalog belongs to a different root".into());
    }
    valid_notice(notice.as_deref())?;
    Ok(notice)
}

fn valid_notice(notice: Option<&str>) -> Result<(), String> {
    if notice.is_some_and(|notice| notice.len() > MAX_NOTICE_BYTES || notice.contains('\0')) {
        return Err("Invalid search catalog notice".into());
    }
    Ok(())
}

fn validate_entry(entry: &Entry, sources: &Sources) -> Result<(), String> {
    sources.relative(&entry.path)?;
    let identity = entry
        .identity
        .as_ref()
        .ok_or("Search catalog source has no identity")?;
    if entry
        .modified_unix_seconds
        .is_some_and(|modified| modified != identity.modified.0)
        || entry
            .music_tags
            .as_ref()
            .is_some_and(|tags| !super::cache::valid_tags(tags))
        || entry.music_tags.is_some() && !entry.is_audio()
        || super::entry_bytes(entry) > MAX_ROW_BYTES
    {
        return Err("Invalid or oversized search catalog entry".into());
    }
    decode_identity(&identity_bytes(identity))?;
    Ok(())
}

fn location(path: &Path, create: bool) -> Result<(DirectoryHandle, OsString), String> {
    let name = path.file_name().ok_or("Invalid search catalog filename")?;
    if name.as_bytes().contains(&0) || path.as_os_str().len() > MAX_PATH_BYTES {
        return Err("Invalid search catalog filename".into());
    }
    let directory = DirectoryHandle::open(
        path.parent().ok_or("Invalid search catalog directory")?,
        create,
    )?;
    directory.private()?;
    directory.0.try_lock().map_err(error)?;
    Ok((directory, name.to_owned()))
}

fn existing(directory: &DirectoryHandle, name: &OsStr) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(directory.pinned().join(name))
        .map_err(error)?;
    private_file(&file)?;
    Ok(file)
}

pub(super) fn read(
    path: &Path,
    root: &Path,
    entry_limit: usize,
    byte_limit: usize,
    cancellation: &Cancellation,
) -> Result<(Vec<Entry>, Option<String>), String> {
    cancelled(cancellation)?;
    let sources = Sources::new(root)?;
    let (directory, name) = location(path, false)?;
    let stage = Stage::new(&directory)?;
    stage.copy(existing(&directory, &name)?, cancellation)?;
    let connection = stage.connect(false)?;
    let mut notice = metadata(&connection, &sources, true)?;
    let entry_limit = entry_limit.min(super::MAX_ENTRIES);
    let byte_limit = byte_limit.min(super::MAX_INDEX_BYTES);
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM (SELECT sequence FROM entries LIMIT ?1)",
            [entry_limit as i64 + 1],
            |row| row.get(0),
        )
        .map_err(error)?;
    let count = usize::try_from(count).map_err(error)?;
    if count > entry_limit || count.saturating_mul(size_of::<Entry>()) > byte_limit {
        return Err("Search catalog exceeds the entry or memory budget".into());
    }
    let mut entries = Vec::with_capacity(count);
    let mut bytes = entries.capacity().saturating_mul(size_of::<Entry>());
    let mut statement = connection.prepare("SELECT path, directory, modified, identity, title, artist, album_artist, album, disc, track, tagged FROM entries ORDER BY sequence LIMIT ?1").map_err(error)?;
    let mut rows = statement.query([entry_limit as i64 + 1]).map_err(error)?;
    while let Some(row) = rows.next().map_err(error)? {
        cancelled(cancellation)?;
        let native: Vec<u8> = row.get(0).map_err(error)?;
        let directory: i64 = row.get(1).map_err(error)?;
        let tagged: i64 = row.get(10).map_err(error)?;
        let tags = MusicTags {
            title: row.get(4).map_err(error)?,
            artist: row.get(5).map_err(error)?,
            album_artist: row.get(6).map_err(error)?,
            album: row.get(7).map_err(error)?,
            disc: row.get(8).map_err(error)?,
            track: row.get(9).map_err(error)?,
        };
        if !(0..=1).contains(&directory)
            || !(0..=1).contains(&tagged)
            || tagged == 0 && tags != MusicTags::default()
        {
            return Err("Invalid search catalog entry type".into());
        }
        let identity: Vec<u8> = row.get(3).map_err(error)?;
        let entry = Entry {
            path: PathBuf::from(OsString::from_vec(native)),
            is_directory: directory == 1,
            modified_unix_seconds: row.get(2).map_err(error)?,
            identity: Some(decode_identity(&identity)?),
            music_tags: (tagged == 1).then_some(tags),
        };
        validate_entry(&entry, &sources)?;
        bytes = bytes
            .checked_add(super::entry_bytes(&entry) - size_of::<Entry>())
            .ok_or("Search catalog memory budget overflow")?;
        if bytes > byte_limit || entries.len() >= entry_limit {
            return Err("Search catalog exceeds the entry or memory budget".into());
        }
        entries.push(entry);
    }
    let mut stale = false;
    let mut kept = 0;
    for index in 0..entries.len() {
        cancelled(cancellation)?;
        if sources.fresh(&entries[index])? {
            entries.swap(kept, index);
            kept += 1;
        } else {
            stale = true;
        }
    }
    entries.truncate(kept);
    sources.root_unchanged()?;
    cancelled(cancellation)?;
    if stale {
        super::append_notice(
            &mut notice,
            "Cached sources changed; refreshing search catalog",
        );
    }
    Ok((entries, notice))
}

pub(super) fn write(
    path: &Path,
    root: &Path,
    entries: &[Entry],
    notice: Option<&str>,
    cancellation: &Cancellation,
) -> Result<(), String> {
    cancelled(cancellation)?;
    valid_notice(notice)?;
    if entries.len() > super::MAX_ENTRIES {
        return Err("Search catalog exceeds the entry budget".into());
    }
    let sources = Sources::new(root)?;
    let bytes = entries
        .iter()
        .try_fold(0usize, |total, entry| -> Result<usize, String> {
            cancelled(cancellation)?;
            validate_entry(entry, &sources)?;
            let total = total
                .checked_add(super::entry_bytes(entry))
                .ok_or("Search catalog memory budget overflow".to_owned())?;
            if total > super::MAX_INDEX_BYTES {
                return Err("Search catalog exceeds the memory budget".into());
            }
            Ok(total)
        })?;
    if entries.len() > super::MAX_ENTRIES || bytes > super::MAX_INDEX_BYTES {
        return Err("Search catalog exceeds the entry or memory budget".into());
    }
    let (directory, name) = location(path, true)?;
    match fs::symlink_metadata(directory.pinned().join(&name)) {
        Ok(_) => {
            let old = Stage::new(&directory)?;
            old.copy(existing(&directory, &name)?, cancellation)?;
            metadata(&old.connect(false)?, &sources, false)?;
        }
        Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {}
        Err(failure) => return Err(error(failure)),
    }
    let stage = Stage::new(&directory)?;
    let mut connection = stage.connect(true)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(error)?;
    transaction.execute_batch(META_SCHEMA).map_err(error)?;
    transaction.execute_batch(ENTRY_SCHEMA).map_err(error)?;
    transaction
        .execute_batch("PRAGMA user_version=1;")
        .map_err(error)?;
    transaction
        .execute(
            "INSERT INTO metadata(singleton, root, root_identity, notice) VALUES(1, ?1, ?2, ?3)",
            params![
                root.as_os_str().as_bytes(),
                identity_bytes(&sources.identity),
                notice
            ],
        )
        .map_err(error)?;
    {
        let mut statement = transaction.prepare("INSERT INTO entries(sequence, path, directory, modified, identity, title, artist, album_artist, album, disc, track, tagged) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)").map_err(error)?;
        for (sequence, entry) in entries.iter().enumerate() {
            cancelled(cancellation)?;
            if !sources.fresh(entry)? {
                return Err("Search catalog source changed before saving".into());
            }
            let tags = entry.music_tags.as_ref();
            statement
                .execute(params![
                    sequence as i64,
                    entry.path.as_os_str().as_bytes(),
                    entry.is_directory,
                    entry.modified_unix_seconds,
                    identity_bytes(entry.identity.as_ref().ok_or("Missing source identity")?),
                    tags.and_then(|tags| tags.title.as_deref()),
                    tags.and_then(|tags| tags.artist.as_deref()),
                    tags.and_then(|tags| tags.album_artist.as_deref()),
                    tags.and_then(|tags| tags.album.as_deref()),
                    tags.and_then(|tags| tags.disc),
                    tags.and_then(|tags| tags.track),
                    tags.is_some(),
                ])
                .map_err(error)?;
        }
    }
    cancelled(cancellation)?;
    transaction.commit().map_err(error)?;
    connection.close().map_err(|(_, failure)| error(failure))?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(stage.file())
        .map_err(error)?;
    private_file(&file)?;
    file.sync_all().map_err(error)?;
    for entry in entries {
        cancelled(cancellation)?;
        if !sources.fresh(entry)? {
            return Err("Search catalog source changed before publication".into());
        }
    }
    sources.root_unchanged()?;
    cancelled(cancellation)?;
    fs::rename(stage.file(), directory.pinned().join(&name)).map_err(error)?;
    directory.0.sync_all().map_err(error)
}

#[cfg(test)]
mod tests;
