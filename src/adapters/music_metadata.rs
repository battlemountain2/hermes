use std::{
    collections::VecDeque,
    fs,
    os::unix::fs::MetadataExt,
    path::PathBuf,
    rc::Rc,
    sync::{Mutex, OnceLock},
};

use gtk::gio;

use crate::{
    app::music_order::is_audio,
    model::{FileEntry, MusicTags},
    sandbox::{self, Cancellation, ParseOperation},
};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Identity {
    path: PathBuf,
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl Identity {
    fn read(path: &std::path::Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        metadata.is_file().then(|| Self {
            path: path.to_path_buf(),
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

#[derive(Default)]
struct Cache(VecDeque<(Identity, MusicTags)>);

impl Cache {
    fn get(&mut self, identity: &Identity) -> Option<MusicTags> {
        let position = self.0.iter().position(|(current, _)| current == identity)?;
        let entry = self.0.remove(position)?;
        let tags = entry.1.clone();
        self.0.push_back(entry);
        Some(tags)
    }

    fn put(&mut self, identity: Identity, tags: MusicTags) {
        self.0.retain(|(current, _)| current.path != identity.path);
        self.0.push_back((identity, tags));
        while self.0.len() > 512 {
            self.0.pop_front();
        }
    }
}

pub(crate) fn read_tags(path: &std::path::Path, cancellation: &Cancellation) -> Option<MusicTags> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Cache::default()));
    let identity = Identity::read(path)?;
    if let Some(tags) = crate::services::music_library::cached_tags(path) {
        return Some(tags);
    }
    if let Some(tags) = cache.lock().ok()?.get(&identity) {
        return Some(tags);
    }
    let output = sandbox::parse(path, ParseOperation::MusicTags, 0, cancellation).ok()?;
    if Identity::read(path).as_ref() != Some(&identity) {
        return None;
    }
    let tags: MusicTags = serde_json::from_slice(&output.data).ok()?;
    cache.lock().ok()?.put(identity, tags.clone());
    Some(tags)
}

pub(crate) fn read_audio_artwork(
    path: &std::path::Path,
    cancellation: &Cancellation,
) -> Option<Vec<u8>> {
    if cancellation.is_cancelled() {
        return None;
    }
    let identity = Identity::read(path)?;
    if let Some(artwork) = unchanged_artwork(path, ParseOperation::ThumbnailVideo, cancellation) {
        return Some(artwork);
    }
    let mut covers = std::fs::read_dir(path.parent()?)
        .ok()?
        .take(4096)
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp") {
                return None;
            }
            let rank = match path.file_stem()?.to_str()?.to_ascii_lowercase().as_str() {
                "cover" => 0,
                "folder" => 1,
                "albumart" | "album-art" => 2,
                "front" => 3,
                _ => return None,
            };
            Some((rank, path))
        })
        .collect::<Vec<_>>();
    covers.sort();
    for (_, cover) in covers.into_iter().take(16) {
        if cancellation.is_cancelled() || Identity::read(path).as_ref() != Some(&identity) {
            return None;
        }
        if let Some(artwork) =
            unchanged_artwork(&cover, ParseOperation::ThumbnailImage, cancellation)
        {
            if Identity::read(path).as_ref() == Some(&identity) && !cancellation.is_cancelled() {
                return Some(artwork);
            }
            return None;
        }
    }
    None
}

pub(crate) fn read_audio_metadata(
    path: &std::path::Path,
    cancellation: &Cancellation,
) -> Result<sandbox::ParseOutput, String> {
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".into());
    }
    let identity = Identity::read(path).ok_or("Audio input is not a regular file")?;
    let output = sandbox::parse(path, ParseOperation::PreviewAudioMetadata, 0, cancellation)?;
    if cancellation.is_cancelled() || Identity::read(path).as_ref() != Some(&identity) {
        return Err("Audio changed or metadata request was cancelled".into());
    }
    Ok(output)
}

fn unchanged_artwork(
    path: &std::path::Path,
    operation: ParseOperation,
    cancellation: &Cancellation,
) -> Option<Vec<u8>> {
    if cancellation.is_cancelled() {
        return None;
    }
    let identity = Identity::read(path)?;
    let output = sandbox::parse(path, operation, 256, cancellation).ok()?;
    (Identity::read(path).as_ref() == Some(&identity) && !cancellation.is_cancelled())
        .then_some(output.data)
}

pub(super) async fn enrich(
    mut entries: Vec<FileEntry>,
    cancellation: Cancellation,
) -> Vec<FileEntry> {
    let paths: Vec<_> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| is_audio(entry))
        .filter_map(|(index, entry)| Some((index, entry.location.native_path()?.to_path_buf())))
        .collect();
    if paths.is_empty() {
        return entries;
    }
    let result = gio::spawn_blocking(move || {
        paths
            .into_iter()
            .take_while(|_| !cancellation.is_cancelled())
            .map(|(index, path)| (index, read_tags(&path, &cancellation)))
            .collect::<Vec<_>>()
    })
    .await;
    if let Ok(tags) = result {
        for (index, tags) in tags {
            Rc::make_mut(&mut entries[index].0).music_tags = tags;
        }
    }
    entries
}
