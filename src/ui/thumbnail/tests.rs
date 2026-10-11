// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs::{self, File, FileTimes},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, UNIX_EPOCH},
};

use gtk::glib;

use crate::sandbox::{Cancellation, ParseOperation};
use crate::services::thumbnail_handler_for_name;

use super::{
    ACTIVE_REQUESTS, ActiveRequest, MAX_CACHE_BYTES, MAX_CACHE_ENTRIES, SourceIdentity,
    THUMBNAIL_CACHE, ThumbnailCache, ThumbnailKey, ThumbnailKind, ThumbnailSource,
    complete_thumbnail, desktop_icon, folder_album_art, render_thumbnail, thumbnail_operation,
    thumbnail_source,
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "hermes-ui-thumbnail-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create generated fixture");
        Self(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).expect("write generated source");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove generated fixture");
    }
}

fn identity() -> SourceIdentity {
    SourceIdentity {
        device: 1,
        inode: 1,
        size: 1,
        modified: (1, 1),
        changed: (1, 1),
    }
}

fn key(path: impl Into<PathBuf>) -> ThumbnailKey {
    let path = path.into();
    ThumbnailKey {
        source_path: path.clone(),
        path,
        identity: identity(),
        source_identity: identity(),
        thumbnail_size: 64,
    }
}

fn source(path: &Path, kind: ThumbnailKind, candidate: Option<&ThumbnailKey>) -> ThumbnailSource {
    thumbnail_source(path, kind, 64, candidate, folder_album_art).expect("resolve generated source")
}

fn set_modified(path: &Path, nanoseconds: u32) {
    File::open(path)
        .expect("open source")
        .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::new(1000, nanoseconds)))
        .expect("set precise generated mtime");
}

#[test]
fn recognizes_mainstream_image_and_video_formats() {
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("photo.JPEG")).map(thumbnail_operation),
        Some(ParseOperation::ThumbnailImage)
    );
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("animation.webp")).map(thumbnail_operation),
        Some(ParseOperation::ThumbnailImage)
    );
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("iphone-photo.HEIC"))
            .map(thumbnail_operation),
        Some(ParseOperation::ThumbnailHeif)
    );
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("camera-photo.heif"))
            .map(thumbnail_operation),
        Some(ParseOperation::ThumbnailHeif)
    );
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("capture.CR3")).map(thumbnail_operation),
        Some(ParseOperation::ThumbnailRaw)
    );
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("photo.nef")).map(thumbnail_operation),
        Some(ParseOperation::ThumbnailRaw)
    );
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("document.PDF")).map(thumbnail_operation),
        Some(ParseOperation::ThumbnailPdf)
    );
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("clip.mkv")).map(thumbnail_operation),
        Some(ParseOperation::ThumbnailVideo)
    );
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("clip.ogv")).map(thumbnail_operation),
        Some(ParseOperation::ThumbnailVideo)
    );
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("album-track.FLAC"))
            .map(thumbnail_operation),
        Some(ParseOperation::ThumbnailVideo)
    );
}

#[test]
fn thumbnail_cache_evicts_the_least_recent_entry() {
    let mut cache = ThumbnailCache::default();
    for index in 0..=MAX_CACHE_ENTRIES {
        cache.insert(
            key(format!("image-{index}.png")),
            glib::Bytes::from_static(&[1]),
        );
    }

    let oldest = key("image-0.png");
    assert!(cache.get(&oldest).is_none());
    assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
}

#[test]
fn same_size_same_second_edits_and_replacements_miss_the_ui_cache() {
    let fixture = Fixture::new();
    let path = fixture.write("source.png", b"first");
    set_modified(&path, 100);
    let before = source(
        &path,
        ThumbnailKind::Sandboxed(ParseOperation::ThumbnailImage),
        None,
    );
    let mut cache = ThumbnailCache::default();
    cache.insert(before.key.clone(), glib::Bytes::from_static(b"old"));
    fs::write(&path, b"other").expect("edit same-size source");
    set_modified(&path, 200);
    let edited = source(
        &path,
        ThumbnailKind::Sandboxed(ParseOperation::ThumbnailImage),
        None,
    );
    assert_eq!(before.key.identity.size, edited.key.identity.size);
    assert_eq!(
        before.key.identity.modified.0,
        edited.key.identity.modified.0
    );
    assert_ne!(before.key.identity, edited.key.identity);
    assert!(cache.get(&edited.key).is_none());
    cache.insert(edited.key.clone(), glib::Bytes::from_static(b"new"));
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(cache.byte_count, 3);
    let replacement = fixture.write("replacement.png", b"third");
    set_modified(&replacement, 200);
    fs::rename(replacement, &path).expect("replace source atomically");
    let replaced = source(
        &path,
        ThumbnailKind::Sandboxed(ParseOperation::ThumbnailImage),
        None,
    );
    assert_eq!(edited.key.identity.modified, replaced.key.identity.modified);
    assert_eq!(edited.key.identity.size, replaced.key.identity.size);
    assert_ne!(edited.key.identity.inode, replaced.key.identity.inode);
    assert!(cache.get(&replaced.key).is_none());
}

#[test]
fn ctime_and_device_changes_are_part_of_cache_identity() {
    let original = key("image.png");
    let mut cache = ThumbnailCache::default();
    cache.insert(original.clone(), glib::Bytes::from_static(b"cached"));
    for changed in [
        SourceIdentity {
            changed: (1, 2),
            ..identity()
        },
        SourceIdentity {
            device: 2,
            ..identity()
        },
    ] {
        let mut updated = original.clone();
        updated.identity = changed;
        updated.source_identity = changed;
        assert!(cache.get(&updated).is_none());
    }
}

#[test]
fn selected_cover_edits_invalidate_without_rescanning_an_unchanged_folder() {
    let fixture = Fixture::new();
    let cover = fixture.write("cover.png", b"first");
    set_modified(&cover, 100);
    let before = source(&fixture.0, ThumbnailKind::FolderAlbum, None);
    let mut cache = ThumbnailCache::default();
    cache.insert(
        before.key.clone(),
        glib::Bytes::from_static(b"cached cover"),
    );
    let warm = thumbnail_source(
        &fixture.0,
        ThumbnailKind::FolderAlbum,
        64,
        Some(&before.key),
        |_| panic!("warm hit must not scan the folder"),
    )
    .expect("warm identity lookup");
    assert!(cache.get(&warm.key).is_some());
    fs::write(&cover, b"other").expect("edit existing cover");
    set_modified(&cover, 200);
    let edited = thumbnail_source(
        &fixture.0,
        ThumbnailKind::FolderAlbum,
        64,
        Some(&before.key),
        |_| panic!("editing the selected source needs no folder scan"),
    )
    .expect("edited cover identity");
    assert_eq!(before.key.identity, edited.key.identity);
    assert_eq!(before.key.source_path, edited.key.source_path);
    assert_ne!(before.key.source_identity, edited.key.source_identity);
    assert!(cache.get(&edited.key).is_none());
}

#[test]
fn new_or_removed_preferred_cover_changes_the_selected_source() {
    let fixture = Fixture::new();
    let folder = fixture.write("folder.png", b"folder");
    let initial = source(&fixture.0, ThumbnailKind::FolderAlbum, None);
    assert_eq!(initial.key.source_path, folder);
    let cover = fixture.write("cover.png", b"cover");
    let preferred = source(&fixture.0, ThumbnailKind::FolderAlbum, Some(&initial.key));
    assert_eq!(preferred.key.source_path, cover);
    assert_ne!(initial.key, preferred.key);
    fs::remove_file(cover).expect("remove preferred cover");
    let fallback = source(&fixture.0, ThumbnailKind::FolderAlbum, Some(&preferred.key));
    assert_eq!(fallback.key.source_path, folder);
}

#[test]
fn embedded_cover_source_identity_is_checked_and_selection_is_deterministic() {
    let fixture = Fixture::new();
    fixture.write("02.flac", b"second");
    let first = fixture.write("01.flac", b"first");
    set_modified(&first, 100);
    let before = source(&fixture.0, ThumbnailKind::FolderAlbum, None);
    assert_eq!(before.key.source_path, first);
    assert_eq!(before.operation, ParseOperation::ThumbnailVideo);
    fs::write(&first, b"other").expect("edit embedded artwork source");
    set_modified(&first, 200);
    let edited = source(&fixture.0, ThumbnailKind::FolderAlbum, Some(&before.key));
    assert_eq!(before.key.identity, edited.key.identity);
    assert_ne!(before.key.source_identity, edited.key.source_identity);
    fixture.write("cover.jpg", b"jpeg");
    let jpeg = fixture.write("cover.jpeg", b"jpeg");
    assert_eq!(folder_album_art(&fixture.0).expect("named cover").0, jpeg);
}

#[test]
fn changes_before_or_during_render_are_rejected() {
    let fixture = Fixture::new();
    let path = fixture.write("source.png", b"first");
    set_modified(&path, 100);
    let initial = source(
        &path,
        ThumbnailKind::Sandboxed(ParseOperation::ThumbnailImage),
        None,
    );
    let cancellation = Cancellation::default();
    fs::write(&path, b"other").expect("edit before render");
    set_modified(&path, 200);
    assert!(
        render_thumbnail(&initial, &cancellation, |_, _, _, _| panic!(
            "stale source must not render"
        ))
        .is_err()
    );
    let current = source(
        &path,
        ThumbnailKind::Sandboxed(ParseOperation::ThumbnailImage),
        None,
    );
    assert!(
        render_thumbnail(&current, &cancellation, |path, _, _, _| {
            fs::write(path, b"third").expect("edit during rendering");
            set_modified(path, 300);
            Ok(vec![1])
        })
        .is_err()
    );
}

#[test]
fn cover_content_or_selection_changed_during_render_is_rejected() {
    let fixture = Fixture::new();
    let path = fixture.write("folder.png", b"first");
    set_modified(&path, 100);
    let initial = source(&fixture.0, ThumbnailKind::FolderAlbum, None);
    let cancellation = Cancellation::default();
    assert!(
        render_thumbnail(&initial, &cancellation, |path, _, _, _| {
            fs::write(path, b"other").expect("edit cover while rendering");
            set_modified(path, 200);
            Ok(vec![1])
        })
        .is_err()
    );
    let current = source(&fixture.0, ThumbnailKind::FolderAlbum, None);
    assert!(
        render_thumbnail(&current, &cancellation, |_, _, _, _| {
            fixture.write("cover.png", b"preferred");
            Ok(vec![1])
        })
        .is_err()
    );
}

#[test]
fn cancellation_before_or_during_render_rejects_the_result() {
    let fixture = Fixture::new();
    let path = fixture.write("source.png", b"source");
    let current = source(
        &path,
        ThumbnailKind::Sandboxed(ParseOperation::ThumbnailImage),
        None,
    );
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        render_thumbnail(&current, &cancellation, |_, _, _, _| panic!(
            "cancelled request must not render"
        ))
        .is_err()
    );
    let cancellation = Cancellation::default();
    assert!(
        render_thumbnail(&current, &cancellation, |_, _, _, cancellation| {
            cancellation.cancel();
            Ok(vec![1])
        })
        .is_err()
    );
}

#[test]
fn stale_cancelled_or_removed_bindings_never_paint_or_populate_the_cache() {
    let image_id = 123;
    let cancellation = Cancellation::default();
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(
            image_id,
            ActiveRequest {
                id: 2,
                image: glib::WeakRef::new(),
                cancellation: cancellation.clone(),
            },
        );
    });
    THUMBNAIL_CACHE.with(|cache| *cache.borrow_mut() = ThumbnailCache::default());
    complete_thumbnail(
        image_id,
        1,
        &Cancellation::default(),
        key("old.png"),
        glib::Bytes::from_static(b"stale"),
        |_| panic!("rebound image must not paint stale output"),
    );
    cancellation.cancel();
    complete_thumbnail(
        image_id,
        2,
        &cancellation,
        key("cancelled.png"),
        glib::Bytes::from_static(b"cancelled"),
        |_| panic!("cancelled image must not paint"),
    );
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().remove(&image_id);
    });
    complete_thumbnail(
        image_id,
        2,
        &Cancellation::default(),
        key("removed.png"),
        glib::Bytes::from_static(b"removed"),
        |_| panic!("unbound image must not paint"),
    );
    THUMBNAIL_CACHE.with(|cache| assert!(cache.borrow().entries.is_empty()));
}

#[test]
fn only_successfully_applied_current_results_enter_the_cache() {
    let image_id = 456;
    let cancellation = Cancellation::default();
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(
            image_id,
            ActiveRequest {
                id: 1,
                image: glib::WeakRef::new(),
                cancellation: cancellation.clone(),
            },
        );
    });
    THUMBNAIL_CACHE.with(|cache| *cache.borrow_mut() = ThumbnailCache::default());
    complete_thumbnail(
        image_id,
        1,
        &cancellation,
        key("broken.png"),
        glib::Bytes::from_static(b"invalid"),
        |_| false,
    );
    THUMBNAIL_CACHE.with(|cache| assert!(cache.borrow().entries.is_empty()));
    complete_thumbnail(
        image_id,
        1,
        &cancellation,
        key("valid.png"),
        glib::Bytes::from_static(b"valid"),
        |_| true,
    );
    THUMBNAIL_CACHE.with(|cache| {
        assert_eq!(
            cache
                .borrow_mut()
                .get(&key("valid.png"))
                .expect("cached result")
                .as_ref(),
            b"valid"
        )
    });
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().remove(&image_id);
    });
    THUMBNAIL_CACHE.with(|cache| *cache.borrow_mut() = ThumbnailCache::default());
}

#[test]
fn thumbnail_cache_preserves_byte_budget_and_rejects_oversized_payloads() {
    let mut cache = ThumbnailCache::default();
    cache.insert(
        key("first.png"),
        glib::Bytes::from_owned(vec![0; MAX_CACHE_BYTES / 2]),
    );
    cache.insert(
        key("second.png"),
        glib::Bytes::from_owned(vec![0; MAX_CACHE_BYTES / 2]),
    );
    assert!(cache.get(&key("first.png")).is_some());
    cache.insert(key("third.png"), glib::Bytes::from_static(b"one"));
    assert!(cache.get(&key("second.png")).is_none());
    assert!(cache.byte_count <= MAX_CACHE_BYTES);
    cache.insert(
        key("too-large.png"),
        glib::Bytes::from_owned(vec![0; MAX_CACHE_BYTES + 1]),
    );
    assert!(cache.get(&key("too-large.png")).is_none());
    assert!(cache.get(&key("first.png")).is_some());
}

#[test]
fn desktop_icon_lookup_never_executes_the_generated_launcher() {
    let fixture = Fixture::new();
    let sentinel = fixture.0.join("must-not-exist");
    let launcher = fixture.write("fixture.desktop", format!("[Desktop Entry]\nType=Application\nName=Generated fixture\nExec=/bin/touch {}\nIcon=folder\n", sentinel.display()).as_bytes());
    assert!(
        desktop_icon(&launcher, &Cancellation::default())
            .expect("launcher icon")
            .contains("folder")
    );
    assert!(!sentinel.exists());
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(desktop_icon(&launcher, &cancelled).is_none());
}

#[test]
fn rejects_files_without_a_thumbnail_provider() {
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("README.md")),
        None
    );
    assert_eq!(
        thumbnail_handler_for_name(std::ffi::OsStr::new("no-extension")),
        None
    );
}
