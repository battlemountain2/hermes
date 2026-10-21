use super::*;
use crate::{
    model::{FileEntryInner, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, LoadHandle, LocationValidationError, RequestId},
};

type DirectoryEmitter = Rc<dyn Fn(DirectoryEvent)>;

#[derive(Default)]
struct GeneratedMetadataSource {
    requests: Cell<usize>,
    request: Cell<Option<RequestId>>,
    emit: RefCell<Option<DirectoryEmitter>>,
}

impl FileSource for GeneratedMetadataSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: DirectoryEmitter) -> LoadHandle {
        self.requests.set(self.requests.get() + 1);
        self.request.set(Some(request.id));
        self.emit.replace(Some(emit));
        LoadHandle::new(|| {})
    }
}

impl GeneratedMetadataSource {
    fn replace(&self, entries: Vec<FileEntry>) {
        let emit = self.emit.borrow().as_ref().unwrap().clone();
        emit(DirectoryEvent::Snapshot {
            request_id: self.request.get().unwrap(),
            entries,
        });
    }
}

fn generated_entry(
    kind: EntryKind,
    size: MetadataValue<u64>,
    modified_unix_seconds: MetadataValue<i64>,
) -> FileEntry {
    FileEntry(Rc::new(FileEntryInner {
        location: Location::uri("memory:///formatting/metadata.txt"),
        native_name: "metadata.txt".into(),
        display_name: "metadata.txt".into(),
        kind,
        size,
        modified_unix_seconds,
        music_tags: None,
    }))
}

fn find_list(widget: &gtk::Widget, class: &str) -> Option<gtk::ListView> {
    if widget.has_css_class(class) {
        return widget.clone().downcast().ok();
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(list) = find_list(&widget, class) {
            return Some(list);
        }
    }
    None
}

fn metadata_labels(item: &gtk::ListItem, mode: BrowserMode) -> (gtk::Label, Option<gtk::Label>) {
    let row = item.child().unwrap();
    if mode == BrowserMode::Columns {
        let size = row.last_child().unwrap().prev_sibling().unwrap();
        (size.downcast().unwrap(), None)
    } else {
        let size = row.first_child().unwrap().next_sibling().unwrap();
        let modified = row.last_child().unwrap();
        (size.downcast().unwrap(), Some(modified.downcast().unwrap()))
    }
}

fn pump_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(6);
    while !condition() {
        for _ in 0..128 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        assert!(Instant::now() < deadline, "Metadata row fixture timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn assert_current_metadata(
    size: &gtk::Label,
    modified: Option<&gtk::Label>,
    bytes: u64,
    seconds: i64,
) {
    assert_eq!(size.text().as_str(), formatting::format_size(bytes));
    if let Some(modified) = modified {
        assert_eq!(
            modified.text().as_str(),
            formatting::format_timestamp(seconds)
        );
        let absolute = glib::DateTime::from_unix_local(seconds)
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S %Z (%z)")
            .unwrap();
        assert_eq!(modified.tooltip_text().as_deref(), Some(absolute.as_str()));
    }
}

fn exercise_recycled_metadata_row(mode: BrowserMode) {
    let source = Rc::new(GeneratedMetadataSource::default());
    let view = BrowserView::new(source.clone(), PeekBehavior::default());
    formatting::configure("iso", "decimal");
    view.set_view_mode(mode);
    view.navigate_location(Location::uri("memory:///formatting"));
    let list = find_list(
        &view.widget().upcast(),
        if mode == BrowserMode::Columns {
            "file-list"
        } else {
            "explorer-list"
        },
    )
    .unwrap();
    let factory = list
        .factory()
        .unwrap()
        .downcast::<gtk::SignalListItemFactory>()
        .unwrap();
    let captured_item = Rc::new(RefCell::new(None));
    let captured_for_bind = captured_item.clone();
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        if item.position() == 0 {
            captured_for_bind.replace(Some(item.clone()));
        }
    });
    source.replace(vec![generated_entry(
        EntryKind::File,
        MetadataValue::Known(2_048),
        MetadataValue::Known(glib::DateTime::now_local().unwrap().to_unix() - 120),
    )]);
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(300)
        .child(&view.widget())
        .build();
    window.present();
    pump_until(|| captured_item.borrow().is_some());
    let item = captured_item.borrow().as_ref().unwrap().clone();
    let (size, modified) = metadata_labels(&item, mode);
    let entry = view.browser().entry_at(0, 0).unwrap();
    let MetadataValue::Known(initial_seconds) = entry.modified_unix_seconds else {
        panic!("Generated date must be known");
    };
    assert_current_metadata(&size, modified.as_ref(), 2_048, initial_seconds);
    let model = list.model().unwrap();
    let original_item = model.item(0).unwrap();
    let model_changes = Rc::new(Cell::new(0));
    let changes_for_signal = model_changes.clone();
    model.connect_items_changed(move |_, _, _, _| {
        changes_for_signal.set(changes_for_signal.get() + 1);
    });
    formatting::configure("relative", "binary");
    assert_current_metadata(&size, modified.as_ref(), 2_048, initial_seconds);
    assert_eq!(size.text(), "2 KiB");
    assert_eq!(model.item(0).unwrap(), original_item);
    assert_eq!(model_changes.get(), 0);
    assert_eq!(source.requests.get(), 1);
    assert_eq!(
        view.browser().entry_at(0, 0).unwrap().size,
        MetadataValue::Known(2_048)
    );

    view.browser().clear_observer();
    let current_seconds = initial_seconds - 120;
    for entry in [
        generated_entry(
            EntryKind::File,
            MetadataValue::Known(8_192),
            MetadataValue::Known(current_seconds),
        ),
        generated_entry(
            EntryKind::File,
            MetadataValue::Unknown,
            MetadataValue::Unknown,
        ),
        generated_entry(
            EntryKind::File,
            MetadataValue::Unavailable,
            MetadataValue::Unavailable,
        ),
        generated_entry(
            EntryKind::Directory,
            MetadataValue::Known(65_536),
            MetadataValue::Unknown,
        ),
    ] {
        factory.emit_by_name::<()>("unbind", &[&item]);
        formatting::configure("iso", "decimal");
        assert!(size.text().is_empty());
        if let Some(modified) = &modified {
            assert!(modified.text().is_empty());
            assert!(modified.tooltip_text().is_none());
        }
        source.replace(vec![entry.clone()]);
        factory.emit_by_name::<()>("bind", &[&item]);
        formatting::configure("relative", "binary");
        if entry.is_directory() || entry.size != MetadataValue::Known(8_192) {
            assert!(size.text().is_empty());
            if let Some(modified) = &modified {
                assert!(modified.text().is_empty());
                assert!(modified.tooltip_text().is_none());
            }
        } else {
            assert_current_metadata(&size, modified.as_ref(), 8_192, current_seconds);
            assert_eq!(size.text(), "8 KiB");
        }
        assert_eq!(metadata_labels(&item, mode).0, size);
    }
    source.replace(Vec::new());
    factory.emit_by_name::<()>("bind", &[&item]);
    formatting::configure("iso", "decimal");
    assert!(size.text().is_empty());
    if let Some(modified) = &modified {
        assert!(modified.text().is_empty());
        assert!(modified.tooltip_text().is_none());
    }
    source.replace(vec![generated_entry(
        EntryKind::File,
        MetadataValue::Known(16_384),
        MetadataValue::Known(current_seconds),
    )]);
    factory.emit_by_name::<()>("bind", &[&item]);
    formatting::configure("relative", "binary");
    assert_current_metadata(&size, modified.as_ref(), 16_384, current_seconds);
    assert_eq!(source.requests.get(), 1);
    assert_eq!(model_changes.get(), 0);
    window.destroy();
}

#[test]
#[ignore = "Requires private GTK display and private HOME/XDG; generated metadata only"]
fn desktop_recycled_browser_metadata_rows_follow_current_values_and_preferences() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let _manager = super::super::theme::ThemeManager::shared();
    for mode in [BrowserMode::Columns, BrowserMode::Explorer] {
        exercise_recycled_metadata_row(mode);
    }
    formatting::configure("relative", "decimal");
}

#[test]
#[ignore = "Requires private GTK display; in-memory properties metadata and tooltips only"]
fn desktop_properties_metadata_rebinds_without_losing_folder_total_tooltips() {
    gtk::init().unwrap();
    formatting::configure("iso", "decimal");
    let size = gtk::Label::new(None);
    let modified = gtk::Label::new(None);
    let initial = generated_entry(
        EntryKind::File,
        MetadataValue::Known(2_048),
        MetadataValue::Known(0),
    );
    bind_entry_size(&size, Some(&initial), "—");
    bind_entry_modified(&modified, Some(&initial), "—");
    assert_current_metadata(&size, Some(&modified), 2_048, 0);
    let info = gio::FileInfo::new();
    info.set_size(8_192);
    info.set_modification_date_time(&glib::DateTime::from_unix_utc(123_456).unwrap());
    bind_properties_metadata(&size, &modified, &info, false);
    formatting::configure("relative", "binary");
    assert_current_metadata(&size, Some(&modified), 8_192, 123_456);
    bind_properties_metadata(&size, &modified, &gio::FileInfo::new(), false);
    formatting::configure("iso", "decimal");
    assert_eq!(size.text(), "—");
    assert_eq!(modified.text(), "—");
    assert!(modified.tooltip_text().is_none());
    formatting::bind(&size, Value::Size(65_536));
    size.set_tooltip_text(Some("7 items"));
    bind_properties_metadata(&size, &modified, &info, true);
    formatting::configure("relative", "binary");
    assert_eq!(size.text(), "64 KiB");
    assert_eq!(size.tooltip_text().as_deref(), Some("7 items"));
    assert_eq!(
        modified.text().as_str(),
        formatting::format_timestamp(123_456)
    );
    formatting::configure("relative", "decimal");
}
