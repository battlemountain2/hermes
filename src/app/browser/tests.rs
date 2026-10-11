// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, ffi::OsString};

use super::*;
use crate::{
    model::{EntryKind, MetadataValue},
    services::LoadHandle,
};

#[test]
fn search_snapshot_observer_can_navigate_without_stale_selection_events() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.state.borrow_mut().navigate(Location::local("/search"), RequestId(101));
    let song = crate::app::music_order::tests::entry("song.flac", Some(1), Some(1));
    browser.handle_directory_event(DirectoryEvent::Snapshot { request_id: RequestId(101), entries: vec![song.clone()] });
    browser.state.borrow_mut().select(0, 0);
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let navigated = browser.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::EntriesReplaced { .. }) {
            navigated.state.borrow_mut().navigate(Location::local("/elsewhere"), RequestId(102));
        }
        captured.borrow_mut().push(event);
    });
    browser.handle_directory_event(DirectoryEvent::Snapshot { request_id: RequestId(101), entries: vec![song] });
    assert!(!events.borrow().iter().any(|event| matches!(event, BrowserEvent::SelectionSetChanged { .. })));
    browser.handle_directory_event(DirectoryEvent::Snapshot { request_id: RequestId(101), entries: vec![] });
    assert_eq!(events.borrow().len(), 1);
}

#[test]
fn late_album_metadata_updates_folder_and_peek_snapshots() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.state.borrow_mut().navigate(Location::local("/album"), RequestId(101));
    let first = crate::app::music_order::tests::entry("9. First.flac", Some(1), Some(1));
    let second = crate::app::music_order::tests::entry("1. Second.flac", Some(1), Some(2));
    let untagged = |entry: &FileEntry| {
        let mut entry = entry.clone();
        Rc::make_mut(&mut entry.0).music_tags = None;
        entry
    };
    browser.handle_directory_event(DirectoryEvent::Batch { request_id: RequestId(101), entries: vec![untagged(&first), untagged(&second)] });
    browser.handle_directory_event(DirectoryEvent::Finished { request_id: RequestId(101) });
    browser.handle_directory_event(DirectoryEvent::MusicMetadata { request_id: RequestId(101), entries: vec![first.clone(), second.clone()] });
    assert_eq!(browser.column_entries(0).unwrap()[0].location, first.location);
    assert!(events.borrow().iter().any(|event| matches!(event,
        BrowserEvent::EntriesReplaced { entries, .. } if entries.first().is_some_and(|entry| entry.location == first.location))));
    browser.state.borrow_mut().begin_peek(0, Location::local("/album"), RequestId(102));
    browser.handle_directory_event(DirectoryEvent::Batch { request_id: RequestId(102), entries: vec![untagged(&second), untagged(&first)] });
    browser.handle_directory_event(DirectoryEvent::Finished { request_id: RequestId(102) });
    browser.handle_directory_event(DirectoryEvent::MusicMetadata { request_id: RequestId(102), entries: vec![first.clone(), second] });
    assert!(events.borrow().iter().any(|event| matches!(event,
        BrowserEvent::PeekEntriesReplaced { entries } if entries.first().is_some_and(|entry| entry.location == first.location))));
}

#[test]
fn deleted_trash_entries_refresh_the_trash_root() {
    let entry = crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { music_tags: None, location: Location::uri("trash:///photo.jpg"),
        native_name: "photo.jpg".into(),
        display_name: "photo.jpg".into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(10),
        modified_unix_seconds: MetadataValue::Unknown,
    }));

    assert_eq!(
        deletion_parent_location(&entry.location),
        Some(Location::uri("trash:///"))
    );
}

#[test]
fn invalid_new_folder_names_are_rejected_before_an_operation_starts() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));

    browser.create_directory(Location::local("/fixture"), "../escaped".to_owned());

    assert_eq!(browser.current_operation.get(), None);
    assert!(browser.operation_load.borrow().is_none());
    assert!(matches!(
        events.borrow().as_slice(),
        [BrowserEvent::OperationFailed { message }] if message == "Names cannot contain /"
    ));
}

struct FakeFileSource;

#[test]
fn restarted_directory_events_reset_models_without_stale_peek_duplicates() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let captured = Rc::new(RefCell::new(Vec::new()));
    let observed = captured.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.state.borrow_mut().navigate(Location::local("/album"), RequestId(101));
    browser.state.borrow_mut().begin_peek(0, Location::local("/album/child"), RequestId(102));
    let first = crate::app::music_order::tests::entry("1. First.flac", Some(1), Some(1));
    let second = crate::app::music_order::tests::entry("2. Second.flac", Some(1), Some(2));
    browser.handle_directory_event(DirectoryEvent::Batch { request_id: RequestId(102), entries: vec![first] });
    captured.borrow_mut().clear();
    browser.handle_directory_event(DirectoryEvent::Restarted { request_id: RequestId(100) });
    assert!(captured.borrow().is_empty());
    browser.handle_directory_event(DirectoryEvent::Restarted { request_id: RequestId(101) });
    browser.handle_directory_event(DirectoryEvent::Restarted { request_id: RequestId(102) });
    assert!(matches!(captured.borrow()[0], BrowserEvent::ColumnReloaded { depth: 0 }));
    assert!(matches!(&captured.borrow()[1], BrowserEvent::PeekEntriesReplaced { entries } if entries.is_empty()));
    browser.handle_directory_event(DirectoryEvent::Batch { request_id: RequestId(102), entries: vec![second.clone()] });
    browser.handle_directory_event(DirectoryEvent::Finished { request_id: RequestId(102) });
    assert_eq!(browser.state.borrow().peek_entries(), vec![second]);
}

#[test]
fn refreshing_a_column_invalidates_warm_entries_before_enumerating() {
    struct FreshSource(Rc<RefCell<Vec<String>>>);

    impl FileSource for FreshSource {
        fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
            Ok(())
        }

        fn enumerate(&self, request: DirectoryRequest, _: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
            self.0.borrow_mut().push(format!("load {}", request.location.display_path()));
            LoadHandle::new(|| {})
        }

        fn invalidate(&self, location: &Location) {
            self.0.borrow_mut().push(format!("invalidate {}", location.display_path()));
        }
    }

    let calls = Rc::new(RefCell::new(Vec::new()));
    let browser = Browser::new(Rc::new(FreshSource(calls.clone())));
    browser.navigate(Location::local("/fixture"));
    browser.retry_column(0);
    browser.retry_column(9);
    assert_eq!(calls.borrow().as_slice(), ["load /fixture", "invalidate /fixture", "load /fixture"]);
}

#[derive(Default)]
struct ControlledOperations {
    callbacks: RefCell<Vec<(OperationRequestId, Rc<dyn Fn(OperationEvent)>)>>,
    cancellations: Rc<Cell<usize>>,
    synchronous_create: Cell<bool>,
}

impl ControlledOperations {
    fn start(&self, id: OperationRequestId, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.callbacks.borrow_mut().push((id, emit));
        let cancellations = self.cancellations.clone();
        LoadHandle::new(move || cancellations.set(cancellations.get() + 1))
    }

    fn callback(&self, index: usize) -> (OperationRequestId, Rc<dyn Fn(OperationEvent)>) {
        self.callbacks.borrow()[index].clone()
    }
}

impl OperationProvider for ControlledOperations {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.start(request.id, emit)
    }

    fn create_directory(&self, request: CreateDirectoryRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        let load = self.start(request.id, emit.clone());
        if self.synchronous_create.get() {
            emit(OperationEvent::Created { request_id: request.id });
        }
        load
    }

    fn create_file(&self, request: CreateFileRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.start(request.id, emit)
    }

    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.start(request.id, emit)
    }

    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.start(request.id, emit)
    }

    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.start(request.id, emit)
    }

    fn extract_archive(&self, request: ExtractArchiveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.start(request.id, emit)
    }

    fn compress_archive(&self, request: CompressArchiveRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle {
        self.start(request.id, emit)
    }
}

fn controlled_browser() -> (Rc<Browser>, Rc<ControlledOperations>, Rc<RefCell<Vec<BrowserEvent>>>) {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let provider = Rc::new(ControlledOperations::default());
    browser.set_operation_provider(provider.clone());
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    browser.observe(move |event| captured.borrow_mut().push(event));
    (browser, provider, events)
}

#[test]
fn overlapping_operations_never_replace_or_cancel_the_active_handle() {
    let (browser, provider, events) = controlled_browser();
    let source = Location::local("/fixture/source");
    let destination = Location::local("/fixture/destination");
    let entry = crate::app::music_order::tests::entry("source", None, None);
    browser.transfer(destination.clone(), vec![source.clone()], true, false);
    let request_id = browser.current_operation.get();
    browser.rename(entry.clone(), "renamed".into());
    browser.create_directory(destination.clone(), "folder".into());
    browser.create_file(destination.clone(), "file".into());
    browser.transfer(destination.clone(), vec![source.clone()], false, false);
    browser.delete(vec![entry.clone()], false);
    browser.restore(vec![entry]);
    browser.extract_archive(source.clone(), destination.clone());
    browser.compress_archive(vec![source], destination.clone());
    assert_eq!(browser.current_operation.get(), request_id);
    assert!(browser.operation_load.borrow().is_some());
    assert_eq!(provider.callbacks.borrow().len(), 1);
    assert_eq!(provider.cancellations.get(), 0);
    assert_eq!(events.borrow().iter().filter(|event| matches!(event, BrowserEvent::OperationRejected { .. })).count(), 7);
    assert!(events.borrow().iter().any(|event| matches!(event, BrowserEvent::RenameFailed { .. })));
    assert!(!events.borrow().iter().any(|event| matches!(event, BrowserEvent::TransferFinished)));
    let (request_id, emit) = provider.callback(0);
    emit(OperationEvent::Pasted { request_id });
    assert_eq!(provider.cancellations.get(), 1);
    browser.create_file(destination, "file".into());
    assert_eq!(provider.callbacks.borrow().len(), 2);
}

#[test]
fn transfer_item_success_is_nonterminal_and_partial_errors_finish_once() {
    let (browser, provider, events) = controlled_browser();
    let first = Location::local("/fixture/first");
    let second = Location::local("/fixture/second");
    browser.transfer(Location::local("/fixture/destination"), vec![first.clone(), second.clone()], true, false);
    let (request_id, emit) = provider.callback(0);
    for (event_id, source, move_source) in [
        (OperationRequestId(999), first.clone(), true),
        (request_id, Location::local("/fixture/unrelated"), true),
        (request_id, first.clone(), false),
        (request_id, first.clone(), true),
        (request_id, first.clone(), true),
    ] {
        emit(OperationEvent::TransferItemCompleted { request_id: event_id, source, move_source });
    }
    assert_eq!(browser.current_operation.get(), Some(request_id));
    assert_eq!(*browser.transfer_sources.borrow(), vec![second]);
    assert_eq!(provider.cancellations.get(), 0);
    assert_eq!(events.borrow().iter().filter(|event| matches!(event, BrowserEvent::TransferItemCompleted { .. })).count(), 1);
    emit(OperationEvent::TransferCompletedWithErrors { request_id, message: "second failed".into() });
    assert_eq!(browser.current_operation.get(), None);
    assert!(browser.operation_load.borrow().is_none());
    assert_eq!(provider.cancellations.get(), 1);
    assert!(matches!(events.borrow().last(), Some(BrowserEvent::OperationCompletedWithErrors { message }) if message == "second failed"));
    let count = events.borrow().len();
    emit(OperationEvent::Pasted { request_id });
    assert_eq!(events.borrow().len(), count);
}

#[test]
fn cancellation_rejects_late_outcomes_and_handles_rename_and_create() {
    let (browser, provider, events) = controlled_browser();
    let source = Location::local("/fixture/source");
    browser.transfer(Location::local("/fixture/destination"), vec![source.clone()], true, false);
    let (request_id, emit) = provider.callback(0);
    browser.cancel_file_operation();
    let count = events.borrow().len();
    emit(OperationEvent::TransferItemCompleted { request_id, source, move_source: true });
    emit(OperationEvent::Failed { request_id, message: "late".into() });
    assert_eq!(events.borrow().len(), count);
    browser.rename(crate::app::music_order::tests::entry("source", None, None), "renamed".into());
    browser.cancel_file_operation();
    browser.create_directory(Location::local("/fixture"), "folder".into());
    browser.cancel_file_operation();
    assert_eq!(provider.cancellations.get(), 3);
    assert_eq!(browser.current_operation.get(), None);
}

#[test]
fn synchronous_completion_cannot_install_a_finished_handle() {
    let (browser, provider, _) = controlled_browser();
    provider.synchronous_create.set(true);
    browser.create_directory(Location::local("/fixture"), "folder".into());
    assert_eq!(browser.current_operation.get(), None);
    assert!(browser.operation_load.borrow().is_none());
    assert_eq!(provider.cancellations.get(), 1);
}

#[test]
fn finishing_observer_can_start_an_operation_without_losing_its_handle() {
    let (browser, provider, _) = controlled_browser();
    let weak = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::TransferFinished) {
            weak.upgrade().unwrap().create_file(Location::local("/fixture"), "next".into());
        }
    });
    browser.transfer(Location::local("/fixture/destination"), vec![Location::local("/fixture/source")], false, false);
    let (request_id, emit) = provider.callback(0);
    emit(OperationEvent::Pasted { request_id });
    assert_eq!(provider.callbacks.borrow().len(), 2);
    assert_eq!(browser.current_operation.get(), Some(provider.callback(1).0));
    assert!(browser.operation_load.borrow().is_some());
    assert_eq!(provider.cancellations.get(), 1);
}

#[test]
fn cancellation_from_started_observer_does_not_launch_the_provider() {
    let (browser, provider, _) = controlled_browser();
    let weak = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::TransferStarted { .. }) {
            weak.upgrade().unwrap().cancel_file_operation();
        }
    });
    browser.transfer(Location::local("/fixture/destination"), vec![Location::local("/fixture/source")], true, false);
    assert_eq!(browser.current_operation.get(), None);
    assert!(provider.callbacks.borrow().is_empty());
}

struct FilePreviewSource;

struct RejectingFileSource;

struct RetryFileSource {
    attempts: Rc<Cell<usize>>,
}

struct TrackingFileSource {
    cancellations: Rc<Cell<usize>>,
}

struct RecordingFileSource {
    include_hidden: Rc<RefCell<Vec<bool>>>,
}

type WatchCallback = Rc<dyn Fn(DirectoryChange)>;

struct WatchingFileSource {
    notify: Rc<RefCell<Option<WatchCallback>>>,
}

impl FileSource for WatchingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { music_tags: None, location: Location::local("/fixture/child"),
                native_name: OsString::from("child"),
                display_name: "child".into(),
                kind: EntryKind::Directory,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
            }))],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
        });
        LoadHandle::new(|| {})
    }

    fn watch(
        &self,
        _location: Location,
        _include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        self.notify.replace(Some(notify));
        Some(LoadHandle::new(|| {}))
    }
}

impl FileSource for RecordingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(
        &self,
        request: DirectoryRequest,
        _emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        self.include_hidden
            .borrow_mut()
            .push(request.include_hidden);
        LoadHandle::new(|| {})
    }
}

impl FileSource for TrackingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(
        &self,
        _request: DirectoryRequest,
        _emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        let cancellations = self.cancellations.clone();
        LoadHandle::new(move || cancellations.set(cancellations.get() + 1))
    }
}

impl FileSource for RetryFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let attempt = self.attempts.get();
        self.attempts.set(attempt + 1);
        if attempt == 0 {
            emit(DirectoryEvent::Failed {
                request_id: request.id,
                message: "temporarily unavailable".into(),
            });
        } else {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries: vec![crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { music_tags: None, location: Location::local("/fixture/recovered"),
                    native_name: OsString::from("recovered"),
                    display_name: "recovered".into(),
                    kind: EntryKind::Directory,
                    size: MetadataValue::Unknown,
                    modified_unix_seconds: MetadataValue::Unknown,
                }))],
            });
            emit(DirectoryEvent::Finished {
                request_id: request.id,
            });
        }
        LoadHandle::new(|| {})
    }
}

impl FileSource for RejectingFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Err(LocationValidationError::Inaccessible)
    }

    fn enumerate(
        &self,
        _request: DirectoryRequest,
        _emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        LoadHandle::new(|| {})
    }
}

impl FileSource for FilePreviewSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { music_tags: None, location: Location::local("/fixture/example.conf"),
                native_name: OsString::from("example.conf"),
                display_name: "example.conf".into(),
                kind: EntryKind::File,
                size: MetadataValue::Known(12),
                modified_unix_seconds: MetadataValue::Known(1),
            }))],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
        });
        LoadHandle::new(|| {})
    }
}

impl FileSource for FakeFileSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { music_tags: None, location: Location::local("/fixture/child"),
                native_name: OsString::from("child"),
                display_name: "child".into(),
                kind: EntryKind::Directory,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
            }))],
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
        });
        LoadHandle::new(|| {})
    }
}

#[test]
fn navigation_events_are_delivered_to_every_observer() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let first_reset = Rc::new(Cell::new(false));
    let observed_first = first_reset.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::Reset) {
            observed_first.set(true);
        }
    });
    let second_reset = Rc::new(Cell::new(false));
    let observed_second = second_reset.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::Reset) {
            observed_second.set(true);
        }
    });

    browser.navigate(Location::local("/fixture"));

    assert!(first_reset.get());
    assert!(second_reset.get());
}

#[test]
fn filesystem_notifications_update_the_affected_column_incrementally() {
    let notify = Rc::new(RefCell::new(None::<WatchCallback>));
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: notify.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    let callback = notify
        .borrow()
        .clone()
        .expect("the directory watcher should be installed");
    callback(DirectoryChange::Upsert(crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { music_tags: None, location: Location::local("/fixture/added"),
        native_name: OsString::from("added"),
        display_name: "added".into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(4),
        modified_unix_seconds: MetadataValue::Known(1),
    }))));

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesSpliced { depth: 0, splices, .. }
            if splices.len() == 1 && splices[0].removed == 0 && splices[0].entries.len() == 1
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
    );
}

#[test]
fn ambiguous_filesystem_notifications_fall_back_to_reload() {
    let notify = Rc::new(RefCell::new(None::<WatchCallback>));
    let browser = Browser::new(Rc::new(WatchingFileSource {
        notify: notify.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    let callback = notify
        .borrow()
        .clone()
        .expect("the directory watcher should be installed");
    callback(DirectoryChange::Rescan);

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
}

#[test]
fn retrying_a_failed_column_preserves_navigation_history() {
    let attempts = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(RetryFileSource {
        attempts: attempts.clone(),
    }));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.retry_column(0);

    assert_eq!(attempts.get(), 2);
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { depth: 0 }))
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::EntriesInserted { depth: 0, .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::Reset))
    );
}

#[test]
fn hidden_file_preference_is_applied_to_reloaded_requests() {
    let include_hidden = Rc::new(RefCell::new(Vec::new()));
    let browser = Browser::new(Rc::new(RecordingFileSource {
        include_hidden: include_hidden.clone(),
    }));

    browser.navigate(Location::local("/fixture"));
    browser.toggle_hidden();

    assert_eq!(*include_hidden.borrow(), vec![false, true]);
}

#[test]
fn navigating_away_cancels_the_previous_directory_request() {
    let cancellations = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(TrackingFileSource {
        cancellations: cancellations.clone(),
    }));

    browser.navigate(Location::local("/first"));
    browser.navigate(Location::local("/second"));

    assert_eq!(cancellations.get(), 1);
}

#[test]
fn navigating_to_the_active_location_is_a_noop() {
    let cancellations = Rc::new(Cell::new(0));
    let browser = Browser::new(Rc::new(TrackingFileSource {
        cancellations: cancellations.clone(),
    }));
    let resets = Rc::new(Cell::new(0));
    let observed_resets = resets.clone();
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::Reset) {
            observed_resets.set(observed_resets.get() + 1);
        }
    });

    browser.navigate(Location::uri("trash:///"));
    browser.navigate(Location::uri("trash:///"));

    assert_eq!(cancellations.get(), 0);
    assert_eq!(resets.get(), 1);
}

#[test]
fn completed_deletions_remove_entries_without_reloading_the_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));

    browser.remove_deleted_locations(&[Location::local("/fixture/child")]);

    assert!(browser.entry_at(0, 0).is_none());
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesSpliced { splices, .. }
            if splices.iter().any(|splice| splice.removed == 1)
    )));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnReloaded { .. }))
    );
}

#[test]
fn file_source_can_be_replaced_without_constructing_the_ui() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));

    browser.navigate(Location::local("/fixture"));

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::EntriesInserted { insertions, .. }
            if insertions.iter().map(|insertion| insertion.entries.len()).sum::<usize>() == 1
    )));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::LoadFinished { .. }))
    );
}

#[test]
fn valid_location_input_navigates_through_the_controller() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    assert_eq!(browser.navigate_input("/accepted"), Ok(()));

    assert_eq!(
        browser.active_location(),
        Some(Location::local("/accepted"))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ColumnAdded { depth: 0, location }
            if location == &Location::local("/accepted")
    )));
}

#[test]
fn rejected_directory_activation_preserves_navigation_state() {
    let browser = Browser::new(Rc::new(RejectingFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.descend(0, Location::local("/fixture/restricted"));

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::NavigationRejected { .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );
}

#[test]
fn rejected_location_input_preserves_navigation_state() {
    let browser = Browser::new(Rc::new(RejectingFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    assert_eq!(
        browser.navigate_input("/restricted"),
        Err(LocationValidationError::Inaccessible)
    );

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert!(events.borrow().is_empty());
}

#[test]
fn invalid_location_text_is_rejected_before_the_provider() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    assert_eq!(
        browser.navigate_input(""),
        Err(LocationValidationError::Empty)
    );
    assert_eq!(
        browser.navigate_input("relative/path"),
        Err(LocationValidationError::NotAbsolute)
    );
    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
}

#[test]
fn peeking_streams_results_without_committing_navigation_history() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));

    browser.begin_peek(0, Location::local("/fixture/child"));

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekStarted { .. }))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::PeekEntriesReplaced { entries } if entries.len() == 1
    )));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekFinished))
    );

    browser.back();
    let resets = events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::Reset))
        .count();
    assert_eq!(resets, 1, "a peek must not create a history entry");
}

#[test]
fn peek_closed_by_started_observer_does_not_start_loading() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let browser = Browser::new(Rc::new(RecordingFileSource {
        include_hidden: requests.clone(),
    }));
    browser.navigate(Location::local("/fixture"));
    let weak = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::PeekStarted { .. }) {
            weak.upgrade().unwrap().close_peek();
        }
    });

    browser.begin_peek(0, Location::local("/fixture/child"));

    assert_eq!(requests.borrow().len(), 1);
    assert!(browser.peek_load.borrow().is_none());
}

#[test]
fn peek_closed_during_synchronous_enumeration_cancels_returned_handle() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));
    let weak = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::PeekEntriesReplaced { .. }) {
            weak.upgrade().unwrap().close_peek();
        }
    });

    browser.begin_peek(0, Location::local("/fixture/child"));

    assert!(browser.state.borrow().peek_target().is_none());
    assert!(browser.peek_load.borrow().is_none());
}

#[test]
fn peek_replaced_by_started_observer_preserves_the_newer_load() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let browser = Browser::new(Rc::new(RecordingFileSource {
        include_hidden: requests.clone(),
    }));
    browser.navigate(Location::local("/fixture"));
    let weak = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::PeekStarted { location }
            if location == Location::local("/fixture/first"))
        {
            weak.upgrade().unwrap().begin_peek(0, Location::local("/fixture/second"));
        }
    });

    browser.begin_peek(0, Location::local("/fixture/first"));

    assert_eq!(requests.borrow().len(), 2);
    assert_eq!(browser.state.borrow().peek_target(), Some((0, Location::local("/fixture/second"))));
    assert!(browser.peek_load.borrow().is_some());
}

#[test]
fn peek_replaced_during_enumeration_cancels_only_the_obsolete_handle() {
    struct ReentrantSource {
        on_enumerate: Rc<RefCell<Option<Box<dyn FnOnce()>>>>,
        cancelled: Rc<RefCell<Vec<Location>>>,
    }
    impl FileSource for ReentrantSource {
        fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
            Ok(())
        }

        fn enumerate(&self, request: DirectoryRequest, _: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
            let callback = self.on_enumerate.take();
            if let Some(callback) = callback {
                callback();
            }
            let cancelled = self.cancelled.clone();
            LoadHandle::new(move || cancelled.borrow_mut().push(request.location))
        }
    }
    let on_enumerate = Rc::new(RefCell::new(None));
    let cancelled = Rc::new(RefCell::new(Vec::new()));
    let browser = Browser::new(Rc::new(ReentrantSource {
        on_enumerate: on_enumerate.clone(),
        cancelled: cancelled.clone(),
    }));
    browser.navigate(Location::local("/fixture"));
    let weak = Rc::downgrade(&browser);
    on_enumerate.replace(Some(Box::new(move || {
        weak.upgrade().unwrap().begin_peek(0, Location::local("/fixture/second"));
    })));

    browser.begin_peek(0, Location::local("/fixture/first"));

    assert_eq!(*cancelled.borrow(), vec![Location::local("/fixture/first")]);
    assert_eq!(browser.state.borrow().peek_target(), Some((0, Location::local("/fixture/second"))));
    assert!(browser.peek_load.borrow().is_some());
    browser.close_peek();
    assert_eq!(*cancelled.borrow(), vec![Location::local("/fixture/first"), Location::local("/fixture/second")]);
}

#[test]
fn navigation_from_peek_started_does_not_load_the_abandoned_peek() {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let browser = Browser::new(Rc::new(RecordingFileSource {
        include_hidden: requests.clone(),
    }));
    browser.navigate(Location::local("/fixture"));
    let weak = Rc::downgrade(&browser);
    browser.observe(move |event| {
        if matches!(event, BrowserEvent::PeekStarted { .. }) {
            weak.upgrade().unwrap().navigate(Location::local("/different"));
        }
    });

    browser.begin_peek(0, Location::local("/fixture/child"));

    assert_eq!(browser.active_location(), Some(Location::local("/different")));
    assert_eq!(requests.borrow().len(), 2);
    assert!(browser.peek_load.borrow().is_none());
}

#[test]
fn peek_cancellation_rejects_synchronous_late_events_without_borrowing_the_handle() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));
    let request_id = RequestId(browser.next_request.get());
    browser.begin_peek(0, Location::local("/fixture/child"));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    let weak = Rc::downgrade(&browser);
    let old = browser.peek_load.replace(Some(LoadHandle::new(move || {
        let browser = weak.upgrade().unwrap();
        assert!(browser.peek_load.borrow().is_none());
        browser.handle_directory_event(DirectoryEvent::Finished { request_id });
    })));
    drop(old);

    assert!(browser.close_peek());

    assert_eq!(events.borrow().len(), 1);
    assert!(matches!(events.borrow()[0], BrowserEvent::PeekClosed));
}

#[test]
fn committing_a_peek_descends_and_creates_history() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    browser.begin_peek(0, Location::local("/fixture/child"));

    browser.commit_peek();

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ColumnAdded { depth: 1, location }
            if location == &Location::local("/fixture/child")
    )));
    browser.back();
    let resets = events
        .borrow()
        .iter()
        .filter(|event| matches!(event, BrowserEvent::Reset))
        .count();
    assert_eq!(resets, 2, "committing a peek must create a history entry");
}

#[test]
fn single_click_action_descends_into_directories() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.preview(0, 0);

    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );
}

#[test]
fn activating_an_open_list_item_does_not_reload_its_pane() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    browser.preview(0, 0);
    events.borrow_mut().clear();

    browser.preview(0, 0);

    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ColumnsTruncated { .. } | BrowserEvent::ColumnAdded { .. }
    )));
    assert_eq!(browser.active_depth(), Some(1));
}

#[test]
fn explorer_activation_replaces_the_directory_instead_of_adding_a_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.activate_in_place(0, 0);

    assert_eq!(
        browser.active_location(),
        Some(Location::local("/fixture/child"))
    );
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 0, .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );
}

#[test]
fn open_folder_remains_the_rename_target_until_its_pane_has_a_selection() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    browser.preview(0, 0);

    let (depth, position, entry) = browser.rename_item().expect("open folder rename target");
    assert_eq!((depth, position), (0, 0));
    assert_eq!(entry.location, Location::local("/fixture/child"));
}

#[test]
fn preview_and_open_are_distinct_file_actions() {
    let browser = Browser::new(Rc::new(FilePreviewSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.preview(0, 0);

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::PreviewRequested { entry }
            if entry.location == Location::local("/fixture/example.conf")
    )));
    events.borrow_mut().clear();

    browser.activate(0, 0);

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OpenRequested { location }
            if location == &Location::local("/fixture/example.conf")
    )));
}

#[test]
fn reveal_selects_a_file_after_its_parent_finishes_loading() {
    let browser = Browser::new(Rc::new(FilePreviewSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));

    browser.reveal(Location::local("/fixture/example.conf"));

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert_eq!(
        browser.focused_entry().map(|entry| entry.location.clone()),
        Some(Location::local("/fixture/example.conf"))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::SelectionSetChanged {
            depth: 0,
            focused: 0,
            ..
        }
    )));
}

#[test]
fn reveal_selects_an_item_when_its_parent_is_already_open() {
    let browser = Browser::new(Rc::new(FilePreviewSource));
    browser.navigate(Location::local("/fixture"));

    browser.reveal(Location::local("/fixture/example.conf"));

    assert_eq!(
        browser.focused_entry().map(|entry| entry.location.clone()),
        Some(Location::local("/fixture/example.conf"))
    );
}

#[test]
fn keyboard_selection_and_activation_descend_without_the_ui() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));

    browser.move_selection(1);
    browser.activate_focused();

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::FocusChanged {
            depth: 0,
            position: Some(0)
        }
    )));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );
}

#[test]
fn escape_closes_a_peek_before_the_deepest_column() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event));
    browser.navigate(Location::local("/fixture"));
    browser.move_selection(1);
    browser.activate_focused();
    browser.begin_peek(1, Location::local("/fixture/child/child"));
    events.borrow_mut().clear();

    browser.escape();
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::PeekClosed))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnsTruncated { .. }))
    );

    events.borrow_mut().clear();
    browser.escape();
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnsTruncated { len: 1 }))
    );
}
