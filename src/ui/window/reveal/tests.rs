use super::*;
use std::{cell::RefCell, ffi::OsString};

use crate::{
    model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
    ui::browser::PeekBehavior,
};

type Emitter = Rc<dyn Fn(DirectoryEvent)>;
type Loads = Rc<RefCell<Vec<(DirectoryRequest, Emitter)>>>;

struct Pending(Loads);

impl FileSource for Pending {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Emitter) -> LoadHandle {
        self.0.borrow_mut().push((request, emit));
        LoadHandle::new(|| {})
    }
}

fn entry(name: &str) -> FileEntry {
    FileEntry(Rc::new(FileEntryInner {
        location: Location::local(format!("/generated/{name}")),
        native_name: OsString::from(name),
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(4),
        modified_unix_seconds: MetadataValue::Unknown,
        music_tags: None,
    }))
}

fn request() -> Request {
    Request {
        directory: Location::local("/generated"),
        selection: vec![
            Location::local("/generated/second.txt"),
            Location::local("/generated/first.txt"),
        ],
        properties: false,
        startup_id: String::new(),
    }
}

#[test]
fn explicit_hidden_reveals_override_only_the_new_windows_visibility() {
    assert!(!includes_hidden(&request()));
    let mut hidden = request();
    hidden.selection = vec![Location::local("/generated/.hidden.txt")];
    assert!(includes_hidden(&hidden));
    hidden.selection = vec![Location::uri("sftp://host/folder/.hidden.txt")];
    assert!(includes_hidden(&hidden));
}

#[test]
#[ignore = "Private GTK display and fixture XDG roots; synthetic hidden file and actual Properties widgets"]
fn external_hidden_properties_request_opens_the_file_properties_not_parent_properties() {
    use gtk::prelude::*;
    use std::time::{Duration, Instant};
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let directory = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    let target = directory.0.join(".hidden.txt");
    std::fs::write(&target, b"Generated properties fixture").unwrap();
    let application = gtk::Application::builder()
        .application_id("io.github.battlemountain2.Hermes.Test")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gtk::gio::Cancellable>)
        .unwrap();
    super::super::super::present_reveal(
        &application,
        Request {
            directory: Location::local(&directory.0),
            selection: vec![Location::local(&target)],
            properties: true,
            startup_id: String::new(),
        },
    );
    let window = application.windows().into_iter().next().unwrap();
    fn find(widget: &gtk::Widget, class: &str) -> Option<gtk::Label> {
        if widget.has_css_class(class) {
            return widget.clone().downcast::<gtk::Label>().ok();
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(label) = find(&widget, class) {
                return Some(label);
            }
            child = widget.next_sibling();
        }
        None
    }
    let started = Instant::now();
    loop {
        gtk::glib::MainContext::default().iteration(false);
        if let Some(title) = find(window.upcast_ref(), "properties-title") {
            assert_eq!(title.text(), ".hidden.txt");
            assert_eq!(
                find(window.upcast_ref(), "properties-kind").unwrap().text(),
                "File"
            );
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "Expected hidden file properties"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    window.close();
    application.quit();
}

#[test]
#[ignore = "Private GTK display; synthetic delayed folder batches, no desktop connection or documents"]
fn reveal_waits_for_all_batches_selects_once_and_ignores_departed_folders() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let loads = Rc::new(RefCell::new(Vec::new()));
    let view = BrowserView::new(Rc::new(Pending(loads.clone())), PeekBehavior::default());
    let browser = view.browser();
    browser.navigate(Location::local("/generated"));
    install(&browser, &view, request());
    let (load, emit) = loads.borrow().last().unwrap().clone();
    emit(DirectoryEvent::Batch {
        request_id: load.id,
        entries: vec![entry("first.txt")],
    });
    assert!(browser.selected_entries().is_empty());
    emit(DirectoryEvent::Batch {
        request_id: load.id,
        entries: vec![entry("second.txt")],
    });
    emit(DirectoryEvent::Finished {
        request_id: load.id,
    });
    assert_eq!(browser.selected_entries().len(), 2);
    assert_eq!(browser.focused_entry().unwrap().display_name, "second.txt");
    browser.select_locations(0, &[Location::local("/generated/first.txt")]);
    emit(DirectoryEvent::Finished {
        request_id: load.id,
    });
    assert_eq!(browser.selected_entries().len(), 1);
    assert_eq!(browser.focused_entry().unwrap().display_name, "first.txt");

    browser.navigate(Location::local("/generated"));
    install(&browser, &view, request());
    let (old, stale) = loads.borrow().last().unwrap().clone();
    browser.navigate(Location::local("/elsewhere"));
    stale(DirectoryEvent::Batch {
        request_id: old.id,
        entries: vec![entry("first.txt"), entry("second.txt")],
    });
    stale(DirectoryEvent::Finished { request_id: old.id });
    assert_eq!(
        browser.active_location(),
        Some(Location::local("/elsewhere"))
    );
    assert!(browser.selected_entries().is_empty());
    browser.clear_observer();
}
