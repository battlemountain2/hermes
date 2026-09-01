// SPDX-License-Identifier: GPL-3.0-or-later

mod file_source;
mod operations;
mod preview;
mod search;
mod trails;

pub use file_source::{
    DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
    LocationValidationError, RequestId,
};
pub use operations::{
    CreateDirectoryRequest, CreateFileRequest, DeleteRequest, OperationEvent, OperationProvider,
    OperationRequestId, PasteRequest, RenameRequest, RestoreRequest, validate_basename,
};
pub use preview::{
    Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
};
pub(crate) use preview::{content_family, has_plain_text_extension};
pub(crate) use search::{
    SearchEvent, SearchFilterValues, SearchHandle, SearchItem, index_tree, search_filter_values,
};
pub use trails::{StoredTrails, TRAIL_SCHEMA_VERSION, TrailStore};
