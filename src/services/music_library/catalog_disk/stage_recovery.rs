//! Reclaim only versioned, unlocked stages in a locked private catalog directory.

use super::*;

const MAX_DIRECTORY_ENTRIES: usize = 4096;
const FILES: [&str; 4] = [
    "snapshot.sqlite",
    "snapshot.sqlite-journal",
    "snapshot.sqlite-wal",
    "snapshot.sqlite-shm",
];

pub(super) fn recover(parent: &DirectoryHandle, cancellation: &Cancellation) -> Result<(), String> {
    cancelled(cancellation)?;
    for (count, entry) in fs::read_dir(parent.pinned()).map_err(error)?.enumerate() {
        cancelled(cancellation)?;
        if count >= MAX_DIRECTORY_ENTRIES {
            tracing::debug!(
                "Search catalog stage recovery deferred after directory inspection limit"
            );
            break;
        }
        let name = entry.map_err(error)?.file_name();
        if !leased_stage_name(&name) {
            continue;
        }
        let directory = match DirectoryHandle::at(parent, &name) {
            Ok(directory) => directory,
            Err(_) => continue,
        };
        if directory.private().is_err() {
            continue;
        }
        recover_stage(parent, &name, directory, cancellation)?;
    }
    cancelled(cancellation)
}

fn recover_stage(
    parent: &DirectoryHandle,
    name: &OsStr,
    directory: DirectoryHandle,
    cancellation: &Cancellation,
) -> Result<(), String> {
    match directory.0.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(failure) => return Err(error(failure)),
    }
    // A reader may have unlinked its stage before releasing this lease.
    // A pinned but already detached directory has nothing left to recover.
    let current = match DirectoryHandle::at(parent, name) {
        Ok(current) => current,
        Err(_) => return Ok(()),
    };
    if !same_directory(&directory, &current)? {
        return Ok(());
    }
    if let Some(files) = removable_files(&directory, cancellation)? {
        // Recheck the parent entry before unlinking through pinned handles.
        let current = DirectoryHandle::at(parent, name)?;
        if !same_directory(&directory, &current)? {
            return Err("Search catalog recovery stage changed".into());
        }
        for (name, identity) in files {
            cancelled(cancellation)?;
            let path = directory.pinned().join(name);
            let metadata = fs::symlink_metadata(&path).map_err(error)?;
            if FileIdentity::from_metadata(&metadata) != identity {
                return Err("Search catalog recovery file changed".into());
            }
            fs::remove_file(path).map_err(error)?;
        }
        directory.0.sync_all().map_err(error)?;
        if !same_directory(&directory, &DirectoryHandle::at(parent, name)?)? {
            return Err("Search catalog recovery stage moved".into());
        }
        fs::remove_dir(parent.pinned().join(name)).map_err(error)?;
        parent.0.sync_all().map_err(error)?;
    }
    Ok(())
}

fn leased_stage_name(name: &OsStr) -> bool {
    let Some(name) = name
        .to_str()
        .and_then(|name| name.strip_prefix(".snapshot-v2-"))
    else {
        return false;
    };
    let Some((pid, sequence)) = name.split_once('-') else {
        return false;
    };
    !pid.is_empty()
        && !sequence.is_empty()
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && sequence.bytes().all(|byte| byte.is_ascii_digit())
        && pid.parse::<u32>().is_ok_and(|pid| pid > 0)
        && sequence.parse::<u64>().is_ok()
}

fn same_directory(left: &DirectoryHandle, right: &DirectoryHandle) -> Result<bool, String> {
    let left = left.0.metadata().map_err(error)?;
    let right = right.0.metadata().map_err(error)?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

fn removable_files(
    directory: &DirectoryHandle,
    cancellation: &Cancellation,
) -> Result<Option<Vec<(OsString, FileIdentity)>>, String> {
    let mut files = Vec::new();
    for (count, entry) in fs::read_dir(directory.pinned()).map_err(error)?.enumerate() {
        cancelled(cancellation)?;
        if count >= FILES.len() {
            return Ok(None);
        }
        let name = entry.map_err(error)?.file_name();
        if !FILES.iter().any(|known| name == OsStr::new(known)) {
            return Ok(None);
        }
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | NON_BLOCK)
            .open(directory.pinned().join(&name))
        {
            Ok(file) => file,
            Err(_) => return Ok(None),
        };
        let metadata = file.metadata().map_err(error)?;
        if !metadata.is_file()
            || metadata.uid() != gio::Credentials::new().unix_user().map_err(error)?
            || metadata.mode() & 0o7777 != 0o600
            || metadata.nlink() != 1
        {
            return Ok(None);
        }
        files.push((name, FileIdentity::from_metadata(&metadata)));
    }
    Ok(Some(files))
}

#[cfg(test)]
mod tests;
