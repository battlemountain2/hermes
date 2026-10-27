use super::*;
use crate::services::table::SheetData;

#[test]
fn extract_missing_workbook_returns_error() {
    assert!(extract_spreadsheet_text(Path::new("/nonexistent.xlsx"), 100).is_err());
}

#[test]
fn spreadsheet_text_preserves_sheet_names_rows_empty_cells_and_utf8_limits() {
    let data = SpreadsheetData {
        sheets: vec![
            SheetData {
                name: "Sheet one".into(),
                headers: vec!["Name".into(), "Value".into()],
                rows: vec![vec!["".into(), "🦀".into()]],
                ..SheetData::default()
            },
            SheetData {
                name: "Sheet two".into(),
                headers: vec!["Second".into()],
                rows: vec![vec!["é".into()]],
                ..SheetData::default()
            },
        ],
        ..SpreadsheetData::default()
    };
    let expected = "Sheet one\nName\tValue\n\t🦀\nSheet two\nSecond\né\n";
    assert_eq!(spreadsheet_text(&data, 1024), expected);
    for limit in 0..expected.len() {
        let text = spreadsheet_text(&data, limit);
        assert!(text.len() <= limit);
        assert!(expected.starts_with(&text));
    }
}
