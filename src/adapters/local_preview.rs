// SPDX-License-Identifier: GPL-3.0-or-later

use std::rc::Rc;

use gtk::{gio, glib, prelude::*};

use crate::{
    model::{FileEntry, Location},
    sandbox::{Cancellation, ParseOperation},
    services::{
        FormatFamily, LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewHandler,
        PreviewProvider, PreviewRequest, classify_by_mime, classify_by_name,
    },
    ui::thumbnail::PreviewPolicy,
};

#[derive(Default)]
pub struct LocalPreviewProvider;

impl LocalPreviewProvider {
    pub(crate) fn check_policy(
        entry: &FileEntry,
        content_type: Option<&str>,
    ) -> Result<(), String> {
        PreviewPolicy::current().for_streaming_preview().check_entry(entry, content_type)
    }
}

fn preview_family(content_type: &str, name: &std::ffi::OsStr) -> FormatFamily {
    let family = classify_by_mime(content_type);
    if family != FormatFamily::Unknown {
        family
    } else if gio::content_type_is_a(content_type, "text/plain") {
        FormatFamily::PlainText
    } else {
        classify_by_name(name)
    }
}

impl LocalPreviewProvider {
    fn load_with_layout(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>, layout: bool) -> LoadHandle {
        let request_id = request.id;
        let entry = request.entry.clone();
        if let Err(message) = Self::check_policy(&entry, None) {
            emit(PreviewEvent::Failed { request_id, entry, message });
            return LoadHandle::new(|| {});
        }
        let cancellation = Cancellation::default();
        let cancellation_for_task = cancellation.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let file = file_for_location(&entry.location);
            let info = match file
                .query_info_future(
                    "standard::content-type,standard::size",
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
            let family = preview_family(&content_type, &entry.native_name);
            let source_size = (!entry.is_directory()
                && info.has_attribute("standard::size")
                && info.size() >= 0)
                .then(|| info.size() as u64);
            let policy = PreviewPolicy::current();
            let permitted = policy.for_streaming_preview().check_source(&entry, &content_type, source_size);
            if let Err(message) = permitted {
                emit(PreviewEvent::Failed { request_id, entry, message });
                return;
            }

            let handler = if entry.is_directory() { None } else { family.preview_handler() };
            let mut folder_source = None;
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
                Some(PreviewHandler::Audio) => PreviewContent::Audio {
                    metadata: crate::services::preview::AudioMetadata::default(),
                },
                Some(PreviewHandler::Video) => {
                    let Some(path) = entry.location.native_path() else {
                        emit(PreviewEvent::Failed {
                            request_id,
                            entry,
                            message: "Only local files can be played safely".into(),
                        });
                        return;
                    };
                    PreviewContent::StreamingMedia {
                        path: path.to_owned(),
                    }
                }
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

            let workbook_layout = layout && family == FormatFamily::Spreadsheet;
            let operation = if workbook_layout {
                Some(ParseOperation::PreviewOffice)
            } else if !entry.is_directory() && crate::services::is_gif_image(&entry.native_name, &content_type) {
                Some(ParseOperation::PreviewAnimation)
            } else {
                handler.and_then(preview_operation)
            };
            if let Some(operation) = operation {
                let Some(path) = entry.location.native_path().map(ToOwned::to_owned) else {
                    emit(PreviewEvent::Failed {
                        request_id,
                        entry,
                        message: "Only local files can be previewed safely".to_owned(),
                    });
                    return;
                };
                let value =
                    preview_operation_value(operation, request.pdf_page, request.text_byte_limit);
                let page = request.pdf_page;
                let cancellation = cancellation_for_task.clone();
                let result = gio::spawn_blocking(move || {
                    let output = if operation == ParseOperation::PreviewOffice {
                        match crate::sandbox::office::render(&path, page, &cancellation) {
                            Ok(output) => return Ok((output, None)),
                            Err(_) if cancellation.is_cancelled() => return Err("Preview cancelled".into()),
                            Err(error) if workbook_layout => return Err(format!("Workbook layout unavailable: {error}")),
                            Err(error) => {
                                tracing::debug!(%error, "Office layout unavailable; using extracted text");
                                let reason = if crate::sandbox::office::available() {
                                    "Page layout could not be rendered. Showing extracted text instead."
                                } else {
                                    "Install LibreOffice for page and slide layouts. Showing extracted text instead."
                                };
                                return crate::sandbox::parse(&path, operation, value, &cancellation)
                                    .map(|output| (output, Some(reason.to_owned())));
                            }
                        }
                    } else if operation == ParseOperation::PreviewAudioMetadata {
                        super::music_metadata::read_audio_metadata(&path, &cancellation)
                    } else {
                        crate::sandbox::parse(&path, operation, value, &cancellation)
                    }?;
                    Ok((output, None))
                })
                .await;
                content = match result {
                    Ok(Ok((output, None))) if matches!(operation, ParseOperation::PreviewPdf | ParseOperation::PreviewOffice) => {
                        PreviewContent::Pdf {
                            png: output.data,
                            page: output.page,
                            pages: output.pages,
                            text_layer: output.text_layer.map(std::sync::Arc::new),
                        }
                    }
                    Ok(Ok((output, _))) if operation == ParseOperation::PreviewGeoTiff => {
                        let metadata: Option<crate::sandbox_helper::geotiff::GeoTiffMetadata> =
                            output
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
                    Ok(Ok((output, _))) if operation == ParseOperation::PreviewAudioMetadata => {
                        let metadata: Option<crate::services::preview::AudioMetadata> =
                            serde_json::from_slice(&output.data).ok();
                        match metadata {
                            Some(metadata) => PreviewContent::Audio { metadata },
                            None => {
                                emit(PreviewEvent::Failed {
                                    request_id,
                                    entry,
                                    message: "Invalid audio metadata response".into(),
                                });
                                return;
                            }
                        }
                    }
                    Ok(Ok((output, _))) if operation == ParseOperation::PreviewSpreadsheet => {
                        let table: Option<crate::services::table::SpreadsheetData> = output
                            .metadata
                            .as_ref()
                            .and_then(|bytes| serde_json::from_slice(bytes).ok());
                        match table {
                            Some(table) => PreviewContent::Spreadsheet { table },
                            None => PreviewContent::Unsupported,
                        }
                    }
                    Ok(Ok((output, _))) if operation == ParseOperation::PreviewAnimation => {
                        PreviewContent::SandboxedAnimation { data: output.data }
                    }
                    Ok(Ok((output, _))) if operation == ParseOperation::PreviewAudio => {
                        PreviewContent::SandboxedMedia { data: output.data }
                    }
                    Ok(Ok((output, reason)))
                        if matches!(
                            operation,
                            ParseOperation::PreviewArchive | ParseOperation::PreviewOffice
                        ) =>
                    {
                        let content = String::from_utf8_lossy(&output.data).into_owned();
                        let truncated = output.data.len() >= request.text_byte_limit;
                        if let Some(reason) = reason {
                            PreviewContent::OfficeText { content, truncated, reason }
                        } else {
                            PreviewContent::Text { content, truncated }
                        }
                    }
                    Ok(Ok((output, _))) => PreviewContent::Rasterized { png: output.data },
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
            } else if entry.is_directory() && (policy.images || policy.audio) {
                if let Some(path) = entry.location.native_path().map(ToOwned::to_owned) {
                    let cancellation = cancellation_for_task.clone();
                    if let Ok(Some((output, family, size))) = gio::spawn_blocking(move || {
                        crate::ui::thumbnail::folder_album_art(&path).and_then(|(art, op)| {
                            let size = std::fs::metadata(&art).ok()?.len();
                            let family = classify_by_name(art.as_os_str());
                            policy.check(family, Some(size)).ok()?;
                            let output = crate::sandbox::parse(&art, op, 800, &cancellation).ok()?;
                            Some((output, family, size))
                        })
                    })
                    .await
                    {
                        folder_source = Some((family, size));
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

            let current_policy = PreviewPolicy::current();
            let permitted = current_policy.for_streaming_preview().check_source(&entry, &content_type, source_size).and_then(|()| {
                if let Some((family, size)) = folder_source {
                    current_policy.check(family, Some(size))
                } else {
                    Ok(())
                }
            });
            if let Err(message) = permitted {
                emit(PreviewEvent::Failed { request_id, entry, message });
                return;
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

impl PreviewProvider for LocalPreviewProvider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.load_with_layout(request, emit, false)
    }

    fn load_layout(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.load_with_layout(request, emit, true)
    }
}

fn preview_operation(handler: PreviewHandler) -> Option<ParseOperation> {
    match handler {
        PreviewHandler::Text => None,
        PreviewHandler::Image => Some(ParseOperation::PreviewImage),
        PreviewHandler::Heif => Some(ParseOperation::PreviewHeif),
        PreviewHandler::Pdf => Some(ParseOperation::PreviewPdf),
        PreviewHandler::Audio => Some(ParseOperation::PreviewAudioMetadata),
        PreviewHandler::Video => None,
        PreviewHandler::Archive => Some(ParseOperation::PreviewArchive),
        PreviewHandler::Office => Some(ParseOperation::PreviewOffice),
        PreviewHandler::GeoTiff => Some(ParseOperation::PreviewGeoTiff),
        PreviewHandler::Model => Some(ParseOperation::PreviewModel),
        PreviewHandler::ArchiveCover => Some(ParseOperation::PreviewArchiveCover),
        PreviewHandler::Spreadsheet => Some(ParseOperation::PreviewSpreadsheet),
    }
}

fn preview_operation_value(
    operation: ParseOperation,
    pdf_page: i32,
    text_byte_limit: usize,
) -> i32 {
    match operation {
        ParseOperation::PreviewPdf => pdf_page,
        ParseOperation::PreviewArchive | ParseOperation::PreviewOffice => {
            text_byte_limit.clamp(1, crate::sandbox::MAX_OUTPUT_BYTES as usize) as i32
        }
        ParseOperation::PreviewModel => 800,
        ParseOperation::PreviewArchiveCover => 1400,
        _ => 0,
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
