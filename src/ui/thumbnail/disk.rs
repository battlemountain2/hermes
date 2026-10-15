use super::{SourceIdentity, ThumbnailKey};
use gtk::{gdk, gio, glib, prelude::*};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

const MAGIC: &[u8] = b"HERMES-RGBA-1\n";
const MAX_HEADER: usize = 64 * 1024;
const MAX_PIXELS: usize = 256 * 256 * 4;
const MAX_FILE: usize = MAGIC.len() + 4 + MAX_HEADER + MAX_PIXELS;
const MAX_FILES: usize = 512;
const MAX_BYTES: u64 = 128 * 1024 * 1024;
const NO_FOLLOW: i32 = 0x20000;
const DIRECTORY: i32 = 0x10000;
const NON_BLOCK: i32 = 0x800;

#[derive(Serialize, Deserialize, PartialEq)]
struct Key {
    path: Vec<u8>,
    identity: SourceIdentity,
    source_path: Vec<u8>,
    source_identity: SourceIdentity,
    size: i32,
}

impl From<&ThumbnailKey> for Key {
    fn from(key: &ThumbnailKey) -> Self {
        Self {
            path: key.path.as_os_str().as_bytes().to_vec(),
            identity: key.identity,
            source_path: key.source_path.as_os_str().as_bytes().to_vec(),
            source_identity: key.source_identity,
            size: key.thumbnail_size,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Header {
    key: Key,
    width: i32,
    height: i32,
    checksum: String,
}

struct Save {
    key: ThumbnailKey,
    pixels: glib::Bytes,
    width: i32,
    height: i32,
    stride: usize,
}

struct DirectoryHandle(File);

impl DirectoryHandle {
    fn open(path: &Path, create: bool) -> Option<Self> {
        if create {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)
                .ok()?;
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | DIRECTORY)
            .open(path)
            .ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_dir()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != gio::Credentials::new().unix_user().ok()?
        {
            return None;
        }
        Some(Self(file))
    }

    fn path(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.0.as_raw_fd()))
    }
}

pub(super) fn root() -> PathBuf {
    glib::user_cache_dir()
        .join("strata")
        .join("thumbnails-rgba-v1")
}

pub(super) fn filename(path: &Path) -> Option<String> {
    Some(format!(
        "{}.rgba",
        glib::compute_checksum_for_data(glib::ChecksumType::Sha256, path.as_os_str().as_bytes())?
    ))
}

fn unchanged(key: &Key) -> bool {
    let original = Path::new(std::ffi::OsStr::from_bytes(&key.path));
    let source = Path::new(std::ffi::OsStr::from_bytes(&key.source_path));
    SourceIdentity::read(original) == Some(key.identity)
        && SourceIdentity::read(source) == Some(key.source_identity)
}

fn dimensions(width: i32, height: i32) -> Option<usize> {
    if !(1..=256).contains(&width) || !(1..=256).contains(&height) {
        return None;
    }
    Some(width as usize * height as usize * 4)
}

fn read(root: &Path, path: &Path) -> Option<(Header, Vec<u8>)> {
    let directory = DirectoryHandle::open(root, false)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(directory.path().join(filename(path)?))
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file()
        || metadata.len() > MAX_FILE as u64
        || metadata.uid() != gio::Credentials::new().unix_user().ok()?
    {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_FILE || !bytes.starts_with(MAGIC) {
        return None;
    }
    let header_size =
        u32::from_le_bytes(bytes.get(MAGIC.len()..MAGIC.len() + 4)?.try_into().ok()?) as usize;
    if header_size > MAX_HEADER {
        return None;
    }
    let start = MAGIC.len() + 4;
    let end = start.checked_add(header_size)?;
    let header: Header = serde_json::from_slice(bytes.get(start..end)?).ok()?;
    let expected = dimensions(header.width, header.height)?;
    let pixels = bytes.get(end..)?;
    if pixels.len() != expected
        || header.checksum
            != glib::compute_checksum_for_data(glib::ChecksumType::Sha256, pixels)?.as_str()
        || header.key.path != path.as_os_str().as_bytes()
    {
        return None;
    }
    Some((header, pixels.to_vec()))
}

fn encode(header: Header, pixels: Vec<u8>) -> Option<Vec<u8>> {
    if !unchanged(&header.key) {
        return None;
    }
    let pixels = gtk::gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from_owned(pixels),
        gtk::gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        header.width,
        header.height,
        header.width * 4,
    );
    let png = pixels
        .save_to_bufferv("png", &[("compression", "1")])
        .ok()?;
    unchanged(&header.key).then_some(png)
}

pub(super) fn load(key: &ThumbnailKey) -> Option<Vec<u8>> {
    load_at(&root(), key)
}

fn load_at(root: &Path, key: &ThumbnailKey) -> Option<Vec<u8>> {
    let (header, pixels) = read(root, &key.path)?;
    let mut wanted = Key::from(key);
    if header.key.size < wanted.size {
        return None;
    }
    wanted.size = header.key.size;
    if header.key != wanted {
        return None;
    }
    encode(header, pixels)
}

pub(super) fn preview(path: &Path) -> Option<Vec<u8>> {
    let (header, pixels) = read(&root(), path)?;
    if header.key.path != header.key.source_path {
        return None;
    }
    encode(header, pixels)
}

pub(super) fn audio_candidate(path: &Path, size: i32) -> Option<ThumbnailKey> {
    audio_candidate_at(&root(), path, size)
}

fn audio_candidate_at(root: &Path, path: &Path, size: i32) -> Option<ThumbnailKey> {
    use std::os::unix::ffi::OsStringExt;
    let (header, _) = read(root, path)?;
    let expected = if header.key.path == header.key.source_path {
        path.to_owned()
    } else {
        super::folder_album_art(path.parent()?)?.0
    };
    if expected.as_os_str().as_bytes() != header.key.source_path
        || header.key.size < size
        || !unchanged(&header.key)
    {
        return None;
    }
    Some(ThumbnailKey {
        path: path.to_owned(),
        identity: header.key.identity,
        source_path: PathBuf::from(std::ffi::OsString::from_vec(header.key.source_path)),
        source_identity: header.key.source_identity,
        thumbnail_size: size,
    })
}

fn prune(directory: &DirectoryHandle, replacing: &str, incoming: u64) -> Option<()> {
    let mut files = Vec::new();
    for (index, entry) in fs::read_dir(directory.path()).ok()?.take(4097).enumerate() {
        if index == 4096 {
            return None;
        }
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name = name.to_str()?;
        if let Some(suffix) = name.strip_prefix(".tmp-") {
            let parts = suffix.split('-').collect::<Vec<_>>();
            if parts.len() == 2 && parts.iter().all(|part| part.parse::<u64>().is_ok()) {
                fs::remove_file(entry.path()).ok()?;
            }
            continue;
        }
        if name.len() != 69
            || !name.ends_with(".rgba")
            || !name.as_bytes()[..64]
                .iter()
                .all(|character| character.is_ascii_hexdigit())
            || name == replacing
        {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).ok()?;
        if metadata.is_file() {
            files.push((metadata.modified().ok()?, metadata.len(), entry.path()));
        }
        if files.len() > 4096 {
            return None;
        }
    }
    files.sort_by_key(|entry| entry.0);
    let mut bytes = files
        .iter()
        .fold(incoming, |total, entry| total.saturating_add(entry.1));
    let mut count = files.len() + 1;
    for (_, size, path) in files {
        if count <= MAX_FILES && bytes <= MAX_BYTES {
            break;
        }
        fs::remove_file(path).ok()?;
        count -= 1;
        bytes = bytes.saturating_sub(size);
    }
    (count <= MAX_FILES && bytes <= MAX_BYTES).then_some(())
}

fn save(root: &Path, item: Save) -> Option<()> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let expected = dimensions(item.width, item.height)?;
    let row = item.width as usize * 4;
    if item.stride < row
        || item.stride > 4096
        || item.pixels.len() < item.stride.checked_mul(item.height as usize)?
    {
        return None;
    }
    let mut pixels = Vec::with_capacity(expected);
    for input in item.pixels.chunks(item.stride).take(item.height as usize) {
        pixels.extend_from_slice(&input[..row]);
    }
    let header = Header {
        key: Key::from(&item.key),
        width: item.width,
        height: item.height,
        checksum: glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &pixels)?.to_string(),
    };
    if !unchanged(&header.key) {
        return None;
    }
    let metadata = serde_json::to_vec(&header).ok()?;
    if metadata.len() > MAX_HEADER {
        return None;
    }
    let directory = DirectoryHandle::open(root, true)?;
    directory.0.try_lock().ok()?;
    let name = filename(&item.key.path)?;
    let length = MAGIC.len() + 4 + metadata.len() + expected;
    prune(&directory, &name, length as u64)?;
    let temporary = directory.path().join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Option<()> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(NO_FOLLOW)
            .open(&temporary)
            .ok()?;
        output.write_all(MAGIC).ok()?;
        output
            .write_all(&(metadata.len() as u32).to_le_bytes())
            .ok()?;
        output.write_all(&metadata).ok()?;
        output.write_all(&pixels).ok()?;
        output.flush().ok()?;
        if !unchanged(&header.key) {
            return None;
        }
        fs::rename(&temporary, directory.path().join(name)).ok()?;
        Some(())
    })();
    let _removed = fs::remove_file(temporary);
    result
}

pub(super) fn persist(key: ThumbnailKey, texture: &gdk::Texture) {
    static WRITER: OnceLock<mpsc::SyncSender<Save>> = OnceLock::new();
    if dimensions(texture.width(), texture.height()).is_none() {
        return;
    }
    let writer = WRITER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel(8);
        let root = root();
        std::thread::spawn(move || {
            while let Ok(item) = receiver.recv() {
                let _saved = save(&root, item);
            }
        });
        sender
    });
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (pixels, stride) = downloader.download_bytes();
    let _queued = writer.try_send(Save {
        key,
        pixels,
        stride,
        width: texture.width(),
        height: texture.height(),
    });
}

#[cfg(test)]
mod tests;
