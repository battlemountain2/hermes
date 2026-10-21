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

pub(in crate::ui) fn cache_fixture(path: &Path, bytes: glib::Bytes) {
    let identity = SourceIdentity::read(path).unwrap();
    THUMBNAIL_CACHE.with(|cache| {
        cache.borrow_mut().insert(
            ThumbnailKey {
                path: path.to_owned(),
                source_path: path.to_owned(),
                identity,
                source_identity: identity,
                thumbnail_size: 256,
            },
            bytes,
        )
    });
}

#[test]
fn preview_cache_only_reuses_the_requested_file_not_adjacent_album_art() {
    let mut cache = ThumbnailCache::default();
    let path = PathBuf::from("/generated/track.flac");
    let mut key = ThumbnailKey {
        path: path.clone(),
        source_path: PathBuf::from("/generated/cover.jpg"),
        identity: identity(),
        source_identity: identity(),
        thumbnail_size: 256,
    };
    cache.insert(key.clone(), glib::Bytes::from_static(b"cover"));
    assert!(cache.preview(&path).is_none());
    key.source_path = path.clone();
    cache.insert(key, glib::Bytes::from_static(b"same file"));
    assert_eq!(cache.preview(&path).unwrap().1.as_ref(), b"same file");
    assert!(cache.preview(Path::new("/generated/other.flac")).is_none());
}

fn preview_policy() -> super::PreviewPolicy {
    super::PreviewPolicy {
        max_bytes: 1_000_000,
        streaming_media: false,
        audio: true,
        images: true,
        videos: true,
        pdf: true,
        spreadsheets: true,
        code: true,
    }
}

#[test]
fn cached_previews_obey_format_size_and_policy_changes_without_decoding() {
    let fixture = Fixture::new();
    let path = fixture.write("source.png", b"generated source");
    let bytes = glib::Bytes::from_static(b"already rendered preview");
    cache_fixture(&path, bytes.clone());
    let context = glib::MainContext::new();
    let disabled = super::PreviewPolicy { images: false, ..preview_policy() };
    assert!(context.block_on(super::cached_preview_bytes_with_policy(&path, || disabled)).is_none());
    let too_small = super::PreviewPolicy { max_bytes: 1, ..preview_policy() };
    assert!(context.block_on(super::cached_preview_bytes_with_policy(&path, || too_small)).is_none());
    assert_eq!(context.block_on(super::cached_preview_bytes_with_policy(&path, preview_policy)), Some(bytes));
    let calls = std::cell::Cell::new(0);
    let changed = context.block_on(super::cached_preview_bytes_with_policy(&path, || {
        let count = calls.get() + 1;
        calls.set(count);
        if count >= 3 { disabled } else { preview_policy() }
    }));
    assert!(changed.is_none());
    assert!(calls.get() >= 3);
    THUMBNAIL_CACHE.with(|cache| *cache.borrow_mut() = ThumbnailCache::default());
}

#[test]
fn clearing_memory_cancels_inflight_results_and_prevents_cache_repopulation() {
    let cancellation = Cancellation::default();
    let request = 9_100;
    let image_id = 9_100;
    ACTIVE_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(image_id, ActiveRequest {
            id: request,
            image: glib::WeakRef::new(),
            cancellation: cancellation.clone(),
            wake_handlers: Vec::new(),
        });
    });
    THUMBNAIL_CACHE.with(|cache| cache.borrow_mut().insert(key("cached.png"), glib::Bytes::from_static(b"cached")));
    let before = super::CACHE_GENERATION.with(std::cell::Cell::get);
    super::invalidate_memory_cache();
    assert!(cancellation.is_cancelled());
    assert_ne!(before, super::CACHE_GENERATION.with(std::cell::Cell::get));
    assert!(ACTIVE_REQUESTS.with(|requests| requests.borrow().is_empty()));
    complete_thumbnail(image_id, request, &cancellation, key("late.png"), glib::Bytes::from_static(b"late"), |_| panic!("cancelled result must not paint"));
    THUMBNAIL_CACHE.with(|cache| {
        let cache = cache.borrow();
        assert!(cache.entries.is_empty());
        assert!(cache.recent.is_empty());
        assert_eq!(cache.byte_count, 0);
    });
}

#[test]
fn audio_thumbnails_prefer_embedded_art_then_track_identity_checked_folder_covers() {
    let fixture = Fixture::new();
    let path = fixture.write("track.MP3", b"generated audio placeholder");
    let cover = fixture.write("cover.jpg", b"generated cover placeholder");
    let initial = source(&path, ThumbnailKind::AudioAlbum, None);
    let mut calls = Vec::new();
    let (fallback, bytes) = super::render_thumbnail_with_cover(
        initial,
        ThumbnailKind::AudioAlbum,
        &Cancellation::default(),
        |path, operation, _, _| {
            calls.push((path.to_owned(), operation));
            if operation == ParseOperation::ThumbnailVideo {
                Err("No embedded art".into())
            } else {
                Ok(vec![1, 2, 3])
            }
        },
    )
    .unwrap();
    assert_eq!(
        calls,
        vec![
            (path.clone(), ParseOperation::ThumbnailVideo),
            (cover.clone(), ParseOperation::ThumbnailImage)
        ]
    );
    assert_eq!(bytes, vec![1, 2, 3]);
    assert_eq!(fallback.key.path, path);
    assert_eq!(fallback.key.source_path, cover);
    let reused = source(&path, ThumbnailKind::AudioAlbum, Some(&fallback.key));
    assert_eq!(reused.key, fallback.key);
    fixture.write("cover.jpg", b"edited cover placeholder");
    assert!(!fallback.unchanged());
    let refreshed = source(&path, ThumbnailKind::AudioAlbum, Some(&fallback.key));
    assert_ne!(refreshed.key.source_identity, fallback.key.source_identity);
    fixture.write("track.MP3", b"new Picard tags and embedded artwork");
    let edited = source(&path, ThumbnailKind::AudioAlbum, Some(&fallback.key));
    assert_eq!(edited.key.source_path, path);
    let (embedded, _) = super::render_thumbnail_with_cover(
        edited,
        ThumbnailKind::AudioAlbum,
        &Cancellation::default(),
        |_, operation, _, _| {
            assert_eq!(operation, ParseOperation::ThumbnailVideo);
            Ok(vec![4])
        },
    )
    .unwrap();
    assert_eq!(embedded.key.source_path, path);
}

#[test]
fn cancelled_or_replaced_audio_never_falls_back_to_an_album_cover() {
    let fixture = Fixture::new();
    let path = fixture.write("track.mp3", b"audio");
    fixture.write("cover.png", b"cover");
    let cancellation = Cancellation::default();
    let initial = source(&path, ThumbnailKind::AudioAlbum, None);
    let result = super::render_thumbnail_with_cover(
        initial,
        ThumbnailKind::AudioAlbum,
        &cancellation,
        |_, operation, _, cancellation| {
            assert_eq!(operation, ParseOperation::ThumbnailVideo);
            cancellation.cancel();
            Err("Cancelled".into())
        },
    );
    assert!(result.is_err());
    let initial = source(&path, ThumbnailKind::AudioAlbum, None);
    assert!(
        super::render_thumbnail_with_cover(
            initial,
            ThumbnailKind::AudioAlbum,
            &Cancellation::default(),
            |_, operation, _, _| {
                assert_eq!(operation, ParseOperation::ThumbnailVideo);
                std::fs::write(&path, b"replaced audio").unwrap();
                Err("Changed".into())
            }
        )
        .is_err()
    );
}

#[test]
fn audio_formats_can_request_artwork_even_without_flac_files() {
    for extension in ["mp3", "flac", "m4a", "ogg", "opus", "aac", "wav", "wma"] {
        assert_eq!(
            thumbnail_handler_for_name(std::ffi::OsStr::new(&format!("track.{extension}"))),
            Some(crate::services::ThumbnailHandler::Media)
        );
    }
    let fixture = Fixture::new();
    let path = fixture.write("track.mp3", b"audio placeholder");
    assert_eq!(folder_album_art(&fixture.0).unwrap().0, path);
}

#[test]
#[ignore = "Requires FFmpeg, image loaders, bubblewrap and built worker; generated artwork only"]
fn generated_mp3_tracks_decode_embedded_art_and_adjacent_cover_fallback() {
    use gtk::gdk_pixbuf::prelude::*;
    let fixture = Fixture::new();
    let cover = fixture.0.join("cover.png");
    let image =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 320, 320).unwrap();
    image.fill(0xd02040ff);
    image.savev(&cover, "png", &[]).unwrap();
    let plain = fixture.0.join("plain.mp3");
    let embedded = fixture.0.join("embedded.mp3");
    for path in [&plain, &embedded] {
        let mut command = std::process::Command::new("ffmpeg");
        command.args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
        ]);
        if path == &embedded {
            command.arg("-i").arg(&cover).args([
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:v",
                "png",
                "-disposition:v",
                "attached_pic",
                "-id3v2_version",
                "3",
            ]);
        }
        assert!(
            command
                .args(["-c:a", "libmp3lame", "-threads", "1"])
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
    }
    for path in [&embedded, &plain] {
        let source = source(path, ThumbnailKind::AudioAlbum, None);
        let (result, bytes) = super::render_thumbnail_with_cover(
            source,
            ThumbnailKind::AudioAlbum,
            &Cancellation::default(),
            |path, operation, size, cancellation| {
                crate::sandbox::parse(path, operation, size, cancellation).map(|output| output.data)
            },
        )
        .unwrap();
        assert_eq!(
            result.key.source_path,
            if path == &embedded {
                embedded.clone()
            } else {
                cover.clone()
            }
        );
        let loader = gtk::gdk_pixbuf::PixbufLoader::new();
        loader.write(&bytes).unwrap();
        loader.close().unwrap();
        let pixels = loader.pixbuf().unwrap();
        assert_eq!((pixels.width(), pixels.height()), (256, 256));
        let decoded = pixels.read_pixel_bytes();
        assert!(decoded[0] > 180 && decoded[1] < 80);
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
                wake_handlers: Vec::new(),
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
                wake_handlers: Vec::new(),
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

#[test]
#[ignore = "Private GTK display, isolated cache and built worker; generated PNG only"]
fn desktop_thumbnails_persist_and_reload_after_the_memory_cache_is_cleared() {
    use gtk::prelude::*;
    let root = PathBuf::from(std::env::var_os("HERMES_PREVIEW_PROBE").unwrap());
    assert!(PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap()).starts_with(&root));
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let fixture = Fixture::new();
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 256, 128).unwrap();
    pixels.fill(0x3050d0ff);
    let path = fixture.write("source.png", &pixels.save_to_bufferv("png", &[]).unwrap());
    let entry = crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner {
        location: crate::model::Location::local(&path),
        native_name: path.file_name().unwrap().to_owned(),
        display_name: "source.png".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        music_tags: None,
    }));
    let image = gtk::Image::new();
    let window = gtk::Window::builder().child(&image).build();
    window.present();
    super::set_thumbnail_or_icon(&image, &entry, crate::assets::icons::DOCUMENTS, 32, 64);
    let cache_file = super::disk::root().join(super::disk::filename(&path).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    while !cache_file.is_file() || image.paintable().is_none() {
        assert!(std::time::Instant::now() < deadline);
        for _ in 0..64 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    THUMBNAIL_CACHE.with(|cache| *cache.borrow_mut() = ThumbnailCache::default());
    let original = image
        .paintable()
        .unwrap()
        .downcast::<gtk::gdk::Texture>()
        .unwrap();
    let started = std::time::Instant::now();
    let bytes = glib::MainContext::default()
        .block_on(super::cached_preview_bytes(&path))
        .unwrap();
    eprintln!(
        "Persistent preview after memory eviction: {:?}",
        started.elapsed()
    );
    let reloaded = gtk::gdk::Texture::from_bytes(&bytes).unwrap();
    assert_eq!(
        (original.width(), original.height()),
        (reloaded.width(), reloaded.height())
    );
    let download = |texture: &gtk::gdk::Texture| {
        let mut reader = gtk::gdk::TextureDownloader::new(texture);
        reader.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
        reader.download_bytes().0
    };
    assert_eq!(download(&original), download(&reloaded));
    window.close();
}
