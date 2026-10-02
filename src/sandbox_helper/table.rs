// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use calamine::Reader;
use crate::services::table::SpreadsheetData;

pub(crate) fn render_spreadsheet(path: &Path) -> Result<(Vec<u8>, String), String> {
    let data = SpreadsheetData::read_workbook(path)?;
    let json = data.to_json()?;
    Ok((Vec::new(), json))
}

pub(crate) fn extract_spreadsheet_text(path: &Path, byte_limit: usize) -> Result<Vec<u8>, String> {
    let mut workbook = calamine::open_workbook_auto(path)
        .map_err(|e| format!("Invalid spreadsheet: {e}"))?;

    let mut text = String::new();
    let sheet_names: Vec<String> = workbook.sheet_names();

    for name in sheet_names {
        if let Ok(range) = workbook.worksheet_range(&name) {
            for row in range.rows() {
                let mut first_cell = true;
                for cell in row {
                    let s = cell.to_string();
                    if !s.is_empty() {
                        if !first_cell {
                            text.push(' ');
                        }
                        text.push_str(&s);
                        first_cell = false;
                    }
                    if text.len() >= byte_limit {
                        break;
                    }
                }
                text.push('\n');
                if text.len() >= byte_limit {
                    break;
                }
            }
        }
        if text.len() >= byte_limit {
            break;
        }
    }

    if text.len() > byte_limit {
        let mut end = byte_limit;
        while !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        text.truncate(end);
    }

    Ok(text.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_empty_or_nonexistent_returns_err() {
        assert!(extract_spreadsheet_text(Path::new("/nonexistent.xlsx"), 100).is_err());
    }
}
