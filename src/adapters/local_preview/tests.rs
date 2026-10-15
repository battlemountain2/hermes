// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;

use crate::services::{FormatFamily, classify_by_name};

#[path = "tests/generated_probe.rs"]
mod generated_probe;

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
