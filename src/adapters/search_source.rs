// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

use gtk::glib;

use crate::{
    adapters::LocalTextExtractionProvider,
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{
        DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
        SearchEvent, index_tree,
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
        if !uri.starts_with("search://") {
            return Err(LocationValidationError::NotAbsolute);
        }
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let uri = request.location.uri_value().unwrap_or_default();
        let query = search_query(uri).to_owned();

        let root = gtk::glib::home_dir();
        let (handle, receiver) = index_tree(root, Arc::new(LocalTextExtractionProvider));
        handle.query(&query);

        let active = Rc::new(Cell::new(true));
        let poll_active = active.clone();

        glib::timeout_add_local(Duration::from_millis(16), move || {
            if !poll_active.get() {
                return glib::ControlFlow::Break;
            }

            let latest = receiver.try_iter().last();
            let Some(SearchEvent::Results {
                query: event_query,
                items,
                indexing: false,
                ..
            }) = latest
            else {
                return glib::ControlFlow::Continue;
            };
            if event_query != query {
                return glib::ControlFlow::Continue;
            }

            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries: items.iter().map(search_entry).collect(),
            });
            emit(DirectoryEvent::Finished {
                request_id: request.id,
            });
            glib::ControlFlow::Break
        });

        LoadHandle::new(move || {
            active.set(false);
            drop(handle);
        })
    }
}

fn search_query(uri: &str) -> &str {
    uri.strip_prefix("search://").unwrap_or_default()
}

fn search_entry(item: &crate::services::SearchItem) -> FileEntry {
    FileEntry {
        location: Location::local(&item.path),
        native_name: item.path.file_name().unwrap_or_default().to_owned(),
        display_name: item.name.clone(),
        kind: if item.is_directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        size: MetadataValue::Unknown,
        modified_unix_seconds: item
            .modified_unix_seconds
            .map_or(MetadataValue::Unknown, MetadataValue::Known),
    }
}

#[cfg(test)]
mod tests {
    use super::search_query;

    #[test]
    fn extracts_search_queries_without_accepting_other_uris() {
        assert_eq!(search_query("search://modified:today"), "modified:today");
        assert_eq!(search_query("file:///tmp"), "");
    }
}
