// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use crate::services::table::{SpreadsheetData, TABLE_TEXT_LIMIT};

pub(crate) fn render_spreadsheet(path: &Path) -> Result<(Vec<u8>, String), String> {
    let data = SpreadsheetData::read_workbook(path)?;
    let json = data.to_json()?;
    Ok((Vec::new(), json))
}

pub(crate) fn extract_spreadsheet_text(path: &Path, byte_limit: usize) -> Result<Vec<u8>, String> {
    let data = SpreadsheetData::read_workbook(path)?;
    let text = spreadsheet_text(&data, byte_limit.min(TABLE_TEXT_LIMIT));
    Ok(text.into_bytes())
}

fn spreadsheet_text(data: &SpreadsheetData, limit: usize) -> String {
    let mut text = String::new();
    for sheet in &data.sheets {
        for fragment in [sheet.name.as_str(), "\n"] {
            if !append_text(&mut text, fragment, limit) {
                return text;
            }
        }
        for row in std::iter::once(&sheet.headers).chain(&sheet.rows) {
            for (index, cell) in row.iter().enumerate() {
                if index > 0 && !append_text(&mut text, "\t", limit) {
                    return text;
                }
                if !append_text(&mut text, cell, limit) {
                    return text;
                }
            }
            if !append_text(&mut text, "\n", limit) {
                return text;
            }
        }
    }
    text
}

fn append_text(output: &mut String, fragment: &str, limit: usize) -> bool {
    let mut end = fragment.len().min(limit.saturating_sub(output.len()));
    while !fragment.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&fragment[..end]);
    end == fragment.len() && output.len() < limit
}

#[cfg(test)]
mod tests;
