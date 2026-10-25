// SPDX-License-Identifier: GPL-3.0-or-later

mod cached_files;
mod local_files;
mod local_operations;
mod local_preview;
mod local_text_extraction;
mod local_trails;
pub(crate) mod music_metadata;

mod search_source;

pub use local_files::LocalFileSource;
pub use local_operations::LocalOperationProvider;
pub use local_preview::LocalPreviewProvider;
pub use local_text_extraction::LocalTextExtractionProvider;
pub use local_trails::LocalTrailStore;
pub use search_source::SearchFileSource;

use crate::{
    model::Location,
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
        LocationValidationError,
    },
};
use std::rc::Rc;

pub struct RoutedFileSource {
    local: cached_files::CachedFileSource,
    search: SearchFileSource,
}

impl RoutedFileSource {
    pub fn new() -> Self {
        Self {
            local: cached_files::CachedFileSource::new(Rc::new(LocalFileSource)),
            search: SearchFileSource::new(),
        }
    }
}

impl FileSource for RoutedFileSource {
    fn invalidate(&self, location: &Location) {
        if location.is_home_dashboard() {
            return;
        }
        if location
            .uri_value()
            .is_some_and(|uri| uri.starts_with("search://"))
        {
            self.search.invalidate(location);
        } else {
            self.local.invalidate(location);
        }
    }

    fn watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        if location.is_home_dashboard() {
            return None;
        }
        if location
            .uri_value()
            .is_some_and(|uri| uri.starts_with("search://"))
        {
            None
        } else {
            self.local.watch(location, include_hidden, notify)
        }
    }

    fn watch_invalidation(&self, location: &Location, notify: Rc<dyn Fn()>) -> Option<LoadHandle> {
        if location.is_home_dashboard() {
            return None;
        }
        if location
            .uri_value()
            .is_some_and(|uri| uri.starts_with("search://"))
        {
            None
        } else {
            self.local.watch_invalidation(location, notify)
        }
    }

    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if location.is_home_dashboard() {
            return Ok(());
        }
        if location
            .uri_value()
            .is_some_and(|u| u.starts_with("search://"))
        {
            self.search.validate_location(location)
        } else {
            self.local.validate_location(location)
        }
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if request.location.is_home_dashboard() {
            let request_id = request.id;
            gtk::glib::idle_add_local_once(move || {
                emit(DirectoryEvent::Finished { request_id });
            });
            return LoadHandle::new(|| {});
        }
        if request
            .location
            .uri_value()
            .is_some_and(|u| u.starts_with("search://"))
        {
            self.search.enumerate(request, emit)
        } else {
            self.local.enumerate(request, emit)
        }
    }
}
