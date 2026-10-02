// SPDX-License-Identifier: GPL-3.0-or-later

use std::rc::Rc;

use crate::model::FileEntry;

use super::LoadHandle;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreviewRequestId(pub u64);

#[derive(Clone, Debug)]
pub struct PreviewRequest {
    pub id: PreviewRequestId,
    pub entry: FileEntry,
    pub text_byte_limit: usize,
    pub pdf_page: i32,
}

pub use crate::sandbox_helper::geotiff::GeoTiffMetadata;
pub use crate::services::table::SpreadsheetData;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PdfTextLayer {
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub glyphs: Vec<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioMetadata {
    pub format: String,
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
#[expect(dead_code, reason = "Preview content variants")]
pub enum PreviewContent {
    Text { content: String, truncated: bool },
    Image,
    Media,
    Rasterized { png: Vec<u8> },
    SandboxedMedia { data: Vec<u8> },
    Pdf {
        png: Vec<u8>,
        page: i32,
        pages: i32,
        text_layer: Option<std::sync::Arc<PdfTextLayer>>,
    },
    Code { language: String, content: String },
    Markdown { content: String },
    Model3D { format: String, data: Vec<u8> },
    GeoTiff { png: Vec<u8>, metadata: GeoTiffMetadata },
    Spreadsheet { table: SpreadsheetData },
    AudioWaveform { png: Vec<u8>, metadata: AudioMetadata },
    Unsupported,
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub request_id: PreviewRequestId,
    pub entry: FileEntry,
    pub content_type: String,
    pub content: PreviewContent,
}

#[derive(Clone, Debug)]
pub enum PreviewEvent {
    Ready(Preview),
    Failed {
        request_id: PreviewRequestId,
        entry: FileEntry,
        message: String,
    },
}

pub trait PreviewProvider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle;
}

#[cfg(test)]
mod tests;
