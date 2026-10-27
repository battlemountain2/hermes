use std::{fs, path::PathBuf};

use gdk_pixbuf::prelude::*;

use crate::sandbox::{Cancellation, ParseOperation, ParseOutput};
use crate::services::table::SpreadsheetData;

fn fixtures() -> PathBuf {
    PathBuf::from(
        std::env::var_os("HERMES_OFFICE_FIXTURE_DIR")
            .expect("Generate synthetic fixtures with tests/fixtures/office/generate.py"),
    )
}

fn save_shot(name: &str, output: &ParseOutput) {
    if let Some(directory) = std::env::var_os("HERMES_OFFICE_TEST_SHOTS") {
        fs::create_dir_all(&directory).unwrap();
        fs::write(PathBuf::from(directory).join(name), &output.data).unwrap();
    }
}

fn assert_color(png: &[u8], expected: [u8; 3]) {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(png).unwrap();
    loader.close().unwrap();
    let image = loader.pixbuf().unwrap();
    let pixels = image.read_pixel_bytes();
    let channels = image.n_channels() as usize;
    let stride = image.rowstride() as usize;
    let mut matches = 0;
    for vertical in 0..image.height() as usize {
        for horizontal in 0..image.width() as usize {
            let offset = vertical * stride + horizontal * channels;
            if pixels[offset..offset + 3]
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual.abs_diff(expected) <= 8)
            {
                matches += 1;
            }
        }
    }
    assert!(
        matches > 100,
        "Expected visible original color {expected:?}"
    );
}

#[test]
#[ignore = "Generated XLSX/DOCX fixtures, built helper and installed system LibreOffice; no user documents"]
fn formatted_workbook_layout_preserves_styles_merged_cells_and_second_sheet_navigation() {
    assert!(
        crate::sandbox::office::available(),
        "Install system LibreOffice"
    );
    let input = fixtures().join("layout.xlsx");
    let original = fs::read(&input).unwrap();
    let cancellation = Cancellation::default();
    let first = crate::sandbox::office::render(&input, 0, &cancellation).unwrap();
    assert!(first.pages >= 2);
    assert_eq!((first.page, first.pages), (0, 2));
    let first_layer = first.text_layer.as_ref().expect("Selectable workbook text");
    save_shot("layout.xlsx-1.png", &first);
    for expected in [
        "Item",
        "Count",
        "Status",
        "Generated images",
        "Selectable cells",
        "Field station totals",
        "Merged summary",
        "Aspen",
        "Cedar",
    ] {
        assert!(
            first_layer.text.contains(expected),
            "Missing {expected}: {}",
            first_layer.text
        );
    }
    assert!(!first_layer.text.contains("Details page only"));
    assert_eq!(first_layer.glyphs.len(), first_layer.text.chars().count());
    assert!(!first_layer.glyphs.is_empty());
    assert!(first_layer.width > first_layer.height);
    for color in [[36, 70, 105], [39, 125, 91], [244, 221, 121]] {
        assert_color(&first.data, color);
    }
    save_shot("layout.xlsx-1.png", &first);

    for page in [1, 0, 1] {
        let output = crate::sandbox::office::render(&input, page, &cancellation).unwrap();
        assert_eq!((output.page, output.pages), (page, 2));
        let layer = output.text_layer.as_ref().unwrap();
        let expected = if page == 0 {
            "Merged summary"
        } else {
            "Details page only"
        };
        assert!(layer.text.contains(expected), "{}", layer.text);
        assert!(!layer.glyphs.is_empty());
        if page == 1 {
            assert!(layer.text.contains("Another sheet"));
            assert!(layer.text.contains("Keeps navigation"));
            assert!(!layer.text.contains("Field station totals"));
            assert!(layer.width < layer.height);
            assert_ne!(first.data, output.data);
            save_shot("layout.xlsx-2.png", &output);
        }
    }

    let document =
        crate::sandbox::office::render(&fixtures().join("layout.docx"), 0, &cancellation).unwrap();
    let document_layer = document.text_layer.as_ref().unwrap();
    assert!(document_layer.width < document_layer.height);
    assert!(
        first_layer.width / first_layer.height > document_layer.width / document_layer.height * 1.5
    );

    let table = crate::sandbox::parse(&input, ParseOperation::PreviewSpreadsheet, 0, &cancellation)
        .unwrap();
    let table = SpreadsheetData::from_json(table.metadata.as_ref().unwrap()).unwrap();
    assert_eq!(table.sheet_names, ["Overview", "Details"]);
    assert_eq!(table.headers, ["Item", "Count", "Status"]);
    assert_eq!(table.rows[0], ["Generated images", "3", "Ready"]);
    assert_eq!(table.rows[1], ["Selectable cells", "12", "Checked"]);
    assert_eq!(
        table.sheets[1].headers[..2],
        ["Another sheet", "Keeps navigation"]
    );
    assert_eq!(original, fs::read(&input).unwrap());
}

#[test]
#[ignore = "Generated 225-row workbook, built helper and installed system LibreOffice; no user documents"]
fn full_workbook_layout_includes_printed_rows_beyond_bounded_data_preview() {
    assert!(
        crate::sandbox::office::available(),
        "Install system LibreOffice"
    );
    let input = fixtures().join("full-workbook.xlsx");
    let original = fs::read(&input).unwrap();
    let cancellation = Cancellation::default();
    let table = crate::sandbox::parse(&input, ParseOperation::PreviewSpreadsheet, 0, &cancellation)
        .unwrap();
    let table = SpreadsheetData::from_json(table.metadata.as_ref().unwrap()).unwrap();
    assert!(table.truncated);
    assert_eq!(table.rows.len(), crate::services::table::TABLE_ROW_LIMIT);
    assert!(
        !table
            .rows
            .iter()
            .flatten()
            .any(|cell| cell.contains("Beyond row 200 sentinel"))
    );
    let first = crate::sandbox::office::render(&input, 0, &cancellation).unwrap();
    assert!(first.pages >= 2);
    let first_layer = first.text_layer.as_ref().unwrap();
    assert!(first_layer.text.contains("Full workbook first page"));
    assert!(!first_layer.text.contains("Beyond row 200 sentinel"));
    let last = crate::sandbox::office::render(&input, first.pages - 1, &cancellation).unwrap();
    assert_eq!(last.pages, first.pages);
    assert_eq!(last.page, first.pages - 1);
    let last_layer = last.text_layer.as_ref().unwrap();
    assert!(
        last_layer.text.contains("Beyond row 200 sentinel"),
        "{}",
        last_layer.text
    );
    assert!(!last_layer.glyphs.is_empty());
    assert_ne!(first.data, last.data);
    save_shot("full-workbook.xlsx-1.png", &first);
    save_shot("full-workbook.xlsx-last.png", &last);
    assert_eq!(original, fs::read(&input).unwrap());
}
