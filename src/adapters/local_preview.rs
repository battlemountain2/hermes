// SPDX-License-Identifier: GPL-3.0-or-later

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::{
    model::Location,
    sandbox::{Cancellation, ParseOperation},
    services::{
        FormatFamily, LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewHandler,
        PreviewProvider, PreviewRequest, classify_by_mime, classify_by_name,
    },
};

#[derive(Default)]
pub struct LocalPreviewProvider;

impl PreviewProvider for LocalPreviewProvider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        let request_id = request.id;
        let entry = request.entry.clone();
        let cancellation = Cancellation::default();
        let cancellation_for_task = cancellation.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let file = file_for_location(&entry.location);
            let info = match file
                .query_info_future(
                    "standard::content-type",
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await
            {
                Ok(info) => info,
                Err(error) => {
                    emit(PreviewEvent::Failed {
                        request_id,
                        entry,
                        message: error.to_string(),
                    });
                    return;
                }
            };
            let content_type = info
                .content_type()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "application/octet-stream".to_owned());
            let mut family = classify_by_mime(&content_type);
            if family == FormatFamily::Unknown {
                if gio::content_type_is_a(&content_type, "text/plain") {
                    family = FormatFamily::PlainText;
                } else {
                    family = classify_by_name(&entry.native_name);
                }
            }

            let handler = family.preview_handler();
            let mut content = match handler {
                Some(PreviewHandler::Text) => PreviewContent::Text {
                    content: String::new(),
                    truncated: false,
                },
                Some(PreviewHandler::Pdf) => PreviewContent::Pdf {
                    png: Vec::new(),
                    page: 0,
                    pages: 0,
                    text_layer: None,
                },
                Some(PreviewHandler::Audio) => PreviewContent::AudioWaveform {
                    png: Vec::new(),
                    metadata: crate::services::preview::AudioMetadata {
                        format: String::new(),
                        duration_seconds: 0.0,
                        sample_rate: 0,
                        channels: 0,
                        bitrate: None,
                    },
                },
                Some(PreviewHandler::Video) => PreviewContent::Media,
                Some(PreviewHandler::Image | PreviewHandler::Heif | PreviewHandler::GeoTiff) => {
                    PreviewContent::Image
                }
                Some(PreviewHandler::Archive | PreviewHandler::Office) => PreviewContent::Text {
                    content: String::new(),
                    truncated: false,
                },
                Some(PreviewHandler::Model | PreviewHandler::ArchiveCover) => {
                    PreviewContent::Rasterized { png: Vec::new() }
                }
                Some(PreviewHandler::Spreadsheet) => PreviewContent::Spreadsheet {
                    table: crate::services::table::SpreadsheetData::default(),
                },
                _ => PreviewContent::Unsupported,
            };

            let operation = handler.and_then(preview_operation);
            if let Some(operation) = operation {
                let Some(path) = entry.location.native_path().map(ToOwned::to_owned) else {
                    emit(PreviewEvent::Failed {
                        request_id,
                        entry,
                        message: "Only local files can be previewed safely".to_owned(),
                    });
                    return;
                };
                let value = request.pdf_page;
                let cancellation = cancellation_for_task.clone();
                content = match gio::spawn_blocking(move || {
                    crate::sandbox::parse(&path, operation, value, &cancellation)
                })
                .await
                {
                    Ok(Ok(output)) if operation == ParseOperation::PreviewPdf => {
                        PreviewContent::Pdf {
                            png: output.data,
                            page: output.page,
                            pages: output.pages,
                            text_layer: output.text_layer.map(std::sync::Arc::new),
                        }
                    }
                    Ok(Ok(output)) if operation == ParseOperation::PreviewGeoTiff => {
                        let metadata: Option<crate::sandbox_helper::geotiff::GeoTiffMetadata> = output
                            .metadata
                            .as_ref()
                            .and_then(|bytes| serde_json::from_slice(bytes).ok());
                        match metadata {
                            Some(metadata) => PreviewContent::GeoTiff {
                                png: output.data,
                                metadata,
                            },
                            None => PreviewContent::Rasterized { png: output.data },
                        }
                    }
                    Ok(Ok(output)) if operation == ParseOperation::PreviewAudioWaveform => {
                        let metadata: Option<crate::services::preview::AudioMetadata> = output
                            .metadata
                            .as_ref()
                            .and_then(|bytes| serde_json::from_slice(bytes).ok());
                        match metadata {
                            Some(metadata) => PreviewContent::AudioWaveform {
                                png: output.data,
                                metadata,
                            },
                            None => PreviewContent::Rasterized { png: output.data },
                        }
                    }
                    Ok(Ok(output)) if operation == ParseOperation::PreviewSpreadsheet => {
                        let table: Option<crate::services::table::SpreadsheetData> = output
                            .metadata
                            .as_ref()
                            .and_then(|bytes| serde_json::from_slice(bytes).ok());
                        match table {
                            Some(table) => PreviewContent::Spreadsheet { table },
                            None => PreviewContent::Unsupported,
                        }
                    }
                    Ok(Ok(output))
                        if matches!(
                            operation,
                            ParseOperation::PreviewMedia | ParseOperation::PreviewAudio
                        ) =>
                    {
                        PreviewContent::SandboxedMedia { data: output.data }
                    }
                    Ok(Ok(output))
                        if matches!(
                            operation,
                            ParseOperation::PreviewArchive | ParseOperation::PreviewOffice
                        ) =>
                    {
                        PreviewContent::Text {
                            content: String::from_utf8_lossy(&output.data).into_owned(),
                            truncated: output.data.len() >= request.text_byte_limit,
                        }
                    }
                    Ok(Ok(output)) => PreviewContent::Rasterized { png: output.data },
                    Ok(Err(message)) => {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message,
                        });
                        return;
                    }
                    Err(_) => return,
                };
            } else if entry.kind == crate::model::EntryKind::Directory {
                if let Some(path) = entry.location.native_path().map(ToOwned::to_owned) {
                    let cancellation = cancellation_for_task.clone();
                    if let Ok(Some(output)) = gio::spawn_blocking(move || {
                        crate::ui::thumbnail::folder_album_art(&path)
                            .and_then(|(art, op)| crate::sandbox::parse(&art, op, 800, &cancellation).ok())
                    })
                    .await
                    {
                        content = PreviewContent::Rasterized { png: output.data };
                    }
                }
            } else if matches!(content, PreviewContent::Text { .. }) {
                content = match read_text(&file, request.text_byte_limit).await {
                    Ok((content, truncated)) => PreviewContent::Text { content, truncated },
                    Err(error) => {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: error.to_string(),
                        });
                        return;
                    }
                };
            }

            emit(PreviewEvent::Ready(Preview {
                request_id,
                entry,
                content_type,
                content,
            }));
        });

        LoadHandle::new(move || {
            cancellation.cancel();
            task.abort();
        })
    }
}

fn preview_operation(handler: PreviewHandler) -> Option<ParseOperation> {
    match handler {
        PreviewHandler::Text => None,
        PreviewHandler::Image => Some(ParseOperation::PreviewImage),
        PreviewHandler::Heif => Some(ParseOperation::PreviewHeif),
        PreviewHandler::Pdf => Some(ParseOperation::PreviewPdf),
        PreviewHandler::Audio => Some(ParseOperation::PreviewAudioWaveform),
        PreviewHandler::Video => Some(ParseOperation::PreviewMedia),
        PreviewHandler::Archive => Some(ParseOperation::PreviewArchive),
        PreviewHandler::Office => Some(ParseOperation::PreviewOffice),
        PreviewHandler::GeoTiff => Some(ParseOperation::PreviewGeoTiff),
        PreviewHandler::Model => Some(ParseOperation::PreviewModel),
        PreviewHandler::ArchiveCover => Some(ParseOperation::PreviewArchiveCover),
        PreviewHandler::Spreadsheet => Some(ParseOperation::PreviewSpreadsheet),
    }
}

fn file_for_location(location: &Location) -> gio::File {
    location
        .native_path()
        .map(gio::File::for_path)
        .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
}

async fn read_text(file: &gio::File, byte_limit: usize) -> Result<(String, bool), glib::Error> {
    let stream = file.read_future(glib::Priority::DEFAULT).await?;
    let bytes = stream
        .read_bytes_future(byte_limit.saturating_add(1), glib::Priority::DEFAULT)
        .await?;
    let bytes = bytes.as_ref();
    let truncated = bytes.len() > byte_limit;
    let sample = &bytes[..bytes.len().min(byte_limit)];
    Ok((String::from_utf8_lossy(sample).into_owned(), truncated))
}

#[cfg(test)]
mod tests;
