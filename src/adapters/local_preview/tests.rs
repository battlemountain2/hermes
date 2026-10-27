// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;

use crate::services::{FormatFamily, classify_by_name};

#[path = "tests/generated_probe.rs"]
mod generated_probe;

#[test]
#[ignore = "Private GTK preferences and generated Office fixtures; no document mutation"]
fn desktop_office_adapter_returns_pages_or_a_readable_dependency_fallback() {
    use crate::services::{
        PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
    };
    use gtk::glib;
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };
    gtk::init().unwrap();
    let root = std::path::PathBuf::from(
        std::env::var_os("HERMES_OFFICE_FIXTURE_DIR").expect("Generated Office fixtures"),
    );
    for name in ["layout.docx", "layout.odt", "layout.pptx"] {
        let path = root.join(name);
        let entry = crate::model::FileEntry(Rc::new(crate::model::FileEntryInner {
            music_tags: None,
            location: crate::model::Location::local(&path),
            native_name: name.into(),
            display_name: name.into(),
            kind: crate::model::EntryKind::File,
            size: crate::model::MetadataValue::Known(fs::metadata(&path).unwrap().len()),
            modified_unix_seconds: crate::model::MetadataValue::Unknown,
        }));
        let captured = Rc::new(RefCell::new(None));
        let delivered = captured.clone();
        let handle = super::LocalPreviewProvider.load(
            PreviewRequest {
                id: PreviewRequestId(1),
                entry,
                text_byte_limit: 4096,
                pdf_page: 0,
            },
            Rc::new(move |event| {
                delivered.replace(Some(event));
            }),
        );
        let until = Instant::now() + Duration::from_secs(30);
        while captured.borrow().is_none() && Instant::now() < until {
            for _ in 0..64 {
                if !glib::MainContext::default().iteration(false) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        match captured
            .borrow_mut()
            .take()
            .expect("Office adapter completed")
        {
            PreviewEvent::Ready(preview) => match preview.content {
                PreviewContent::Pdf {
                    png,
                    pages,
                    text_layer,
                    ..
                } => {
                    assert_eq!(pages, 2);
                    assert!(!png.is_empty() && text_layer.is_some());
                }
                PreviewContent::OfficeText {
                    content, reason, ..
                } => {
                    assert!(
                        std::env::var_os("HERMES_OFFICE_REQUIRE_LAYOUT").is_none(),
                        "Expected a page layout, received text fallback: {reason}"
                    );
                    assert!(!content.is_empty());
                    assert!(reason.contains("LibreOffice") || reason.contains("Page layout"));
                    if !crate::sandbox::office::available() {
                        assert!(reason.contains("Install LibreOffice"));
                    }
                }
                _ => panic!("Office must be a visual document or explicit text fallback"),
            },
            PreviewEvent::Failed { message, .. } => panic!("Office preview failed: {message}"),
        }
        drop(handle);
    }
}

#[test]
#[ignore = "Private GTK preferences, generated workbook and system renderer; no edits or user documents"]
fn desktop_workbook_adapter_switches_between_layout_and_data_without_changing_the_file() {
    use crate::services::{PreviewProvider, PreviewRequest, PreviewRequestId, PreviewEvent, PreviewContent};
    use gtk::glib;
    use std::{cell::RefCell, rc::Rc, time::{Duration, Instant}};
    gtk::init().unwrap();
    let path = std::path::PathBuf::from(std::env::var_os("HERMES_OFFICE_FIXTURE_DIR").unwrap()).join("layout.xlsx");
    let original = fs::read(&path).unwrap();
    let entry = crate::model::FileEntry(Rc::new(crate::model::FileEntryInner {
        location: crate::model::Location::local(&path), native_name: "layout.xlsx".into(), display_name: "layout.xlsx".into(), kind: crate::model::EntryKind::File, size: crate::model::MetadataValue::Known(original.len() as u64), modified_unix_seconds: crate::model::MetadataValue::Unknown, music_tags: None,
    }));
    for layout in [false, true, false] {
        let captured = Rc::new(RefCell::new(None));
        let result = captured.clone();
        let emit = Rc::new(move |event| { result.replace(Some(event)); });
        let request = PreviewRequest { id: PreviewRequestId(1), entry: entry.clone(), pdf_page: 0, text_byte_limit: 4096 };
        let started = Instant::now();
        let handle = if layout { super::LocalPreviewProvider.load_layout(request, emit) } else { super::LocalPreviewProvider.load(request, emit) };
        while captured.borrow().is_none() && started.elapsed() < Duration::from_secs(40) {
            for _ in 0..64 { if !glib::MainContext::default().iteration(false) { break; } }
            std::thread::sleep(Duration::from_millis(2));
        }
        match captured.borrow_mut().take().expect("Workbook preview completed") {
            PreviewEvent::Ready(preview) => match preview.content {
                PreviewContent::Pdf { png, pages, text_layer, .. } if layout => {
                    assert!(!png.is_empty() && pages >= 2 && text_layer.is_some());
                }
                PreviewContent::Spreadsheet { table } if !layout => assert!(table.sheets.len() >= 2),
                _ => panic!("Wrong workbook view"),
            },
            PreviewEvent::Failed { message, .. } => panic!("Workbook failed: {message}"),
        }
        eprintln!("Workbook layout={layout}: {:?}", started.elapsed());
        drop(handle);
    }
    assert_eq!(original, fs::read(path).unwrap());
}

fn policy() -> super::PreviewPolicy {
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
fn streaming_previews_allow_large_media_but_keep_thumbnail_and_document_limits() {
    let details = policy().for_streaming_preview();
    for (name, mime) in [("track.flac", "audio/flac"), ("clip.mp4", "video/mp4"), ("clip.mov", "video/quicktime")] {
        let large = entry(name, crate::model::MetadataValue::Known(172_400_000));
        assert!(details.check_entry(&large, None).is_ok());
        assert!(details.check_source(&large, mime, Some(172_400_000)).is_ok());
        assert!(details.check_source(&large, "application/octet-stream", Some(172_400_000)).is_ok());
        assert!(policy().check_source(&large, mime, Some(172_400_000)).is_err());
        assert!(details.check_source(&large, mime, None).is_err());
    }
    for (name, mime) in [("photo.jpg", "image/jpeg"), ("paper.pdf", "application/pdf"), ("data.bin", "application/octet-stream")] {
        let large = entry(name, crate::model::MetadataValue::Known(75_400_000));
        assert!(details.check_source(&large, mime, Some(75_400_000)).is_err());
    }
}

#[test]
fn streaming_previews_preserve_format_switches_and_validate_detected_types() {
    use crate::model::MetadataValue;
    let details = policy().for_streaming_preview();
    let large_audio = entry("track.flac", MetadataValue::Known(75_400_000));
    let large_video = entry("clip.mp4", MetadataValue::Known(172_400_000));
    let no_audio = super::PreviewPolicy { audio: false, ..details };
    let no_video = super::PreviewPolicy { videos: false, ..details };
    assert!(no_audio.check_entry(&large_audio, None).is_err());
    assert!(no_video.check_source(&large_video, "video/mp4", Some(172_400_000)).is_err());
    let unnamed = entry("media.bin", MetadataValue::Known(75_400_000));
    assert!(details.check_entry(&unnamed, None).is_ok());
    assert!(details.check_source(&unnamed, "audio/flac", Some(75_400_000)).is_ok());
    assert!(no_audio.check_source(&unnamed, "audio/flac", Some(75_400_000)).is_err());
    assert!(details.check_source(&unnamed, "application/octet-stream", Some(75_400_000)).is_err());
    assert!(details.check_source(&large_audio, "image/png", Some(75_400_000)).is_err());
    let disguised_image = entry("photo.png", MetadataValue::Known(75_400_000));
    assert!(details.check_source(&disguised_image, "audio/flac", Some(75_400_000)).is_err());
    let stale = entry("track.flac", MetadataValue::Known(1));
    assert!(details.check_source(&stale, "audio/flac", Some(75_400_000)).is_ok());
}

#[test]
#[ignore = "Requires isolated GTK/private preferences and built sandbox; generated sparse media, no playback"]
fn desktop_large_sources_reach_metadata_and_streaming_routes() {
    use std::{cell::RefCell, io::Write, rc::Rc, time::{Duration, Instant}};
    use crate::{model::{FileEntry, FileEntryInner, MetadataValue}, services::{PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId}};
    use gtk::glib;

    gtk::init().unwrap();
    assert_eq!(super::PreviewPolicy::current().max_bytes, 50_000_000);
    let root = std::env::temp_dir().join(format!("hermes-large-streams-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let wav = root.join("large.wav");
    let data_bytes = 75_400_000_u32 - 44;
    let mut header = Vec::new();
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(data_bytes + 36).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16_u32.to_le_bytes());
    header.extend_from_slice(&1_u16.to_le_bytes());
    header.extend_from_slice(&2_u16.to_le_bytes());
    header.extend_from_slice(&48_000_u32.to_le_bytes());
    header.extend_from_slice(&192_000_u32.to_le_bytes());
    header.extend_from_slice(&4_u16.to_le_bytes());
    header.extend_from_slice(&16_u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_bytes.to_le_bytes());
    let mut file = fs::File::create(&wav).unwrap();
    file.write_all(&header).unwrap();
    file.set_len(75_400_000).unwrap();
    drop(file);
    let mp4 = root.join("large.mp4");
    assert!(std::process::Command::new("ffmpeg").args([
        "-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i", "color=c=black:s=32x32:r=1",
        "-t", "1", "-an", "-c:v", "mpeg4", "-threads", "1",
    ]).arg(&mp4).status().unwrap().success());
    let mut file = fs::OpenOptions::new().append(true).open(&mp4).unwrap();
    let original_size = file.metadata().unwrap().len();
    file.write_all(&(172_400_000_u32 - original_size as u32).to_be_bytes()).unwrap();
    file.write_all(b"free").unwrap();
    file.set_len(172_400_000).unwrap();
    drop(file);
    let context = glib::MainContext::default();
    for path in [&wav, &mp4] {
        let captured = Rc::new(RefCell::new(None));
        let emitted = captured.clone();
        let request_entry = FileEntry(Rc::new(FileEntryInner {
            location: crate::model::Location::local(path),
            native_name: path.file_name().unwrap().into(), display_name: path.file_name().unwrap().to_string_lossy().into_owned(),
            kind: crate::model::EntryKind::File, size: MetadataValue::Known(path.metadata().unwrap().len()),
            modified_unix_seconds: MetadataValue::Unknown, music_tags: None,
        }));
        let handle = super::LocalPreviewProvider.load(PreviewRequest {
            id: PreviewRequestId(1), entry: request_entry, text_byte_limit: 1024, pdf_page: 0,
        }, Rc::new(move |event| { emitted.replace(Some(event)); }));
        let deadline = Instant::now() + Duration::from_secs(20);
        while captured.borrow().is_none() && Instant::now() < deadline {
            while context.iteration(false) {}
            std::thread::sleep(Duration::from_millis(2));
        }
        match captured.borrow_mut().take().expect("Adapter completed") {
            PreviewEvent::Ready(preview) => match preview.content {
                PreviewContent::Audio { metadata } if path == &wav => assert!(metadata.properties.duration_seconds.unwrap() > 300.0),
                PreviewContent::StreamingMedia { path: returned } if path == &mp4 => assert_eq!(returned, *path),
                _ => panic!("Incorrect large-media route"),
            },
            PreviewEvent::Failed { message, .. } => panic!("Large-media adapter failed: {message}"),
        }
        drop(handle);
    }
    fs::remove_dir_all(root).unwrap();
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
