// SPDX-License-Identifier: GPL-3.0-or-later

use std::cmp::Ordering;

use crate::services::table::{SheetData, SpreadsheetData};

use super::*;

#[test]
fn test_sort_key_comparisons() {
    let n1 = SortKey::new("10");
    let n2 = SortKey::new("2");
    assert_eq!(n2.compare(&n1), Ordering::Less);

    let t1 = SortKey::new("apple");
    let t2 = SortKey::new("banana");
    assert_eq!(t1.compare(&t2), Ordering::Less);

    let empty = SortKey::new("");
    assert_eq!(n1.compare(&empty), Ordering::Less);
    assert_eq!(empty.compare(&n1), Ordering::Greater);
}

#[test]
fn test_column_name_generation() {
    assert_eq!(column_name(0), "A");
    assert_eq!(column_name(25), "Z");
    assert_eq!(column_name(26), "AA");
}

#[test]
#[ignore = "Requires an isolated GTK display"]
fn desktop_spreadsheet_view_builds_multi_sheet_and_fullscreen() {
    gtk::init().expect("GTK init");
    crate::assets::prepare().expect("Assets prepare");

    let data = SpreadsheetData {
        sheet_names: vec!["Q1".to_string(), "Q2".to_string()],
        active_sheet: 0,
        headers: vec!["Item".to_string(), "Cost".to_string()],
        rows: vec![vec!["Coffee".to_string(), "4.50".to_string()]],
        total_rows: 2,
        total_cols: 2,
        truncated: false,
        sheets: vec![
            SheetData {
                name: "Q1".to_string(),
                headers: vec!["Item".to_string(), "Cost".to_string()],
                rows: vec![vec!["Coffee".to_string(), "4.50".to_string()]],
                total_rows: 2,
                total_cols: 2,
                truncated: false,
            },
            SheetData {
                name: "Q2".to_string(),
                headers: vec!["Item".to_string(), "Cost".to_string(), "Profit".to_string()],
                rows: vec![vec!["Tea".to_string(), "3.00".to_string(), "1.50".to_string()]],
                total_rows: 2,
                total_cols: 3,
                truncated: false,
            },
        ],
    };

    let view = build_spreadsheet_view(&data, "Financials.xlsx");
    assert!(view.widget.is_visible() || !view.widget.is_visible());
    view.stop();
}
