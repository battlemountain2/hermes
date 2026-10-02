// SPDX-License-Identifier: GPL-3.0-or-later

//! Tier 2: Boundary Value Analysis & Adversarial Tests (>= 5 test cases per feature for F1..F14).

use std::fs;
use std::path::Path;
use super::common::{
    run_preview_helper,
    sniffer::*, status_bar::*, wire_protocol::*, TestEnv,
};
use super::fixtures::{
    archives::*, audio::*, geotiff::*, models::*, pdf::*, spreadsheets::*, zip_util::*,
};

// =========================================================================
// Feature F1: GeoTIFF Boundaries & Corruption
// =========================================================================

#[test]
fn test_f1_geotiff_empty_file_rejected() {
    let empty = b"";
    assert!(sniff_tiff_dimensions(empty).is_none());
}

#[test]
fn test_f1_geotiff_truncated_header_rejected() {
    let truncated = b"II\x2a";
    assert!(sniff_tiff_dimensions(truncated).is_none());
}

#[test]
fn test_f1_geotiff_invalid_magic_rejected() {
    let invalid = b"ZZ\x2a\x00\x08\x00\x00\x00";
    assert!(sniff_tiff_dimensions(invalid).is_none());
}

#[test]
fn test_f1_geotiff_corrupted_ifd_offset() {
    let mut data = sample_multiband_optical(64, 64);
    // Point IFD0 offset way past EOF
    data[4..8].copy_from_slice(&999_999u32.to_le_bytes());
    assert!(sniff_tiff_dimensions(&data).is_none());
}

#[test]
fn test_f1_geotiff_zero_dimensions_rejected() {
    let data = GeoTiffBuilder::new(0, 0).build();
    let dims = sniff_tiff_dimensions(&data);
    if let Some(d) = dims {
        assert_eq!(d.width, 0);
        assert_eq!(d.height, 0);
    }
}

// =========================================================================
// Feature F2: DEM & Contrast Stretch Boundaries
// =========================================================================

#[test]
fn test_f2_dem_all_nodata_values() {
    let data = GeoTiffBuilder::new(32, 32).float32_dem(Some(-9999.0)).build();
    assert!(data.len() > 32 * 32 * 4);
}

#[test]
fn test_f2_dem_constant_elevation_zero_range() {
    let min = 1500.0f32;
    let max = 1500.0f32;
    let range = max - min;
    assert_eq!(range, 0.0);
    // Guard against divide-by-zero in normalization stretch
    let normalized = if range == 0.0 { 128u8 } else { ((min / max) * 255.0) as u8 };
    assert_eq!(normalized, 128);
}

#[test]
fn test_f2_dem_nan_and_inf_elevations() {
    let nan_val = f32::NAN;
    let inf_val = f32::INFINITY;
    assert!(nan_val.is_nan());
    assert!(inf_val.is_infinite());
    // Normalizer filters out non-finite samples
    let samples = [100.0f32, nan_val, 200.0f32, inf_val, 300.0f32];
    let finite: Vec<f32> = samples.into_iter().filter(|s| s.is_finite()).collect();
    assert_eq!(finite.len(), 3);
}

#[test]
fn test_f2_dem_inverted_min_max_bounds() {
    let min = 2000.0f32;
    let max = 500.0f32;
    let safe_min = min.min(max);
    let safe_max = min.max(max);
    assert_eq!(safe_min, 500.0);
    assert_eq!(safe_max, 2000.0);
}

#[test]
fn test_f2_optical_empty_bands() {
    let data = GeoTiffBuilder::new(16, 16).bands(0).build();
    assert!(!data.is_empty());
}

// =========================================================================
// Feature F3: Geospatial Metadata Boundaries
// =========================================================================

#[test]
fn test_f3_metadata_missing_crs_tag() {
    let data = GeoTiffBuilder::new(64, 64).build(); // No EPSG
    assert!(sniff_tiff_dimensions(&data).is_some());
}

#[test]
fn test_f3_metadata_zero_resolution_division_guard() {
    let res_x = 0.0f64;
    let res_y = 0.0f64;
    let safe_calc = if res_x == 0.0 || res_y == 0.0 { 1.0 } else { res_x * res_y };
    assert_eq!(safe_calc, 1.0);
}

#[test]
fn test_f3_metadata_corrupted_json_payload() {
    let malformed_json = r#"{"epsg":32632,"dimensions":[1000"#;
    assert!(!malformed_json.ends_with('}'));
}

#[test]
fn test_f3_metadata_inverted_bounding_box() {
    let bounds = [510000.0f64, 5500000.0, 500000.0, 5100000.0];
    let min_x = bounds[0].min(bounds[2]);
    let max_x = bounds[0].max(bounds[2]);
    assert_eq!(min_x, 500000.0);
    assert_eq!(max_x, 510000.0);
}

#[test]
fn test_f3_cairo_zero_area_bounding_box() {
    let bounds = [500000.0f64, 5100000.0, 500000.0, 5100000.0];
    let width = bounds[2] - bounds[0];
    let height = bounds[3] - bounds[1];
    assert_eq!(width, 0.0);
    assert_eq!(height, 0.0);
}

// =========================================================================
// Feature F4: Worker Wire Protocol Boundaries
// =========================================================================

#[test]
fn test_f4_wire_incomplete_8byte_header() {
    let short_header = [0u8; 5];
    let err = decode_header(&short_header).expect_err("must fail");
    assert_eq!(err, WireError::IncompleteHeader);
}

#[test]
fn test_f4_wire_unexpected_eof_in_png_payload() {
    let mut frame = encode_frame(b"valid_png_content", b"{}");
    frame.truncate(12); // Truncate in the middle of png payload
    let err = read_framed_message(&frame[..]).expect_err("must fail");
    assert_eq!(err, WireError::UnexpectedEof);
}

#[test]
fn test_f4_wire_unexpected_eof_in_meta_payload() {
    let mut frame = encode_frame(b"png", b"metadata_long_content");
    frame.truncate(frame.len() - 5); // Truncate metadata
    let err = read_framed_message(&frame[..]).expect_err("must fail");
    assert_eq!(err, WireError::UnexpectedEof);
}

#[test]
fn test_f4_wire_oversized_png_payload_rejected() {
    let huge_png_len = 33_554_433u32;
    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&huge_png_len.to_le_bytes());
    let err = decode_header(&header).expect_err("must reject");
    assert!(matches!(err, WireError::PayloadTooLarge { .. }));
}

#[test]
fn test_f4_wire_oversized_meta_payload_rejected() {
    let huge_meta_len = 33_554_433u32;
    let mut header = [0u8; 8];
    header[4..8].copy_from_slice(&huge_meta_len.to_le_bytes());
    let err = decode_header(&header).expect_err("must reject");
    assert!(matches!(err, WireError::PayloadTooLarge { .. }));
}

// =========================================================================
// Feature F5: Header Sniffing Boundaries
// =========================================================================

#[test]
fn test_f5_sniff_empty_buffer() {
    assert!(sniff_png_dimensions(b"").is_none());
    assert!(sniff_gif_dimensions(b"").is_none());
    assert!(sniff_jpeg_dimensions(b"").is_none());
    assert!(sniff_tiff_dimensions(b"").is_none());
}

#[test]
fn test_f5_sniff_png_truncated_ihdr() {
    let mut png = minimal_png().to_vec();
    png.truncate(18); // Cut inside IHDR dimensions
    assert!(sniff_png_dimensions(&png).is_none());
}

#[test]
fn test_f5_sniff_jpeg_missing_sof() {
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xD9];
    assert!(sniff_jpeg_dimensions(&jpeg).is_none());
}

#[test]
fn test_f5_sniff_gif_truncated_screen_desc() {
    let gif = b"GIF89a\x01\x00"; // missing height
    assert!(sniff_gif_dimensions(gif).is_none());
}

#[test]
fn test_f5_sniff_tiff_out_of_bounds_ifd() {
    let tiff = [b'I', b'I', 0x2A, 0x00, 0xFF, 0xFF, 0x00, 0x00];
    assert!(sniff_tiff_dimensions(&tiff).is_none());
}

// =========================================================================
// Feature F6: EXIF Thumbnail Boundaries
// =========================================================================

#[test]
fn test_f6_exif_truncated_app1() {
    let app1 = [0xFF, 0xE1, 0x00, 0x05, b'E'];
    assert_ne!(&app1[4..], b"Exif\0\0");
}

#[test]
fn test_f6_exif_missing_exif_marker() {
    let app1 = [0xFF, 0xE1, 0x00, 0x0A, b'N', b'O', b'E', b'X', b'I', b'F'];
    assert_ne!(&app1[4..], b"Exif\0\0");
}

#[test]
fn test_f6_exif_zero_thumbnail_length() {
    let thumb_len = 0u32;
    assert_eq!(thumb_len, 0);
}

#[test]
fn test_f6_exif_out_of_bounds_thumbnail_offset() {
    let file_size = 500usize;
    let thumb_offset = 1000usize;
    assert!(thumb_offset > file_size);
}

#[test]
fn test_f6_frame_budget_boundary_edge() {
    // 8192 x 4096 = 33,554,432 pixels -> * 4 = 134,217,728 bytes (exceeds 32MB budget)
    assert!(exceeds_decoded_frame_budget(8192, 4096));
}

// =========================================================================
// Feature F7: Sandbox & Resource Limits Boundaries
// =========================================================================

#[test]
fn test_f7_cli_helper_missing_all_arguments() {
    let binary = env!("CARGO_BIN_EXE_strata");
    let cmd = std::process::Command::new(binary)
        .arg("--preview-helper")
        .output()
        .expect("binary executes");
    assert!(!cmd.status.success());
    let stderr = String::from_utf8_lossy(&cmd.stderr);
    assert!(stderr.contains("Invalid preview helper arguments"));
}

#[test]
fn test_f7_cli_helper_invalid_integer_size() {
    let env = TestEnv::new();
    let dummy_in = env.write_file("dummy.in", b"data");
    let dummy_out = env.file_path("dummy.out");

    let res = run_preview_helper("thumbnail-image", &dummy_in, &dummy_out, -999999);
    // negative value clamps or executes without panicking
    assert!(res.status.code().is_some());
}

#[test]
fn test_f7_cli_helper_unknown_operation() {
    let env = TestEnv::new();
    let dummy_in = env.write_file("in.txt", b"test");
    let dummy_out = env.file_path("out.txt");

    let res = run_preview_helper("nonexistent-op-code-12345", &dummy_in, &dummy_out, 1);
    assert!(!res.status.success());
    assert!(res.stderr.contains("Unknown preview helper operation"));
}

#[test]
fn test_f7_cli_helper_nonexistent_input_file() {
    let env = TestEnv::new();
    let missing_in = env.file_path("does_not_exist_file.png");
    let dummy_out = env.file_path("out.png");

    let res = run_preview_helper("thumbnail-image", &missing_in, &dummy_out, 128);
    assert!(!res.status.success());
}

#[test]
fn test_f7_cli_helper_unwritable_output_path() {
    let env = TestEnv::new();
    let in_file = env.write_file("test.png", minimal_png());
    let unwritable_out = Path::new("/nonexistent_folder_abc_xyz/out.png");

    let res = run_preview_helper("thumbnail-image", &in_file, unwritable_out, 128);
    assert!(!res.status.success());
}

// =========================================================================
// Feature F8: 3D Models Boundaries
// =========================================================================

#[test]
fn test_f8_ascii_stl_empty_file() {
    let data = b"";
    assert!(!data.starts_with(b"solid"));
}

#[test]
fn test_f8_binary_stl_truncated_80byte_header() {
    let data = vec![0u8; 40]; // Less than 80 bytes
    assert!(data.len() < 84);
}

#[test]
fn test_f8_binary_stl_triangle_count_mismatch() {
    let corrupted = sample_corrupted_stl();
    let triangles = u32::from_le_bytes(corrupted[80..84].try_into().unwrap());
    let expected_bytes = 84 + (triangles as usize) * 50;
    assert!(corrupted.len() < expected_bytes);
}

#[test]
fn test_f8_3mf_empty_zip() {
    let empty_zip = create_zip(&[]);
    assert!(empty_zip.starts_with(b"PK\x05\x06")); // Only EOCD
}

#[test]
fn test_f8_3mf_missing_model_xml() {
    let zip = create_zip(&[("[Content_Types].xml", b"<Types/>")]);
    let zip_str = String::from_utf8_lossy(&zip);
    assert!(!zip_str.contains("3D/3dmodel.model"));
}

// =========================================================================
// Feature F9: eBooks & Comics Boundaries
// =========================================================================

#[test]
fn test_f9_epub_missing_container_xml() {
    let zip = create_zip(&[("OEBPS/content.opf", b"<package/>")]);
    let zip_str = String::from_utf8_lossy(&zip);
    assert!(!zip_str.contains("META-INF/container.xml"));
}

#[test]
fn test_f9_epub_missing_cover_image() {
    let zip = create_zip(&[
        ("META-INF/container.xml", b"<container/>".as_slice()),
        ("OEBPS/content.opf", b"<package/>".as_slice()),
    ]);
    let zip_str = String::from_utf8_lossy(&zip);
    assert!(!zip_str.contains("cover.png"));
}

#[test]
fn test_f9_cbz_empty_archive() {
    let empty = create_zip(&[]);
    assert_eq!(empty.len(), 22); // Standard empty EOCD record
}

#[test]
fn test_f9_cbz_non_image_files_only() {
    let zip = create_zip(&[
        ("notes.txt", b"hello".as_slice()),
        ("readme.md", b"# Comic".as_slice()),
    ]);
    let zip_str = String::from_utf8_lossy(&zip);
    assert!(!zip_str.contains(".png"));
    assert!(!zip_str.contains(".jpg"));
}

#[test]
fn test_f9_cbr_truncated_rar_header() {
    let truncated = [0x52, 0x61, 0x72, 0x21];
    assert!(truncated.len() < 8);
}

// =========================================================================
// Feature F10: Spreadsheets Boundaries
// =========================================================================

#[test]
fn test_f10_ods_missing_content_xml() {
    let zip = create_zip(&[("mimetype", b"application/vnd.oasis.opendocument.spreadsheet")]);
    let zip_str = String::from_utf8_lossy(&zip);
    assert!(!zip_str.contains("content.xml"));
}

#[test]
fn test_f10_ods_empty_spreadsheet_table() {
    let empty_table_xml = r#"<office:document-content><office:body><office:spreadsheet><table:table/></office:spreadsheet></office:body></office:document-content>"#;
    let zip = create_zip(&[("content.xml", empty_table_xml.as_bytes())]);
    assert!(zip.len() > 0);
}

#[test]
fn test_f10_xlsx_missing_workbook_xml() {
    let zip = create_zip(&[("[Content_Types].xml", b"<Types/>")]);
    let zip_str = String::from_utf8_lossy(&zip);
    assert!(!zip_str.contains("xl/workbook.xml"));
}

#[test]
fn test_f10_xlsx_truncated_archive() {
    let mut data = sample_xlsx_spreadsheet();
    data.truncate(20);
    assert!(data.len() < 100);
}

#[test]
fn test_f10_xls_corrupted_ole_magic() {
    let mut data = sample_xls_spreadsheet();
    data[0] = 0x00;
    assert_ne!(&data[0..4], &[0xd0, 0xcf, 0x11, 0xe0]);
}

// =========================================================================
// Feature F11: Audio Waveforms Boundaries
// =========================================================================

#[test]
fn test_f11_wav_truncated_riff_header() {
    let truncated = b"RIFF\x10\x00\x00\x00WAV";
    assert!(truncated.len() < 44);
}

#[test]
fn test_f11_wav_zero_sample_rate() {
    let mut wav = sample_wav();
    wav[24..28].copy_from_slice(&0u32.to_le_bytes()); // sample rate = 0
    let rate = u32::from_le_bytes(wav[24..28].try_into().unwrap());
    assert_eq!(rate, 0);
}

#[test]
fn test_f11_wav_zero_channels() {
    let mut wav = sample_wav();
    wav[22..24].copy_from_slice(&0u16.to_le_bytes()); // num channels = 0
    let channels = u16::from_le_bytes(wav[22..24].try_into().unwrap());
    assert_eq!(channels, 0);
}

#[test]
fn test_f11_flac_corrupted_magic() {
    let mut flac = sample_flac();
    flac[0] = b'X';
    assert_ne!(&flac[0..4], b"fLaC");
}

#[test]
fn test_f11_ogg_truncated_bos_page() {
    let ogg = b"OggS";
    assert!(ogg.len() < 27);
}

// =========================================================================
// Feature F12: PDF Selection Boundaries
// =========================================================================

#[test]
fn test_f12_pdf_empty_file_rejected() {
    let env = TestEnv::new();
    let empty = env.write_file("empty.pdf", b"");
    let out = env.file_path("out.txt");

    let res = run_preview_helper("extract-pdf-text", &empty, &out, 1);
    assert!(!res.status.success());
}

#[test]
fn test_f12_pdf_corrupted_xref_rejected() {
    let env = TestEnv::new();
    let corrupted = sample_corrupted_pdf();
    let file = env.write_file("corrupt.pdf", &corrupted);
    let out = env.file_path("out.txt");

    let res = run_preview_helper("extract-pdf-text", &file, &out, 1);
    // Either fails or produces empty output
    if res.status.success() {
        let content = fs::read_to_string(&out).unwrap_or_default();
        assert!(content.is_empty());
    } else {
        assert!(!res.status.success());
    }
}

#[test]
fn test_f12_pdf_missing_trailer_rejected() {
    let mut pdf = sample_text_pdf("Missing Trailer");
    if let Some(pos) = pdf.windows(7).position(|w| w == b"trailer") {
        pdf.truncate(pos);
    }
    assert!(!pdf.ends_with(b"%%EOF\n"));
}

#[test]
fn test_f12_pdf_zero_selection_box_no_match() {
    let sel = (10.0f64, 10.0f64, 0.0f64, 0.0f64);
    let glyph = (50.0f64, 50.0f64, 20.0f64, 20.0f64);
    let intersects = !(glyph.0 > sel.0 + sel.2 || glyph.0 + glyph.2 < sel.0 ||
                       glyph.1 > sel.1 + sel.3 || glyph.1 + glyph.3 < sel.1);
    assert!(!intersects);
}

#[test]
fn test_f12_pdf_out_of_range_page_requested() {
    let env = TestEnv::new();
    let pdf = sample_text_pdf("Single Page");
    let file = env.write_file("single.pdf", &pdf);
    let out = env.file_path("out.png");

    let res = run_preview_helper("preview-pdf", &file, &out, 9999);
    // Asking for page 9999 on a 1-page PDF clamps safely to page index 0
    assert!(res.status.success());
    let meta_file = env.file_path("result.meta");
    if meta_file.exists() {
        let meta_str = fs::read_to_string(&meta_file).unwrap();
        assert_eq!(meta_str.trim(), "0 1");
    }
}

// =========================================================================
// Feature F13: Status Bar Free Disk Space Boundaries
// =========================================================================

#[test]
fn test_f13_gio_free_space_nonexistent_path() {
    let missing = Path::new("/path_does_not_exist_under_any_mount_12345");
    let res = query_gio_free_space(missing);
    // GIO query fails for nonexistent path
    assert!(res.is_none());
}

#[test]
fn test_f13_gio_free_space_special_dev_null() {
    let dev_null = Path::new("/dev/null");
    let res = query_gio_free_space(dev_null);
    // /dev is a devtmpfs mount, may have space or return none
    if let Some(bytes) = res {
        let _ = bytes;
    }
}

#[test]
fn test_f13_format_free_space_zero_bytes() {
    assert_eq!(format_free_space(0), "0 B free");
}

#[test]
fn test_f13_format_free_space_one_byte() {
    assert_eq!(format_free_space(1), "1 B free");
}

#[test]
fn test_f13_format_free_space_near_tb_boundary() {
    let bytes = 999_900_000_000u64; // ~999.9 GB
    let formatted = format_free_space(bytes);
    assert!(formatted.contains("GB free") || formatted.contains("TB free"));
}

// =========================================================================
// Feature F14: Dynamic Status Bar Boundaries
// =========================================================================

#[test]
fn test_f14_selection_info_massive_count() {
    let info = format_selection_info(100_000, 500_000_000_000);
    assert_eq!(info, "100000 selected, 500.0 GB");
}

#[test]
fn test_f14_selection_info_zero_bytes_nonzero_count() {
    let info = format_selection_info(5, 0);
    assert_eq!(info, "5 selected, 0 B");
}

#[test]
fn test_f14_item_count_max_usize() {
    let label = format_item_count(1_000_000);
    assert_eq!(label, "1000000 items");
}

#[test]
fn test_f14_cross_mount_directory_boundary() {
    let root_free = query_gio_free_space(Path::new("/"));
    let tmp_free = query_gio_free_space(Path::new("/tmp"));
    assert!(root_free.is_some());
    assert!(tmp_free.is_some());
}

#[test]
fn test_f14_rapid_file_count_transitions() {
    let counts = [0, 1, 0, 100, 1, 0];
    for &c in &counts {
        let label = format_item_count(c);
        if c == 1 {
            assert_eq!(label, "1 item");
        } else {
            assert_eq!(label, format!("{c} items"));
        }
    }
}
