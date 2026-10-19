use super::*;

fn range_with_cells(rows: u32, columns: u32, value: &str) -> calamine::Range<Data> {
    let mut range = calamine::Range::new((0, 0), (rows - 1, columns - 1));
    for row in 0..rows {
        for column in 0..columns {
            range.set_value((row, column), Data::String(value.to_string()));
        }
    }
    range
}

fn assert_workbook_budget(data: &SpreadsheetData) {
    let mut values = 0;
    let mut text_bytes = 0;
    let mut serialized_bytes = 0;
    for (index, sheet) in data.sheets.iter().enumerate() {
        let copies = if index == 0 { 2 } else { 1 };
        for text in sheet.headers.iter().chain(sheet.rows.iter().flatten()) {
            values += copies;
            text_bytes += text.len() * copies;
            serialized_bytes +=
                (serde_json::to_string(text).expect("cell JSON").len() + 1) * copies;
        }
    }
    assert!(values <= TABLE_VALUE_LIMIT, "{values} values");
    assert!(text_bytes <= TABLE_TEXT_LIMIT, "{text_bytes} text bytes");
    assert!(
        serialized_bytes <= TABLE_SERIALIZED_TEXT_LIMIT,
        "{serialized_bytes} JSON bytes"
    );
    let json = data.to_json().expect("bounded workbook JSON");
    assert!(json.len() <= TABLE_SERIALIZED_LIMIT);
    assert!(json.len() < 32 * 1024 * 1024);
    assert_eq!(
        SpreadsheetData::from_json(json.as_bytes()).expect("roundtrip"),
        *data
    );
    assert_eq!(data.headers, data.sheets[0].headers);
    assert_eq!(data.rows, data.sheets[0].rows);
    assert_eq!(data.total_rows, data.sheets[0].total_rows);
    assert_eq!(data.total_cols, data.sheets[0].total_cols);
}

#[test]
fn test_spreadsheet_data_json_roundtrip() {
    let data = SpreadsheetData {
        sheet_names: vec!["Sheet1".to_string()],
        active_sheet: 0,
        headers: vec!["Name".to_string(), "Amount".to_string()],
        rows: vec![vec!["Alice".to_string(), "100".to_string()]],
        total_rows: 2,
        total_cols: 2,
        truncated: false,
        sheets: vec![SheetData {
            name: "Sheet1".to_string(),
            headers: vec!["Name".to_string(), "Amount".to_string()],
            rows: vec![vec!["Alice".to_string(), "100".to_string()]],
            total_rows: 2,
            total_cols: 2,
            truncated: false,
        }],
    };
    assert_workbook_budget(&data);
}

#[test]
fn legacy_json_without_sheets_remains_readable() {
    let data = SpreadsheetData::from_json(
        br#"{"sheet_names":["Sheet1"],"active_sheet":0,"headers":["Name"],"rows":[["Alice"]],"total_rows":2,"total_cols":1,"truncated":false}"#,
    ).expect("legacy JSON");
    assert!(data.sheets.is_empty());
    assert_eq!(data.headers, ["Name"]);
    assert_eq!(data.rows, [vec!["Alice".to_string()]]);
}

#[test]
fn synthetic_multisheet_text_limits_preserve_sixteen_useful_previews() {
    let names = (0..TABLE_SHEET_LIMIT)
        .map(|index| format!("Sheet {index}"))
        .collect();
    let long_text = "\u{01}\"\\\n\té🦀".repeat(100_000);
    let data = SpreadsheetData::preview_workbook(names, |index| {
        let mut range = range_with_cells(3, 2, "Header");
        range.set_value((1, 0), Data::String(format!("Sheet {index}: {long_text}")));
        Some(range)
    })
    .expect("synthetic workbook");

    assert_eq!(data.sheets.len(), TABLE_SHEET_LIMIT);
    assert_eq!(data.sheet_names.len(), TABLE_SHEET_LIMIT);
    assert!(data.truncated);
    for (index, sheet) in data.sheets.iter().enumerate() {
        assert_eq!(sheet.headers, ["Header", "Header"]);
        assert!(sheet.rows[0][0].starts_with(&format!("Sheet {index}: ")));
        assert_eq!((sheet.total_rows, sheet.total_cols), (3, 2));
        assert!(sheet.truncated);
    }
    assert_workbook_budget(&data);
}

#[test]
fn synthetic_multisheet_empty_cells_share_the_value_budget() {
    let names = (0..TABLE_SHEET_LIMIT)
        .map(|index| format!("Sheet {index}"))
        .collect();
    let data = SpreadsheetData::preview_workbook(names, |_| {
        Some(calamine::Range::new((0, 0), (201, 299)))
    })
    .expect("synthetic workbook");

    assert!(data.truncated);
    assert_eq!(data.sheets.len(), TABLE_SHEET_LIMIT);
    let mut values = data.headers.len() + data.rows.iter().map(Vec::len).sum::<usize>();
    for sheet in &data.sheets {
        assert_eq!(sheet.headers.len(), TABLE_COLUMN_LIMIT);
        assert!(!sheet.rows.is_empty());
        assert!(sheet.rows.iter().all(|row| row.len() <= TABLE_COLUMN_LIMIT));
        assert_eq!((sheet.total_rows, sheet.total_cols), (202, 300));
        assert!(sheet.truncated);
        values += sheet.headers.len() + sheet.rows.iter().map(Vec::len).sum::<usize>();
    }
    assert_eq!(values, TABLE_VALUE_LIMIT);
    assert_workbook_budget(&data);
}

#[test]
fn oversized_escaped_header_is_bounded_before_insertion_and_duplication() {
    let data = SpreadsheetData::preview_workbook(vec!["Sheet1".to_string()], |_| {
        let mut range = range_with_cells(2, 2, "Remaining");
        range.set_value((0, 0), Data::String("\u{01}".repeat(TABLE_TEXT_LIMIT)));
        Some(range)
    })
    .expect("oversized header");

    assert!(data.truncated);
    assert_eq!(data.headers.len(), 1);
    assert!(!data.headers[0].is_empty());
    assert!(data.headers[0].len() < TABLE_TEXT_LIMIT / 2);
    assert!(data.rows.is_empty());
    assert_eq!((data.total_rows, data.total_cols), (2, 2));
    assert_workbook_budget(&data);
}

#[test]
fn serialized_prefix_matches_json_escaping_and_preserves_utf8() {
    let mut text: String = ('\u{00}'..='\u{1f}').collect();
    text.push_str("\"\\/é🦀");
    let json = serde_json::to_string(&text).expect("escaped JSON");
    assert_eq!(
        text_prefix(&text, usize::MAX, usize::MAX),
        (text.len(), json.len() - 2)
    );
    for limit in 0..json.len() {
        let (end, encoded) = text_prefix(&text, limit, limit);
        assert!(text.is_char_boundary(end));
        assert!(end <= limit && encoded <= limit);
        assert_eq!(
            serde_json::to_string(&text[..end])
                .expect("prefix JSON")
                .len(),
            encoded + 2
        );
    }

    let mut budget = PreviewBudget {
        values: 1,
        text_bytes: 5,
        serialized_bytes: 8,
    };
    assert_eq!(
        budget.cell(&Data::String("é🦀".to_string())),
        Some(("é".to_string(), true))
    );
    assert_eq!(
        (budget.values, budget.text_bytes, budget.serialized_bytes),
        (0, 3, 3)
    );
    assert_eq!(budget.cell(&Data::Empty), None);
}

#[test]
fn row_and_column_limits_report_only_actual_omissions() {
    for (rows, columns, truncated) in [
        (201, 2, false),
        (202, 2, true),
        (2, 256, false),
        (2, 257, true),
    ] {
        let data = SpreadsheetData::preview_workbook(vec!["Sheet1".to_string()], |_| {
            Some(range_with_cells(rows, columns, "Value"))
        })
        .expect("bounded dimensions");
        assert_eq!(data.truncated, truncated);
        assert_eq!(data.sheets[0].truncated, truncated);
        assert_eq!(data.rows.len(), (rows as usize - 1).min(TABLE_ROW_LIMIT));
        assert_eq!(
            data.headers.len(),
            (columns as usize).min(TABLE_COLUMN_LIMIT)
        );
        assert_workbook_budget(&data);
    }
}

#[test]
fn omitted_unreadable_sheets_and_bounded_names_report_truncation() {
    let names = (0..TABLE_SHEET_LIMIT + 2)
        .map(|index| format!("{index}{}", "🦀".repeat(1024)))
        .collect();
    let mut requested = Vec::new();
    let data = SpreadsheetData::preview_workbook(names, |index| {
        requested.push(index);
        (index != 0).then(|| range_with_cells(2, 1, &format!("Value {index}")))
    })
    .expect("readable worksheets");

    assert_eq!(requested, (0..TABLE_SHEET_LIMIT).collect::<Vec<_>>());
    assert_eq!(data.sheets.len(), TABLE_SHEET_LIMIT - 1);
    assert!(data.truncated);
    assert_eq!(data.headers, ["Value 1"]);
    for (name, sheet) in data.sheet_names.iter().zip(&data.sheets) {
        assert_eq!(name, &sheet.name);
        assert!(name.len() <= TABLE_SHEET_NAME_LIMIT);
        assert!(sheet.truncated);
    }
    assert_workbook_budget(&data);
}

#[test]
fn sheet_count_omission_alone_marks_the_workbook_truncated() {
    let names = (0..TABLE_SHEET_LIMIT + 1)
        .map(|index| format!("Sheet {index}"))
        .collect();
    let data = SpreadsheetData::preview_workbook(names, |_| Some(range_with_cells(2, 1, "Value")))
        .expect("limited sheets");
    assert!(data.truncated);
    assert_eq!(data.sheets.len(), TABLE_SHEET_LIMIT);
    assert!(data.sheets.iter().all(|sheet| !sheet.truncated));
    assert_workbook_budget(&data);
}

#[test]
fn empty_workbooks_and_unreadable_worksheets_are_rejected() {
    assert!(SpreadsheetData::preview_workbook(Vec::new(), |_| panic!("no worksheets")).is_err());
    assert!(SpreadsheetData::preview_workbook(vec!["Broken".to_string()], |_| None).is_err());
    let data = SpreadsheetData::preview_workbook(vec!["Empty".to_string()], |_| {
        Some(calamine::Range::empty())
    })
    .expect("empty worksheet");
    assert!(!data.truncated);
    assert!(data.headers.is_empty() && data.rows.is_empty());
    assert_workbook_budget(&data);
}

#[test]
fn serialization_rejects_unbudgeted_oversized_data() {
    let data = SpreadsheetData {
        headers: vec!["x".repeat(TABLE_SERIALIZED_LIMIT)],
        ..SpreadsheetData::default()
    };
    assert!(data.to_json().is_err());
}
