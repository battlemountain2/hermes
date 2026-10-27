use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::sandbox::{self, Cancellation, ParseOperation};

#[cfg(test)]
#[path = "waveform/tests.rs"]
mod tests;

const ANALYZER_VERSION: u32 = 1;
const MAX_ENTRY_BYTES: u64 = 32 * 1024;
const MAX_CACHE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CACHE_ENTRIES: usize = 256;
const MAX_DIRECTORY_ENTRIES: usize = 4096;
const NO_FOLLOW: i32 = 0o400000;
const NONBLOCK: i32 = 0o4000;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Identity {
    version: u32,
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl Identity {
    fn read(path: &Path) -> Result<Self, String> {
        let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("Waveform input is not a regular file".to_owned());
        }
        Ok(Self {
            version: ANALYZER_VERSION,
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }

    fn filename(&self) -> String {
        format!(
            "waveform-v{}-{:x}-{:x}-{:x}-{:x}-{:x}-{:x}-{:x}.json",
            self.version,
            self.device,
            self.inode,
            self.size,
            self.modified.0 as u64,
            self.modified.1 as u64,
            self.changed.0 as u64,
            self.changed.1 as u64
        )
    }
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    identity: Identity,
    peaks: Vec<f32>,
}

pub(crate) fn load(path: &Path, cancellation: &Cancellation) -> Result<Vec<f32>, String> {
    let directory = crate::paths::waveform_cache_dir();
    load_with(path, cancellation, &directory, |path, cancellation| {
        sandbox::parse(path, ParseOperation::MusicWaveform, 0, cancellation)
            .map(|output| output.data)
    })
}

fn load_with(
    path: &Path,
    cancellation: &Cancellation,
    directory: &Path,
    analyze: impl FnOnce(&Path, &Cancellation) -> Result<Vec<u8>, String>,
) -> Result<Vec<f32>, String> {
    check_cancelled(cancellation)?;
    let identity = Identity::read(path)?;
    let cache_ready = prepare_cache(directory).is_ok();
    if cache_ready && let Some(peaks) = read_cache(directory, &identity) {
        check_cancelled(cancellation)?;
        check_identity(path, &identity)?;
        return Ok(peaks);
    }
    let data = analyze(path, cancellation)?;
    check_cancelled(cancellation)?;
    check_identity(path, &identity)?;
    if data.len() as u64 > MAX_ENTRY_BYTES {
        return Err("Waveform renderer returned too much numeric data".to_owned());
    }
    let peaks: Vec<f32> = serde_json::from_slice(&data)
        .map_err(|error| format!("Invalid numeric waveform: {error}"))?;
    validate_peaks(&peaks)?;
    if cache_ready {
        let _cached = write_cache(directory, identity.clone(), &peaks);
    }
    check_cancelled(cancellation)?;
    check_identity(path, &identity)?;
    Ok(peaks)
}

fn check_cancelled(cancellation: &Cancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Waveform loading cancelled".to_owned())
    } else {
        Ok(())
    }
}

fn check_identity(path: &Path, identity: &Identity) -> Result<(), String> {
    if Identity::read(path).as_ref() == Ok(identity) {
        Ok(())
    } else {
        Err("Audio source changed during waveform loading".to_owned())
    }
}

fn validate_peaks(peaks: &[f32]) -> Result<(), String> {
    if matches!(peaks.len(), 512 | 1024)
        && peaks
            .iter()
            .all(|peak| peak.is_finite() && (0.0..=1.0).contains(peak))
    {
        Ok(())
    } else {
        Err("Waveform must contain 512 or 1024 finite peaks between zero and one".to_owned())
    }
}

fn prepare_cache(directory: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|error| error.to_string())?;
    let metadata = fs::symlink_metadata(directory).map_err(|error| error.to_string())?;
    let owner = fs::metadata("/proc/self")
        .map_err(|error| error.to_string())?
        .uid();
    if !metadata.is_dir() || metadata.uid() != owner || metadata.mode() & 0o077 != 0 {
        return Err("Waveform cache is not a private owned directory".to_owned());
    }
    Ok(())
}

fn read_cache(directory: &Path, identity: &Identity) -> Option<Vec<f32>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW | NONBLOCK)
        .open(directory.join(identity.filename()))
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file()
        || metadata.len() > MAX_ENTRY_BYTES
        || metadata.uid() != fs::metadata(directory).ok()?.uid()
    {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return None;
    }
    let entry: CacheEntry = serde_json::from_slice(&bytes).ok()?;
    if &entry.identity != identity || validate_peaks(&entry.peaks).is_err() {
        return None;
    }
    Some(entry.peaks)
}

struct OwnedFile(PathBuf);

impl Drop for OwnedFile {
    fn drop(&mut self) {
        let _removed = fs::remove_file(&self.0);
    }
}

fn create_owned_file(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(NO_FOLLOW)
        .open(path)
        .map_err(|error| error.to_string())
}

fn write_cache(directory: &Path, identity: Identity, peaks: &[f32]) -> Result<(), String> {
    prepare_cache(directory)?;
    let lock_path = directory.join(".waveform-cache.lock");
    let lock_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(NO_FOLLOW | NONBLOCK)
        .open(lock_path)
        .map_err(|error| error.to_string())?;
    let metadata = lock_file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.len() != 0
        || metadata.uid()
            != fs::metadata(directory)
                .map_err(|error| error.to_string())?
                .uid()
    {
        return Err("Waveform cache lock is not an owned regular file".to_owned());
    }
    lock_file.try_lock().map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(&CacheEntry {
        identity: identity.clone(),
        peaks: peaks.to_vec(),
    })
    .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err("Waveform cache entry exceeds its byte budget".to_owned());
    }
    cleanup(directory, bytes.len() as u64)?;
    let temporary = directory.join(format!(
        ".waveform-{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = create_owned_file(&temporary)?;
    let _temporary = OwnedFile(temporary.clone());
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    drop(file);
    fs::rename(temporary, directory.join(identity.filename())).map_err(|error| error.to_string())
}

fn owned_name(name: &str) -> bool {
    let Some(identity) = name
        .strip_prefix("waveform-v1-")
        .and_then(|name| name.strip_suffix(".json"))
    else {
        return false;
    };
    let fields: Vec<_> = identity.split('-').collect();
    fields.len() == 7
        && fields.iter().all(|field| {
            !field.is_empty()
                && field.len() <= 16
                && field.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

fn owned_temporary_name(name: &str) -> bool {
    let Some(identity) = name
        .strip_prefix(".waveform-")
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return false;
    };
    let fields: Vec<_> = identity.split('-').collect();
    fields.len() == 2
        && fields.iter().all(|field| {
            !field.is_empty()
                && field.len() <= 20
                && field.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn cleanup(directory: &Path, reserve_bytes: u64) -> Result<(), String> {
    let owner = fs::metadata(directory)
        .map_err(|error| error.to_string())?
        .uid();
    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;
    for (index, entry) in fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .enumerate()
    {
        if index >= MAX_DIRECTORY_ENTRIES {
            return Err("Waveform cache directory exceeds its scan budget".to_owned());
        }
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let temporary = name.to_str().is_some_and(owned_temporary_name);
        if !temporary && !name.to_str().is_some_and(owned_name) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.uid() != owner {
            continue;
        }
        if temporary {
            fs::remove_file(entry.path()).map_err(|error| error.to_string())?;
            continue;
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        entries.push((metadata.modified().ok(), entry.path(), metadata.len()));
    }
    entries.sort_unstable();
    let mut count = entries.len();
    for (_, path, size) in entries {
        if size <= MAX_ENTRY_BYTES
            && count < MAX_CACHE_ENTRIES
            && total_bytes <= MAX_CACHE_BYTES.saturating_sub(reserve_bytes)
        {
            continue;
        }
        fs::remove_file(path).map_err(|error| error.to_string())?;
        total_bytes = total_bytes.saturating_sub(size);
        count -= 1;
    }
    Ok(())
}
