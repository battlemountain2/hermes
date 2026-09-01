// SPDX-License-Identifier: GPL-3.0-or-later

mod local_files;
mod local_operations;
mod local_preview;
mod local_trails;

mod search_source;

pub use local_files::LocalFileSource;
pub use local_operations::LocalOperationProvider;
pub use local_preview::LocalPreviewProvider;
pub use local_trails::LocalTrailStore;
pub use search_source::SearchFileSource;

use crate::{
    model::Location,
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
};
use std::rc::Rc;

pub struct RoutedFileSource {
    local: LocalFileSource,
    search: SearchFileSource,
}

impl RoutedFileSource {
    pub fn new() -> Self {
        Self {
            local: LocalFileSource::default(),
            search: SearchFileSource::new(),
        }
    }
}

impl FileSource for RoutedFileSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if location.uri_value().is_some_and(|u| u.starts_with("search://")) {
            self.search.validate_location(location)
        } else {
            self.local.validate_location(location)
        }
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if request.location.uri_value().is_some_and(|u| u.starts_with("search://")) {
            self.search.enumerate(request, emit)
        } else {
            self.local.enumerate(request, emit)
        }
    }
}
