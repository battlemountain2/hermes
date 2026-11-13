//! Shared budget for published main databases. Private revisions and SQLite
//! workspaces have separate lifetimes and are deliberately outside this quota.

use super::{
    Cancellation, DirectoryHandle, NO_FOLLOW, NON_BLOCK, Stage, cancelled, error, lock_directory,
};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, Metadata, OpenOptions},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileExt, MetadataExt, OpenOptionsExt},
    },
    time::SystemTime,
};

const PUBLIC_BYTES: u64 = 512 * 1024 * 1024;
const DISCOVERY_ENTRIES: usize = 4096;
const LOCK_NAME: &str = ".publication-lock";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    General,
    Normalized,
}

struct Namespace {
    root: DirectoryHandle,
    current: Option<OsString>,
}

struct Candidate {
    directory: Option<OsString>,
    directory_identity: (u64, u64),
    name: OsString,
    kind: Kind,
    metadata: Metadata,
}

pub(super) fn publish(
    directory: &DirectoryHandle,
    name: &OsStr,
    stage: &Stage,
    cancellation: &Cancellation,
) -> Result<(), String> {
    publish_with_budget(directory, name, stage, cancellation, PUBLIC_BYTES)
}

fn publish_with_budget(
    directory: &DirectoryHandle,
    name: &OsStr,
    stage: &Stage,
    cancellation: &Cancellation,
    budget: u64,
) -> Result<(), String> {
    publish_with_budget_and_sync(directory, name, stage, cancellation, budget, File::sync_all)
}

fn publish_with_budget_and_sync(
    directory: &DirectoryHandle,
    name: &OsStr,
    stage: &Stage,
    cancellation: &Cancellation,
    budget: u64,
    sync_directory: impl FnOnce(&File) -> std::io::Result<()>,
) -> Result<(), String> {
    cancelled(cancellation)?;
    if name.as_bytes().is_empty()
        || name.as_bytes().contains(&b'/')
        || name.as_bytes().contains(&0)
        || name == "."
        || name == ".."
    {
        return Err("Search catalog: invalid publication filename".into());
    }
    directory.private()?;
    stage.directory.private()?;
    if inode(&directory.0.metadata().map_err(error)?)
        != inode(&stage.parent.0.metadata().map_err(error)?)
    {
        return Err("Search catalog: publication stage belongs to another directory".into());
    }
    let incoming = open_file(&stage.file())?;
    let incoming_metadata = incoming.metadata().map_err(error)?;
    if !safe_file(&incoming_metadata)? {
        return Err(
            "Search catalog: publication requires a private regular single-link file".into(),
        );
    }
    let namespace = namespace(directory, name)?;
    // Arbitrary fixture filenames retain checked ordinary rename semantics.
    let Some(namespace) = namespace else {
        let target = target_metadata(directory, name)?;
        return commit(
            directory,
            name,
            stage,
            &incoming,
            target.as_ref(),
            cancellation,
            sync_directory,
        );
    };
    if incoming_metadata.len() > budget {
        return Err("Search catalog: incoming database exceeds the public cache budget".into());
    }
    namespace.root.private()?;
    let _publication_lock = acquire_control_lock(&namespace.root, OsStr::new(LOCK_NAME))?;
    cancelled(cancellation)?;
    let target = target_metadata(directory, name)?;
    let (mut bytes, mut candidates) = discover(&namespace, name, cancellation)?;
    bytes = bytes
        .checked_sub(target.as_ref().map_or(0, Metadata::len))
        .and_then(|bytes| bytes.checked_add(incoming_metadata.len()))
        .ok_or("Search catalog: public cache size overflow or changed target")?;
    candidates.sort_by(|left, right| {
        (left.metadata.mtime(), left.metadata.mtime_nsec())
            .cmp(&(right.metadata.mtime(), right.metadata.mtime_nsec()))
            .then_with(|| left.directory.cmp(&right.directory))
            .then_with(|| left.name.cmp(&right.name))
    });
    for candidate in candidates {
        cancelled(cancellation)?;
        if bytes <= budget {
            break;
        }
        if evict(&namespace, directory, &candidate, cancellation)? {
            bytes -= candidate.metadata.len();
        }
    }
    if bytes > budget {
        return Err("Search catalog: public cache budget cannot be satisfied because remaining databases are busy or protected".into());
    }
    commit(
        directory,
        name,
        stage,
        &incoming,
        target.as_ref(),
        cancellation,
        sync_directory,
    )
}

pub(super) fn digest_name(name: &OsStr, prefix: &[u8], suffix: &[u8]) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == prefix.len() + 64 + suffix.len()
        && bytes.starts_with(prefix)
        && bytes.ends_with(suffix)
        && bytes[prefix.len()..prefix.len() + 64]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn namespace(directory: &DirectoryHandle, name: &OsStr) -> Result<Option<Namespace>, String> {
    if digest_name(name, b"general-", b".sqlite") {
        return Ok(Some(Namespace {
            root: DirectoryHandle(directory.0.try_clone().map_err(error)?),
            current: None,
        }));
    }
    if name != "source.sqlite" {
        return Ok(None);
    }
    let path = fs::read_link(directory.pinned()).map_err(error)?;
    let Some(component) = path.file_name() else {
        return Ok(None);
    };
    if !digest_name(component, b"normalized-", b"") {
        return Ok(None);
    }
    let root = DirectoryHandle::at(directory, OsStr::new(".."))?;
    root.private()?;
    let linked = DirectoryHandle::at(&root, component)?;
    if inode(&linked.0.metadata().map_err(error)?) != inode(&directory.0.metadata().map_err(error)?)
    {
        return Err("Search catalog: normalized publication directory moved".into());
    }
    Ok(Some(Namespace {
        root,
        current: Some(component.to_owned()),
    }))
}

pub(super) fn snapshot_root(directory: &DirectoryHandle) -> Result<DirectoryHandle, String> {
    match namespace(directory, OsStr::new("source.sqlite"))? {
        Some(namespace) => Ok(namespace.root),
        None => Ok(DirectoryHandle(directory.0.try_clone().map_err(error)?)),
    }
}

fn inode(metadata: &Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

fn unchanged(left: &Metadata, right: &Metadata) -> bool {
    inode(left) == inode(right)
        && left.len() == right.len()
        && left.mode() == right.mode()
        && left.uid() == right.uid()
        && left.nlink() == right.nlink()
        && (left.mtime(), left.mtime_nsec()) == (right.mtime(), right.mtime_nsec())
        && (left.ctime(), left.ctime_nsec()) == (right.ctime(), right.ctime_nsec())
}

fn safe_file(metadata: &Metadata) -> Result<bool, String> {
    Ok(metadata.is_file()
        && metadata.uid() == gio::Credentials::new().unix_user().map_err(error)?
        && metadata.mode() & 0o7777 == 0o600
        && metadata.nlink() == 1)
}

fn open_file(path: &std::path::Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(path)
        .map_err(error)
}

#[cfg(test)]
fn publication_lock(root: &DirectoryHandle) -> Result<File, String> {
    control_lock(root, OsStr::new(LOCK_NAME))
}

pub(super) fn acquire_control_lock(root: &DirectoryHandle, name: &OsStr) -> Result<File, String> {
    let file = control_lock(root, name)?;
    lock_directory(&file)?;
    revalidate_file(root, name, &file.metadata().map_err(error)?)?;
    Ok(file)
}

pub(super) fn control_lock(root: &DirectoryHandle, name: &OsStr) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(root.pinned().join(name))
        .map_err(error)?;
    let metadata = file.metadata().map_err(error)?;
    if !safe_file(&metadata)? || metadata.len() != 0 {
        return Err(
            "Search catalog: publication lock must be an owned regular 0600 single-link empty file"
                .into(),
        );
    }
    revalidate_file(root, name, &metadata)?;
    Ok(file)
}

fn target_metadata(directory: &DirectoryHandle, name: &OsStr) -> Result<Option<Metadata>, String> {
    match fs::symlink_metadata(directory.pinned().join(name)) {
        Ok(metadata) if safe_file(&metadata)? => Ok(Some(metadata)),
        Ok(_) => Err("Search catalog: unsafe publication target".into()),
        Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(failure) => Err(error(failure)),
    }
}

fn discover(
    namespace: &Namespace,
    target: &OsStr,
    cancellation: &Cancellation,
) -> Result<(u64, Vec<Candidate>), String> {
    let mut bytes = 0u64;
    let mut candidates = Vec::new();
    for (index, entry) in fs::read_dir(namespace.root.pinned())
        .map_err(error)?
        .enumerate()
    {
        cancelled(cancellation)?;
        if index >= DISCOVERY_ENTRIES {
            return Err("Search catalog: public cache discovery exceeds 4096 entries".into());
        }
        let name = entry.map_err(error)?.file_name();
        let (directory, component, file_name, kind) = if digest_name(&name, b"general-", b".sqlite")
        {
            (
                DirectoryHandle(namespace.root.0.try_clone().map_err(error)?),
                None,
                name,
                Kind::General,
            )
        } else if digest_name(&name, b"normalized-", b"") {
            (
                DirectoryHandle::at(&namespace.root, &name)?,
                Some(name),
                OsString::from("source.sqlite"),
                Kind::Normalized,
            )
        } else {
            continue;
        };
        directory.private()?;
        let metadata = match fs::symlink_metadata(directory.pinned().join(&file_name)) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => {
                return Err("Search catalog: managed public database is not a regular file".into());
            }
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => continue,
            Err(failure) => return Err(error(failure)),
        };
        bytes = bytes
            .checked_add(metadata.len())
            .ok_or("Search catalog: public cache size overflow")?;
        if component == namespace.current && file_name == target {
            continue;
        }
        candidates.push(Candidate {
            directory: component,
            directory_identity: inode(&directory.0.metadata().map_err(error)?),
            name: file_name,
            kind,
            metadata,
        });
    }
    cancelled(cancellation)?;
    Ok((bytes, candidates))
}

fn supported_header(file: &File, metadata: &Metadata, kind: Kind) -> Result<bool, String> {
    let mut header = [0; 100];
    match file.read_exact_at(&mut header, 0) {
        Ok(()) => {}
        Err(failure) if failure.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
        Err(failure) => return Err(error(failure)),
    }
    let page_size = match u16::from_be_bytes([header[16], header[17]]) {
        1 => 65536u64,
        size => u64::from(size),
    };
    let word =
        |offset| u32::from_be_bytes(header[offset..offset + 4].try_into().expect("header word"));
    let schema = match kind {
        Kind::General => word(60) == 1 && word(68) == 0,
        Kind::Normalized => word(60) == 2 && word(68) == 0x48515331,
    };
    Ok(header.starts_with(b"SQLite format 3\0")
        && (512..=65536).contains(&page_size)
        && page_size.is_power_of_two()
        && metadata.len() >= page_size
        && metadata.len().is_multiple_of(page_size)
        && u64::from(word(28)) == metadata.len() / page_size
        && word(24) == word(92)
        && matches!(header[18], 1 | 2)
        && matches!(header[19], 1 | 2)
        && header[21..24] == [64, 32, 32]
        && (1..=4).contains(&word(44))
        && (1..=3).contains(&word(56))
        && header[72..92].iter().all(|byte| *byte == 0)
        && schema)
}

fn revalidate_file(
    directory: &DirectoryHandle,
    name: &OsStr,
    expected: &Metadata,
) -> Result<(), String> {
    let actual = fs::symlink_metadata(directory.pinned().join(name)).map_err(error)?;
    if !unchanged(expected, &actual) {
        return Err("Search catalog: public cache file identity changed".into());
    }
    Ok(())
}

fn evict(
    namespace: &Namespace,
    current: &DirectoryHandle,
    candidate: &Candidate,
    cancellation: &Cancellation,
) -> Result<bool, String> {
    let directory = match &candidate.directory {
        Some(name) => DirectoryHandle::at(&namespace.root, name)?,
        None => DirectoryHandle(namespace.root.0.try_clone().map_err(error)?),
    };
    directory.private()?;
    let identity = inode(&directory.0.metadata().map_err(error)?);
    if identity != candidate.directory_identity {
        return Err("Search catalog: eviction directory identity changed".into());
    }
    // The caller already holds its directory lock. Never acquire that inode
    // again through a separate open description, and never wait on any peer.
    if identity != inode(&current.0.metadata().map_err(error)?) {
        match directory.0.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
            Err(failure) => return Err(error(failure)),
        }
    }
    if !safe_file(&candidate.metadata)? {
        return Ok(false);
    }
    let file = open_file(&directory.pinned().join(&candidate.name))?;
    let metadata = file.metadata().map_err(error)?;
    if !unchanged(&candidate.metadata, &metadata) {
        return Err("Search catalog: eviction candidate changed".into());
    }
    if !supported_header(&file, &metadata, candidate.kind)? {
        return Ok(false);
    }
    cancelled(cancellation)?;
    if let Some(name) = &candidate.directory {
        let linked = fs::symlink_metadata(namespace.root.pinned().join(name)).map_err(error)?;
        if !linked.is_dir() || inode(&linked) != identity {
            return Err("Search catalog: eviction directory moved".into());
        }
    }
    revalidate_file(&directory, &candidate.name, &metadata)?;
    fs::remove_file(directory.pinned().join(&candidate.name)).map_err(error)?;
    directory.0.sync_all().map_err(error)?;
    Ok(true)
}

fn commit(
    directory: &DirectoryHandle,
    name: &OsStr,
    stage: &Stage,
    incoming: &File,
    target: Option<&Metadata>,
    cancellation: &Cancellation,
    sync_directory: impl FnOnce(&File) -> std::io::Result<()>,
) -> Result<(), String> {
    cancelled(cancellation)?;
    // Record publication order, including stages that waited a long time after
    // their last write. Reads never update this timestamp.
    incoming.set_modified(SystemTime::now()).map_err(error)?;
    incoming.sync_all().map_err(error)?;
    let metadata = incoming.metadata().map_err(error)?;
    revalidate_file(&stage.directory, OsStr::new("snapshot.sqlite"), &metadata)?;
    let actual = target_metadata(directory, name)?;
    if !match (target, actual.as_ref()) {
        (Some(expected), Some(actual)) => unchanged(expected, actual),
        (None, None) => true,
        _ => false,
    } {
        return Err("Search catalog: publication target changed".into());
    }
    cancelled(cancellation)?;
    fs::rename(stage.file(), directory.pinned().join(name)).map_err(error)?;
    sync_directory(&directory.0).map_err(error)
}

#[cfg(test)]
mod tests;
