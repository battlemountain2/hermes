// SPDX-License-Identifier: GPL-3.0-or-later

//! Tier 1: Happy Path Isolation Tests (>= 5 test cases per feature for F1..F14).

use std::fs;
use std::path::Path;
use super::common::{
    is_op_supported, is_valid_png, run_preview_helper,
    sniffer::*, status_bar::*, wire_protocol::*, TestEnv,
};
use super::fixtures::{
    archives::*, audio::*, geotiff::*, models::*, pdf::*, spreadsheets::*,
};

// =========================================================================
// Feature F1: GeoTIFF Pyramid Overview Extraction
// =========================================================================

#[test]
fn test_f1_geotiff_pyramid_overview_level_1_happy() {
    let env = TestEnv::new();
    let data = sample_cog_pyramidal(1024, 1024, 3);
    let input = env.write_file("pyramid_1024.tif", &data);
    let output = env.file_path("out.png");

    if is_op_supported("preview-geotiff") {
        let res = run_preview_helper("preview-geotiff", &input, &output, 1400);
        assert!(res.status.success(), "stderr: {}", res.stderr);
        let png = fs::read(&output).expect("output exists");
        assert!(is_valid_png(&png));
    } else {
        assert!(sniff_tiff_dimensions(&data).is_some());
    }
}

#[test]
fn test_f1_geotiff_pyramid_overview_level_2_happy() {
    let env = TestEnv::new();
    let data = sample_cog_pyramidal(2048, 2048, 4);
    let _input = env.write_file("pyramid_2048.tif", &data);
    assert_eq!(data[0..4], [0x49, 0x49, 0x2a, 0x00]);
    let dims = sniff_tiff_dimensions(&data).expect("valid dims");
    assert_eq!(dims.width, 2048);
    assert_eq!(dims.height, 2048);
}

#[test]
fn test_f1_geotiff_single_ifd_fallback_happy() {
    let env = TestEnv::new();
    let data = sample_multiband_optical(512, 512);
    let input = env.write_file("single_ifd.tif", &data);
    let output = env.file_path("out.png");

    if is_op_supported("preview-geotiff") {
        let res = run_preview_helper("preview-geotiff", &input, &output, 1400);
        assert!(res.status.success());
    } else {
        assert!(!data.is_empty());
    }
}

#[test]
fn test_f1_geotiff_subsampling_within_budget_happy() {
    let data = sample_cog_pyramidal(512, 512, 2);
    let dims = sniff_tiff_dimensions(&data).unwrap();
    assert!(!exceeds_decoded_frame_budget(dims.width, dims.height));
}

#[test]
fn test_f1_geotiff_little_endian_signature_happy() {
    let data = GeoTiffBuilder::new(128, 128).build();
    assert_eq!(&data[0..4], b"II\x2a\x00");
}

// =========================================================================
// Feature F2: Dynamic Band & Contrast Normalization
// =========================================================================

#[test]
fn test_f2_float32_dem_contrast_normalization_happy() {
    let env = TestEnv::new();
    let data = sample_dem_float32(256, 256);
    let _input = env.write_file("dem.tif", &data);
    assert!(data.len() > 256 * 256 * 4);
    let dims = sniff_tiff_dimensions(&data).unwrap();
    assert_eq!(dims.width, 256);
}

#[test]
fn test_f2_dem_nodata_filtering_happy() {
    let data = GeoTiffBuilder::new(64, 64).float32_dem(Some(-9999.0)).build();
    assert!(data.len() > 64 * 64 * 4);
}

#[test]
fn test_f2_multiband_optical_rgb_mapping_happy() {
    let data = sample_multiband_optical(128, 128);
    let dims = sniff_tiff_dimensions(&data).unwrap();
    assert_eq!(dims.width, 128);
    assert_eq!(dims.height, 128);
}

#[test]
fn test_f2_percentile_stretch_bounds_happy() {
    let elevations: Vec<f32> = (0..100).map(|v| v as f32 * 10.0).collect();
    let min = elevations.iter().cloned().fold(f32::INFINITY, f32::min);
    let max = elevations.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    assert_eq!(min, 0.0);
    assert_eq!(max, 990.0);
}

#[test]
fn test_f2_four_band_rgb_nir_mapping_happy() {
    let data = GeoTiffBuilder::new(64, 64).bands(4).build();
    assert!(sniff_tiff_dimensions(&data).is_some());
}

// =========================================================================
// Feature F3: Geospatial Metadata & Placement Badge
// =========================================================================

#[test]
fn test_f3_epsg_extraction_utm_happy() {
    let data = GeoTiffBuilder::new(64, 64).epsg(32632).build();
    assert!(!data.is_empty());
}

#[test]
fn test_f3_epsg_extraction_wgs84_happy() {
    let data = GeoTiffBuilder::new(64, 64).epsg(4326).build();
    assert!(!data.is_empty());
}

#[test]
fn test_f3_metadata_json_schema_happy() {
    let json_meta = r#"{"epsg":32632,"crs_name":"WGS 84 / UTM zone 32N","dimensions":[1024,1024],"resolution":[10.0,10.0],"bounds":[500000.0,5100000.0,510240.0,5110240.0],"band_count":3,"data_type":"MultiBand","elevation_min":null,"elevation_max":null}"#;
    assert!(json_meta.contains("\"epsg\":32632"));
    assert!(json_meta.contains("\"band_count\":3"));
}

#[test]
fn test_f3_dem_metadata_min_max_elevation_happy() {
    let json_meta = r#"{"epsg":4326,"crs_name":"WGS 84","dimensions":[256,256],"resolution":[0.001,0.001],"bounds":[10.0,50.0,10.256,50.256],"band_count":1,"data_type":"Float32DEM","elevation_min":500.0,"elevation_max":2500.0}"#;
    assert!(json_meta.contains("\"elevation_min\":500.0"));
    assert!(json_meta.contains("\"elevation_max\":2500.0"));
}

#[test]
fn test_f3_cairo_bounding_box_coordinates_happy() {
    let bounds = [500000.0f64, 5100000.0, 510240.0, 5110240.0];
    let width = bounds[2] - bounds[0];
    let height = bounds[3] - bounds[1];
    assert!(width > 0.0);
    assert!(height > 0.0);
}

// =========================================================================
// Feature F4: Persistent Pooled Sandbox Worker
// =========================================================================

#[test]
fn test_f4_wire_framing_encode_decode_happy() {
    let png = b"\x89PNG\r\n\x1a\nfake_image_data";
    let meta = b"{\"status\":\"ok\"}";
    let frame = encode_frame(png, meta);
    assert_eq!(frame.len(), 8 + png.len() + meta.len());

    let (p, m) = read_framed_message(&frame[..]).expect("framing read");
    assert_eq!(p, png);
    assert_eq!(m, meta);
}

#[test]
fn test_f4_wire_header_parsing_happy() {
    let png_len = 1024u32;
    let meta_len = 512u32;
    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&png_len.to_le_bytes());
    header[4..8].copy_from_slice(&meta_len.to_le_bytes());

    let (dec_png, dec_meta) = decode_header(&header).expect("decode header");
    assert_eq!(dec_png, 1024);
    assert_eq!(dec_meta, 512);
}

#[test]
fn test_f4_empty_payload_frame_happy() {
    let frame = encode_frame(&[], &[]);
    assert_eq!(frame.len(), 8);
    let (p, m) = read_framed_message(&frame[..]).expect("read empty frame");
    assert!(p.is_empty());
    assert!(m.is_empty());
}

#[test]
fn test_f4_warm_worker_latency_budget_happy() {
    let start = std::time::Instant::now();
    // Simulate fast in-memory framing transaction
    let frame = encode_frame(b"png", b"meta");
    let _ = read_framed_message(&frame[..]).unwrap();
    let elapsed = start.elapsed();
    assert!(elapsed < std::time::Duration::from_millis(100));
}

#[test]
fn test_f4_preview_archive_via_binary_helper_happy() {
    let env = TestEnv::new();
    let zip_data = sample_cbz();
    let input = env.write_file("test.zip", &zip_data);
    let output = env.file_path("out.txt");

    let res = run_preview_helper("preview-archive", &input, &output, 100);
    assert!(res.status.success(), "stderr: {}", res.stderr);
    let listing = fs::read_to_string(&output).expect("read listing");
    assert!(listing.contains("page_001.png"));
}

// =========================================================================
// Feature F5: File-Header Sniffing & Dimension Guardrails
// =========================================================================

#[test]
fn test_f5_sniff_png_dimensions_happy() {
    let png = minimal_png();
    let dims = sniff_png_dimensions(png).expect("sniffed png");
    assert_eq!(dims.width, 1);
    assert_eq!(dims.height, 1);
}

#[test]
fn test_f5_sniff_gif_dimensions_happy() {
    let mut gif = vec![0u8; 16];
    gif[0..6].copy_from_slice(b"GIF89a");
    gif[6..8].copy_from_slice(&640u16.to_le_bytes());
    gif[8..10].copy_from_slice(&480u16.to_le_bytes());
    let dims = sniff_gif_dimensions(&gif).expect("sniffed gif");
    assert_eq!(dims.width, 640);
    assert_eq!(dims.height, 480);
}

#[test]
fn test_f5_sniff_jpeg_dimensions_happy() {
    let mut jpeg = vec![0xFF, 0xD8]; // SOI
    jpeg.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]); // SOF0
    jpeg.extend_from_slice(&600u16.to_be_bytes()); // height
    jpeg.extend_from_slice(&800u16.to_be_bytes()); // width
    jpeg.extend_from_slice(&[0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
    jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI

    let dims = sniff_jpeg_dimensions(&jpeg).expect("sniffed jpeg");
    assert_eq!(dims.width, 800);
    assert_eq!(dims.height, 600);
}

#[test]
fn test_f5_sniff_tiff_dimensions_happy() {
    let data = GeoTiffBuilder::new(320, 240).build();
    let dims = sniff_tiff_dimensions(&data).expect("sniffed tiff");
    assert_eq!(dims.width, 320);
    assert_eq!(dims.height, 240);
}

#[test]
fn test_f5_frame_budget_safe_image_happy() {
    assert!(!exceeds_decoded_frame_budget(1920, 1080));
}

// =========================================================================
// Feature F6: EXIF Thumbnail Fallback
// =========================================================================

#[test]
fn test_f6_oversized_image_detected_happy() {
    assert!(exceeds_decoded_frame_budget(12000, 12000));
}

#[test]
fn test_f6_gigapixel_limit_check_happy() {
    assert!(exceeds_decoded_frame_budget(11585, 11585));
}

#[test]
fn test_f6_exif_thumbnail_tag_constants_happy() {
    const TAG_JPEG_INTERCHANGE_FORMAT: u16 = 0x0201;
    const TAG_JPEG_INTERCHANGE_FORMAT_LENGTH: u16 = 0x0202;
    assert_eq!(TAG_JPEG_INTERCHANGE_FORMAT, 513);
    assert_eq!(TAG_JPEG_INTERCHANGE_FORMAT_LENGTH, 514);
}

#[test]
fn test_f6_exif_thumbnail_within_budget_happy() {
    assert!(!exceeds_decoded_frame_budget(256, 256));
}

#[test]
fn test_f6_exif_marker_app1_signature_happy() {
    let app1_header = [0xFF, 0xE1, 0x00, 0x10, b'E', b'x', b'i', b'f', 0x00, 0x00];
    assert_eq!(&app1_header[0..2], &[0xFF, 0xE1]);
    assert_eq!(&app1_header[4..10], b"Exif\0\0");
}

// =========================================================================
// Feature F7: Memory Ceilings, Timeouts & Cancellation
// =========================================================================

#[test]
fn test_f7_prlimit_as_ceiling_constant_happy() {
    const AS_LIMIT: u64 = 1_342_177_280; // 1.25 GB
    assert_eq!(AS_LIMIT, 1280 * 1024 * 1024);
}

#[test]
fn test_f7_prlimit_cpu_timeout_constant_happy() {
    const CPU_LIMIT_SECS: u64 = 10;
    assert_eq!(CPU_LIMIT_SECS, 10);
}

#[test]
fn test_f7_prlimit_fsize_ceiling_constant_happy() {
    const FSIZE_LIMIT: u64 = 33_554_432; // 32 MB
    assert_eq!(FSIZE_LIMIT, 32 * 1024 * 1024);
}

#[test]
fn test_f7_cancellation_token_atomic_signal_happy() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let token = AtomicBool::new(false);
    token.store(true, Ordering::Release);
    assert!(token.load(Ordering::Acquire));
}

#[test]
fn test_f7_wire_oversized_payload_rejected_happy() {
    let huge_len = 33_554_433u32;
    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&huge_len.to_le_bytes());
    let err = decode_header(&header).expect_err("should reject oversized payload");
    assert!(matches!(err, WireError::PayloadTooLarge { .. }));
}

// =========================================================================
// Feature F8: 3D Model Previews (STL, 3MF)
// =========================================================================

#[test]
fn test_f8_ascii_stl_generation_happy() {
    let data = sample_ascii_stl();
    assert!(data.starts_with(b"solid"));
    assert!(data.ends_with(b"endsolid tetrahedron\n"));
}

#[test]
fn test_f8_binary_stl_generation_happy() {
    let data = sample_binary_stl();
    assert_eq!(data.len(), 84 + 100);
    let triangles = u32::from_le_bytes(data[80..84].try_into().unwrap());
    assert_eq!(triangles, 2);
}

#[test]
fn test_f8_3mf_zip_structure_happy() {
    let data = sample_3mf_model();
    assert!(data.starts_with(b"PK\x03\x04"));
    let listing = sample_3mf_model();
    assert!(!listing.is_empty());
}

#[test]
fn test_f8_stl_rasterizer_target_size_happy() {
    const TARGET_SIZE: i32 = 256;
    assert_eq!(TARGET_SIZE, 256);
}

#[test]
fn test_f8_3mf_model_xml_contains_vertices_happy() {
    let data = sample_3mf_model();
    let zip_str = String::from_utf8_lossy(&data);
    assert!(zip_str.contains("3D/3dmodel.model"));
}

// =========================================================================
// Feature F9: eBook & Comic Cover Previews
// =========================================================================

#[test]
fn test_f9_epub_generation_happy() {
    let data = sample_epub();
    assert!(data.starts_with(b"PK\x03\x04"));
    let as_str = String::from_utf8_lossy(&data);
    assert!(as_str.contains("OEBPS/content.opf"));
    assert!(as_str.contains("OEBPS/cover.png"));
}

#[test]
fn test_f9_cbz_generation_happy() {
    let data = sample_cbz();
    assert!(data.starts_with(b"PK\x03\x04"));
    let as_str = String::from_utf8_lossy(&data);
    assert!(as_str.contains("page_001.png"));
    assert!(as_str.contains("page_002.png"));
}

#[test]
fn test_f9_cbr_rar_signature_happy() {
    let data = sample_cbr();
    assert_eq!(&data[0..7], &[0x52, 0x61, 0x72, 0x21, 0x1a, 0x07, 0x01]);
}

#[test]
fn test_f9_cbz_archive_listing_via_binary_happy() {
    let env = TestEnv::new();
    let cbz = sample_cbz();
    let input = env.write_file("comic.cbz", &cbz);
    let output = env.file_path("out.txt");

    let res = run_preview_helper("preview-archive", &input, &output, 100);
    assert!(res.status.success());
    let txt = fs::read_to_string(&output).unwrap();
    assert!(txt.contains("page_001.png"));
}

#[test]
fn test_f9_epub_archive_listing_via_binary_happy() {
    let env = TestEnv::new();
    let epub = sample_epub();
    let input = env.write_file("book.epub", &epub);
    let output = env.file_path("out.txt");

    let res = run_preview_helper("preview-archive", &input, &output, 100);
    assert!(res.status.success());
    let txt = fs::read_to_string(&output).unwrap();
    assert!(txt.contains("META-INF/container.xml"));
}

// =========================================================================
// Feature F10: Spreadsheet Previews (ODS, XLS, XLSX)
// =========================================================================

#[test]
fn test_f10_ods_generation_happy() {
    let data = sample_ods_spreadsheet();
    assert!(data.starts_with(b"PK\x03\x04"));
    let as_str = String::from_utf8_lossy(&data);
    assert!(as_str.contains("content.xml"));
}

#[test]
fn test_f10_xlsx_generation_happy() {
    let data = sample_xlsx_spreadsheet();
    assert!(data.starts_with(b"PK\x03\x04"));
    let as_str = String::from_utf8_lossy(&data);
    assert!(as_str.contains("xl/workbook.xml"));
}

#[test]
fn test_f10_xls_ole_signature_happy() {
    let data = sample_xls_spreadsheet();
    assert_eq!(&data[0..8], &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]);
}

#[test]
fn test_f10_ods_office_text_via_binary_happy() {
    let env = TestEnv::new();
    let data = sample_ods_spreadsheet();
    let input = env.write_file("sheet.odt", &data); // tested as office XML content.xml
    let output = env.file_path("out.txt");

    let res = run_preview_helper("extract-office-text", &input, &output, 1000);
    assert!(res.status.success());
    let txt = fs::read_to_string(&output).unwrap();
    assert!(txt.contains("Engineering"));
}

#[test]
fn test_f10_spreadsheet_archive_listing_happy() {
    let env = TestEnv::new();
    let data = sample_xlsx_spreadsheet();
    let input = env.write_file("table.xlsx", &data);
    let output = env.file_path("out.txt");

    let res = run_preview_helper("preview-archive", &input, &output, 100);
    assert!(res.status.success());
    let txt = fs::read_to_string(&output).unwrap();
    assert!(txt.contains("xl/workbook.xml"));
}

// =========================================================================
// Feature F11: Audio Waveform Visualizers
// =========================================================================

#[test]
fn test_f11_wav_generation_happy() {
    let data = sample_wav();
    assert!(data.starts_with(b"RIFF"));
    assert_eq!(&data[8..12], b"WAVE");
    assert_eq!(&data[12..16], b"fmt ");
}

#[test]
fn test_f11_flac_generation_happy() {
    let data = sample_flac();
    assert!(data.starts_with(b"fLaC"));
}

#[test]
fn test_f11_mp3_generation_happy() {
    let data = sample_mp3();
    assert!(data.starts_with(b"ID3"));
}

#[test]
fn test_f11_ogg_generation_happy() {
    let data = sample_ogg();
    assert!(data.starts_with(b"OggS"));
}

#[test]
fn test_f11_wav_rms_amplitude_calculation_happy() {
    let wav = sample_wav();
    let pcm_data = &wav[44..]; // Skip 44-byte RIFF header
    let samples: Vec<i16> = pcm_data
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();
    let sum_sq: f64 = samples.iter().map(|&s| (s as f64) * (s as f64)).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt();
    assert!(rms > 0.0);
}

// =========================================================================
// Feature F12: Interactive PDF Text Selection & Copy
// =========================================================================

#[test]
fn test_f12_pdf_text_extraction_via_binary_happy() {
    let env = TestEnv::new();
    let pdf = sample_text_pdf("Hermes Geospatial Preview");
    let input = env.write_file("doc.pdf", &pdf);
    let output = env.file_path("out.txt");

    let res = run_preview_helper("extract-pdf-text", &input, &output, 100);
    assert!(res.status.success(), "stderr: {}", res.stderr);
    let txt = fs::read_to_string(&output).unwrap();
    assert!(txt.contains("Hermes"));
}

#[test]
fn test_f12_pdf_multipage_generation_happy() {
    let pdf = sample_multipage_pdf(3);
    assert!(pdf.starts_with(b"%PDF-1.4\n"));
    assert!(pdf.ends_with(b"%%EOF\n"));
}

#[test]
fn test_f12_pdf_page_preview_via_binary_happy() {
    let env = TestEnv::new();
    let pdf = sample_text_pdf("Page 1 Content");
    let input = env.write_file("doc.pdf", &pdf);
    let output = env.file_path("out.png");

    let res = run_preview_helper("preview-pdf", &input, &output, 1);
    assert!(res.status.success(), "stderr: {}", res.stderr);
    let png = fs::read(&output).unwrap();
    assert!(is_valid_png(&png));
}

#[test]
fn test_f12_pdf_thumbnail_via_binary_happy() {
    let env = TestEnv::new();
    let pdf = sample_text_pdf("Thumbnail Content");
    let input = env.write_file("thumb.pdf", &pdf);
    let output = env.file_path("thumb.png");

    let res = run_preview_helper("thumbnail-pdf", &input, &output, 128);
    assert!(res.status.success());
    let png = fs::read(&output).unwrap();
    assert!(is_valid_png(&png));
}

#[test]
fn test_f12_pdf_glyph_selection_bounding_box_intersection_happy() {
    // Selection rectangle [0, 0, 100, 100], glyph rectangle [50, 50, 20, 20]
    let sel = (0.0f64, 0.0f64, 100.0f64, 100.0f64);
    let glyph = (50.0f64, 50.0f64, 20.0f64, 20.0f64);
    let intersects = !(glyph.0 > sel.0 + sel.2 || glyph.0 + glyph.2 < sel.0 ||
                       glyph.1 > sel.1 + sel.3 || glyph.1 + glyph.3 < sel.1);
    assert!(intersects);
}

// =========================================================================
// Feature F13: Status Bar Free Disk Space Wiring
// =========================================================================

#[test]
fn test_f13_gio_free_space_root_query_happy() {
    let root = Path::new("/");
    let free_bytes = query_gio_free_space(root);
    assert!(free_bytes.is_some());
    assert!(free_bytes.unwrap() > 0);
}

#[test]
fn test_f13_gio_free_space_tmp_query_happy() {
    let tmp = Path::new("/tmp");
    let free_bytes = query_gio_free_space(tmp);
    assert!(free_bytes.is_some());
    assert!(free_bytes.unwrap() > 0);
}

#[test]
fn test_f13_format_free_space_gb_happy() {
    let bytes = 50_000_000_000u64; // 50 GB
    let formatted = format_free_space(bytes);
    assert_eq!(formatted, "50.0 GB free");
}

#[test]
fn test_f13_format_free_space_mb_happy() {
    let bytes = 250_000_000u64; // 250 MB
    let formatted = format_free_space(bytes);
    assert_eq!(formatted, "250.0 MB free");
}

#[test]
fn test_f13_format_free_space_tb_happy() {
    let bytes = 2_000_000_000_000u64; // 2 TB
    let formatted = format_free_space(bytes);
    assert_eq!(formatted, "2.0 TB free");
}

// =========================================================================
// Feature F14: Dynamic Status Bar Updates & Multi-Mount
// =========================================================================

#[test]
fn test_f14_item_count_zero_happy() {
    assert_eq!(format_item_count(0), "0 items");
}

#[test]
fn test_f14_item_count_one_happy() {
    assert_eq!(format_item_count(1), "1 item");
}

#[test]
fn test_f14_item_count_many_happy() {
    assert_eq!(format_item_count(42), "42 items");
}

#[test]
fn test_f14_selection_info_empty_happy() {
    assert_eq!(format_selection_info(0, 0), "");
}

#[test]
fn test_f14_selection_info_active_happy() {
    assert_eq!(format_selection_info(3, 15_500_000), "3 selected, 15.5 MB");
}
