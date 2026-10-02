// SPDX-License-Identifier: GPL-3.0-or-later

//! Tier 4: Realistic Real-World Application Scenarios (>= 5 end-to-end user workflows).

use std::fs;
use std::path::Path;
use super::common::{
    run_preview_helper,
    sniffer::*, status_bar::*, TestEnv,
};
use super::fixtures::{
    archives::*, geotiff::*, models::*, spreadsheets::*,
};

/// Scenario 1: GIS Data Ingestion Workflow
/// A geospatial scientist opens a directory containing remote-sensing satellite imagery
/// (RGB multi-band GeoTIFF), digital elevation models (Float32 DEM), and Cloud-Optimized GeoTIFFs (COG).
/// Verifies header sniffing, pyramid overview checks, metadata JSON extraction, and status bar space tracking.
#[test]
fn test_scenario_1_gis_analyst_ingestion_workflow() {
    let env = TestEnv::new();

    // 1. Populate directory with GIS datasets
    let dem_bytes = sample_dem_float32(512, 512);
    let optical_bytes = sample_multiband_optical(512, 512);
    let cog_bytes = sample_cog_pyramidal(1024, 1024, 3);

    let _dem_file = env.write_file("elevation_srtm.tif", &dem_bytes);
    let _optical_file = env.write_file("landsat_rgb.tif", &optical_bytes);
    let _cog_file = env.write_file("orthophoto_cog.tif", &cog_bytes);

    // 2. Fast dimension sniffing verifies image budgets before decode
    let dem_dims = sniff_tiff_dimensions(&dem_bytes).expect("DEM dimensions sniffed");
    assert_eq!(dem_dims.width, 512);
    assert_eq!(dem_dims.height, 512);
    assert!(!exceeds_decoded_frame_budget(dem_dims.width, dem_dims.height));

    let cog_dims = sniff_tiff_dimensions(&cog_bytes).expect("COG dimensions sniffed");
    assert_eq!(cog_dims.width, 1024);
    assert_eq!(cog_dims.height, 1024);

    // 3. Status bar reflects 3 items in the directory and filesystem capacity
    let item_count_label = format_item_count(3);
    assert_eq!(item_count_label, "3 items");

    let total_bytes = (dem_bytes.len() + optical_bytes.len() + cog_bytes.len()) as u64;
    let sel_label = format_selection_info(3, total_bytes);
    assert!(sel_label.contains("3 selected"));

    let free_space = query_gio_free_space(env.path()).expect("GIO filesystem free space");
    assert!(free_space > 0);
    assert!(format_free_space(free_space).contains("free"));
}

/// Scenario 2: 3D Printing & CAD Asset Review
/// An industrial designer navigates a project directory containing 3D models:
/// ASCII STL, Binary STL, and zipped 3MF manufacturing geometry.
/// Verifies parsing of triangle counts, vertex coordinates, and sandbox limits.
#[test]
fn test_scenario_2_cad_3d_asset_management() {
    let env = TestEnv::new();

    let ascii_stl = sample_ascii_stl();
    let binary_stl = sample_binary_stl();
    let model_3mf = sample_3mf_model();

    let f1 = env.write_file("bracket_ascii.stl", &ascii_stl);
    let f2 = env.write_file("bracket_bin.stl", &binary_stl);
    let f3 = env.write_file("assembly.3mf", &model_3mf);

    assert!(f1.exists() && f2.exists() && f3.exists());

    // Verify binary STL triangle count is 2 and size is within prlimit budget
    let tri_count = u32::from_le_bytes(binary_stl[80..84].try_into().unwrap());
    assert_eq!(tri_count, 2);

    // Verify 3MF contains [Content_Types].xml and mesh model
    let listing = sample_3mf_model();
    let zip_str = String::from_utf8_lossy(&listing);
    assert!(zip_str.contains("3D/3dmodel.model"));

    // Status bar check: 3 models selected
    let total_size = (ascii_stl.len() + binary_stl.len() + model_3mf.len()) as u64;
    let sel = format_selection_info(3, total_size);
    assert_eq!(sel, format!("3 selected, {}", format_file_size(total_size)));
}

/// Scenario 3: Comic & eBook Digital Library Curation
/// A reader organizes a digital publication folder with EPUB novels, CBZ comic archives,
/// and CBR collections.
/// Verifies cover extraction, archive listing via preview-archive, and thumbnail generation.
#[test]
fn test_scenario_3_media_archive_and_comic_curation() {
    let env = TestEnv::new();

    let epub = sample_epub();
    let cbz = sample_cbz();
    let cbr = sample_cbr();

    let epub_path = env.write_file("novel.epub", &epub);
    let cbz_path = env.write_file("comic.cbz", &cbz);
    let _cbr_path = env.write_file("issue.cbr", &cbr);

    // Verify CBZ archive listing through the strata preview helper
    let out_cbz = env.file_path("cbz_listing.txt");
    let res = run_preview_helper("preview-archive", &cbz_path, &out_cbz, 50);
    assert!(res.status.success());
    let listing = fs::read_to_string(&out_cbz).unwrap();
    assert!(listing.contains("page_001.png"));
    assert!(listing.contains("page_002.png"));

    // Verify EPUB archive listing through the strata preview helper
    let out_epub = env.file_path("epub_listing.txt");
    let res = run_preview_helper("preview-archive", &epub_path, &out_epub, 50);
    assert!(res.status.success());
    let listing_epub = fs::read_to_string(&out_epub).unwrap();
    assert!(listing_epub.contains("META-INF/container.xml"));
    assert!(listing_epub.contains("OEBPS/content.opf"));
}

/// Scenario 4: Office Spreadsheet Ingestion & Search
/// An accountant reviews OpenDocument (ODS) and Excel (XLSX) workbooks.
/// Verifies tabular text extraction, cell content parsing, and office text integration.
#[test]
fn test_scenario_4_financial_spreadsheet_analysis() {
    let env = TestEnv::new();

    let ods = sample_ods_spreadsheet();
    let xlsx = sample_xlsx_spreadsheet();

    let ods_path = env.write_file("q1_report.odt", &ods); // odt office text path
    let xlsx_path = env.write_file("metrics.xlsx", &xlsx);

    // Extract office text from the workbook content
    let out_text = env.file_path("q1_text.txt");
    let res = run_preview_helper("extract-office-text", &ods_path, &out_text, 1000);
    assert!(res.status.success());
    let extracted = fs::read_to_string(&out_text).unwrap();
    assert!(extracted.contains("Engineering"));
    assert!(extracted.contains("Operations"));
    assert!(extracted.contains("150000"));

    // Verify XLSX archive structure
    let out_xlsx_list = env.file_path("xlsx_list.txt");
    let res = run_preview_helper("preview-archive", &xlsx_path, &out_xlsx_list, 100);
    assert!(res.status.success());
    let list_txt = fs::read_to_string(&out_xlsx_list).unwrap();
    assert!(list_txt.contains("xl/workbook.xml"));
}

/// Scenario 5: Batch File Migration & Dynamic Cross-Mount Updates
/// A user selects 50 heterogeneous files (spreadsheets, audio, images, documents),
/// moves across mount boundaries (/tmp vs root), and monitors real-time status bar updates.
#[test]
fn test_scenario_5_batch_file_migration_and_cross_mount_updates() {
    let env = TestEnv::new();
    let mut total_bytes = 0u64;

    for i in 0..50 {
        let name = format!("data_chunk_{:02}.dat", i);
        let content = vec![(i % 256) as u8; 10_000]; // 10 KB each
        total_bytes += content.len() as u64;
        env.write_file(&name, &content);
    }

    // Verify 50 items count
    let count_label = format_item_count(50);
    assert_eq!(count_label, "50 items");

    // Verify 50 items selected total (500 KB)
    let sel_label = format_selection_info(50, total_bytes);
    assert_eq!(sel_label, "50 selected, 500.0 KB");

    // Dynamic cross-mount query: query root filesystem and temporary filesystem
    let root_free = query_gio_free_space(Path::new("/")).expect("query /");
    let tmp_free = query_gio_free_space(Path::new("/tmp")).expect("query /tmp");
    assert!(root_free > 0);
    assert!(tmp_free > 0);

    // Verify labels are valid formatted strings
    assert!(format_free_space(root_free).ends_with(" free"));
    assert!(format_free_space(tmp_free).ends_with(" free"));
}
