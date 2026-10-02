// SPDX-License-Identifier: GPL-3.0-or-later

mod custom_actions;
mod file_source;
mod formats;
pub(crate) mod model_preview;
mod operations;
pub(crate) mod preview;
mod search;
pub(crate) mod table;
mod trails;

pub(crate) use custom_actions::{
    CustomAction, actions_path, create_custom_actions_template, load_custom_actions,
};
pub use file_source::{
    DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
    LocationValidationError, RequestId,
};
#[cfg(test)]
pub(crate) use formats::thumbnail_handler_for_name;
pub(crate) use formats::{
    FormatCapabilities, FormatFamily, PreviewHandler, TextExtractor, ThumbnailHandler,
    capabilities_by_name, classify_by_mime, classify_by_name,
};
pub use operations::{
    CompressArchiveRequest, CreateDirectoryRequest, CreateFileRequest, DeleteRequest,
    ExtractArchiveRequest, OperationEvent, OperationProvider, OperationRequestId, PasteRequest,
    RenameRequest, RestoreRequest, validate_basename,
};
pub use preview::{
    PdfTextLayer, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
    PreviewRequestId,
};
pub(crate) use search::{
    SearchEvent, SearchFilterValues, SearchHandle, SearchItem, TextExtractionProvider, index_tree,
    search_filter_values,
};
pub use trails::{StoredTrails, TRAIL_SCHEMA_VERSION, TrailStore};
