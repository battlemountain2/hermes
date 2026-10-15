// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use gtk::{gdk, gio, glib, prelude::*};

mod disk;
mod scheduling;

use crate::{
    model::FileEntry,
    sandbox::{Cancellation, ParseOperation},
    services::{FormatCapabilities, ThumbnailHandler, capabilities_by_name},
};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
const MAX_CACHE_ENTRIES: usize = 256;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

thread_local! {
    static ACTIVE_REQUESTS: RefCell<HashMap<usize, ActiveRequest>> =
        RefCell::new(HashMap::new());
    static THUMBNAIL_CACHE: RefCell<ThumbnailCache> = RefCell::new(ThumbnailCache::default());
}

struct ActiveRequest {
    id: u64,
    image: glib::WeakRef<gtk::Image>,
    cancellation: Cancellation,
    wake_handlers: Vec<glib::SignalHandlerId>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ThumbnailKey {
    path: PathBuf,
    identity: SourceIdentity,
    source_path: PathBuf,
    source_identity: SourceIdentity,
    thumbnail_size: i32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
struct SourceIdentity {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl SourceIdentity {
    fn read(path: &Path) -> Option<Self> {
        let metadata = path.metadata().ok()?;
        Some(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ThumbnailSource {
    key: ThumbnailKey,
    operation: ParseOperation,
}

impl ThumbnailSource {
    fn unchanged(&self) -> bool {
        SourceIdentity::read(&self.key.path) == Some(self.key.identity)
            && (self.key.source_path == self.key.path
                || SourceIdentity::read(&self.key.source_path) == Some(self.key.source_identity))
    }
}

#[derive(Default)]
struct ThumbnailCache {
    entries: HashMap<ThumbnailKey, glib::Bytes>,
    recent: VecDeque<ThumbnailKey>,
    byte_count: usize,
}

impl ThumbnailCache {
    fn preview(&mut self, path: &Path) -> Option<(ThumbnailKey, glib::Bytes)> {
        let key = self
            .recent
            .iter()
            .rev()
            .find(|key| key.path == path && key.source_path == path)?
            .clone();
        self.get(&key).map(|bytes| (key, bytes))
    }

    fn candidate(&self, path: &Path, thumbnail_size: i32) -> Option<ThumbnailKey> {
        self.recent
            .iter()
            .rev()
            .find(|key| key.path == path && key.thumbnail_size == thumbnail_size)
            .cloned()
    }

    fn get(&mut self, key: &ThumbnailKey) -> Option<glib::Bytes> {
        let bytes = self.entries.get(key)?.clone();
        self.recent.retain(|candidate| candidate != key);
        self.recent.push_back(key.clone());
        Some(bytes)
    }

    fn insert(&mut self, key: ThumbnailKey, bytes: glib::Bytes) {
        if bytes.len() > MAX_CACHE_BYTES {
            return;
        }
        self.entries.retain(|candidate, _| {
            candidate.path != key.path || candidate.thumbnail_size != key.thumbnail_size
        });
        self.byte_count = self.entries.values().map(|bytes| bytes.len()).sum();
        self.recent
            .retain(|candidate| self.entries.contains_key(candidate));
        self.byte_count = self.byte_count.saturating_add(bytes.len());
        self.recent.push_back(key.clone());
        self.entries.insert(key, bytes);
        while self.entries.len() > MAX_CACHE_ENTRIES || self.byte_count > MAX_CACHE_BYTES {
            let Some(oldest) = self.recent.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.byte_count = self.byte_count.saturating_sub(removed.len());
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThumbnailKind {
    FolderAlbum,
    AudioAlbum,
    Sandboxed(ParseOperation),
}

pub(super) async fn cached_preview_bytes(path: &Path) -> Option<glib::Bytes> {
    if let Some((key, bytes)) = THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().preview(path)) {
        let unchanged =
            gio::spawn_blocking(move || SourceIdentity::read(&key.path) == Some(key.identity))
                .await
                .ok()?;
        if unchanged {
            return Some(bytes);
        }
    }
    let path = path.to_owned();
    gio::spawn_blocking(move || disk::preview(&path))
        .await
        .ok()?
        .map(glib::Bytes::from_owned)
}

pub(super) fn set_thumbnail_or_icon(
    image: &gtk::Image,
    entry: &FileEntry,
    fallback_icon: &str,
    icon_size: i32,
    thumbnail_size: i32,
) {
    let audio = !entry.is_directory() && crate::app::music_order::is_audio(entry);
    let fallback_icon = if audio {
        crate::assets::icons::MUSIC
    } else {
        fallback_icon
    };
    let (image_id, request, cancellation) = set_fallback_icon(image, fallback_icon, icon_size);

    let Some(path) = entry.location.native_path().map(Path::to_path_buf) else {
        return;
    };
    if entry.is_directory() {
        mark_nonempty_folder(image, image_id, request, path.clone());
    }
    if path.extension().and_then(std::ffi::OsStr::to_str) == Some("desktop") {
        let weak_image = image.downgrade();
        let worker_cancellation = cancellation.clone();
        glib::MainContext::default().spawn_local(async move {
            let Ok(Some(icon)) =
                gio::spawn_blocking(move || desktop_icon(&path, &worker_cancellation)).await
            else {
                return;
            };
            if request_is_current(image_id, request, &cancellation)
                && let Some(image) = weak_image.upgrade()
                && let Ok(icon) = gio::Icon::for_string(&icon)
            {
                crate::assets::remove_primary_icon(&image);
                image.set_from_gicon(&icon);
                image.set_pixel_size(thumbnail_size.clamp(16, 256));
            }
        });
        return;
    }
    let kind = if entry.is_directory() {
        ThumbnailKind::FolderAlbum
    } else if audio {
        ThumbnailKind::AudioAlbum
    } else if let FormatCapabilities {
        thumbnail: Some(handler),
        ..
    } = capabilities_by_name(path.as_os_str())
    {
        ThumbnailKind::Sandboxed(thumbnail_operation(handler))
    } else {
        return;
    };
    let thumbnail_size = thumbnail_size.clamp(16, 256);
    scheduling::enqueue(
        image,
        image_id,
        request,
        cancellation,
        path,
        kind,
        thumbnail_size,
    );
}

fn desktop_icon(path: &Path, cancellation: &Cancellation) -> Option<String> {
    if cancellation.is_cancelled() {
        return None;
    }
    let identity = SourceIdentity::read(path)?;
    let icon = gio::DesktopAppInfo::from_filename(path)?
        .icon()?
        .to_string()?
        .to_string();
    (!cancellation.is_cancelled() && SourceIdentity::read(path) == Some(identity)).then_some(icon)
}

fn request_is_current(image_id: usize, request: u64, cancellation: &Cancellation) -> bool {
    !cancellation.is_cancelled()
        && ACTIVE_REQUESTS.with(|requests| {
            requests
                .borrow()
                .get(&image_id)
                .is_some_and(|active| active.id == request && !active.cancellation.is_cancelled())
        })
}

fn complete_thumbnail(
    image_id: usize,
    request: u64,
    cancellation: &Cancellation,
    key: ThumbnailKey,
    bytes: glib::Bytes,
    apply: impl FnOnce(&glib::Bytes) -> bool,
) {
    if request_is_current(image_id, request, cancellation)
        && apply(&bytes)
        && request_is_current(image_id, request, cancellation)
    {
        THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert(key, bytes));
    }
}

fn thumbnail_operation(handler: ThumbnailHandler) -> ParseOperation {
    match handler {
        ThumbnailHandler::Image => ParseOperation::ThumbnailImage,
        ThumbnailHandler::Heif => ParseOperation::ThumbnailHeif,
        ThumbnailHandler::RawImage => ParseOperation::ThumbnailRaw,
        ThumbnailHandler::Pdf => ParseOperation::ThumbnailPdf,
        ThumbnailHandler::Media => ParseOperation::ThumbnailVideo,
        ThumbnailHandler::Model => ParseOperation::PreviewModel,
        ThumbnailHandler::Cover => ParseOperation::PreviewArchiveCover,
    }
}

fn mark_nonempty_folder(image: &gtk::Image, image_id: usize, request: u64, path: PathBuf) {
    let weak_image = glib::WeakRef::new();
    weak_image.set(Some(image));
    glib::MainContext::default().spawn_local(async move {
        let Ok(nonempty) = gio::spawn_blocking(move || {
            std::fs::read_dir(path)
                .ok()
                .and_then(|mut entries| entries.next())
                .is_some()
        })
        .await
        else {
            return;
        };
        if !nonempty {
            return;
        }
        let is_current = ACTIVE_REQUESTS.with(|requests| {
            requests
                .borrow()
                .get(&image_id)
                .is_some_and(|active| active.id == request)
        });
        if is_current
            && let Some(image) = weak_image.upgrade()
            && crate::assets::has_primary_icon(&image)
        {
            crate::assets::set_primary_icon(&image, crate::assets::icons::FOLDER_FILLED);
        }
    });
}

fn apply_thumbnail(image: &gtk::Image, bytes: &glib::Bytes, thumbnail_size: i32) -> bool {
    if let Ok(texture) = gdk::Texture::from_bytes(bytes) {
        crate::assets::remove_primary_icon(image);
        image.set_pixel_size(thumbnail_size);
        image.set_size_request(thumbnail_size, thumbnail_size);
        image.set_paintable(Some(&texture));
        image.set_opacity(1.0);
        return true;
    }
    false
}

pub(super) fn show_fallback_icon(image: &gtk::Image, icon: &str, size: i32) {
    set_fallback_icon(image, icon, size);
}

pub(super) fn cancel_thumbnail(image: &gtk::Image) {
    ACTIVE_REQUESTS.with(|requests| {
        if let Some(request) = requests.borrow_mut().remove(&(image.as_ptr() as usize)) {
            request.cancellation.cancel();
            for handler in request.wake_handlers {
                image.disconnect(handler);
            }
        }
    });
    scheduling::wake();
}

fn set_fallback_icon(image: &gtk::Image, icon: &str, size: i32) -> (usize, u64, Cancellation) {
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let image_id = image.as_ptr() as usize;
    let weak_image = glib::WeakRef::new();
    weak_image.set(Some(image));
    let cancellation = Cancellation::default();
    let wake_handlers = vec![
        image.connect_map(|_| scheduling::wake()),
        image.connect_unmap(|_| scheduling::wake()),
    ];
    ACTIVE_REQUESTS.with(|requests| {
        let mut requests = requests.borrow_mut();
        requests.retain(|_, active| {
            let alive = active.image.upgrade().is_some();
            if !alive {
                active.cancellation.cancel();
            }
            alive
        });
        if let Some(previous) = requests.insert(
            image_id,
            ActiveRequest {
                id: request,
                image: weak_image,
                cancellation: cancellation.clone(),
                wake_handlers,
            },
        ) {
            previous.cancellation.cancel();
            for handler in previous.wake_handlers {
                image.disconnect(handler);
            }
        }
    });
    image.set_pixel_size(size);
    image.set_size_request(size, size);
    crate::assets::set_primary_icon(image, icon);
    (image_id, request, cancellation)
}

fn thumbnail_source(
    path: &Path,
    kind: ThumbnailKind,
    size: i32,
    candidate: Option<&ThumbnailKey>,
    find_cover: impl FnOnce(&Path) -> Option<(PathBuf, ParseOperation)>,
) -> Result<ThumbnailSource, String> {
    let identity =
        SourceIdentity::read(path).ok_or_else(|| "Thumbnail source is unavailable".to_owned())?;
    let (source_path, operation) = match kind {
        ThumbnailKind::Sandboxed(operation) => (path.to_path_buf(), operation),
        ThumbnailKind::AudioAlbum => {
            if let Some(candidate) = candidate
                && candidate.path == path
                && candidate.identity == identity
                && candidate.source_path != path
                && let Some(source_identity) = SourceIdentity::read(&candidate.source_path)
            {
                let mut key = candidate.clone();
                key.source_identity = source_identity;
                return Ok(ThumbnailSource {
                    key,
                    operation: ParseOperation::ThumbnailImage,
                });
            }
            (path.to_path_buf(), ParseOperation::ThumbnailVideo)
        }
        ThumbnailKind::FolderAlbum => {
            if let Some(candidate) = candidate
                && candidate.path == path
                && candidate.identity == identity
                && let Some(source_identity) = SourceIdentity::read(&candidate.source_path)
            {
                let operation =
                    if crate::services::classify_by_name(candidate.source_path.as_os_str())
                        == crate::services::FormatFamily::Audio
                    {
                        ParseOperation::ThumbnailVideo
                    } else {
                        ParseOperation::ThumbnailImage
                    };
                let mut key = candidate.clone();
                key.source_identity = source_identity;
                key.thumbnail_size = size;
                return Ok(ThumbnailSource { key, operation });
            }
            find_cover(path).ok_or_else(|| "No folder artwork was found".to_owned())?
        }
    };
    let source_identity = if source_path == path {
        identity
    } else {
        SourceIdentity::read(&source_path)
            .ok_or_else(|| "Folder artwork is unavailable".to_owned())?
    };
    Ok(ThumbnailSource {
        key: ThumbnailKey {
            path: path.to_path_buf(),
            identity,
            source_path,
            source_identity,
            thumbnail_size: size,
        },
        operation,
    })
}

fn render_thumbnail(
    source: &ThumbnailSource,
    cancellation: &Cancellation,
    render: impl FnOnce(&Path, ParseOperation, i32, &Cancellation) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    if cancellation.is_cancelled() || !source.unchanged() {
        return Err("Thumbnail source changed or request was cancelled".to_owned());
    }
    let png = render(
        &source.key.source_path,
        source.operation,
        source.key.thumbnail_size,
        cancellation,
    )?;
    if cancellation.is_cancelled() || !source.unchanged() {
        return Err("Thumbnail source changed or request was cancelled".to_owned());
    }
    Ok(png)
}

fn render_thumbnail_with_cover(
    mut source: ThumbnailSource,
    kind: ThumbnailKind,
    cancellation: &Cancellation,
    mut render: impl FnMut(&Path, ParseOperation, i32, &Cancellation) -> Result<Vec<u8>, String>,
) -> Result<(ThumbnailSource, Vec<u8>), String> {
    match render_thumbnail(&source, cancellation, &mut render) {
        Ok(png) => return Ok((source, png)),
        Err(error) => {
            if kind != ThumbnailKind::AudioAlbum
                || cancellation.is_cancelled()
                || !source.unchanged()
            {
                return Err(error);
            }
        }
    }
    let parent = source.key.path.parent().ok_or("No album folder")?;
    let (cover, operation) = folder_album_art(parent)
        .filter(|(_, operation)| *operation == ParseOperation::ThumbnailImage)
        .ok_or("No album artwork was found")?;
    source.key.source_identity = SourceIdentity::read(&cover).ok_or("Album artwork unavailable")?;
    source.key.source_path = cover;
    source.operation = operation;
    let png = render_thumbnail(&source, cancellation, render)?;
    Ok((source, png))
}

pub(crate) fn folder_album_art(directory: &Path) -> Option<(std::path::PathBuf, ParseOperation)> {
    let mut embedded = None;
    let mut images = Vec::new();
    for entry in std::fs::read_dir(directory).ok()?.take(4096).flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let extension = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .map(str::to_ascii_lowercase);
        if crate::services::classify_by_name(path.as_os_str())
            == crate::services::FormatFamily::Audio
            && embedded
                .as_ref()
                .is_none_or(|(candidate, _)| &path < candidate)
        {
            embedded = Some((path.clone(), ParseOperation::ThumbnailVideo));
        }
        if matches!(extension.as_deref(), Some("jpg" | "jpeg" | "png" | "webp")) {
            let stem = path
                .file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let rank = match stem.as_str() {
                "cover" => Some(0),
                "folder" => Some(1),
                "albumart" | "album-art" => Some(2),
                "front" => Some(3),
                _ => None,
            };
            if let Some(rank) = rank {
                images.push((rank, path));
            }
        }
    }
    images.sort();
    images
        .into_iter()
        .next()
        .map(|(_, path)| (path, ParseOperation::ThumbnailImage))
        .or(embedded)
}

#[cfg(test)]
pub(super) mod tests;
