//! Cooperative main-file admission across private catalog revisions.

use super::*;
use std::os::unix::fs::FileExt;

const TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const INSPECTION_ENTRIES: usize = 16_384;
const MAX_RECLAIMED_STAGES: usize = 16;
const LOCK: &str = ".staging-budget-lock";
pub(super) const RECORD: &str = ".reservation";
const MAGIC: &[u8; 8] = b"HRMSRSV1";

pub(super) struct Admission {
    root: DirectoryHandle,
    _lock: File,
    bytes: u64,
}

impl Admission {
    pub(super) fn acquire(
        parent: &DirectoryHandle,
        bytes: u64,
        cancellation: &Cancellation,
    ) -> Result<Self, String> {
        Self::with_limit(parent, bytes, TOTAL_BYTES, cancellation)
    }

    fn with_limit(
        parent: &DirectoryHandle,
        bytes: u64,
        limit: u64,
        cancellation: &Cancellation,
    ) -> Result<Self, String> {
        cancelled(cancellation)?;
        parent.private()?;
        if bytes > MAX_DATABASE_BYTES || bytes > limit {
            return Err("Search catalog snapshot reservation exceeds its main-file budget".into());
        }
        let root = cache_policy::snapshot_root(parent)?;
        root.private()?;
        let lock = cache_policy::acquire_control_lock(&root, OsStr::new(LOCK))?;
        cancelled(cancellation)?;
        let used = discover(&root, parent, cancellation)?;
        if used.checked_add(bytes).is_none_or(|total| total > limit) {
            return Err("Search catalog temporary snapshot budget is busy or exhausted".into());
        }
        Ok(Self {
            root,
            _lock: lock,
            bytes,
        })
    }

    pub(super) fn record(&self, stage: &Stage) -> Result<(), String> {
        let root = cache_policy::snapshot_root(&stage.parent)?;
        if stage.allowance != self.bytes || !stage_recovery::same_directory(&root, &self.root)? {
            return Err("Search catalog snapshot does not match its admitted reservation".into());
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(NO_FOLLOW | NON_BLOCK)
            .open(stage.directory.pinned().join(RECORD))
            .map_err(error)?;
        write_record(&file, stage.allowance)?;
        stage.directory.0.sync_all().map_err(error)
    }
}

pub(super) fn reduce(
    stage: &Stage,
    previous: u64,
    cancellation: &Cancellation,
) -> Result<(), String> {
    reduce_with_write(stage, previous, cancellation, write_record)
}

fn reduce_with_write(
    stage: &Stage,
    previous: u64,
    cancellation: &Cancellation,
    write: impl FnOnce(&File, u64) -> Result<(), String>,
) -> Result<(), String> {
    cancelled(cancellation)?;
    let root = cache_policy::snapshot_root(&stage.parent)?;
    root.private()?;
    let _lock = cache_policy::acquire_control_lock(&root, OsStr::new(LOCK))?;
    cancelled(cancellation)?;
    let file = record_file(&stage.directory)?;
    if read_record(&file)? != previous || stage.allowance > previous {
        return Err("Search catalog snapshot reservation changed before sealing".into());
    }
    write(&file, stage.allowance)?;
    stage.directory.0.sync_all().map_err(error)
}

fn record_file(directory: &DirectoryHandle) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(directory.pinned().join(RECORD))
        .map_err(error)?;
    let metadata = private_file(&file)?;
    if metadata.len() != 16 {
        return Err("Search catalog snapshot reservation has an invalid length".into());
    }
    Ok(file)
}

fn read_record(file: &File) -> Result<u64, String> {
    let mut record = [0u8; 16];
    file.read_exact_at(&mut record, 0).map_err(error)?;
    if &record[..8] != MAGIC {
        return Err("Search catalog snapshot reservation has an unknown version".into());
    }
    let bytes = u64::from_le_bytes(record[8..].try_into().expect("fixed reservation word"));
    if bytes > MAX_DATABASE_BYTES {
        return Err("Search catalog snapshot reservation exceeds the per-file limit".into());
    }
    Ok(bytes)
}

fn write_record(file: &File, bytes: u64) -> Result<(), String> {
    let mut record = [0u8; 16];
    record[..8].copy_from_slice(MAGIC);
    record[8..].copy_from_slice(&bytes.to_le_bytes());
    file.write_all_at(&record, 0).map_err(error)?;
    file.sync_all().map_err(error)
}

fn inspected(count: &mut usize, cancellation: &Cancellation) -> Result<(), String> {
    cancelled(cancellation)?;
    if *count >= INSPECTION_ENTRIES {
        return Err("Search catalog snapshot discovery exceeded its inspection budget".into());
    }
    *count += 1;
    Ok(())
}

fn discover(
    root: &DirectoryHandle,
    current: &DirectoryHandle,
    cancellation: &Cancellation,
) -> Result<u64, String> {
    let mut count = 0;
    let mut used = 0u64;
    let mut remaining_reclaims = MAX_RECLAIMED_STAGES;
    for entry in fs::read_dir(root.pinned()).map_err(error)? {
        inspected(&mut count, cancellation)?;
        let name = entry.map_err(error)?.file_name();
        if cache_policy::digest_name(&name, b"normalized-", b"") {
            let directory = DirectoryHandle::at(root, &name)?;
            directory.private()?;
            for entry in fs::read_dir(directory.pinned()).map_err(error)? {
                inspected(&mut count, cancellation)?;
                let name = entry.map_err(error)?.file_name();
                used = add_charge(
                    used,
                    &directory,
                    current,
                    &name,
                    &mut remaining_reclaims,
                    cancellation,
                )?;
            }
        } else {
            used = add_charge(
                used,
                root,
                current,
                &name,
                &mut remaining_reclaims,
                cancellation,
            )?;
        }
    }
    cancelled(cancellation)?;
    Ok(used)
}

fn add_charge(
    used: u64,
    parent: &DirectoryHandle,
    current: &DirectoryHandle,
    name: &OsStr,
    remaining_reclaims: &mut usize,
    cancellation: &Cancellation,
) -> Result<u64, String> {
    if !name.as_bytes().starts_with(b".snapshot-") {
        return Ok(used);
    }
    let directory = match DirectoryHandle::at(parent, name) {
        Ok(directory) => directory,
        Err(failure) => {
            if fs::symlink_metadata(parent.pinned().join(name))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                return Ok(used);
            }
            return Err(failure);
        }
    };
    directory.private()?;
    // Parent locks prevent creation/rebinding races. Peer acquisition is always
    // nonblocking while the namespace reservation lock is held.
    let can_recover = if stage_recovery::same_directory(parent, current)? {
        true
    } else {
        match parent.0.try_lock() {
            Ok(()) => true,
            Err(std::fs::TryLockError::WouldBlock) => false,
            Err(failure) => return Err(error(failure)),
        }
    };
    if can_recover
        && *remaining_reclaims > 0
        && stage_recovery::leased_stage_name(name)
        && stage_recovery::recover_stage(parent, name, directory, cancellation)?
    {
        *remaining_reclaims -= 1;
    }
    let directory = match DirectoryHandle::at(parent, name) {
        Ok(directory) => directory,
        Err(failure) => {
            if fs::symlink_metadata(parent.pinned().join(name))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                return Ok(used);
            }
            return Err(failure);
        }
    };
    directory.private()?;
    let bytes = reservation_charge(parent, name, &directory)?;
    used.checked_add(bytes)
        .ok_or_else(|| "Search catalog snapshot reservation total overflow".into())
}

fn reservation_charge(
    parent: &DirectoryHandle,
    name: &OsStr,
    directory: &DirectoryHandle,
) -> Result<u64, String> {
    let bytes = if name.as_bytes().starts_with(b".snapshot-v3-")
        && stage_recovery::leased_stage_name(name)
    {
        match record_file(directory) {
            Ok(file) => read_record(&file)?,
            Err(failure) => {
                if fs::symlink_metadata(directory.pinned().join(RECORD))
                    .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                {
                    // Drop removes payloads and marker before releasing the
                    // stage lease. Conservatively charge the old full allowance.
                    MAX_DATABASE_BYTES
                } else {
                    return Err(failure);
                }
            }
        }
    } else {
        MAX_DATABASE_BYTES
    };
    match fs::symlink_metadata(directory.pinned().join("snapshot.sqlite")) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= bytes => {}
        Ok(_) => return Err("Search catalog private snapshot exceeds its reservation".into()),
        Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {}
        Err(failure) => return Err(error(failure)),
    }
    match DirectoryHandle::at(parent, name) {
        Ok(current) if stage_recovery::same_directory(directory, &current)? => Ok(bytes),
        Ok(_) => Err("Search catalog snapshot directory changed during accounting".into()),
        Err(failure) => {
            if fs::symlink_metadata(parent.pinned().join(name))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            {
                Ok(0)
            } else {
                Err(failure)
            }
        }
    }
}

#[cfg(test)]
mod tests;
