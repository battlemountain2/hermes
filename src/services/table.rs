// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    borrow::Cow,
    fs::File,
    io::{BufReader, Read, Seek},
    path::Path,
};

use calamine::{Data, Reader};
use serde::{Deserialize, Serialize};

pub const WORKBOOK_BYTE_LIMIT: u64 = 20 * 1024 * 1024; // 20 MiB
pub const TABLE_ROW_LIMIT: usize = 200;
pub const TABLE_COLUMN_LIMIT: usize = 256;
pub const TABLE_VALUE_LIMIT: usize = 100_000;
pub const TABLE_TEXT_LIMIT: usize = 4 * 1024 * 1024; // 4 MiB

pub const TABLE_SHEET_LIMIT: usize = 16;
pub const TABLE_SERIALIZED_LIMIT: usize = 28 * 1024 * 1024;
const TABLE_SERIALIZED_TEXT_LIMIT: usize = 24 * 1024 * 1024;
const TABLE_SHEET_NAME_LIMIT: usize = 1024;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SheetData {
    pub name: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: usize,
    pub total_cols: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpreadsheetData {
    pub sheet_names: Vec<String>,
    pub active_sheet: usize,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub total_rows: usize,
    pub total_cols: usize,
    pub truncated: bool,
    #[serde(default)]
    pub sheets: Vec<SheetData>,
}

impl SpreadsheetData {
    #[cfg_attr(not(test), expect(dead_code, reason = "JSON deserialization helper"))]
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> Result<String, String> {
        let json = serde_json::to_string(self).map_err(|error| error.to_string())?;
        if json.len() > TABLE_SERIALIZED_LIMIT {
            return Err("Spreadsheet preview exceeds the serialized size limit".to_string());
        }
        Ok(json)
    }

    pub fn read_workbook(path: &Path) -> Result<Self, String> {
        let mut workbook = open_workbook(path)?;

        let sheet_names: Vec<String> = workbook.sheet_names();
        Self::preview_workbook(sheet_names, |idx| {
            workbook.worksheet_range_at(idx).and_then(Result::ok)
        })
    }

    fn preview_workbook(
        sheet_names: Vec<String>,
        mut read_sheet: impl FnMut(usize) -> Option<calamine::Range<Data>>,
    ) -> Result<Self, String> {
        if sheet_names.is_empty() {
            return Err("Workbook contains no worksheets".to_string());
        }

        let mut sheets = Vec::new();
        let mut overall_truncated = sheet_names.len() > TABLE_SHEET_LIMIT;
        let sheet_count = sheet_names.len().min(TABLE_SHEET_LIMIT);
        let mut budget = PreviewBudget {
            values: TABLE_VALUE_LIMIT,
            text_bytes: TABLE_TEXT_LIMIT,
            serialized_bytes: TABLE_SERIALIZED_TEXT_LIMIT,
        };

        for (idx, name) in sheet_names.iter().enumerate().take(TABLE_SHEET_LIMIT) {
            let Some(range) = read_sheet(idx) else {
                overall_truncated = true;
                continue;
            };
            let copies = if sheets.is_empty() { 2 } else { 1 };
            let allocation = budget.share(sheet_count - idx + copies - 1);
            let mut remaining = allocation;
            let sheet = preview_sheet(name, &range, &mut remaining);
            budget.values -= (allocation.values - remaining.values) * copies;
            budget.text_bytes -= (allocation.text_bytes - remaining.text_bytes) * copies;
            budget.serialized_bytes -=
                (allocation.serialized_bytes - remaining.serialized_bytes) * copies;
            overall_truncated |= sheet.truncated;
            sheets.push(sheet);
        }

        if sheets.is_empty() {
            return Err("Workbook contains no readable worksheets".to_string());
        }

        let first = &sheets[0];
        let headers = first.headers.clone();
        let rows = first.rows.clone();
        let total_rows = first.total_rows;
        let total_cols = first.total_cols;
        let truncated = overall_truncated;

        Ok(SpreadsheetData {
            sheet_names: sheets.iter().map(|sheet| sheet.name.clone()).collect(),
            active_sheet: 0,
            headers,
            rows,
            total_rows,
            total_cols,
            truncated,
            sheets,
        })
    }
}

fn open_workbook(path: &Path) -> Result<calamine::Sheets<BufReader<File>>, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > WORKBOOK_BYTE_LIMIT {
        return Err("Workbook is too large to preview safely".into());
    }
    let mut magic = [0; 8];
    file.read_exact(&mut magic)
        .map_err(|error| error.to_string())?;
    file.rewind().map_err(|error| error.to_string())?;
    if magic == [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1] {
        return calamine::Xls::new(BufReader::new(file))
            .map(calamine::Sheets::Xls)
            .map_err(|error| error.to_string());
    }
    let mut package = crate::sandbox_helper::office::Package::open(
        file.try_clone().map_err(|error| error.to_string())?,
    )?;
    let xlsx = package.workbook_part()?.is_some();
    if !xlsx && !package.contains("content.xml") {
        return Err("This package does not contain a supported workbook".into());
    }
    package.validate_spreadsheet_xml()?;
    drop(package);
    file.rewind().map_err(|error| error.to_string())?;
    if xlsx {
        calamine::Xlsx::new(BufReader::new(file))
            .map(calamine::Sheets::Xlsx)
            .map_err(|error| error.to_string())
    } else {
        calamine::Ods::new(BufReader::new(file))
            .map(calamine::Sheets::Ods)
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone, Copy)]
struct PreviewBudget {
    values: usize,
    text_bytes: usize,
    serialized_bytes: usize,
}

impl PreviewBudget {
    fn share(self, count: usize) -> Self {
        Self {
            values: self.values / count,
            text_bytes: self.text_bytes / count,
            serialized_bytes: self.serialized_bytes / count,
        }
    }

    fn cell(&mut self, cell: &Data) -> Option<(String, bool)> {
        if self.values == 0 || self.serialized_bytes < 3 {
            return None;
        }
        let text = match cell {
            Data::String(text) | Data::DateTimeIso(text) | Data::DurationIso(text) => {
                Cow::Borrowed(text.as_str())
            }
            _ => Cow::Owned(format_cell(cell)),
        };
        let (end, serialized_bytes) =
            text_prefix(&text, self.text_bytes, self.serialized_bytes - 3);
        let truncated = end < text.len();
        let text = text[..end].to_string();
        self.values -= 1;
        self.text_bytes -= text.len();
        self.serialized_bytes -= serialized_bytes + 3;
        Some((text, truncated))
    }
}

fn text_prefix(text: &str, byte_limit: usize, serialized_limit: usize) -> (usize, usize) {
    let mut end = 0;
    let mut serialized_bytes = 0;
    for character in text.chars() {
        let bytes = character.len_utf8();
        let escaped_bytes = match character {
            '"' | '\\' | '\u{08}' | '\u{0c}' | '\n' | '\r' | '\t' => 2,
            '\u{00}'..='\u{1f}' => 6,
            _ => bytes,
        };
        if end + bytes > byte_limit || serialized_bytes + escaped_bytes > serialized_limit {
            break;
        }
        end += bytes;
        serialized_bytes += escaped_bytes;
    }
    (end, serialized_bytes)
}

fn preview_row(row: &[Data], budget: &mut PreviewBudget) -> (Vec<String>, bool) {
    let mut cells = Vec::new();
    for cell in row.iter().take(TABLE_COLUMN_LIMIT) {
        let Some((text, truncated)) = budget.cell(cell) else {
            return (cells, true);
        };
        cells.push(text);
        if truncated {
            return (cells, true);
        }
    }
    (cells, false)
}

fn preview_sheet(
    name: &str,
    range: &calamine::Range<Data>,
    budget: &mut PreviewBudget,
) -> SheetData {
    let (name_end, _) = text_prefix(name, TABLE_SHEET_NAME_LIMIT, TABLE_SHEET_NAME_LIMIT);
    let mut sheet = SheetData {
        name: name[..name_end].to_string(),
        total_rows: range.height(),
        total_cols: range.width(),
        truncated: name_end < name.len() || range.width() > TABLE_COLUMN_LIMIT,
        ..SheetData::default()
    };
    let mut row_iter = range.rows();
    let mut exhausted = false;
    if let Some(first_row) = row_iter.next() {
        (sheet.headers, exhausted) = preview_row(first_row, budget);
    }
    if !exhausted {
        for row in row_iter.take(TABLE_ROW_LIMIT) {
            let (cells, truncated) = preview_row(row, budget);
            if !cells.is_empty() {
                sheet.rows.push(cells);
            }
            if truncated {
                exhausted = true;
                break;
            }
        }
    }
    sheet.truncated |= exhausted || sheet.total_rows > sheet.rows.len() + 1;
    sheet
}

fn format_cell(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::Int(i) => format!("{i}"),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{:.0}", f)
            } else {
                format!("{f}")
            }
        }
        Data::DateTime(d) => format!("{d}"),
        Data::DateTimeIso(d) => d.clone(),
        Data::DurationIso(d) => d.clone(),
        Data::Error(e) => e.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests;
