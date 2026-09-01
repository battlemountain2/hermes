// SPDX-License-Identifier: GPL-3.0-or-later

use std::ffi::OsStr;

use super::{
    FormatFamily, PreviewHandler, TextExtractor, ThumbnailHandler, capabilities_by_name,
    classify_by_mime, classify_by_name, thumbnail_handler_for_name,
};

// ---------------------------------------------------------------------------
// classify_by_name — extension table
// ---------------------------------------------------------------------------

#[test]
fn recognizes_configuration_files_as_plain_text() {
    assert_eq!(
        classify_by_name(OsStr::new("settings.conf")),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_name(OsStr::new("SETTINGS.INI")),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_name(OsStr::new("main.rs")),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_name(OsStr::new("README.md")),
        FormatFamily::PlainText
    );
}

#[test]
fn recognizes_exact_filenames_as_plain_text() {
    assert_eq!(
        classify_by_name(OsStr::new("Dockerfile")),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_name(OsStr::new("Makefile")),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_name(OsStr::new("Gemfile")),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_name(OsStr::new("meson.build")),
        FormatFamily::PlainText
    );
}

#[test]
fn classifies_archives_and_desktop_entries() {
    assert_eq!(
        classify_by_name(OsStr::new("archive.zip")),
        FormatFamily::Archive
    );
    assert_eq!(
        classify_by_name(OsStr::new("release.tar")),
        FormatFamily::Archive
    );
    assert_eq!(
        classify_by_name(OsStr::new("app.desktop")),
        FormatFamily::DesktopEntry
    );
}

#[test]
fn classifies_images_by_extension() {
    assert_eq!(
        classify_by_name(OsStr::new("photo.JPEG")),
        FormatFamily::Image
    );
    assert_eq!(
        classify_by_name(OsStr::new("animation.webp")),
        FormatFamily::Image
    );
    assert_eq!(
        classify_by_name(OsStr::new("icon.avif")),
        FormatFamily::Image
    );
}

#[test]
fn classifies_heif_by_extension() {
    assert_eq!(
        classify_by_name(OsStr::new("iphone-photo.HEIC")),
        FormatFamily::Heif
    );
    assert_eq!(
        classify_by_name(OsStr::new("camera-photo.heif")),
        FormatFamily::Heif
    );
}

#[test]
fn classifies_raw_images_by_extension() {
    assert_eq!(
        classify_by_name(OsStr::new("capture.CR3")),
        FormatFamily::RawImage
    );
    assert_eq!(
        classify_by_name(OsStr::new("photo.nef")),
        FormatFamily::RawImage
    );
    assert_eq!(
        classify_by_name(OsStr::new("shot.DNG")),
        FormatFamily::RawImage
    );
}

#[test]
fn classifies_svg_by_extension() {
    assert_eq!(
        classify_by_name(OsStr::new("diagram.svg")),
        FormatFamily::Svg
    );
    assert_eq!(classify_by_name(OsStr::new("LOGO.SVG")), FormatFamily::Svg);
}

#[test]
fn classifies_media_by_extension() {
    assert_eq!(
        classify_by_name(OsStr::new("clip.mkv")),
        FormatFamily::Video
    );
    assert_eq!(
        classify_by_name(OsStr::new("clip.ogv")),
        FormatFamily::Video
    );
    assert_eq!(
        classify_by_name(OsStr::new("album-track.FLAC")),
        FormatFamily::Audio
    );
    assert_eq!(
        classify_by_name(OsStr::new("song.mp3")),
        FormatFamily::Audio
    );
}

#[test]
fn classifies_pdf_by_extension() {
    assert_eq!(
        classify_by_name(OsStr::new("document.PDF")),
        FormatFamily::Pdf
    );
}

#[test]
fn classifies_office_documents() {
    assert_eq!(
        classify_by_name(OsStr::new("letter.docx")),
        FormatFamily::OfficeDocument
    );
    assert_eq!(
        classify_by_name(OsStr::new("notes.ODT")),
        FormatFamily::OfficeDocument
    );
    assert_eq!(
        classify_by_mime("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        FormatFamily::OfficeDocument
    );
}

#[test]
fn unknown_for_unrecognized_or_missing_extension() {
    assert_eq!(
        classify_by_name(OsStr::new("no-extension")),
        FormatFamily::Unknown
    );
    assert_eq!(
        classify_by_name(OsStr::new("mystery.xyz123")),
        FormatFamily::Unknown
    );
}

// ---------------------------------------------------------------------------
// classify_by_mime — MIME table
// ---------------------------------------------------------------------------

#[test]
fn classifies_common_preview_content_types_by_mime() {
    assert_eq!(classify_by_mime("image/png"), FormatFamily::Image);
    assert_eq!(classify_by_mime("image/jpeg"), FormatFamily::Image);
    assert_eq!(classify_by_mime("image/svg+xml"), FormatFamily::Svg);
    assert_eq!(classify_by_mime("image/heif"), FormatFamily::Heif);
    assert_eq!(classify_by_mime("image/heic"), FormatFamily::Heif);
    assert_eq!(classify_by_mime("video/mp4"), FormatFamily::Video);
    assert_eq!(classify_by_mime("audio/mpeg"), FormatFamily::Audio);
    assert_eq!(classify_by_mime("application/pdf"), FormatFamily::Pdf);
    assert_eq!(classify_by_mime("text/x-rust"), FormatFamily::PlainText);
    assert_eq!(classify_by_mime("text/plain"), FormatFamily::PlainText);
}

#[test]
fn classifies_structured_text_mime_types() {
    assert_eq!(
        classify_by_mime("application/json"),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_mime("application/toml"),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_mime("application/x-yaml"),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_mime("application/javascript"),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_mime("application/x-shellscript"),
        FormatFamily::PlainText
    );
}

#[test]
fn classifies_suffix_mime_types() {
    assert_eq!(
        classify_by_mime("application/problem+json"),
        FormatFamily::PlainText
    );
    assert_eq!(
        classify_by_mime("application/xhtml+xml"),
        FormatFamily::PlainText
    );
}

#[test]
fn unknown_for_opaque_mime() {
    assert_eq!(
        classify_by_mime("application/octet-stream"),
        FormatFamily::Unknown
    );
    assert_eq!(
        classify_by_mime("application/x-unknown-format"),
        FormatFamily::Unknown
    );
}

// ---------------------------------------------------------------------------
// Capability methods
// ---------------------------------------------------------------------------

#[test]
fn quick_preview_supported_for_previewable_families() {
    assert!(FormatFamily::PlainText.supports_quick_preview());
    assert!(FormatFamily::Svg.supports_quick_preview());
    assert!(FormatFamily::Image.supports_quick_preview());
    assert!(FormatFamily::Heif.supports_quick_preview());
    assert!(FormatFamily::RawImage.supports_quick_preview());
    assert!(FormatFamily::Pdf.supports_quick_preview());
    assert!(FormatFamily::Audio.supports_quick_preview());
    assert!(FormatFamily::Video.supports_quick_preview());
    assert!(FormatFamily::Archive.supports_quick_preview());
    assert!(FormatFamily::OfficeDocument.supports_quick_preview());
}

#[test]
fn quick_preview_not_supported_for_non_previewable_families() {
    assert!(!FormatFamily::DesktopEntry.supports_quick_preview());
    assert!(!FormatFamily::Unknown.supports_quick_preview());
}

#[test]
fn searchable_text_for_text_families() {
    assert_eq!(
        FormatFamily::PlainText.text_extractor(),
        Some(TextExtractor::PlainText)
    );
    assert_eq!(
        FormatFamily::Svg.text_extractor(),
        Some(TextExtractor::PlainText)
    );
    assert_eq!(
        FormatFamily::OfficeDocument.text_extractor(),
        Some(TextExtractor::Office)
    );
}

#[test]
fn not_searchable_text_for_binary_families() {
    assert_eq!(FormatFamily::Image.text_extractor(), None);
    assert_eq!(FormatFamily::Heif.text_extractor(), None);
    assert_eq!(FormatFamily::Video.text_extractor(), None);
    assert_eq!(FormatFamily::Pdf.text_extractor(), Some(TextExtractor::Pdf));
    assert_eq!(FormatFamily::Unknown.text_extractor(), None);
}

#[test]
fn provides_format_specific_unavailable_reasons() {
    assert_eq!(
        FormatFamily::Archive.unavailable_reason(),
        "Archive contents can be viewed after extraction"
    );
    assert_eq!(
        FormatFamily::DesktopEntry.unavailable_reason(),
        "Application entries are launched, not previewed"
    );
    assert_eq!(
        FormatFamily::Unknown.unavailable_reason(),
        "This file type has no visual preview"
    );
    // Other families default to the generic fallback if queried.
    assert_eq!(
        FormatFamily::PlainText.unavailable_reason(),
        "This file type has no visual preview"
    );
}

#[test]
fn provides_display_labels_for_all_families() {
    assert_eq!(FormatFamily::PlainText.display_label(), "Text");
    assert_eq!(FormatFamily::Svg.display_label(), "SVG Image");
    assert_eq!(FormatFamily::Image.display_label(), "Image");
    assert_eq!(FormatFamily::Heif.display_label(), "HEIF Image");
    assert_eq!(FormatFamily::RawImage.display_label(), "Camera RAW");
    assert_eq!(FormatFamily::Pdf.display_label(), "PDF Document");
    assert_eq!(
        FormatFamily::OfficeDocument.display_label(),
        "Office Document"
    );
    assert_eq!(FormatFamily::Audio.display_label(), "Audio");
    assert_eq!(FormatFamily::Video.display_label(), "Video");
    assert_eq!(FormatFamily::DesktopEntry.display_label(), "Application");
    assert_eq!(FormatFamily::Archive.display_label(), "Archive");
    assert_eq!(FormatFamily::Unknown.display_label(), "File");
}

#[test]
fn preview_handlers_match_format_capabilities() {
    assert_eq!(
        FormatFamily::Image.preview_handler(),
        Some(PreviewHandler::Image)
    );
    assert_eq!(
        FormatFamily::Svg.preview_handler(),
        Some(PreviewHandler::Image)
    );
    assert_eq!(
        FormatFamily::RawImage.preview_handler(),
        Some(PreviewHandler::Image)
    );
    assert_eq!(
        FormatFamily::Heif.preview_handler(),
        Some(PreviewHandler::Heif)
    );
    assert_eq!(
        FormatFamily::Pdf.preview_handler(),
        Some(PreviewHandler::Pdf)
    );
    assert_eq!(
        FormatFamily::Audio.preview_handler(),
        Some(PreviewHandler::Audio)
    );
    assert_eq!(
        FormatFamily::Video.preview_handler(),
        Some(PreviewHandler::Video)
    );
    assert_eq!(
        FormatFamily::PlainText.preview_handler(),
        Some(PreviewHandler::Text)
    );
    assert_eq!(
        FormatFamily::Archive.preview_handler(),
        Some(PreviewHandler::Archive)
    );
    assert_eq!(
        FormatFamily::OfficeDocument.preview_handler(),
        Some(PreviewHandler::Office)
    );
    assert_eq!(FormatFamily::Unknown.preview_handler(), None);
}

// ---------------------------------------------------------------------------
// thumbnail_operation_for_name
// ---------------------------------------------------------------------------

#[test]
fn thumbnail_handlers_for_supported_formats() {
    assert_eq!(
        thumbnail_handler_for_name(OsStr::new("photo.JPEG")),
        Some(ThumbnailHandler::Image)
    );
    assert_eq!(
        thumbnail_handler_for_name(OsStr::new("animation.webp")),
        Some(ThumbnailHandler::Image)
    );
    assert_eq!(
        thumbnail_handler_for_name(OsStr::new("iphone-photo.HEIC")),
        Some(ThumbnailHandler::Heif)
    );
    assert_eq!(
        thumbnail_handler_for_name(OsStr::new("capture.CR3")),
        Some(ThumbnailHandler::RawImage)
    );
    assert_eq!(
        thumbnail_handler_for_name(OsStr::new("photo.nef")),
        Some(ThumbnailHandler::RawImage)
    );
    assert_eq!(
        thumbnail_handler_for_name(OsStr::new("document.PDF")),
        Some(ThumbnailHandler::Pdf)
    );
    assert_eq!(
        thumbnail_handler_for_name(OsStr::new("clip.mkv")),
        Some(ThumbnailHandler::Media)
    );
    assert_eq!(
        thumbnail_handler_for_name(OsStr::new("album-track.FLAC")),
        Some(ThumbnailHandler::Media)
    );
}

#[test]
fn registry_reports_independent_capabilities() {
    let svg = capabilities_by_name(OsStr::new("diagram.svg"));
    assert_eq!(svg.family, FormatFamily::Svg);
    assert_eq!(svg.preview, Some(PreviewHandler::Image));
    assert_eq!(svg.thumbnail, None);
    assert_eq!(svg.text_extractor, Some(TextExtractor::PlainText));

    let pdf = capabilities_by_name(OsStr::new("manual.pdf"));
    assert_eq!(pdf.family, FormatFamily::Pdf);
    assert_eq!(pdf.preview, Some(PreviewHandler::Pdf));
    assert_eq!(pdf.thumbnail, Some(ThumbnailHandler::Pdf));
    assert_eq!(pdf.text_extractor, Some(TextExtractor::Pdf));
}

#[test]
fn no_thumbnail_for_unsupported_formats() {
    assert_eq!(thumbnail_handler_for_name(OsStr::new("README.md")), None);
    assert_eq!(thumbnail_handler_for_name(OsStr::new("no-extension")), None);
    // AVIF is Image but not in the curated thumbnail list.
    assert_eq!(thumbnail_handler_for_name(OsStr::new("photo.avif")), None);
}
