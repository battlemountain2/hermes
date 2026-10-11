// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

use gtk::glib;

use crate::{
    adapters::LocalTextExtractionProvider,
    model::{FileEntry, Location},
    services::{
        DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
        ScopedSearch, SearchEvent, index_tree,
    },
};

#[derive(Default)]
pub struct SearchFileSource;

impl SearchFileSource {
    pub fn new() -> Self {
        Self
    }
}

impl FileSource for SearchFileSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        let uri = location.uri_value().unwrap_or_default();
        ScopedSearch::from_uri(uri)
            .map(|_| ())
            .map_err(LocationValidationError::Unavailable)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let uri = request.location.uri_value().unwrap_or_default();
        let search = match ScopedSearch::from_uri(uri) {
            Ok(search) => search,
            Err(message) => {
                emit(DirectoryEvent::Failed {
                    request_id: request.id,
                    message,
                });
                return LoadHandle::new(|| {});
            }
        };
        let query = search.query;
        let (handle, receiver) = index_tree(search.root, Arc::new(LocalTextExtractionProvider));
        handle.query(&query);
        let live = handle.is_live();

        let active = Rc::new(Cell::new(true));
        let poll_active = active.clone();
        let mut previous = None;

        glib::timeout_add_local(Duration::from_millis(16), move || {
            if !poll_active.get() {
                return glib::ControlFlow::Break;
            }

            let Some(event) = receiver.try_recv() else {
                return glib::ControlFlow::Continue;
            };
            let flow = deliver_results(
                event,
                &query,
                request.id,
                &mut previous,
                &poll_active,
                emit.as_ref(),
            );
            if live && poll_active.get() {
                glib::ControlFlow::Continue
            } else {
                flow
            }
        });

        LoadHandle::new(move || {
            active.set(false);
            drop(handle);
        })
    }
}

fn deliver_results(
    event: SearchEvent,
    query: &str,
    request_id: crate::services::RequestId,
    previous: &mut Option<Vec<FileEntry>>,
    active: &Cell<bool>,
    emit: &dyn Fn(DirectoryEvent),
) -> glib::ControlFlow {
    if !active.get() {
        return glib::ControlFlow::Break;
    }
    let SearchEvent::Results {
        query: event_query,
        items,
        indexing,
        notice,
    } = event;
    if event_query != query {
        return glib::ControlFlow::Continue;
    }
    let entries: Vec<_> = items.iter().map(search_entry).collect();
    if previous.as_ref() != Some(&entries) {
        previous.replace(entries.clone());
        emit(DirectoryEvent::Snapshot {
            request_id,
            entries,
        });
    }
    if !active.get() {
        return glib::ControlFlow::Break;
    }
    if indexing {
        return glib::ControlFlow::Continue;
    }
    if let Some(message) = notice.filter(|_| items.is_empty()) {
        emit(DirectoryEvent::Failed {
            request_id,
            message,
        });
    } else {
        emit(DirectoryEvent::Finished { request_id });
    }
    glib::ControlFlow::Break
}

fn search_entry(item: &crate::services::SearchItem) -> FileEntry {
    item.file_entry()
}

#[cfg(test)]
mod tests;
