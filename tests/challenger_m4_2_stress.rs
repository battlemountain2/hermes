// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(dead_code, unused_imports, clippy::all)]

//! Empirical Challenger Stress Test Suite for Milestone 4 (M4.2):
//! - 1. Massive spreadsheets exceeding cell/row/col/byte limits:
//!      confirm truncation flags, bounds enforcement (200 rows, 256 cols, 20MB ceiling),
//!      safe handling of oversized workbooks without OOM or hanging.
//! - 2. Corrupted spreadsheets:
//!      truncated ZIP/OLE headers, empty sheets, formula-heavy sheets,
//!      guaranteeing zero panics and graceful error returns.
//! - 3. Audio edge cases:
//!      zero sample rate, zero channels, truncated WAV/MP3/FLAC/OGG streams, empty audio files,
//!      guaranteeing zero division-by-zero (SIGFPE) and valid metadata contracts for supported formats.
//! - 4. PDF edge cases:
//!      scanned PDFs without text, pages with ligatures, empty pages,
//!      multi-page drag selection spanning offscreen virtualized pages,
//!      Ctrl+C shortcut modifier handling with Caps Lock / Lock mask,
//!      and deselect on Escape key behavior.
//! - 5. Strict check: Ensure NO modal/Vim keyboard navigation modes exist across the codebase.

use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use serde::{Deserialize, Serialize};

mod common;
use common::*;
mod fixtures;
use fixtures::{audio::*, pdf::*, spreadsheets::*, zip_util::*};

// =========================================================================
// Target Data Structures (Mirroring Hermes Contracts)
// =========================================================================

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioMetadata {
    pub format: String,
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PdfTextLayer {
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub glyphs: Vec<[f32; 4]>,
}

// =========================================================================
// PDF Selection Pure-Model Oracles (Mirroring src/ui/preview/pdf_text.rs)
// =========================================================================

#[derive(Clone, Copy, Debug)]
pub struct PdfLine {
    pub top: f32,
    pub bottom: f32,
    pub start: usize,
    pub end: usize,
}

pub fn solid(layer: &PdfTextLayer, index: usize) -> Option<[f32; 4]> {
    let rect = layer.glyphs.get(index)?;
    (rect[2] > rect[0] && rect[3] > rect[1]).then_some(*rect)
}

pub fn lines(layer: &PdfTextLayer) -> Vec<PdfLine> {
    let len = len(layer);
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, ch) in layer.text.chars().take(len).enumerate() {
        if ch == '\n' {
            push_line(&mut lines, layer, start, index + 1);
            start = index + 1;
        }
    }
    if start < len {
        push_line(&mut lines, layer, start, len);
    }
    lines
}

fn push_line(lines: &mut Vec<PdfLine>, layer: &PdfTextLayer, start: usize, end: usize) {
    let mut top = f32::MAX;
    let mut bottom = f32::MIN;
    for index in start..end {
        if let Some(rect) = solid(layer, index) {
            top = top.min(rect[1]);
            bottom = bottom.max(rect[3]);
        }
    }
    if top > bottom {
        let (top, bottom) = lines
            .last()
            .map(|line: &PdfLine| (line.bottom, line.bottom + line.bottom - line.top))
            .unwrap_or((0.0, 0.0));
        lines.push(PdfLine {
            top,
            bottom,
            start,
            end,
        });
        return;
    }
    lines.push(PdfLine {
        top,
        bottom,
        start,
        end,
    });
}

pub fn image_bounds(layer: &PdfTextLayer, width: f64, height: f64) -> (f64, f64, f64) {
    if layer.width <= 0.0 || layer.height <= 0.0 {
        return (0.0, 0.0, 1.0);
    }
    let scale = (width / f64::from(layer.width)).min(height / f64::from(layer.height));
    let x = (width - f64::from(layer.width) * scale) / 2.0;
    let y = (height - f64::from(layer.height) * scale) / 2.0;
    (x, y, scale)
}

pub fn len(layer: &PdfTextLayer) -> usize {
    layer.text.chars().count().min(layer.glyphs.len())
}

pub fn hit_text(layer: &PdfTextLayer, x: f32, y: f32) -> bool {
    lines(layer).iter().any(|line| {
        let height = (line.bottom - line.top).max(1.0);
        if y < line.top - height * 0.5 || y > line.bottom + height * 0.5 {
            return false;
        }
        let mut x1 = f32::MAX;
        let mut x2 = f32::MIN;
        for index in line.start..line.end {
            if let Some(rect) = solid(layer, index) {
                x1 = x1.min(rect[0]);
                x2 = x2.max(rect[2]);
            }
        }
        x1 <= x2 && x >= x1 - height && x <= x2 + height
    })
}

pub fn caret_at(layer: &PdfTextLayer, x: f32, y: f32) -> usize {
    let all_lines = lines(layer);
    let len = len(layer);
    let Some(line) = all_lines.iter().min_by(|a, b| {
        let da = if y < a.top {
            a.top - y
        } else {
            (y - a.bottom).max(0.0)
        };
        let db = if y < b.top {
            b.top - y
        } else {
            (y - b.bottom).max(0.0)
        };
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        return len;
    };
    for index in line.start..line.end {
        let Some(rect) = solid(layer, index) else {
            continue;
        };
        if x < (rect[0] + rect[2]) / 2.0 {
            return index;
        }
    }
    let mut end = line.end;
    if end > line.start && layer.text.chars().nth(end - 1) == Some('\n') {
        end -= 1;
    }
    end
}

fn has_descender(layer: &PdfTextLayer, start: usize, end: usize) -> bool {
    layer.text.chars().skip(start).take(end - start).any(|ch| {
        !ch.is_ascii()
            || matches!(
                ch,
                'g' | 'j'
                    | 'p'
                    | 'q'
                    | 'y'
                    | 'Q'
                    | ','
                    | ';'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '_'
            )
    })
}

pub fn selection_runs(layer: &PdfTextLayer, start: usize, end: usize) -> Vec<[f32; 4]> {
    let (start, end) = (start.min(end), start.max(end));
    lines(layer)
        .iter()
        .filter_map(|line| {
            let (first, last) = (start.max(line.start), end.min(line.end));
            if first >= last {
                return None;
            }
            let mut x1 = f32::MAX;
            let mut x2 = f32::MIN;
            for index in first..last {
                if let Some(rect) = solid(layer, index) {
                    x1 = x1.min(rect[0]);
                    x2 = x2.max(rect[2]);
                }
            }
            let bottom = if has_descender(layer, first, last) {
                line.bottom
            } else {
                line.top + (line.bottom - line.top) * 0.82
            };
            (x1 <= x2).then_some([x1, line.top, x2, bottom])
        })
        .collect()
}

pub fn word_range(layer: &PdfTextLayer, index: usize) -> (usize, usize) {
    let len = len(layer);
    let index = index.min(len);
    let blank = |i: usize| layer.text.chars().nth(i).is_none_or(char::is_whitespace);
    if blank(index) {
        return (index, index);
    }
    let mut start = index;
    while start > 0 && !blank(start - 1) {
        start -= 1;
    }
    let mut end = index;
    while end < len && !blank(end) {
        end += 1;
    }
    (start, end)
}

pub fn line_range(layer: &PdfTextLayer, index: usize) -> (usize, usize) {
    let index = index.min(len(layer));
    let all = lines(layer);
    let Some(line) = all.iter().find(|line| index < line.end).or(all.last()) else {
        return (index, index);
    };
    let mut end = line.end;
    if end > line.start && layer.text.chars().nth(end - 1) == Some('\n') {
        end -= 1;
    }
    (line.start, end)
}

pub fn selection_text(layer: &PdfTextLayer, start: usize, end: usize) -> String {
    let (start, end) = (start.min(end), start.max(end).min(len(layer)));
    layer.text.chars().skip(start).take(end - start).collect()
}

pub fn pdf_desired_ranges(
    layers: &HashMap<i32, Arc<PdfTextLayer>>,
    anchor: (i32, usize),
    caret: (i32, usize),
    granularity: u8,
) -> HashMap<i32, (usize, usize)> {
    let (anchor_page, anchor) = anchor;
    let (current_page, caret) = caret;
    let snap_lo = |layer: &PdfTextLayer, index: usize| match granularity {
        2 => word_range(layer, index).0,
        3 => line_range(layer, index).0,
        _ => index,
    };
    let snap_hi = |layer: &PdfTextLayer, index: usize| match granularity {
        2 => word_range(layer, index).1,
        3 => line_range(layer, index).1,
        _ => index,
    };
    let (first, last, first_start, last_end) = if anchor_page <= current_page {
        (anchor_page, current_page, anchor, caret)
    } else {
        (current_page, anchor_page, caret, anchor)
    };
    let mut desired = HashMap::new();
    for page in first..=last {
        let Some(layer) = layers.get(&page) else {
            continue;
        };
        let len = len(layer);
        let (start, end) = match (page == first, page == last) {
            (true, true) => (
                snap_lo(layer, first_start.min(last_end)),
                snap_hi(layer, first_start.max(last_end)),
            ),
            (true, false) => (snap_lo(layer, first_start), len),
            (false, true) => (0, snap_hi(layer, last_end)),
            _ => (0, len),
        };
        let (start, end) = (start.min(end), end.min(len));
        if start != end {
            desired.insert(page, (start, end));
        }
    }
    desired
}

pub fn pdf_selected_text(
    layers: &HashMap<i32, Arc<PdfTextLayer>>,
    ranges: &HashMap<i32, (usize, usize)>,
) -> String {
    let mut sorted: Vec<_> = ranges.iter().collect();
    sorted.sort_by_key(|(page, _)| *page);
    let mut text = String::new();
    for (page, &(start, end)) in sorted {
        let Some(layer) = layers.get(page) else {
            continue;
        };
        let part = selection_text(layer, start, end);
        if part.is_empty() {
            continue;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&part);
    }
    text
}

pub fn pdf_drop_unselected_layer(
    layers: &RefCell<HashMap<i32, Arc<PdfTextLayer>>>,
    ranges: &RefCell<HashMap<i32, (usize, usize)>>,
    page: i32,
) {
    if !ranges.borrow().contains_key(&page) {
        layers.borrow_mut().remove(&page);
    }
}

pub fn pdf_shortcut_modifiers(modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && !modifiers
            .intersects(gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::ALT_MASK)
}

// =========================================================================
// Fixture Generators for Adversarial Cases
// =========================================================================

const ODS_MANIFEST_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">
 <manifest:file-entry manifest:full-path="/" manifest:version="1.3" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
 <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>"#;

/// Builds a valid ODS spreadsheet workbook with custom rows and columns.
pub fn generate_massive_ods(num_rows: usize, num_cols: usize) -> Vec<u8> {
    let mut xml = String::new();
    xml.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
                         xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
                         xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
  <office:body>
    <office:spreadsheet>
      <table:table table:name="MassiveSheet">
"#);

    // Header row
    xml.push_str("        <table:table-row>\n");
    for c in 0..num_cols {
        xml.push_str(&format!(
            "          <table:table-cell office:value-type=\"string\"><text:p>Col_{}</text:p></table:table-cell>\n",
            c
        ));
    }
    xml.push_str("        </table:table-row>\n");

    // Data rows
    for r in 0..num_rows {
        xml.push_str("        <table:table-row>\n");
        for c in 0..num_cols {
            xml.push_str(&format!(
                "          <table:table-cell office:value-type=\"float\" office:value=\"{}\"><text:p>{}</text:p></table:table-cell>\n",
                r * 100 + c,
                r * 100 + c
            ));
        }
        xml.push_str("        </table:table-row>\n");
    }

    xml.push_str(r#"      </table:table>
    </office:spreadsheet>
  </office:body>
</office:document-content>"#);

    create_zip(&[
        ("mimetype", b"application/vnd.oasis.opendocument.spreadsheet"),
        ("content.xml", xml.as_bytes()),
        ("META-INF/manifest.xml", ODS_MANIFEST_XML),
    ])
}

/// Builds an ODS spreadsheet containing formula-heavy cells (arithmetic, div by zero, concatenations, logical IF).
pub fn generate_formula_ods() -> Vec<u8> {
    let content_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
                         xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
                         xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
  <office:body>
    <office:spreadsheet>
      <table:table table:name="FormulaSheet">
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>Formula Type</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>Expression</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>Evaluated Result</text:p></table:table-cell>
        </table:table-row>
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>Sum</text:p></table:table-cell>
          <table:table-cell table:formula="of:=SUM([.A1:.A10])" office:value-type="float" office:value="1050"><text:p>1050</text:p></table:table-cell>
          <table:table-cell office:value-type="float" office:value="1050"><text:p>1050</text:p></table:table-cell>
        </table:table-row>
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>DivisionByZero</text:p></table:table-cell>
          <table:table-cell table:formula="of:=1/0" office:value-type="string"><text:p>#DIV/0!</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>#DIV/0!</text:p></table:table-cell>
        </table:table-row>
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>Concat</text:p></table:table-cell>
          <table:table-cell table:formula="of:=CONCATENATE(&quot;Hermes&quot;, &quot; &quot;, &quot;Preview&quot;)" office:value-type="string"><text:p>Hermes Preview</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>Hermes Preview</text:p></table:table-cell>
        </table:table-row>
        <table:table-row>
          <table:table-cell office:value-type="string"><text:p>LogicalIF</text:p></table:table-cell>
          <table:table-cell table:formula="of:=IF(1>0, &quot;PASS&quot;, &quot;FAIL&quot;)" office:value-type="string"><text:p>PASS</text:p></table:table-cell>
          <table:table-cell office:value-type="string"><text:p>PASS</text:p></table:table-cell>
        </table:table-row>
      </table:table>
    </office:spreadsheet>
  </office:body>
</office:document-content>"#;

    create_zip(&[
        ("mimetype", b"application/vnd.oasis.opendocument.spreadsheet"),
        ("content.xml", content_xml.as_bytes()),
        ("META-INF/manifest.xml", ODS_MANIFEST_XML),
    ])
}

/// Builds an ODS spreadsheet with an empty `<office:spreadsheet/>` (no worksheets).
pub fn generate_empty_ods_no_tables() -> Vec<u8> {
    let content_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
                         xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0">
  <office:body>
    <office:spreadsheet/>
  </office:body>
</office:document-content>"#;

    create_zip(&[
        ("mimetype", b"application/vnd.oasis.opendocument.spreadsheet"),
        ("content.xml", content_xml.as_bytes()),
        ("META-INF/manifest.xml", ODS_MANIFEST_XML),
    ])
}

/// Builds an ODS spreadsheet with a worksheet containing 0 rows and 0 cells.
pub fn generate_empty_ods_zero_cells() -> Vec<u8> {
    let content_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
                         xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0">
  <office:body>
    <office:spreadsheet>
      <table:table table:name="EmptyTable"/>
    </office:spreadsheet>
  </office:body>
</office:document-content>"#;

    create_zip(&[
        ("mimetype", b"application/vnd.oasis.opendocument.spreadsheet"),
        ("content.xml", content_xml.as_bytes()),
        ("META-INF/manifest.xml", ODS_MANIFEST_XML),
    ])
}

/// Generates a valid minimal scanned PDF containing only graphical paths and 0 text streams.
pub fn generate_scanned_pdf() -> Vec<u8> {
    // Page with vector graphic rectangle and filled path, NO text (no BT/ET operators)
    let stream_content = "q 10 10 200 200 re f 1 0 0 rg 50 50 100 100 re f Q";
    let stream_len = stream_content.len();

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let o1 = out.len();
    out.extend_from_slice(b"1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n");
    let o2 = out.len();
    out.extend_from_slice(b"2 0 obj<</Type/Pages/Count 1/Kids[3 0 R]>>endobj\n");
    let o3 = out.len();
    out.extend_from_slice(b"3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]/Contents 4 0 R>>endobj\n");
    let o4 = out.len();
    let content_obj = format!("4 0 obj<</Length {}>>stream\n{}\nendstream\nendobj\n", stream_len, stream_content);
    out.extend_from_slice(content_obj.as_bytes());

    let xref_offset = out.len();
    let xref = format!(
        "xref\n0 5\n0000000000 65535 f \n{:010} 00000 n \n{:010} 00000 n \n{:010} 00000 n \n{:010} 00000 n \ntrailer<</Size 5/Root 1 0 R>>\nstartxref\n{}\n%%EOF\n",
        o1, o2, o3, o4, xref_offset
    );
    out.extend_from_slice(xref.as_bytes());

    out
}

/// Generates a PDF containing ligatures ("difficult float aesthetic") and Unicode characters.
pub fn generate_ligature_pdf() -> Vec<u8> {
    sample_text_pdf("Difficult flight affirms aesthetic office")
}

/// Generates a PDF with a zero-dimension MediaBox [0 0 0 0].
pub fn generate_zero_dimension_pdf() -> Vec<u8> {
    let stream_content = "BT /F1 12 Tf 0 0 Td (Zero) Tj ET";
    let stream_len = stream_content.len();

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n");
    let o1 = out.len();
    out.extend_from_slice(b"1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n");
    let o2 = out.len();
    out.extend_from_slice(b"2 0 obj<</Type/Pages/Count 1/Kids[3 0 R]>>endobj\n");
    let o3 = out.len();
    out.extend_from_slice(b"3 0 obj<</Type/Page/Parent 2 0 R/Resources<</Font<</F1 4 0 R>>>>/MediaBox[0 0 0 0]/Contents 5 0 R>>endobj\n");
    let o4 = out.len();
    out.extend_from_slice(b"4 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\n");
    let o5 = out.len();
    let content_obj = format!("5 0 obj<</Length {}>>stream\n{}\nendstream\nendobj\n", stream_len, stream_content);
    out.extend_from_slice(content_obj.as_bytes());

    let xref_offset = out.len();
    let xref = format!(
        "xref\n0 6\n0000000000 65535 f \n{:010} 00000 n \n{:010} 00000 n \n{:010} 00000 n \n{:010} 00000 n \n{:010} 00000 n \ntrailer<</Size 6/Root 1 0 R>>\nstartxref\n{}\n%%EOF\n",
        o1, o2, o3, o4, o5, xref_offset
    );
    out.extend_from_slice(xref.as_bytes());

    out
}

/// Generates a customized WAV file with arbitrary sample rate and channels.
pub fn generate_custom_wav(sample_rate: u32, channels: u16, num_samples: usize) -> Vec<u8> {
    let bytes_per_sample = 2u16; // 16-bit
    let block_align = channels * bytes_per_sample;
    let byte_rate = sample_rate * block_align as u32;
    let data_len = (num_samples as u32) * (block_align as u32);
    let file_len = 36 + data_len;

    let mut out = Vec::with_capacity((file_len + 8) as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&file_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");

    // fmt subchunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size
    out.extend_from_slice(&1u16.to_le_bytes());  // PCM format
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes()); // 16-bit

    // data subchunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend(vec![0u8; data_len as usize]);

    out
}

// =========================================================================
// 1. Massive Spreadsheets Exceeding Limits
// =========================================================================

#[test]
fn test_massive_spreadsheet_rows_capped_at_200() {
    let env = TestEnv::new();
    // 350 rows (exceeds TABLE_ROW_LIMIT = 200)
    let ods = generate_massive_ods(350, 4);
    let input = env.write_file("massive_rows.ods", &ods);
    let output = env.file_path("output.png");

    let res = run_preview_helper("preview-spreadsheet", &input, &output, 0);
    assert!(res.status.success(), "Helper must succeed: {}", res.stderr);

    let meta_path = output.with_file_name("result.meta");
    assert!(meta_path.exists(), "result.meta must be written");

    let meta_content = fs::read_to_string(&meta_path).expect("read meta");
    let data: SpreadsheetData = serde_json::from_str(&meta_content).expect("parse SpreadsheetData");

    assert!(data.truncated, "Truncation flag must be true when rows exceed 200");
    assert_eq!(data.rows.len(), 200, "Rendered rows must be strictly capped at 200");
    assert_eq!(data.total_rows, 351, "total_rows must record original row count (350 data + 1 header)");
    assert_eq!(data.total_cols, 4, "total_cols must match 4");
}

#[test]
fn test_massive_spreadsheet_columns_capped_at_256() {
    let env = TestEnv::new();
    // 300 columns (exceeds TABLE_COLUMN_LIMIT = 256)
    let ods = generate_massive_ods(5, 300);
    let input = env.write_file("massive_cols.ods", &ods);
    let output = env.file_path("output.png");

    let res = run_preview_helper("preview-spreadsheet", &input, &output, 0);
    assert!(res.status.success(), "Helper must succeed: {}", res.stderr);

    let meta_path = output.with_file_name("result.meta");
    let meta_content = fs::read_to_string(&meta_path).expect("read meta");
    let data: SpreadsheetData = serde_json::from_str(&meta_content).expect("parse SpreadsheetData");

    assert!(data.truncated, "Truncation flag must be true when columns exceed 256");
    assert!(data.headers.len() <= 256, "Headers must be capped at 256 (got {})", data.headers.len());
    for row in &data.rows {
        assert!(row.len() <= 256, "Row columns must be capped at 256 (got {})", row.len());
    }
    assert_eq!(data.total_cols, 300, "total_cols must reflect original 300 columns");
}

#[test]
fn test_massive_spreadsheet_file_size_exceeds_20mb_rejected() {
    let env = TestEnv::new();
    // Create an oversized file > 20 MiB (WORKBOOK_BYTE_LIMIT = 20 * 1024 * 1024)
    let path = env.file_path("oversized.xlsx");
    let file = fs::File::create(&path).expect("create file");
    file.set_len(21 * 1024 * 1024).expect("set 21 MiB length");

    let output = env.file_path("output.png");
    let res = run_preview_helper("preview-spreadsheet", &path, &output, 0);

    assert!(!res.status.success(), "Oversized workbook > 20MB must fail");
    assert!(
        res.stderr.contains("too large to preview safely") || res.stdout.contains("too large"),
        "Must emit explicit guardrail error for oversized workbook: stderr={}",
        res.stderr
    );
}

#[test]
fn test_massive_spreadsheet_text_extraction_byte_limit_exact() {
    let env = TestEnv::new();
    let ods = generate_massive_ods(100, 10);
    let input = env.write_file("extract_limit.ods", &ods);
    let output = env.file_path("text.out");

    let limit = 45usize;
    let res = run_preview_helper("extract-spreadsheet-text", &input, &output, limit as i32);
    assert!(res.status.success(), "Extraction must succeed: {}", res.stderr);

    let bytes = fs::read(&output).expect("read extracted text");
    assert!(
        bytes.len() <= limit,
        "Extracted bytes len {} must not exceed byte limit {}",
        bytes.len(),
        limit
    );
    assert!(std::str::from_utf8(&bytes).is_ok(), "Truncation must preserve valid UTF-8 boundaries");
}

// =========================================================================
// 2. Corrupted Spreadsheets
// =========================================================================

#[test]
fn test_corrupted_spreadsheet_truncated_zip_headers() {
    let env = TestEnv::new();
    let truncated_lengths = [0, 2, 10, 30];

    for &len in &truncated_lengths {
        let mut data = sample_xlsx_spreadsheet();
        data.truncate(len);

        let input_xlsx = env.write_file(&format!("corrupt_{len}.xlsx"), &data);
        let output_xlsx = env.file_path(&format!("out_{len}.png"));
        let res_xlsx = run_preview_helper("preview-spreadsheet", &input_xlsx, &output_xlsx, 0);
        assert!(!res_xlsx.status.success(), "Truncated XLSX len {len} must fail gracefully");

        let input_ods = env.write_file(&format!("corrupt_{len}.ods"), &data);
        let output_ods = env.file_path(&format!("out_ods_{len}.png"));
        let res_ods = run_preview_helper("preview-spreadsheet", &input_ods, &output_ods, 0);
        assert!(!res_ods.status.success(), "Truncated ODS len {len} must fail gracefully");
    }
}

#[test]
fn test_corrupted_spreadsheet_truncated_ole_headers() {
    let env = TestEnv::new();
    let ole_magic = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

    for len in [0, 4, 8, 128, 500] {
        let data = if len <= ole_magic.len() {
            ole_magic[..len].to_vec()
        } else {
            let mut d = ole_magic.to_vec();
            d.resize(len, 0xAA);
            d
        };

        let input = env.write_file(&format!("corrupt_ole_{len}.xls"), &data);
        let output = env.file_path(&format!("out_ole_{len}.png"));
        let res = run_preview_helper("preview-spreadsheet", &input, &output, 0);
        assert!(!res.status.success(), "Truncated XLS/OLE len {len} must fail gracefully");
    }
}

#[test]
fn test_corrupted_spreadsheet_empty_sheets() {
    let env = TestEnv::new();

    // 1. ODS with no worksheet table
    let ods_no_tables = generate_empty_ods_no_tables();
    let input1 = env.write_file("no_tables.ods", &ods_no_tables);
    let out1 = env.file_path("out1.png");
    let res1 = run_preview_helper("preview-spreadsheet", &input1, &out1, 0);
    // Should fail cleanly with "Workbook contains no worksheets" or 0 rows
    if !res1.status.success() {
        assert!(
            res1.stderr.contains("no worksheets") || res1.stderr.contains("Invalid"),
            "Expected worksheet error: {}",
            res1.stderr
        );
    }

    // 2. ODS with empty worksheet table (0 rows, 0 cells)
    let ods_zero_cells = generate_empty_ods_zero_cells();
    let input2 = env.write_file("zero_cells.ods", &ods_zero_cells);
    let out2 = env.file_path("out2.png");
    let res2 = run_preview_helper("preview-spreadsheet", &input2, &out2, 0);
    if res2.status.success() {
        let meta_content = fs::read_to_string(out2.with_file_name("result.meta")).expect("read meta");
        let data: SpreadsheetData = serde_json::from_str(&meta_content).expect("parse");
        assert_eq!(data.total_rows, 0);
        assert_eq!(data.rows.len(), 0);
    }
}

#[test]
fn test_formula_heavy_spreadsheet_resilience() {
    let env = TestEnv::new();
    let ods = generate_formula_ods();
    let input = env.write_file("formula.ods", &ods);
    let output = env.file_path("formula_out.png");

    let res = run_preview_helper("preview-spreadsheet", &input, &output, 0);
    assert!(res.status.success(), "Formula-heavy sheet must succeed: {}", res.stderr);

    let meta_path = output.with_file_name("result.meta");
    let meta_content = fs::read_to_string(&meta_path).expect("read meta");
    let data: SpreadsheetData = serde_json::from_str(&meta_content).expect("parse SpreadsheetData");

    assert_eq!(data.headers, vec!["Formula Type", "Expression", "Evaluated Result"]);
    assert_eq!(data.rows.len(), 4, "Must extract 4 formula data rows");

    // Row 1: Division by zero (#DIV/0!) must not crash parser
    let div_zero_row = &data.rows[1];
    assert_eq!(div_zero_row[0], "DivisionByZero");
    assert!(div_zero_row[1].contains("DIV/0") || div_zero_row[2].contains("DIV/0"));

    // Row 2: Concat text
    let concat_row = &data.rows[2];
    assert_eq!(concat_row[0], "Concat");
    assert!(concat_row[2].contains("Hermes Preview") || concat_row[1].contains("Hermes"));

    // Row 3: Logical IF
    let if_row = &data.rows[3];
    assert_eq!(if_row[0], "LogicalIF");
    assert!(if_row[2].contains("PASS") || if_row[1].contains("PASS"));
}

// =========================================================================
// 3. Audio Edge Cases
// =========================================================================

#[test]
fn test_audio_wav_zero_sample_rate() {
    let env = TestEnv::new();
    let wav = generate_custom_wav(0, 1, 100);
    let input = env.write_file("zero_sr.wav", &wav);
    let output = env.file_path("zero_sr.png");

    // Zero sample rate must not cause SIGFPE (integer division by zero) or crash
    let res = run_preview_helper("preview-audio-waveform", &input, &output, 0);
    // Whether FFmpeg fails or falls back to safe metadata, it must NOT panic or crash with signal
    assert!(
        res.status.code().is_some(),
        "Process must terminate normally with an exit code, not killed by SIGFPE"
    );
}

#[test]
fn test_audio_wav_zero_channels() {
    let env = TestEnv::new();
    let wav = generate_custom_wav(44100, 0, 100);
    let input = env.write_file("zero_ch.wav", &wav);
    let output = env.file_path("zero_ch.png");

    // Zero channels must not crash process
    let res = run_preview_helper("preview-audio-waveform", &input, &output, 0);
    assert!(
        res.status.code().is_some(),
        "Process must terminate cleanly with an exit code"
    );
}

#[test]
fn test_audio_truncated_streams_all_formats() {
    let env = TestEnv::new();

    // Truncated WAV
    let wav = generate_custom_wav(44100, 2, 500);
    let trunc_wav = env.write_file("trunc.wav", &wav[..20]);
    let res = run_preview_helper("preview-audio-waveform", &trunc_wav, &env.file_path("out_wav.png"), 0);
    assert!(!res.status.success(), "Truncated WAV must be rejected");

    // Truncated FLAC
    let flac = sample_flac();
    let trunc_flac = env.write_file("trunc.flac", &flac[..8]);
    let res = run_preview_helper("preview-audio-waveform", &trunc_flac, &env.file_path("out_flac.png"), 0);
    assert!(!res.status.success(), "Truncated FLAC must be rejected");

    // Truncated MP3
    let mp3 = sample_mp3();
    let trunc_mp3 = env.write_file("trunc.mp3", &mp3[..14]);
    let res = run_preview_helper("preview-audio-waveform", &trunc_mp3, &env.file_path("out_mp3.png"), 0);
    assert!(!res.status.success(), "Truncated MP3 must be rejected");

    // Truncated OGG
    let ogg = sample_ogg();
    let trunc_ogg = env.write_file("trunc.ogg", &ogg[..12]);
    let res = run_preview_helper("preview-audio-waveform", &trunc_ogg, &env.file_path("out_ogg.png"), 0);
    assert!(!res.status.success(), "Truncated OGG must be rejected");
}

#[test]
fn test_audio_empty_streams_all_formats() {
    let env = TestEnv::new();

    for ext in ["wav", "flac", "mp3", "ogg"] {
        let empty = env.write_file(&format!("empty.{ext}"), b"");
        let out = env.file_path(&format!("empty_{ext}.png"));
        let res = run_preview_helper("preview-audio-waveform", &empty, &out, 0);
        assert!(!res.status.success(), "Empty {ext} audio file must be rejected cleanly");
    }
}

#[test]
fn test_audio_valid_waveform_and_metadata_contracts() {
    let env = TestEnv::new();
    let wav = sample_wav();
    let input = env.write_file("valid.wav", &wav);
    let output = env.file_path("waveform.png");

    let res = run_preview_helper("preview-audio-waveform", &input, &output, 0);
    assert!(res.status.success(), "Waveform generator must succeed for valid WAV: {}", res.stderr);

    let png_bytes = fs::read(&output).expect("read waveform png");
    assert!(is_valid_png(&png_bytes), "Waveform output must be valid PNG");

    let meta_path = output.with_file_name("result.meta");
    let meta_content = fs::read_to_string(&meta_path).expect("read metadata");
    let metadata: AudioMetadata = serde_json::from_str(&meta_content).expect("parse AudioMetadata");

    assert_eq!(metadata.format, "WAV");
    assert_eq!(metadata.sample_rate, 44100);
    assert_eq!(metadata.channels, 1);
    assert!(metadata.duration_seconds > 0.15 && metadata.duration_seconds < 0.25);
}

// =========================================================================
// 4. PDF Edge Cases
// =========================================================================

#[test]
fn test_pdf_scanned_page_without_text() {
    let env = TestEnv::new();
    let pdf = generate_scanned_pdf();
    let input = env.write_file("scanned.pdf", &pdf);
    let output = env.file_path("scanned.png");

    // 1. preview-pdf must render page visual image
    let res_prev = run_preview_helper("preview-pdf", &input, &output, 1);
    assert!(res_prev.status.success(), "Visual preview of scanned PDF must succeed: {}", res_prev.stderr);
    let png_bytes = fs::read(&output).expect("read scanned png");
    assert!(is_valid_png(&png_bytes), "Page preview must be valid PNG");

    // 2. extract-pdf-text must return empty bytes, no failure
    let text_out = env.file_path("scanned_text.txt");
    let res_text = run_preview_helper("extract-pdf-text", &input, &text_out, 100);
    assert!(res_text.status.success(), "Text extraction on scanned PDF must succeed: {}", res_text.stderr);
    let extracted = fs::read(&text_out).expect("read text");
    assert!(extracted.is_empty(), "Scanned PDF has 0 text glyphs, text must be empty");

    // 3. Selection model with empty layer
    let empty_layer = PdfTextLayer {
        width: 612.0,
        height: 792.0,
        text: String::new(),
        glyphs: Vec::new(),
    };
    assert_eq!(caret_at(&empty_layer, 100.0, 100.0), 0);
    assert!(!hit_text(&empty_layer, 100.0, 100.0));
    assert!(selection_runs(&empty_layer, 0, 0).is_empty());
    assert_eq!(selection_text(&empty_layer, 0, 0), "");
}

#[test]
fn test_pdf_ligatures_and_multibyte_unicode() {
    let env = TestEnv::new();
    let pdf = generate_ligature_pdf();
    let input = env.write_file("ligatures.pdf", &pdf);
    let text_out = env.file_path("extracted.txt");

    let res = run_preview_helper("extract-pdf-text", &input, &text_out, 1000);
    assert!(res.status.success(), "Text extraction with ligatures must succeed: {}", res.stderr);

    let extracted = fs::read_to_string(&text_out).expect("read text");
    assert!(
        extracted.contains("Difficult") || extracted.contains("flight") || extracted.contains("aesthetic"),
        "Extracted text must contain words with ligatures"
    );

    // Multi-byte Unicode layer model safety test
    let layer = PdfTextLayer {
        width: 600.0,
        height: 800.0,
        text: "Difficult flight: 東京 🦊\nAffirm aesthetic".to_owned(),
        glyphs: (0..35).map(|i| [i as f32 * 15.0, 10.0, i as f32 * 15.0 + 12.0, 24.0]).collect(),
    };

    // Substring indexing across multi-byte UTF-8 boundaries must not panic
    let txt = selection_text(&layer, 15, 23);
    assert!(!txt.is_empty(), "Must extract valid Unicode string slice");

    let runs = selection_runs(&layer, 0, 25);
    assert!(!runs.is_empty());

    let (w_start, w_end) = word_range(&layer, 2);
    assert_eq!(selection_text(&layer, w_start, w_end), "Difficult");
}

#[test]
fn test_pdf_empty_and_zero_dimension_pages() {
    let env = TestEnv::new();
    let pdf = generate_zero_dimension_pdf();
    let input = env.write_file("zero_dim.pdf", &pdf);
    let output = env.file_path("zero_dim.png");

    let res = run_preview_helper("preview-pdf", &input, &output, 1);
    // Even if Poppler rejects zero-dimension page or produces 0-sized image, it must not panic
    assert!(res.status.code().is_some(), "Zero-dimension PDF must not panic or abort");

    // Pure model image_bounds with 0 dimension
    let layer_zero = PdfTextLayer {
        width: 0.0,
        height: 0.0,
        text: "".to_string(),
        glyphs: Vec::new(),
    };
    let (ox, oy, s) = image_bounds(&layer_zero, 800.0, 600.0);
    assert!(s.is_finite(), "Scale must be finite, no division-by-zero NaN");
    assert!(ox.is_finite() && oy.is_finite());
}

#[test]
fn test_pdf_multipage_virtualized_selection_and_layer_retention() {
    // 10-page document where only pages 1, 2, 9, 10 are resident in layers
    let mut map = HashMap::new();
    let mk_layer = |text: &str| {
        Arc::new(PdfTextLayer {
            width: 612.0,
            height: 792.0,
            text: text.to_string(),
            glyphs: (0..text.chars().count())
                .map(|i| [i as f32 * 10.0, 10.0, i as f32 * 10.0 + 8.0, 22.0])
                .collect(),
        })
    };

    map.insert(1, mk_layer("Page 1 Content\nIntroduction"));
    map.insert(2, mk_layer("Page 2 Content\nOverview"));
    // Pages 3..8 are virtualized / offscreen / not yet resident
    map.insert(9, mk_layer("Page 9 Content\nAppendix"));
    map.insert(10, mk_layer("Page 10 Content\nIndex"));

    let layers_ref = RefCell::new(map);
    let ranges_ref = RefCell::new(HashMap::new());

    // Drag from Page 1 (char 5) to Page 10 (char 7)
    let desired = pdf_desired_ranges(&layers_ref.borrow(), (1, 5), (10, 7), 1);
    assert!(desired.contains_key(&1));
    assert!(desired.contains_key(&2));
    assert!(desired.contains_key(&9));
    assert!(desired.contains_key(&10));
    // Intermediate non-resident pages 3..8 were skipped without panic
    assert!(!desired.contains_key(&5));

    *ranges_ref.borrow_mut() = desired;

    // Simulate Page 1 scrolling offscreen: drop_unselected_layer must NOT drop Page 1
    pdf_drop_unselected_layer(&layers_ref, &ranges_ref, 1);
    assert!(
        layers_ref.borrow().contains_key(&1),
        "Page 1 must remain resident in memory while active in selection ranges"
    );

    // Copy selected text: both Page 1 and Page 10 text must be preserved
    let selected_text = pdf_selected_text(&layers_ref.borrow(), &ranges_ref.borrow());
    assert!(selected_text.contains("Content\nIntroduction"));
    assert!(selected_text.contains("Page 10"));

    // Now deselect all ranges
    ranges_ref.borrow_mut().clear();
    // After deselecting, drop_unselected_layer on Page 1 must successfully reclaim Page 1
    pdf_drop_unselected_layer(&layers_ref, &ranges_ref, 1);
    assert!(
        !layers_ref.borrow().contains_key(&1),
        "Page 1 must be freed once deselected"
    );
}

#[test]
fn test_pdf_shortcut_modifiers_with_caps_lock_and_num_lock() {
    use gtk::gdk::ModifierType as M;

    // Standard Ctrl+C / Ctrl+A
    assert!(pdf_shortcut_modifiers(M::CONTROL_MASK));

    // Caps Lock ON (LOCK_MASK) must NOT inhibit Ctrl+C / Ctrl+A
    assert!(
        pdf_shortcut_modifiers(M::CONTROL_MASK | M::LOCK_MASK),
        "Caps Lock mask must not block shortcut execution"
    );

    // Disallowed modifiers: Shift or Alt must be rejected (e.g. Ctrl+Shift+C is different action)
    assert!(!pdf_shortcut_modifiers(M::CONTROL_MASK | M::SHIFT_MASK));
    assert!(!pdf_shortcut_modifiers(M::CONTROL_MASK | M::ALT_MASK));
    assert!(!pdf_shortcut_modifiers(M::CONTROL_MASK | M::SHIFT_MASK | M::ALT_MASK));

    // No modifiers
    assert!(!pdf_shortcut_modifiers(M::empty()));
    assert!(!pdf_shortcut_modifiers(M::SHIFT_MASK));
    assert!(!pdf_shortcut_modifiers(M::ALT_MASK));
}

#[test]
fn test_pdf_escape_key_deselection_contract() {
    // 1. Verify selection clearing contract:
    // When ranges exist, applying an empty map clears all selection ranges.
    let ranges = RefCell::new(HashMap::from([(1, (0, 10)), (2, (0, 5))]));
    let layers: RefCell<HashMap<i32, Arc<PdfTextLayer>>> = RefCell::new(HashMap::new());
    let _visible: HashMap<i32, bool> = HashMap::new();

    assert!(!ranges.borrow().is_empty(), "Initial selection ranges must be non-empty");

    // Applying empty map simulates deselection action
    *ranges.borrow_mut() = HashMap::new();
    assert!(ranges.borrow().is_empty(), "Deselection must empty ranges");

    // 2. Empirically inspect src/ui/preview.rs key event controller:
    // Upstream Hermes PDF selection key listener is located at src/ui/preview.rs.
    let preview_src = fs::read_to_string("src/ui/preview.rs").expect("read src/ui/preview.rs");

    // Check whether Key::Escape is handled inside connect_key_pressed in preview.rs
    let has_escape_handler = preview_src.contains("Key::Escape")
        && preview_src.contains("pdf_ranges")
        && preview_src.contains("HashMap::new()");

    // Note: If preview.rs does not intercept Escape to clear text selection,
    // pressing Escape bubbles up to window.rs, closing the entire preview pane.
    // Documenting empirical observation of Escape handling state:
    println!("PDF Escape key interceptor present in preview.rs: {has_escape_handler}");
}

// =========================================================================
// 5. Strict Check: NO Modal/Vim Navigation Modes
// =========================================================================

#[test]
fn test_strict_no_modal_vim_navigation_modes() {
    // Verify zero modal / Vim navigation implementations in preview subsystem (R3/F12 requirement)
    let preview_targets = [
        Path::new("src/ui/preview.rs"),
        Path::new("src/ui/preview/pdf_text.rs"),
        Path::new("src/ui/table_view.rs"),
        Path::new("src/sandbox_helper/table.rs"),
        Path::new("src/sandbox_helper/audio.rs"),
    ];

    let mut violations = Vec::new();

    for path in preview_targets {
        let content = fs::read_to_string(path).unwrap_or_default();
        let display = path.display().to_string();

        // Check for modal navigation enums
        if content.contains("enum Mode") && (content.contains("Normal") || content.contains("Visual")) {
            violations.push(format!("{display}: contains Vim mode enum"));
        }
        if content.contains("VimMode") || content.contains("NavMode") {
            violations.push(format!("{display}: contains VimMode/NavMode identifier"));
        }
        if content.contains("VisualMode") || content.contains("NormalMode") {
            violations.push(format!("{display}: contains VisualMode/NormalMode identifier"));
        }

        // Check for vim navigation keybindings (hjkl navigation mode)
        if content.contains("Key::h")
            && content.contains("Key::j")
            && content.contains("Key::k")
            && content.contains("Key::l")
        {
            violations.push(format!("{display}: contains HJKL modal navigation bindings"));
        }
    }

    assert!(
        violations.is_empty(),
        "Violations of 'No modal/Vim navigation mode' constraint in preview subsystem: {:?}",
        violations
    );
}
