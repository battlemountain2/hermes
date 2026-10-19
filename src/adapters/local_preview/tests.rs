// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;

use crate::services::{FormatFamily, classify_by_name};

#[path = "tests/generated_probe.rs"]
mod generated_probe;

fn policy() -> super::PreviewPolicy {
    super::PreviewPolicy {
        max_bytes: 1_000_000,
        audio: true,
        images: true,
        videos: true,
        pdf: true,
        spreadsheets: true,
        code: true,
    }
}

fn entry(name: &str, size: crate::model::MetadataValue<u64>) -> crate::model::FileEntry {
    crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner {
        music_tags: None,
        location: crate::model::Location::local(std::path::Path::new("/generated").join(name)),
        native_name: name.into(),
        display_name: name.into(),
        kind: crate::model::EntryKind::File,
        size,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
    }))
}

#[test]
fn preview_policy_enforces_each_format_switch_and_image_subfamily() {
    let enabled = policy();
    let cases = [
        (super::PreviewPolicy { audio: false, ..enabled }, vec![FormatFamily::Audio]),
        (super::PreviewPolicy { images: false, ..enabled }, vec![
            FormatFamily::Image, FormatFamily::Svg, FormatFamily::Heif,
            FormatFamily::RawImage, FormatFamily::GeoTiff,
        ]),
        (super::PreviewPolicy { videos: false, ..enabled }, vec![FormatFamily::Video]),
        (super::PreviewPolicy { pdf: false, ..enabled }, vec![FormatFamily::Pdf]),
        (super::PreviewPolicy { spreadsheets: false, ..enabled }, vec![FormatFamily::Spreadsheet]),
        (super::PreviewPolicy { code: false, ..enabled }, vec![FormatFamily::PlainText]),
    ];
    for (disabled, families) in cases {
        for family in families {
            assert!(enabled.check(family, Some(12)).is_ok(), "{family:?}");
            assert!(disabled.check(family, Some(12)).is_err(), "{family:?}");
            assert!(disabled.check(family, None).is_err(), "{family:?}");
        }
        assert!(disabled.check(FormatFamily::Archive, Some(12)).is_ok());
    }
}

#[test]
fn preview_policy_ceiling_is_inclusive_and_applies_to_all_payloads() {
    for family in [
        FormatFamily::PlainText, FormatFamily::Image, FormatFamily::Audio,
        FormatFamily::Video, FormatFamily::Pdf, FormatFamily::Spreadsheet,
        FormatFamily::Archive, FormatFamily::OfficeDocument, FormatFamily::Model,
        FormatFamily::Comic, FormatFamily::Unknown,
    ] {
        assert!(policy().check(family, Some(0)).is_ok());
        assert!(policy().check(family, Some(1_000_000)).is_ok());
        assert!(policy().check(family, Some(1_000_001)).is_err());
        assert!(policy().check(family, Some(u64::MAX)).is_err());
    }
}

#[test]
fn preview_policy_requires_actual_size_and_checks_both_name_and_detected_format() {
    use crate::model::MetadataValue;
    let unknown_size = entry("photo.png", MetadataValue::Unknown);
    assert!(policy().check_entry(&unknown_size, None).is_ok());
    assert!(policy().check_source(&unknown_size, "image/png", None).is_err());
    assert!(policy().check_source(&unknown_size, "image/png", Some(1_000_001)).is_err());
    assert!(policy().check_source(&unknown_size, "image/png", Some(100)).is_ok());
    let disabled = super::PreviewPolicy { images: false, ..policy() };
    assert!(disabled.check_entry(&unknown_size, None).is_err());
    let disguised = entry("renamed.bin", MetadataValue::Known(100));
    assert!(disabled.check_source(&disguised, "image/png", Some(100)).is_err());
    let misleading = entry("photo.png", MetadataValue::Known(100));
    assert!(disabled.check_source(&misleading, "text/plain", Some(100)).is_err());
    let stale_size = entry("photo.png", MetadataValue::Known(100));
    assert!(policy().check_source(&stale_size, "image/png", Some(1_000_001)).is_err());
}

#[test]
fn preview_policy_directory_metadata_is_not_a_payload_size() {
    let mut directory = entry("album.pdf", crate::model::MetadataValue::Known(u64::MAX));
    std::rc::Rc::make_mut(&mut directory.0).kind = crate::model::EntryKind::Directory;
    assert!(policy().check_source(&directory, "inode/directory", None).is_ok());
    assert!(policy().check(FormatFamily::Image, Some(1_000_001)).is_err());
}

#[test]
fn preview_values_keep_page_numbers_text_limits_and_render_sizes_separate() {
    use crate::sandbox::ParseOperation;
    for operation in [
        ParseOperation::PreviewOffice,
        ParseOperation::PreviewArchive,
    ] {
        assert_eq!(super::preview_operation_value(operation, 0, 1024), 1024);
        assert_eq!(super::preview_operation_value(operation, 9, 8), 8);
        assert_eq!(
            super::preview_operation_value(operation, 9, usize::MAX),
            crate::sandbox::MAX_OUTPUT_BYTES as i32
        );
    }
    assert_eq!(
        super::preview_operation_value(ParseOperation::PreviewPdf, 9, 1024),
        9
    );
    assert_eq!(
        super::preview_operation_value(ParseOperation::PreviewModel, 9, 1024),
        800
    );
    assert_eq!(
        super::preview_operation_value(ParseOperation::PreviewArchiveCover, 9, 1024),
        1400
    );
    assert_eq!(
        super::preview_operation_value(ParseOperation::PreviewImage, 9, 1024),
        0
    );
}

#[test]
fn audio_preview_requests_metadata_instead_of_waveform_or_encoded_audio() {
    assert_eq!(
        super::preview_operation(crate::services::PreviewHandler::Audio),
        Some(crate::sandbox::ParseOperation::PreviewAudioMetadata)
    );
}

#[test]
fn recognizes_heif_names_without_invoking_the_system_image_loader() {
    assert_eq!(
        classify_by_name(std::ffi::OsStr::new("photo.HEIC")),
        FormatFamily::Heif
    );
    assert_eq!(
        classify_by_name(std::ffi::OsStr::new("photo.heif")),
        FormatFamily::Heif
    );
    assert_ne!(
        classify_by_name(std::ffi::OsStr::new("photo.jpeg")),
        FormatFamily::Heif
    );
}

#[test]
fn renders_requested_pdf_pages_within_the_pixel_budget() {
    let path = std::env::temp_dir().join(format!(
        "strata-preview-{}-{}.pdf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let surface = cairo::PdfSurface::new(612.0, 792.0, &path).expect("create PDF surface");
    {
        let context = cairo::Context::new(&surface).expect("create PDF context");
        context.set_source_rgb(0.2, 0.4, 0.8);
        context.paint().expect("paint PDF page");
        context.show_page().expect("finish first PDF page");
        context.set_source_rgb(0.8, 0.4, 0.2);
        context.paint().expect("paint second PDF page");
        context.show_page().expect("finish second PDF page");
    }
    surface.finish();

    let output_directory = path.with_extension("output");
    fs::create_dir(&output_directory).expect("create output directory");
    let output = output_directory.join("result.png");
    crate::sandbox_helper::run(&[
        "preview-pdf".to_owned(),
        path.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "1".to_owned(),
    ])
    .expect("render second PDF page");
    let png = fs::read(&output).expect("read rendered page");
    let metadata =
        fs::read_to_string(output_directory.join("result.meta")).expect("read PDF metadata");
    let _removed = fs::remove_file(path);
    let _removed = fs::remove_dir_all(output_directory);

    assert_eq!(metadata, "1 2");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn extracts_bounded_text_from_pdf_pages() {
    let path = std::env::temp_dir().join(format!(
        "hermes-search-{}-{}.pdf",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let surface = cairo::PdfSurface::new(612.0, 792.0, &path).expect("create PDF surface");
    {
        let context = cairo::Context::new(&surface).expect("create PDF context");
        context.move_to(72.0, 72.0);
        context
            .show_text("Hermes searchable PDF phrase")
            .expect("write PDF text");
        context.show_page().expect("finish PDF page");
    }
    surface.finish();

    let output_directory = path.with_extension("text-output");
    fs::create_dir(&output_directory).expect("create output directory");
    let output = output_directory.join("result.txt");
    crate::sandbox_helper::run(&[
        "extract-pdf-text".to_owned(),
        path.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "12".to_owned(),
    ])
    .expect("extract PDF text");
    let text = fs::read_to_string(&output).expect("read extracted PDF text");
    let _removed = fs::remove_file(path);
    let _removed = fs::remove_dir_all(output_directory);

    assert_eq!(text, "Hermes searc");
    assert!(text.len() <= 12);
}
