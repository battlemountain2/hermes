use super::*;
use crate::model::{EntryKind, FileEntryInner, MetadataValue};
use std::{collections::HashMap, ffi::OsString};

type Watch = Rc<dyn Fn(DirectoryChange)>;
type Emit = Rc<dyn Fn(DirectoryEvent)>;

#[derive(Default)]
struct Source {
    requests: RefCell<Vec<(DirectoryRequest, Emit)>>,
    watches: RefCell<Vec<(Location, Watch)>>,
    guards: RefCell<HashMap<Location, Rc<dyn Fn()>>>,
    guard_count: Rc<Cell<usize>>,
    monitored: Cell<bool>,
    finish: Cell<bool>,
}

impl Source {
    fn ready() -> Rc<Self> {
        Rc::new(Self {
            monitored: Cell::new(true),
            finish: Cell::new(true),
            ..Self::default()
        })
    }
}

fn entry(location: &Location, name: &str, kind: EntryKind) -> FileEntry {
    FileEntry(Rc::new(FileEntryInner {
        location: Location::local(location.native_path().unwrap().join(name)),
        native_name: OsString::from(name),
        display_name: name.into(),
        kind,
        size: MetadataValue::Known(4),
        modified_unix_seconds: MetadataValue::Unknown,
        music_tags: None,
    }))
}

impl FileSource for Source {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }
    fn enumerate(&self, request: DirectoryRequest, emit: Emit) -> LoadHandle {
        self.requests
            .borrow_mut()
            .push((request.clone(), emit.clone()));
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: vec![
                entry(&request.location, "child", EntryKind::Directory),
                entry(&request.location, "track.flac", EntryKind::File),
            ],
        });
        if self.finish.get() {
            emit(DirectoryEvent::Finished {
                request_id: request.id,
            });
        }
        LoadHandle::new(|| {})
    }
    fn watch(&self, location: Location, _: bool, notify: Watch) -> Option<LoadHandle> {
        self.watches.borrow_mut().push((location, notify));
        Some(LoadHandle::new(|| {}))
    }
    fn watch_invalidation(&self, location: &Location, notify: Rc<dyn Fn()>) -> Option<LoadHandle> {
        if !self.monitored.get() {
            return None;
        }
        self.guards.borrow_mut().insert(location.clone(), notify);
        self.guard_count.set(self.guard_count.get() + 1);
        let count = self.guard_count.clone();
        Some(LoadHandle::new(move || count.set(count.get() - 1)))
    }
}

fn activate(browser: &Rc<Browser>, name: &str) {
    browser.activate_tab(
        TrailId::new(name).unwrap(),
        Location::local(format!("/generated/{name}")),
        ViewPreferences::default(),
    );
}

#[test]
fn unchanged_tabs_retain_selection_multicolumn_path_and_history_without_enumeration() {
    let source = Source::ready();
    let browser = Browser::new(source.clone());
    activate(&browser, "first");
    browser.select(0, 0);
    browser.descend(0, Location::local("/generated/first/child"));
    browser.select(1, 1);
    activate(&browser, "second");
    browser.activate_tab(
        TrailId::new("first").unwrap(),
        Location::local("/generated/first/child"),
        ViewPreferences::default(),
    );
    assert_eq!(source.requests.borrow().len(), 3);
    assert_eq!(browser.active_depth(), Some(1));
    assert_eq!(browser.focused_entry().unwrap().display_name, "track.flac");
    assert!(browser.can_go_back());
    assert_eq!(
        browser.location_at(0),
        Some(Location::local("/generated/first"))
    );
    browser.retain_open_tabs(&[TrailId::new("first").unwrap()]);
    assert!(browser.retained_tabs.borrow().is_empty());
    assert_eq!(source.guard_count.get(), 0);
}

#[test]
fn invalidation_late_metadata_and_expiry_revalidate_without_routing_to_current_tab() {
    let source = Source::ready();
    let browser = Browser::new(source.clone());
    activate(&browser, "first");
    browser.select(0, 1);
    let (request, emit) = source.requests.borrow()[0].clone();
    let stale = source.watches.borrow()[0].1.clone();
    activate(&browser, "second");
    stale(DirectoryChange::Rescan);
    assert_eq!(source.requests.borrow().len(), 2);
    emit(DirectoryEvent::MusicMetadata {
        request_id: request.id,
        entries: vec![],
    });
    assert!(browser.retained_tabs.borrow()[0].dirty.get());
    assert_eq!(
        browser.active_location(),
        Some(Location::local("/generated/second"))
    );
    activate(&browser, "first");
    assert_eq!(source.requests.borrow().len(), 3);
    assert_eq!(browser.focused_entry().unwrap().display_name, "track.flac");
    source.guards.borrow()[&Location::local("/generated/second")]();
    activate(&browser, "second");
    assert_eq!(source.requests.borrow().len(), 4);
    browser.retained_tabs.borrow_mut()[0].saved = Instant::now() - REVALIDATE_AFTER;
    activate(&browser, "first");
    assert_eq!(source.requests.borrow().len(), 5);
}

#[test]
fn separate_tabs_at_same_path_do_not_share_selection_and_closed_tabs_release_guards() {
    let source = Source::ready();
    let browser = Browser::new(source.clone());
    let location = Location::local("/generated/shared");
    let first = TrailId::new("first").unwrap();
    let second = TrailId::new("second").unwrap();
    browser.activate_tab(first.clone(), location.clone(), ViewPreferences::default());
    browser.select(0, 0);
    browser.activate_tab(second.clone(), location.clone(), ViewPreferences::default());
    assert!(browser.focused_entry().is_none());
    browser.select(0, 1);
    browser.activate_tab(first.clone(), location, ViewPreferences::default());
    assert_eq!(browser.focused_entry().unwrap().display_name, "child");
    assert_eq!(source.requests.borrow().len(), 2);
    browser.retain_open_tabs(&[first]);
    assert_eq!(source.guard_count.get(), 0);
}

#[test]
fn automatic_album_order_does_not_force_a_reload_for_saved_name_preferences() {
    let source = Source::ready();
    let browser = Browser::new(source.clone());
    activate(&browser, "first");
    let request_id = source.requests.borrow()[0].0.id;
    let tracks = vec![
        crate::app::music_order::tests::entry("9. First.flac", Some(1), Some(1)),
        crate::app::music_order::tests::entry("1. Second.flac", Some(1), Some(2)),
    ];
    browser.handle_directory_event(DirectoryEvent::Snapshot {
        request_id,
        entries: tracks,
    });
    browser.handle_directory_event(DirectoryEvent::Finished { request_id });
    assert_eq!(
        browser.column_preferences(0).unwrap().sort_key,
        SortKey::Track
    );
    activate(&browser, "second");
    activate(&browser, "first");
    assert_eq!(source.requests.borrow().len(), 2);
    assert_eq!(
        browser.column_preferences(0).unwrap().sort_key,
        SortKey::Track
    );
    assert_eq!(
        browser.column_entries(0).unwrap()[0].display_name,
        "9. First.flac"
    );
}

#[test]
fn changed_preferences_reload_with_new_hidden_policy_and_preserve_selection() {
    let source = Source::ready();
    let browser = Browser::new(source.clone());
    activate(&browser, "first");
    browser.select(0, 1);
    activate(&browser, "second");
    let preferences = ViewPreferences {
        show_hidden: true,
        sort_key: SortKey::Size,
        ..ViewPreferences::default()
    };
    browser.activate_tab(
        TrailId::new("first").unwrap(),
        Location::local("/generated/first"),
        preferences,
    );
    assert_eq!(source.requests.borrow().len(), 3);
    assert!(source.requests.borrow().last().unwrap().0.include_hidden);
    assert_eq!(browser.column_preferences(0), Some(preferences));
    assert_eq!(browser.focused_entry().unwrap().display_name, "track.flac");
    browser.state.borrow_mut().columns[0].location = Location::uri("search:///generated");
    activate(&browser, "third");
    assert!(
        !browser
            .retained_tabs
            .borrow()
            .iter()
            .any(|tab| tab.id == TrailId::new("first").unwrap())
    );
}

#[test]
fn unsupported_incomplete_changed_location_and_memory_limits_fall_back_safely() {
    let source = Source::ready();
    let browser = Browser::new(source.clone());
    source.monitored.set(false);
    activate(&browser, "first");
    activate(&browser, "second");
    assert!(browser.retained_tabs.borrow().is_empty());
    source.monitored.set(true);
    source.finish.set(false);
    activate(&browser, "first");
    activate(&browser, "third");
    assert!(
        !browser
            .retained_tabs
            .borrow()
            .iter()
            .any(|tab| tab.id == TrailId::new("first").unwrap())
    );
    source.finish.set(true);
    for index in 0..10 {
        activate(&browser, &format!("tab-{index}"));
    }
    assert_eq!(browser.retained_tabs.borrow().len(), MAX_RETAINED_TABS);
    assert!(source.guard_count.get() <= MAX_RETAINED_TABS);
    let requested = source.requests.borrow().len();
    browser.activate_tab(
        TrailId::new("tab-8").unwrap(),
        Location::local("/generated/different"),
        ViewPreferences::default(),
    );
    assert_eq!(source.requests.borrow().len(), requested + 1);
    let big = entry(
        &Location::local("/generated/different"),
        "big",
        EntryKind::File,
    );
    browser.state.borrow_mut().columns[0].entries = vec![big; MAX_RETAINED_BYTES / 4096 + 1];
    activate(&browser, "final");
    assert!(
        !browser
            .retained_tabs
            .borrow()
            .iter()
            .any(|tab| tab.id == TrailId::new("tab-8").unwrap())
    );
}
