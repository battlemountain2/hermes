// SPDX-License-Identifier: GPL-3.0-or-later

//! Centralized format classification registry.
//!
//! Every file-format decision — preview capability, thumbnail support,
//! content-search eligibility — flows through this module so that extension
//! and MIME tables are defined in exactly one place.

use std::{ffi::OsStr, path::Path};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PreviewHandler {
    Text,
    Image,
    GeoTiff,
    Heif,
    Pdf,
    Audio,
    Video,
    Archive,
    Office,
    Model,
    ArchiveCover,
    Spreadsheet,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ThumbnailHandler {
    Image,
    Heif,
    RawImage,
    Pdf,
    Media,
    Model,
    Cover,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextExtractor {
    PlainText,
    Pdf,
    Office,
    Spreadsheet,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FormatCapabilities {
    pub family: FormatFamily,
    pub preview: Option<PreviewHandler>,
    pub thumbnail: Option<ThumbnailHandler>,
    pub text_extractor: Option<TextExtractor>,
}

/// Broad format family returned by the classifiers.
///
/// The granularity is chosen so that each variant maps unambiguously to a set
/// of capabilities without needing secondary extension checks.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FormatFamily {
    /// Source code, configuration, markup, and other UTF-8 text.
    PlainText,
    /// SVG — previewable as an image *and* searchable as text.
    Svg,
    /// Standard raster images (PNG, JPEG, WebP, GIF, BMP, AVIF, …).
    Image,
    /// GeoTIFF geospatial raster imagery (.tif, .tiff).
    GeoTiff,
    /// HEIF / HEIC — requires its own ImageMagick sandbox path.
    Heif,
    /// Camera RAW formats (CR2, CR3, NEF, DNG, ARW, …).
    RawImage,
    /// PDF documents.
    Pdf,
    /// OpenDocument and OOXML office documents.
    OfficeDocument,
    /// Audio files (FLAC, MP3, OGG, …).
    Audio,
    /// Video files (MP4, MKV, WebM, MOV, …).
    Video,
    /// `.desktop` launcher entries.
    DesktopEntry,
    /// Compressed archives (ZIP, TAR, …).
    Archive,
    /// 3D geometry files (STL, 3MF).
    Model,
    /// Comic books and eBooks (EPUB, CBZ, CBR).
    Comic,
    /// Spreadsheets (ODS, XLS, XLSX).
    Spreadsheet,
    /// Anything we don't recognize.
    Unknown,
}

impl FormatFamily {
    /// Whether the format can be shown in the quick-preview panel.
    pub fn supports_quick_preview(self) -> bool {
        self.preview_handler().is_some()
    }

    pub fn preview_handler(self) -> Option<PreviewHandler> {
        match self {
            Self::PlainText => Some(PreviewHandler::Text),
            Self::Image | Self::Svg | Self::RawImage => Some(PreviewHandler::Image),
            Self::GeoTiff => Some(PreviewHandler::GeoTiff),
            Self::Heif => Some(PreviewHandler::Heif),
            Self::Pdf => Some(PreviewHandler::Pdf),
            Self::Audio => Some(PreviewHandler::Audio),
            Self::Video => Some(PreviewHandler::Video),
            Self::Archive => Some(PreviewHandler::Archive),
            Self::OfficeDocument => Some(PreviewHandler::Office),
            Self::Model => Some(PreviewHandler::Model),
            Self::Comic => Some(PreviewHandler::ArchiveCover),
            Self::Spreadsheet => Some(PreviewHandler::Spreadsheet),
            Self::DesktopEntry | Self::Unknown => None,
        }
    }

    pub fn text_extractor(self) -> Option<TextExtractor> {
        match self {
            Self::PlainText | Self::Svg => Some(TextExtractor::PlainText),
            Self::Pdf => Some(TextExtractor::Pdf),
            Self::OfficeDocument => Some(TextExtractor::Office),
            Self::Spreadsheet => Some(TextExtractor::Spreadsheet),
            _ => None,
        }
    }

    /// Human-readable reason shown when visual preview is unavailable for
    /// this format.
    pub fn unavailable_reason(self) -> &'static str {
        match self {
            Self::Archive => "Archive contents can be viewed after extraction",
            Self::DesktopEntry => "Application entries are launched, not previewed",
            _ => "This file type has no visual preview",
        }
    }

    /// Short display label for the format family, suitable for UI metadata
    /// fields.
    pub fn display_label(self) -> &'static str {
        match self {
            Self::PlainText => "Text",
            Self::Svg => "SVG Image",
            Self::Image => "Image",
            Self::GeoTiff => "GeoTIFF",
            Self::Heif => "HEIF Image",
            Self::RawImage => "Camera RAW",
            Self::Pdf => "PDF Document",
            Self::OfficeDocument => "Office Document",
            Self::Audio => "Audio",
            Self::Video => "Video",
            Self::DesktopEntry => "Application",
            Self::Archive => "Archive",
            Self::Model => "3D Model",
            Self::Comic => "Comic Archive",
            Self::Spreadsheet => "Spreadsheet",
            Self::Unknown => "File",
        }
    }
}

pub fn capabilities_by_name(name: &OsStr) -> FormatCapabilities {
    let family = classify_by_name(name);
    FormatCapabilities {
        family,
        preview: family.preview_handler(),
        thumbnail: thumbnail_handler_for_name(name),
        text_extractor: family.text_extractor(),
    }
}

// ---------------------------------------------------------------------------
// Classification by MIME content-type
// ---------------------------------------------------------------------------

/// Classify a file by its MIME content-type string.
///
/// This is the primary classifier when GIO type information is available.
pub fn classify_by_mime(content_type: &str) -> FormatFamily {
    // Exact matches first.
    if content_type == "application/pdf" {
        return FormatFamily::Pdf;
    }
    if matches!(
        content_type,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.oasis.opendocument.text"
    ) {
        return FormatFamily::OfficeDocument;
    }
    if matches!(
        content_type,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.ms-excel"
            | "application/vnd.oasis.opendocument.spreadsheet"
    ) {
        return FormatFamily::Spreadsheet;
    }
    if matches!(
        content_type,
        "application/epub+zip"
            | "application/vnd.comicbook+zip"
            | "application/vnd.comicbook-rar"
    ) {
        return FormatFamily::Comic;
    }
    if matches!(
        content_type,
        "model/stl"
            | "application/sla"
            | "model/3mf"
            | "application/vnd.ms-package.3dmanufacturing-3dmodel+xml"
    ) {
        return FormatFamily::Model;
    }
    if content_type == "image/svg+xml" {
        return FormatFamily::Svg;
    }

    // Prefix-based groups.
    if content_type.starts_with("image/") {
        return if content_type == "image/heif" || content_type == "image/heic" {
            FormatFamily::Heif
        } else if content_type == "image/tiff" {
            FormatFamily::GeoTiff
        } else {
            FormatFamily::Image
        };
    }
    if content_type.starts_with("audio/") {
        return FormatFamily::Audio;
    }
    if content_type.starts_with("video/") {
        return FormatFamily::Video;
    }

    // Text-like MIME types.
    if content_type.starts_with("text/")
        || matches!(
            content_type,
            "application/json"
                | "application/ld+json"
                | "application/toml"
                | "application/x-yaml"
                | "application/xml"
                | "application/javascript"
                | "application/x-javascript"
                | "application/x-shellscript"
        )
        || content_type.ends_with("+json")
        || content_type.ends_with("+xml")
    {
        return FormatFamily::PlainText;
    }

    FormatFamily::Unknown
}

// ---------------------------------------------------------------------------
// Classification by filename / extension
// ---------------------------------------------------------------------------

/// Well-known filenames (no extension) that are plain text.
const PLAIN_TEXT_FILENAMES: &[&str] = &["dockerfile", "gemfile", "makefile", "meson.build"];

/// Classify a file by its filename or extension alone.
///
/// Preferred when MIME detection is unavailable or too expensive (e.g. during
/// recursive search tree traversal).
pub fn classify_by_name(name: &OsStr) -> FormatFamily {
    let path = Path::new(name);

    // Check exact filenames first.
    if let Some(filename) = path.file_name().and_then(OsStr::to_str) {
        let lower = filename.to_ascii_lowercase();
        if PLAIN_TEXT_FILENAMES.contains(&lower.as_str()) {
            return FormatFamily::PlainText;
        }
    }

    let extension = match path.extension().and_then(OsStr::to_str) {
        Some(ext) => ext.to_ascii_lowercase(),
        None => return FormatFamily::Unknown,
    };

    match extension.as_str() {
        // Plain text / source / config / markup
        "bash" | "c" | "cc" | "conf" | "cpp" | "cs" | "css" | "csv" | "fish" | "go" | "h"
        | "hpp" | "htm" | "html" | "ini" | "java" | "js" | "json" | "jsonl" | "jsx" | "kt"
        | "kts" | "less" | "log" | "lua" | "md" | "markdown" | "py" | "rb" | "rs" | "rst"
        | "scss" | "service" | "sh" | "sql" | "toml" | "ts" | "tsv" | "tsx" | "txt" | "xml"
        | "yaml" | "yml" | "zsh" => FormatFamily::PlainText,

        // SVG (dual: image-previewable + text-searchable)
        "svg" => FormatFamily::Svg,

        // Standard raster images
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "avif" => {
            FormatFamily::Image
        }

        // GeoTIFF geospatial raster imagery
        "tif" | "tiff" => FormatFamily::GeoTiff,

        // HEIF / HEIC
        "heic" | "heif" => FormatFamily::Heif,

        // Camera RAW
        "3fr" | "arw" | "cr2" | "cr3" | "dcr" | "dng" | "erf" | "kdc" | "mef" | "mos" | "mrw"
        | "nef" | "nrw" | "orf" | "pef" | "raf" | "raw" | "rw2" | "rwl" | "sr2" | "srf" | "srw"
        | "x3f" => FormatFamily::RawImage,

        // PDF
        "pdf" => FormatFamily::Pdf,

        // Office documents with XML content stored in a ZIP container
        "docx" | "odt" => FormatFamily::OfficeDocument,

        // Spreadsheets
        "xlsx" | "xls" | "ods" => FormatFamily::Spreadsheet,

        // 3D Models
        "stl" | "3mf" => FormatFamily::Model,

        // eBooks & Comics
        "epub" | "cbz" | "cbr" => FormatFamily::Comic,

        // Audio
        "flac" | "mp3" | "ogg" | "opus" | "m4a" | "aac" | "wav" | "wma" => FormatFamily::Audio,

        // Video
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" | "ogv" => {
            FormatFamily::Video
        }

        // Desktop entries
        "desktop" => FormatFamily::DesktopEntry,

        // Archives
        "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst" | "tgz" | "tbz2" | "txz" => {
            FormatFamily::Archive
        }

        _ => FormatFamily::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Thumbnail capability (curated extension list)
// ---------------------------------------------------------------------------

/// Returns the sandbox operation needed to generate a thumbnail, or `None` if
/// the file extension is not in the curated thumbnailable set.
///
/// This is intentionally a standalone function rather than a method on
/// [`FormatFamily`] because the set of thumbnailable extensions is smaller than
/// the set of extensions in each family (e.g. AVIF is [`FormatFamily::Image`]
/// but is not currently thumbnailed by the sandbox).
pub fn thumbnail_handler_for_name(name: &OsStr) -> Option<ThumbnailHandler> {
    let extension = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff" => {
            Some(ThumbnailHandler::Image)
        }
        "heic" | "heif" => Some(ThumbnailHandler::Heif),
        "3fr" | "arw" | "cr2" | "cr3" | "dcr" | "dng" | "erf" | "kdc" | "mef" | "mos" | "mrw"
        | "nef" | "nrw" | "orf" | "pef" | "raf" | "raw" | "rw2" | "rwl" | "sr2" | "srf" | "srw"
        | "x3f" => Some(ThumbnailHandler::RawImage),
        "pdf" => Some(ThumbnailHandler::Pdf),
        "stl" | "3mf" => Some(ThumbnailHandler::Model),
        "epub" | "cbz" | "cbr" => Some(ThumbnailHandler::Cover),
        "flac" | "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "mpeg" | "mpg" | "ogv" => {
            Some(ThumbnailHandler::Media)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
