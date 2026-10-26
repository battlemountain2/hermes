use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use gtk::{gio, glib};
use serde::{Deserialize, Serialize};

use super::super::Cancellation;

const MAGIC: &[u8] = b"HERMES-OFFICE-PDF-1\n";
const CONFIG: &[u8] =
    b"office-full-pdf-v2;writer,impress,calc;raster150dpi;fresh-safe-profile;C.UTF-8;svp";
const MAX_PDF: usize = 32 * 1024 * 1024;
const MAX_HEADER: usize = 128 * 1024;
const MAX_FILE: usize = MAGIC.len() + 4 + MAX_HEADER + MAX_PDF;
const MAX_ENTRIES: usize = 8;
const MAX_BYTES: u64 = 128 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 64;
const MAX_ENVIRONMENT_ENTRIES: usize = 8192;
const MAX_ENVIRONMENT_DEPTH: usize = 24;
const NO_FOLLOW: i32 = 0x20000;
const DIRECTORY: i32 = 0x10000;
const NON_BLOCK: i32 = 0x800;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Stamp {
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    mode: u32,
}

impl Stamp {
    fn from(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            mode: metadata.mode(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct Key {
    path: Vec<u8>,
    source: Stamp,
    environment: String,
}

impl Key {
    pub(super) fn input(&self) -> &Path {
        Path::new(OsStr::from_bytes(&self.path))
    }

    pub(super) fn is_current(&self, cancellation: &Cancellation) -> bool {
        !cancellation.is_cancelled()
            && source(self.input()).is_ok_and(|(path, stamp)| {
                path.as_os_str().as_bytes() == self.path && stamp == self.source
            })
            && environment().is_ok_and(|environment| environment == self.environment)
            && !cancellation.is_cancelled()
    }
}

fn source(input: &Path) -> Result<(PathBuf, Stamp), String> {
    let path = input.canonicalize().map_err(|error| error.to_string())?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > 50 * 1024 * 1024 {
        return Err("Office source is not a bounded regular file".into());
    }
    Ok((path, Stamp::from(&metadata)))
}

pub(super) fn key(input: &Path) -> Result<Key, String> {
    let (path, stamp) = source(input)?;
    let key = Key {
        path: path.as_os_str().as_bytes().to_vec(),
        source: stamp,
        environment: environment()?,
    };
    if source(&path)? != (path, key.source.clone()) {
        return Err("Office source changed while capturing its cache identity".into());
    }
    Ok(key)
}

fn checksum(bytes: &[u8]) -> Result<String, String> {
    glib::compute_checksum_for_data(glib::ChecksumType::Sha256, bytes)
        .map(|value| value.to_string())
        .ok_or_else(|| "Office cache checksum is unavailable".into())
}

fn environment() -> Result<String, String> {
    let renderer = crate::sandbox_helper::office::render::renderer()
        .ok_or("Office renderer is unavailable")?;
    let mut roots = [
        "/etc/fonts",
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        "/usr/share/fontconfig",
        "/usr/lib/fontconfig",
        "/var/cache/fontconfig",
        "/etc/ld.so.cache",
        "/usr/bin/libreoffice",
        "/usr/bin/soffice",
    ]
    .map(PathBuf::from)
    .to_vec();
    for runtime in [
        "/usr/lib/libreoffice",
        "/usr/lib64/libreoffice",
        "/opt/libreoffice",
    ] {
        for name in [
            "program/soffice",
            "program/soffice.bin",
            "program/libmergedlo.so",
            "program/fundamentalrc",
            "program/bootstraprc",
            "program/versionrc",
            "share/registry",
            "share/fonts",
            "share/config",
            "share/filter",
        ] {
            roots.push(Path::new(runtime).join(name));
        }
    }
    environment_at(
        &renderer,
        Path::new("/proc/self/exe"),
        &roots.iter().map(PathBuf::as_path).collect::<Vec<_>>(),
    )
}

fn environment_at(renderer: &Path, helper: &Path, fonts: &[&Path]) -> Result<String, String> {
    let mut records = vec![CONFIG.to_vec()];
    let renderer = renderer.canonicalize().map_err(|error| error.to_string())?;
    let program = renderer.parent().ok_or("Invalid Office renderer path")?;
    let mut paths = vec![
        renderer.clone(),
        helper.to_path_buf(),
        program.to_path_buf(),
    ];
    for name in [
        "soffice.bin",
        "libmergedlo.so",
        "fundamentalrc",
        "bootstraprc",
        "versionrc",
        "../share/registry",
        "../share/fonts",
        "../share/config",
        "../share/filter",
    ] {
        paths.push(program.join(name));
    }
    for path in paths {
        record(&path, &mut records)?;
    }
    let mut visited = BTreeSet::new();
    let mut remaining = MAX_ENVIRONMENT_ENTRIES;
    for path in fonts.iter().copied().chain([
        program.join("../share/registry").as_path(),
        program.join("../share/fonts").as_path(),
        program.join("../share/config").as_path(),
        program.join("../share/filter").as_path(),
    ]) {
        walk_environment(path, 0, &mut remaining, &mut visited, &mut records)?;
    }
    checksum(&serde_json::to_vec(&records).map_err(|error| error.to_string())?)
}

fn record(path: &Path, records: &mut Vec<Vec<u8>>) -> Result<(), String> {
    let stamp = match fs::metadata(path) {
        Ok(metadata) => Some(Stamp::from(&metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    records.push(
        serde_json::to_vec(&(path.as_os_str().as_bytes(), stamp))
            .map_err(|error| error.to_string())?,
    );
    Ok(())
}

fn walk_environment(
    path: &Path,
    depth: usize,
    remaining: &mut usize,
    visited: &mut BTreeSet<PathBuf>,
    records: &mut Vec<Vec<u8>>,
) -> Result<(), String> {
    if depth > MAX_ENVIRONMENT_DEPTH || *remaining == 0 {
        return Err("Office font environment exceeds the cache fingerprint budget".into());
    }
    *remaining -= 1;
    record(path, records)?;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_dir() {
        return Ok(());
    }
    let canonical = path.canonicalize().map_err(|error| error.to_string())?;
    if !visited.insert(canonical) {
        return Ok(());
    }
    let mut children = Vec::new();
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        if children.len() >= *remaining {
            return Err("Office font environment exceeds the cache fingerprint budget".into());
        }
        children.push(entry.map_err(|error| error.to_string())?.path());
    }
    children.sort();
    for child in children {
        walk_environment(&child, depth + 1, remaining, visited, records)?;
    }
    Ok(())
}

struct Directory(File);

impl Directory {
    fn open(root: &Path, create: bool) -> Result<Self, String> {
        if !root.is_absolute() || root == Path::new("/") {
            return Err("Office cache requires an absolute private directory".into());
        }
        let mut directory = Self(open_directory(Path::new("/"))?);
        for component in root.components() {
            match component {
                Component::RootDir => continue,
                Component::Normal(name) => {
                    let next = directory.path().join(name);
                    if create {
                        match fs::DirBuilder::new().mode(0o700).create(&next) {
                            Ok(()) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                            Err(error) => return Err(error.to_string()),
                        }
                    }
                    directory = Self(open_directory(&next)?);
                }
                _ => return Err("Office cache path has an unsafe component".into()),
            }
        }
        let metadata = directory.0.metadata().map_err(|error| error.to_string())?;
        if metadata.uid() != user()? || metadata.mode() & 0o7777 != 0o700 {
            return Err("Office cache is not a private, owned directory".into());
        }
        directory.0.try_lock().map_err(|error| error.to_string())?;
        Ok(directory)
    }

    fn path(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.0.as_raw_fd()))
    }
}

fn open_directory(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW | DIRECTORY | NON_BLOCK)
        .open(path)
        .map_err(|error| error.to_string())
}

fn user() -> Result<u32, String> {
    gio::Credentials::new()
        .unix_user()
        .map_err(|error| error.to_string())
}

fn private_file(metadata: &fs::Metadata) -> bool {
    metadata.is_file()
        && metadata.uid() == user().unwrap_or(u32::MAX)
        && metadata.mode() & 0o7777 == 0o600
        && metadata.nlink() == 1
        && metadata.len() <= MAX_FILE as u64
}

fn root() -> PathBuf {
    glib::user_cache_dir().join("strata").join("office-pdf-v1")
}

fn filename(key: &Key) -> Result<String, String> {
    Ok(format!(
        "{}.pdfcache",
        checksum(&serde_json::to_vec(key).map_err(|error| error.to_string())?)?
    ))
}

fn owned_record(name: &str) -> bool {
    name.strip_suffix(".pdfcache").is_some_and(|stem| {
        stem.len() == 64
            && stem
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn owned_temporary(name: &str) -> bool {
    name.strip_prefix(".tmp-").is_some_and(|suffix| {
        suffix.split_once('-').is_some_and(|(process, sequence)| {
            process.parse::<u32>().is_ok() && sequence.parse::<u64>().is_ok()
        })
    })
}

fn valid_pdf(bytes: &[u8]) -> bool {
    bytes.len() >= 15
        && bytes.len() <= MAX_PDF
        && bytes.starts_with(b"%PDF-")
        && matches!((bytes[5], bytes[7]), (b'1', b'0'..=b'7') | (b'2', b'0'))
        && bytes[6] == b'.'
        && matches!(bytes[8], b'\r' | b'\n')
        && bytes.trim_ascii_end().ends_with(b"%%EOF")
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    key: Key,
    length: usize,
    checksum: String,
}

pub(super) fn load(key: &Key, cancellation: &Cancellation) -> Option<Vec<u8>> {
    load_at(&root(), key, cancellation, || key.is_current(cancellation))
}

fn load_at(
    root: &Path,
    key: &Key,
    cancellation: &Cancellation,
    current: impl Fn() -> bool,
) -> Option<Vec<u8>> {
    if cancellation.is_cancelled() || !current() {
        return None;
    }
    let directory = Directory::open(root, false).ok()?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(directory.path().join(filename(key).ok()?))
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !private_file(&metadata) {
        return None;
    }
    let mut prefix = vec![0; MAGIC.len() + 4];
    file.read_exact(&mut prefix).ok()?;
    if !prefix.starts_with(MAGIC) {
        return None;
    }
    let header_length = u32::from_le_bytes(prefix[MAGIC.len()..].try_into().ok()?) as usize;
    if header_length > MAX_HEADER {
        return None;
    }
    let mut header = vec![0; header_length];
    file.read_exact(&mut header).ok()?;
    let header: Header = serde_json::from_slice(&header).ok()?;
    if header.key != *key
        || header.length > MAX_PDF
        || metadata.len() != (prefix.len() + header_length + header.length) as u64
    {
        return None;
    }
    let mut bytes = Vec::with_capacity(header.length);
    (&mut file)
        .take(MAX_PDF as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let after = file.metadata().ok()?;
    if !private_file(&after)
        || Stamp::from(&after) != Stamp::from(&metadata)
        || !valid_pdf(&bytes)
        || bytes.len() != header.length
        || checksum(&bytes).ok()? != header.checksum
        || cancellation.is_cancelled()
        || !current()
    {
        return None;
    }
    file.set_modified(SystemTime::now()).ok()?;
    (!cancellation.is_cancelled()).then_some(bytes)
}

pub(super) fn store(key: &Key, bytes: &[u8], cancellation: &Cancellation) -> Result<(), String> {
    store_at(&root(), key, bytes, cancellation, || {
        key.is_current(cancellation)
    })
}

fn prune(directory: &Directory, replacing: &str, incoming: u64) -> Result<(), String> {
    let mut entries = Vec::new();
    for (count, entry) in fs::read_dir(directory.path())
        .map_err(|error| error.to_string())?
        .enumerate()
    {
        if count >= MAX_DIRECTORY_ENTRIES {
            return Err("Office cache directory exceeds its entry budget".into());
        }
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !owned_record(name) && !owned_temporary(name) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if !private_file(&metadata) {
            return Err("Office cache contains an unsafe record".into());
        }
        if owned_temporary(name) {
            fs::remove_file(entry.path()).map_err(|error| error.to_string())?;
        } else if name != replacing {
            entries.push((
                metadata.modified().map_err(|error| error.to_string())?,
                name.to_owned(),
                metadata.len(),
                entry.path(),
            ));
        }
    }
    entries.sort();
    let mut count = entries.len() + 1;
    let mut bytes = entries
        .iter()
        .fold(incoming, |total, entry| total + entry.2);
    for (_, _, size, path) in entries {
        if count <= MAX_ENTRIES && bytes <= MAX_BYTES {
            break;
        }
        fs::remove_file(path).map_err(|error| error.to_string())?;
        count -= 1;
        bytes -= size;
    }
    if count > MAX_ENTRIES || bytes > MAX_BYTES {
        return Err("Office cache exceeds its storage budget".into());
    }
    Ok(())
}

fn store_at(
    root: &Path,
    key: &Key,
    bytes: &[u8],
    cancellation: &Cancellation,
    current: impl Fn() -> bool,
) -> Result<(), String> {
    let check = || {
        if cancellation.is_cancelled() || !current() || cancellation.is_cancelled() {
            Err("Office cache source changed or preview was cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    if !valid_pdf(bytes) {
        return Err("Office cache requires a bounded complete PDF".into());
    }
    let header = serde_json::to_vec(&Header {
        key: key.clone(),
        length: bytes.len(),
        checksum: checksum(bytes)?,
    })
    .map_err(|error| error.to_string())?;
    if header.len() > MAX_HEADER {
        return Err("Office cache identity exceeds its header budget".into());
    }
    let directory = Directory::open(root, true)?;
    let name = filename(key)?;
    check()?;
    prune(
        &directory,
        &name,
        (MAGIC.len() + 4 + header.len() + bytes.len()) as u64,
    )?;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let temporary = directory.path().join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let destination = directory.path().join(name);
    let result = (|| {
        if !private_file(&output.metadata().map_err(|error| error.to_string())?) {
            return Err("Office cache staging file is not private".into());
        }
        output.write_all(MAGIC).map_err(|error| error.to_string())?;
        output
            .write_all(&(header.len() as u32).to_le_bytes())
            .map_err(|error| error.to_string())?;
        output
            .write_all(&header)
            .map_err(|error| error.to_string())?;
        output.write_all(bytes).map_err(|error| error.to_string())?;
        output.sync_all().map_err(|error| error.to_string())?;
        check()?;
        if !private_file(&output.metadata().map_err(|error| error.to_string())?) {
            return Err("Office cache staging file changed".into());
        }
        fs::rename(&temporary, &destination).map_err(|error| error.to_string())?;
        if let Err(error) = check() {
            let _removed = fs::remove_file(&destination);
            return Err(error);
        }
        directory.0.sync_all().map_err(|error| error.to_string())
    })();
    let _removed = fs::remove_file(temporary);
    result
}

#[cfg(test)]
mod tests;
