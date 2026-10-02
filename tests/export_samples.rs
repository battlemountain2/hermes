// SPDX-License-Identifier: GPL-3.0-or-later

mod fixtures;

use std::fs;
use std::path::PathBuf;

#[test]
fn export_all_user_samples() {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/bry".to_string());
    let base_dir = PathBuf::from(home).join("Hermes-Sample-Files");

    let gis_dir = base_dir.join("1_GIS_GeoTIFF");
    let models_dir = base_dir.join("2_3D_Models");
    let comics_dir = base_dir.join("3_Comics_and_eBooks");
    let sheets_dir = base_dir.join("4_Spreadsheets");
    let audio_dir = base_dir.join("5_Audio_Waveforms");
    let pdf_dir = base_dir.join("6_PDF_Text_Selection");

    for dir in [&gis_dir, &models_dir, &comics_dir, &sheets_dir, &audio_dir, &pdf_dir] {
        fs::create_dir_all(dir).expect("Failed to create sample directory");
    }

    // 1. GIS GeoTIFFs
    fs::write(
        gis_dir.join("elevation_dem.tif"),
        fixtures::geotiff::sample_dem_float32(256, 256),
    ).expect("Failed to write DEM GeoTIFF");

    fs::write(
        gis_dir.join("satellite_optical.tif"),
        fixtures::geotiff::sample_multiband_optical(256, 256),
    ).expect("Failed to write optical GeoTIFF");

    fs::write(
        gis_dir.join("pyramid_cog.tif"),
        fixtures::geotiff::sample_cog_pyramidal(512, 512, 4),
    ).expect("Failed to write COG GeoTIFF");

    // 2. 3D Models
    fs::write(
        models_dir.join("tetrahedron_ascii.stl"),
        fixtures::models::sample_ascii_stl(),
    ).expect("Failed to write ASCII STL");

    fs::write(
        models_dir.join("solid_mesh_binary.stl"),
        fixtures::models::sample_binary_stl(),
    ).expect("Failed to write binary STL");

    fs::write(
        models_dir.join("mechanical_pyramid.3mf"),
        fixtures::models::sample_3mf_model(),
    ).expect("Failed to write 3MF model");

    // 3. Comics and eBooks
    fs::write(
        comics_dir.join("sample_novel.epub"),
        fixtures::archives::sample_epub(),
    ).expect("Failed to write EPUB");

    fs::write(
        comics_dir.join("comic_issue_01.cbz"),
        fixtures::archives::sample_cbz(),
    ).expect("Failed to write CBZ");

    fs::write(
        comics_dir.join("vintage_comic.cbr"),
        fixtures::archives::sample_cbr(),
    ).expect("Failed to write CBR");

    // 4. Spreadsheets
    fs::write(
        sheets_dir.join("quarterly_budget.ods"),
        fixtures::spreadsheets::sample_ods_spreadsheet(),
    ).expect("Failed to write ODS");

    fs::write(
        sheets_dir.join("metrics_report.xlsx"),
        fixtures::spreadsheets::sample_xlsx_spreadsheet(),
    ).expect("Failed to write XLSX");

    let sample_csv = "Item,Category,Price,Quantity,In_Stock\n\
Laptop,Hardware,1299.99,14,true\n\
Mechanical Keyboard,Peripherals,149.50,42,true\n\
4K Monitor,Hardware,499.00,8,true\n\
Desk Mat,Accessories,29.99,105,true\n\
Noise Cancelling Headphones,Audio,299.00,22,true\n";
    fs::write(sheets_dir.join("inventory_catalog.csv"), sample_csv)
        .expect("Failed to write CSV");

    // 5. Audio Waveforms
    fs::write(
        audio_dir.join("test_tone_440hz.wav"),
        fixtures::audio::sample_wav(),
    ).expect("Failed to write WAV");

    fs::write(
        audio_dir.join("sample_track.flac"),
        fixtures::audio::sample_flac(),
    ).expect("Failed to write FLAC");

    // 6. PDF with Selectable Text
    let pdf_text = "Hermes File Manager - Fast & Modern Linux Exploration\n\
\n\
You can click and drag over this text to highlight and select it!\n\
Copy with standard shortcuts (Ctrl+C) directly to your system clipboard.\n\
Enjoy instant preview rendering without loading external viewers.";
    fs::write(
        pdf_dir.join("interactive_document.pdf"),
        fixtures::pdf::sample_text_pdf(pdf_text),
    ).expect("Failed to write text PDF");

    fs::write(
        pdf_dir.join("multi_page_manual.pdf"),
        fixtures::pdf::sample_multipage_pdf(4),
    ).expect("Failed to write multipage PDF");

    println!("All Hermes sample files generated successfully in {}", base_dir.display());
}
