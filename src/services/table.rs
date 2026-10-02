// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use calamine::{Data, Reader};
use serde::{Deserialize, Serialize};

pub const WORKBOOK_BYTE_LIMIT: u64 = 20 * 1024 * 1024; // 20 MiB
pub const TABLE_ROW_LIMIT: usize = 200;
pub const TABLE_COLUMN_LIMIT: usize = 256;
pub const TABLE_VALUE_LIMIT: usize = 100_000;
pub const TABLE_TEXT_LIMIT: usize = 4 * 1024 * 1024; // 4 MiB

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpreadsheetData {
    pub sheet_names: Vec<String>,
    pub active_sheet: usize,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: usize,
    pub total_cols: usize,
    pub truncated: bool,
}

impl SpreadsheetData {
    #[expect(dead_code, reason = "JSON deserialization helper")]
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }

    pub fn read_workbook(path: &Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if meta.len() > WORKBOOK_BYTE_LIMIT {
            return Err("Workbook is too large to preview safely".to_string());
        }

        let mut workbook = calamine::open_workbook_auto(path)
            .map_err(|e| format!("Invalid or corrupted spreadsheet: {e}"))?;

        let sheet_names: Vec<String> = workbook.sheet_names();
        if sheet_names.is_empty() {
            return Err("Workbook contains no worksheets".to_string());
        }

        let range = workbook
            .worksheet_range_at(0)
            .ok_or_else(|| "Workbook contains no worksheets".to_string())?
            .map_err(|e| format!("Failed to read worksheet: {e}"))?;

        let total_rows = range.height();
        let total_cols = range.width();

        let mut headers = Vec::new();
        let mut rows = Vec::new();
        let mut truncated = false;

        let mut values_count = 0usize;
        let mut text_bytes = 0usize;

        let mut row_iter = range.rows();
        if let Some(first_row) = row_iter.next() {
            for cell in first_row.iter().take(TABLE_COLUMN_LIMIT) {
                let cell_str = format_cell(cell);
                values_count += 1;
                text_bytes += cell_str.len();
                headers.push(cell_str);
            }
        }

        for row in row_iter {
            if rows.len() >= TABLE_ROW_LIMIT
                || values_count >= TABLE_VALUE_LIMIT
                || text_bytes >= TABLE_TEXT_LIMIT
            {
                truncated = true;
                break;
            }

            let mut row_vec = Vec::new();
            for cell in row.iter().take(TABLE_COLUMN_LIMIT) {
                let cell_str = format_cell(cell);
                values_count += 1;
                text_bytes += cell_str.len();
                row_vec.push(cell_str);
                if values_count >= TABLE_VALUE_LIMIT || text_bytes >= TABLE_TEXT_LIMIT {
                    truncated = true;
                    break;
                }
            }
            rows.push(row_vec);
        }

        if total_rows > rows.len() + 1 || total_cols > TABLE_COLUMN_LIMIT {
            truncated = true;
        }

        Ok(SpreadsheetData {
            sheet_names,
            active_sheet: 0,
            headers,
            rows,
            total_rows,
            total_cols,
            truncated,
        })
    }
}

fn format_cell(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{:.0}", f)
            } else {
                format!("{f}")
            }
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };

        let json = data.to_json().expect("serialize");
        let deserialized = SpreadsheetData::from_json(json.as_bytes()).expect("deserialize");
        assert_eq!(data, deserialized);
    }
}
