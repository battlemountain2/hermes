// SPDX-License-Identifier: GPL-3.0-or-later

mod file_source;
mod formats;
mod operations;
mod preview;
mod search;
mod trails;

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
    CreateDirectoryRequest, CreateFileRequest, DeleteRequest, OperationEvent, OperationProvider,
    OperationRequestId, PasteRequest, RenameRequest, RestoreRequest, validate_basename,
};
pub use preview::{
    Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
};
pub(crate) use search::{
    SearchEvent, SearchFilterValues, SearchHandle, SearchItem, index_tree, search_filter_values,
};
pub use trails::{StoredTrails, TRAIL_SCHEMA_VERSION, TrailStore};
