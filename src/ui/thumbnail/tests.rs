// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;

use gtk::glib;

use crate::sandbox::ParseOperation;
use crate::services::thumbnail_operation_for_name;

use super::{MAX_CACHE_ENTRIES, ThumbnailCache, ThumbnailKey};

#[test]
fn recognizes_mainstream_image_and_video_formats() {
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("photo.JPEG")),
        Some(ParseOperation::ThumbnailImage)
    );
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("animation.webp")),
        Some(ParseOperation::ThumbnailImage)
    );
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("iphone-photo.HEIC")),
        Some(ParseOperation::ThumbnailHeif)
    );
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("camera-photo.heif")),
        Some(ParseOperation::ThumbnailHeif)
    );
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("capture.CR3")),
        Some(ParseOperation::ThumbnailRaw)
    );
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("photo.nef")),
        Some(ParseOperation::ThumbnailRaw)
    );
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("document.PDF")),
        Some(ParseOperation::ThumbnailPdf)
    );
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("clip.mkv")),
        Some(ParseOperation::ThumbnailVideo)
    );
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("clip.ogv")),
        Some(ParseOperation::ThumbnailVideo)
    );
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("album-track.FLAC")),
        Some(ParseOperation::ThumbnailVideo)
    );
}

#[test]
fn thumbnail_cache_evicts_the_least_recent_entry() {
    let mut cache = ThumbnailCache::default();
    for index in 0..=MAX_CACHE_ENTRIES {
        cache.insert(
            ThumbnailKey {
                path: PathBuf::from(format!("image-{index}.png")),
                modified: Some(1),
                file_size: Some(1),
                thumbnail_size: 64,
            },
            glib::Bytes::from_static(&[1]),
        );
    }

    let oldest = ThumbnailKey {
        path: PathBuf::from("image-0.png"),
        modified: Some(1),
        file_size: Some(1),
        thumbnail_size: 64,
    };
    assert!(cache.get(&oldest).is_none());
    assert_eq!(cache.entries.len(), MAX_CACHE_ENTRIES);
}

#[test]
fn rejects_files_without_a_thumbnail_provider() {
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("README.md")),
        None
    );
    assert_eq!(
        thumbnail_operation_for_name(std::ffi::OsStr::new("no-extension")),
        None
    );
}
