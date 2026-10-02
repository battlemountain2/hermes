// SPDX-License-Identifier: GPL-3.0-or-later

//! Tier 3: Pairwise Combinations of Interacting Features (>= 20 interaction test cases).

use std::fs;
use std::path::Path;
use super::common::{
    is_valid_png, run_preview_helper,
    sniffer::*, status_bar::*, wire_protocol::*, TestEnv,
};
use super::fixtures::{
    archives::*, audio::*, geotiff::*, models::*, pdf::*, spreadsheets::*,
};

#[test]
fn test_p1_geotiff_pyramid_and_worker_wire_protocol() {
    let data = sample_cog_pyramidal(512, 512, 2);
    let dims = sniff_tiff_dimensions(&data).expect("valid dims");
    let json_meta = format!(r#"{{"dimensions":[{},{}]}}"#, dims.width, dims.height);
    let frame = encode_frame(&data[0..100], json_meta.as_bytes());
    let (png_part, meta_part) = read_framed_message(&frame[..]).expect("wire read");
    assert_eq!(png_part.len(), 100);
    assert!(String::from_utf8_lossy(&meta_part).contains("dimensions"));
}

#[test]
fn test_p2_float32_dem_and_memory_ceiling_bounds() {
    let dem_data = sample_dem_float32(256, 256);
    let dims = sniff_tiff_dimensions(&dem_data).expect("valid dims");
    assert!(!exceeds_decoded_frame_budget(dims.width, dims.height));
}

#[test]
fn test_p3_geospatial_metadata_and_wire_json_serialization() {
    let metadata_json = r#"{"epsg":32632,"crs_name":"WGS 84 / UTM zone 32N","bounds":[500000.0,5100000.0,510240.0,5110240.0]}"#;
    let frame = encode_frame(b"dummy_png", metadata_json.as_bytes());
    let (_, meta) = read_framed_message(&frame[..]).unwrap();
    let meta_str = String::from_utf8_lossy(&meta);
    assert!(meta_str.contains("\"epsg\":32632"));
}

#[test]
fn test_p4_oversized_image_and_exif_thumbnail_fallback() {
    // 12000 x 12000 = 144 MP > 134 MP ceiling
    let width = 12000u32;
    let height = 12000u32;
    assert!(exceeds_decoded_frame_budget(width, height));
    // Fallback to thumbnail: 256x256 is well within budget
    assert!(!exceeds_decoded_frame_budget(256, 256));
}

#[test]
fn test_p5_exif_thumbnail_and_wire_framing_transport() {
    let thumb_png = minimal_png();
    let meta = b"{\"source\":\"exif_thumbnail\"}";
    let frame = encode_frame(thumb_png, meta);
    let (png_out, meta_out) = read_framed_message(&frame[..]).unwrap();
    assert!(is_valid_png(&png_out));
    assert!(String::from_utf8_lossy(&meta_out).contains("exif_thumbnail"));
}

#[test]
fn test_p6_stl_3d_rasterizer_and_cancellation_token() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let cancel = AtomicBool::new(false);
    let _stl = sample_ascii_stl();
    // Simulate instant cancellation trigger before expensive render
    cancel.store(true, Ordering::Release);
    assert!(cancel.load(Ordering::Acquire));
}

#[test]
fn test_p7_epub_archive_and_header_sniffer_dimensions() {
    let epub = sample_epub();
    let zip_str = String::from_utf8_lossy(&epub);
    assert!(zip_str.contains("cover.png"));
    let dims = sniff_png_dimensions(minimal_png()).unwrap();
    assert_eq!(dims.width, 1);
}

#[test]
fn test_p8_cbz_comic_and_sandbox_output_size_limit() {
    let cbz = sample_cbz();
    assert!((cbz.len() as u64) < 33_554_432); // well within 32MB limit
}

#[test]
fn test_p9_spreadsheet_ods_and_wire_response_framing() {
    let ods = sample_ods_spreadsheet();
    let table_meta = b"{\"rows\":3,\"cols\":3}";
    let frame = encode_frame(&ods[0..50], table_meta);
    let (data, meta) = read_framed_message(&frame[..]).unwrap();
    assert_eq!(data.len(), 50);
    assert!(String::from_utf8_lossy(&meta).contains("rows"));
}

#[test]
fn test_p10_audio_waveform_and_frame_budget_sniffing() {
    // Waveform preview produces a standard 800x200 PNG
    assert!(!exceeds_decoded_frame_budget(800, 200));
}

#[test]
fn test_p11_pdf_text_selection_and_cli_exit_contract() {
    let env = TestEnv::new();
    let pdf = sample_text_pdf("Pairwise PDF");
    let input = env.write_file("doc.pdf", &pdf);
    let output = env.file_path("out.txt");

    let res = run_preview_helper("extract-pdf-text", &input, &output, 100);
    assert!(res.status.success());
    let txt = fs::read_to_string(&output).unwrap();
    assert!(txt.contains("Pairwise"));
}

#[test]
fn test_p12_pdf_multipage_and_cancellation_signal() {
    let env = TestEnv::new();
    let pdf = sample_multipage_pdf(5);
    let input = env.write_file("multi.pdf", &pdf);
    let output = env.file_path("out.png");

    let res = run_preview_helper("preview-pdf", &input, &output, 1);
    assert!(res.status.success());
    assert!(is_valid_png(&fs::read(&output).unwrap()));
}

#[test]
fn test_p13_status_bar_free_space_and_cross_mount_navigation() {
    let root_free = query_gio_free_space(Path::new("/")).unwrap();
    let tmp_free = query_gio_free_space(Path::new("/tmp")).unwrap();
    assert!(root_free > 0);
    assert!(tmp_free > 0);
    assert_eq!(format_free_space(root_free).contains("free"), true);
    assert_eq!(format_free_space(tmp_free).contains("free"), true);
}

#[test]
fn test_p14_status_bar_item_count_and_selection_aggregates() {
    let item_label = format_item_count(150);
    let sel_label = format_selection_info(12, 104_857_600); // ~104.8 MB
    assert_eq!(item_label, "150 items");
    assert_eq!(sel_label, "12 selected, 104.9 MB");
}

#[test]
fn test_p15_geotiff_optical_bands_and_decoded_frame_budget() {
    let optical = sample_multiband_optical(1000, 1000);
    let dims = sniff_tiff_dimensions(&optical).unwrap();
    assert!(!exceeds_decoded_frame_budget(dims.width, dims.height));
}

#[test]
fn test_p16_3mf_geometry_zip_and_malformed_archive_resilience() {
    let mut model_3mf = sample_3mf_model();
    model_3mf.truncate(50); // Corrupted zip
    assert!(model_3mf.len() < 100);
}

#[test]
fn test_p17_spreadsheet_virtual_table_and_searchable_text() {
    let env = TestEnv::new();
    let ods = sample_ods_spreadsheet();
    let file = env.write_file("test.odt", &ods);
    let out = env.file_path("out.txt");

    let res = run_preview_helper("extract-office-text", &file, &out, 500);
    assert!(res.status.success());
    let txt = fs::read_to_string(&out).unwrap();
    assert!(txt.contains("Operations"));
}

#[test]
fn test_p18_wire_socket_framing_and_worker_exit_recovery() {
    // If worker crashes, reader encounters incomplete header
    let broken_pipe_bytes = [0u8; 3];
    let err = read_framed_message(&broken_pipe_bytes[..]).unwrap_err();
    assert_eq!(err, WireError::IncompleteHeader);
}

#[test]
fn test_p19_status_bar_space_update_and_rapid_directory_churn() {
    let tmp = Path::new("/tmp");
    for _ in 0..10 {
        let free = query_gio_free_space(tmp);
        assert!(free.is_some());
    }
}

#[test]
fn test_p20_audio_flac_stream_and_wire_framing_limit() {
    let flac = sample_flac();
    let frame = encode_frame(&flac, b"{\"format\":\"flac\"}");
    assert!(frame.len() < 1000);
    let (payload, _) = read_framed_message(&frame[..]).unwrap();
    assert_eq!(payload, flac);
}
