use super::{Entry, FileIdentity};
use crate::{
    model::{Location, MusicTags},
    services::FormatFamily,
    services::classify_by_name,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
pub(super) struct CachedIdentity {
    pub path: Location,
    #[serde(flatten)]
    pub file: FileIdentity,
}

#[derive(Deserialize, Serialize)]
pub(super) struct CachedTags {
    pub identity: CachedIdentity,
    pub tags: MusicTags,
}

pub(super) fn path(root: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut hasher);
    crate::paths::music_library_cache_dir()
        .join(format!("v1-{:016x}.json", hasher.finish()))
}

pub(super) fn valid_tags(tags: &MusicTags) -> bool {
    [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
        .into_iter()
        .flatten()
        .all(|value| value.chars().count() <= 256 && !value.chars().any(char::is_control))
}

pub(super) fn read(path: &Path, root: &Path) -> std::collections::HashMap<PathBuf, CachedTags> {
    let load = || -> Option<Vec<CachedTags>> {
        let file = fs::File::open(path).ok()?;
        if !file.metadata().ok()?.is_file() || file.metadata().ok()?.len() > MAX_CACHE_BYTES {
            return None;
        }
        let mut data = Vec::new();
        file.take(MAX_CACHE_BYTES + 1).read_to_end(&mut data).ok()?;
        if data.len() as u64 > MAX_CACHE_BYTES {
            return None;
        }
        let entries: Vec<CachedTags> = serde_json::from_slice(&data).ok()?;
        (entries.len() <= super::MAX_TRACKS).then_some(entries)
    };
    load()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let native = entry.identity.path.native_path()?;
            if !native.is_absolute()
                || !native.starts_with(root)
                || native.as_os_str().len() > 4096
                || native
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
                || classify_by_name(native.as_os_str()) != FormatFamily::Audio
                || !valid_tags(&entry.tags)
            {
                return None;
            }
            Some((native.to_owned(), entry))
        })
        .collect()
}

pub(super) fn write(path: &Path, entries: &[Entry]) -> Result<(), String> {
    static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
    let tags: Vec<_> = entries
        .iter()
        .filter_map(|entry| {
            Some(CachedTags {
                identity: CachedIdentity {
                    path: Location::local(&entry.path),
                    file: entry.identity.clone()?,
                },
                tags: entry.music_tags.clone()?,
            })
        })
        .collect();
    let data = serde_json::to_vec(&tags).map_err(|error| error.to_string())?;
    if data.len() as u64 > MAX_CACHE_BYTES {
        return Err("16 MiB metadata cache limit reached".into());
    }
    fs::create_dir_all(path.parent().ok_or("Invalid library cache location")?)
        .map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
    ));
    let save = || -> std::io::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        output.write_all(&data)?;
        output.sync_all()?;
        fs::rename(&temporary, path)
    };
    let result = save();
    if result.is_err() {
        let _removed = fs::remove_file(&temporary);
    }
    result.map_err(|error| error.to_string())
}
