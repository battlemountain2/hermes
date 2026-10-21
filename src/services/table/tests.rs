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

struct WorkbookFixture(std::path::PathBuf);

impl WorkbookFixture {
    fn new(extra: &[(&str, &str)]) -> Self {
        use std::io::Write;
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "hermes-table-{}-{}.ods",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("new fixture");
        let mut zip = zip::ZipWriter::new(file);
        let members = [
            (
                "_rels/.rels",
                r#"<Relationships><Relationship Id="doc" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Summary" sheetId="2" r:id="summary"/><sheet name="Data" sheetId="1" r:id="data"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="data" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="summary" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/></Relationships>"#,
            ),
            (
                "xl/sharedStrings.xml",
                r#"<sst uniqueCount="2"><si><t>Amount</t></si><si><r><t>Hel</t></r><r><t>lo</t></r></si></sst>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Name</t></is></c><c r="B1" t="s"><v>0</v></c><c r="C1" t="inlineStr"><is><t>Flag</t></is></c></row><row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2"><f>1+2</f><v>3</v></c><c r="C2" t="b"><v>1</v></c></row><row r="3"><c r="A3" t="e"><v>#DIV/0!</v></c><c r="B3"><v>-1.5</v></c></row></sheetData></worksheet>"#,
            ),
            (
                "xl/worksheets/sheet2.xml",
                r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Total</t></is></c></row><row r="2"><c r="A2"><v>42</v></c></row></sheetData></worksheet>"#,
            ),
        ];
        for (name, xml) in members
            .iter()
            .filter(|(name, _)| !extra.iter().any(|(extra_name, _)| extra_name == name))
            .chain(extra)
        {
            zip.start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .expect("member");
            zip.write_all(xml.as_bytes()).expect("XML");
        }
        zip.finish().expect("workbook ZIP");
        Self(path)
    }
}

impl Drop for WorkbookFixture {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).expect("remove workbook fixture");
    }
}

#[test]
fn xlsx_structure_wins_over_extension_and_anonymous_fd_preserves_sheet_order() {
    use std::os::fd::AsRawFd;
    let fixture = WorkbookFixture::new(&[]);
    let data = SpreadsheetData::read_workbook(&fixture.0).expect("XLSX named ODS");
    let file = File::open(&fixture.0).expect("fixture");
    let descriptor = format!("/proc/self/fd/{}", file.as_raw_fd());
    assert_eq!(
        SpreadsheetData::read_workbook(Path::new(&descriptor)).expect("anonymous FD"),
        data
    );
    assert_eq!(data.sheet_names, ["Summary", "Data"]);
    assert_eq!(data.headers, ["Total"]);
    assert_eq!(data.rows, [vec!["42"]]);
    assert_eq!(data.sheets[1].headers, ["Name", "Amount", "Flag"]);
    assert_eq!(
        data.sheets[1].rows,
        [vec!["Hello", "3", "true"], vec!["#DIV/0!", "-1.5", ""]]
    );
    assert!(!data.truncated);
    assert_workbook_budget(&data);
    let text = crate::sandbox_helper::table::extract_spreadsheet_text(Path::new(&descriptor), 1000)
        .expect("text extraction");
    assert_eq!(
        String::from_utf8(text).expect("UTF-8"),
        "Summary\nTotal\n42\nData\nName\tAmount\tFlag\nHello\t3\ttrue\n#DIV/0!\t-1.5\t\n"
    );
    for limit in 0..80 {
        assert!(
            crate::sandbox_helper::table::extract_spreadsheet_text(&fixture.0, limit)
                .expect("bounded extraction")
                .len()
                <= limit
        );
    }
}

#[test]
fn xlsx_external_links_are_rejected_before_calamine_and_text_extraction() {
    let fixture = WorkbookFixture::new(&[(
        "xl/externalLinks/_rels/externalLink1.xml.rels",
        r#"<Relationships><Relationship Id="link" TargetMode="External" Target="file:///outside.xlsx"/></Relationships>"#,
    )]);
    assert!(
        SpreadsheetData::read_workbook(&fixture.0)
            .expect_err("external workbook")
            .contains("External")
    );
    assert!(crate::sandbox_helper::table::extract_spreadsheet_text(&fixture.0, 100).is_err());
}

#[test]
fn xlsx_root_relationship_selects_relocated_workbook_and_worksheet_parts() {
    let fixture = WorkbookFixture::new(&[
        (
            "_rels/.rels",
            "<Relationships><Relationship Id=\"doc\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"custom/workbook.xml\"/></Relationships>",
        ),
        (
            "custom/workbook.xml",
            "<workbook xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Relocated\" sheetId=\"1\" r:id=\"sheet\"/></sheets></workbook>",
        ),
        (
            "custom/_rels/workbook.xml.rels",
            "<Relationships><Relationship Id=\"sheet\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"/parts/sheet.part\"/></Relationships>",
        ),
        (
            "parts/sheet.part",
            "<worksheet><sheetData><row><c t=\"inlineStr\"><is><t>Relocated heading</t></is></c></row><row><c><v>57</v></c></row></sheetData></worksheet>",
        ),
    ]);
    let data = SpreadsheetData::read_workbook(&fixture.0).unwrap();
    assert_eq!(data.sheet_names, ["Relocated"]);
    assert_eq!(data.headers, ["Relocated heading"]);
    assert_eq!(data.rows, [vec!["57"]]);
}

#[test]
fn xlsx_external_hyperlinks_keep_cell_text_without_reading_the_target() {
    let fixture = WorkbookFixture::new(&[
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Label</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>Visible link</t></is></c></row></sheetData><hyperlinks><hyperlink ref="A2" r:id="link"/></hyperlinks></worksheet>"#,
        ),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            r#"<Relationships><Relationship Id="link" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External" Target="file:///hermes-generated-nonexistent-hyperlink-target.xlsx"/></Relationships>"#,
        ),
    ]);
    let data = SpreadsheetData::read_workbook(&fixture.0).expect("hyperlinked workbook");
    assert_eq!(data.sheets[1].headers, ["Label"]);
    assert_eq!(data.sheets[1].rows, [vec!["Visible link"]]);
    let text = crate::sandbox_helper::table::extract_spreadsheet_text(&fixture.0, 1024)
        .expect("text without target resolution");
    assert_eq!(
        String::from_utf8(text).expect("UTF-8"),
        "Summary\nTotal\n42\nData\nLabel\nVisible link\n"
    );
}

#[test]
fn spreadsheet_errors_have_one_excel_error_prefix() {
    for error in [
        calamine::CellErrorType::Div0,
        calamine::CellErrorType::NA,
        calamine::CellErrorType::Ref,
    ] {
        assert_eq!(format_cell(&Data::Error(error.clone())), error.to_string());
    }
}
