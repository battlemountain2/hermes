// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
#[ignore = "Requires isolated GTK display; synthetic deletion provider only"]
fn desktop_permanent_delete_preference_preserves_trash_and_empty_trash_safety() {
    struct EmptySource;

    impl FileSource for EmptySource {
        fn validate_location(&self, _: &Location) -> Result<(), crate::services::LocationValidationError> {
            Ok(())
        }

        fn enumerate(&self, _: crate::services::DirectoryRequest, _: Rc<dyn Fn(crate::services::DirectoryEvent)>) -> crate::services::LoadHandle {
            crate::services::LoadHandle::new(|| {})
        }
    }

    fn cancel_modal(overlay: &gtk::Overlay) {
        let layer = overlay.last_child().unwrap();
        assert!(layer.has_css_class("app-modal-layer"));
        let content = layer.first_child().unwrap().next_sibling().unwrap();
        let actions = content.last_child().unwrap();
        let cancel = actions.first_child().unwrap().next_sibling().unwrap().downcast::<gtk::Button>().unwrap();
        assert_eq!(cancel.label().as_deref(), Some("Cancel"));
        cancel.emit_clicked();
    }

    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = super::super::theme::ThemeManager::shared();
    let view = BrowserView::new(Rc::new(EmptySource), Default::default());
    let operations = Rc::new(TransferFixtureOperations::default());
    view.browser().set_operation_provider(operations.clone());
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view.widget()));
    let window = gtk::Window::builder().child(&overlay).build();
    let entries = vec![crate::app::music_order::tests::entry("generated.flac", None, None)];
    manager.set_confirm_permanent_delete(true);
    view.state.show_delete_confirmation(entries.clone(), true);
    assert!(operations.deletions.borrow().is_empty());
    cancel_modal(&overlay);
    assert_eq!(overlay.last_child(), overlay.child());
    manager.set_confirm_permanent_delete(false);
    view.state.show_delete_confirmation(entries.clone(), false);
    assert!(operations.deletions.borrow().is_empty());
    cancel_modal(&overlay);
    view.state.show_empty_trash_confirmation(TrashSummary { entries: entries.clone(), item_count: 1, total_size: 0 });
    assert!(operations.deletions.borrow().is_empty());
    cancel_modal(&overlay);
    view.state.show_delete_confirmation(Vec::new(), true);
    assert!(operations.deletions.borrow().is_empty());
    view.state.show_delete_confirmation(entries, true);
    assert_eq!(*operations.deletions.borrow(), vec![true]);
    assert_eq!(overlay.last_child(), overlay.child());
    assert!(!window.is_visible() && !window.is_mapped());
    view.browser().clear_observer();
    window.destroy();
}

#[test]
#[ignore = "Requires private GTK display; compact clipboard feedback fixture"]
fn desktop_clipboard_feedback_is_compact_explicit_and_clears_on_replacement() {
    use crate::services::{DirectoryEvent, DirectoryRequest, LoadHandle};
    struct EmptySource;
    impl FileSource for EmptySource {
        fn validate_location(&self, _: &Location) -> Result<(), crate::services::LocationValidationError> { Ok(()) }
        fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
            emit(DirectoryEvent::Finished { request_id: request.id });
            LoadHandle::new(|| {})
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let view = BrowserView::new(Rc::new(EmptySource), PeekBehavior::default());
    let window = gtk::Window::builder().child(&view.widget()).default_width(600).default_height(400).build();
    window.present();
    let entry = FileEntry(Rc::new(crate::model::FileEntryInner {
        location: Location::local("/generated-clipboard/a-very-long-song-name-for-feedback.mp3"),
        native_name: "a-very-long-song-name-for-feedback.mp3".into(),
        display_name: "a-very-long-song-name-for-feedback.mp3".into(), kind: EntryKind::File,
        size: crate::model::MetadataValue::Unknown, modified_unix_seconds: crate::model::MetadataValue::Unknown, music_tags: None,
    }));
    view.state.copy_entries(std::slice::from_ref(&entry));
    assert_eq!(view.state.clipboard_indicator.label().as_deref(), Some("Copied · 1"));
    assert!(!view.state.clipboard_indicator.tooltip_text().unwrap().contains(&entry.display_name));
    assert!(view.state.clipboard_indicator.get_visible());
    assert!(!view.state.clipboard_popover.get_visible());
    let controllers = view.state.clipboard_indicator.observe_controllers();
    for index in 0..controllers.n_items() {
        if let Ok(motion) = controllers.item(index).unwrap().downcast::<gtk::EventControllerMotion>() {
            motion.emit_by_name::<()>("enter", &[&0.0_f64, &0.0_f64]);
        }
    }
    assert!(!view.state.clipboard_popover.get_visible());
    view.state.clipboard_indicator.emit_clicked();
    assert!(view.state.clipboard_popover.get_visible());
    view.state.clipboard_indicator.emit_clicked();
    assert!(!view.state.clipboard_popover.get_visible());
    let item = view.state.clipboard_list.first_child().unwrap().downcast::<gtk::Label>().unwrap();
    assert_eq!(item.text(), entry.display_name);
    assert_eq!(item.max_width_chars(), 30);
    gtk::gdk::Display::default().unwrap().clipboard().set_text("External clipboard replacement");
    assert!(!view.state.clipboard_indicator.get_visible());
    assert!(!view.state.clipboard_popover.get_visible());
    assert!(view.state.indicated_clipboard.borrow().is_none());
    window.close();
}

#[test]
#[ignore = "Requires private GTK display; generated context menu and clipboard fixture"]
fn desktop_context_menu_copies_single_and_multiple_files_to_clipboard() {
    use crate::services::{DirectoryEvent, DirectoryRequest, LoadHandle};
    let actions = crate::services::actions_path();
    assert!(actions.starts_with(std::env::temp_dir()) && !actions.exists(), "Use a private, empty XDG config directory for this fixture");
    struct GeneratedSource;
    impl FileSource for GeneratedSource {
        fn validate_location(&self, _: &Location) -> Result<(), crate::services::LocationValidationError> { Ok(()) }
        fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
            let entries = ["first.bin", "second.bin"].into_iter().map(|name| FileEntry(Rc::new(crate::model::FileEntryInner {
                location: Location::local(format!("/generated-menu/{name}")), native_name: name.into(), display_name: name.into(), kind: EntryKind::File,
                size: crate::model::MetadataValue::Unknown, modified_unix_seconds: crate::model::MetadataValue::Unknown, music_tags: None,
            }))).collect();
            emit(DirectoryEvent::Batch { request_id: request.id, entries });
            emit(DirectoryEvent::Finished { request_id: request.id });
            LoadHandle::new(|| {})
        }
    }
    fn pump() {
        let deadline = Instant::now() + Duration::from_millis(180);
        while Instant::now() < deadline {
            for _ in 0..64 { if !glib::MainContext::default().iteration(false) { break; } }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn menu_button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.child().and_then(|row| row.first_child())
                .and_then(|icon| icon.next_sibling()).and_downcast::<gtk::Label>()
                .is_some_and(|title| title.text() == label)
        { return Some(button.clone()); }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if let Some(button) = menu_button(&current, label) { return Some(button); }
        }
        None
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = super::super::theme::ThemeManager::shared();
    manager.set_context_menu_options(super::super::context_menu::Options::default());
    let view = BrowserView::new(Rc::new(GeneratedSource), PeekBehavior::default());
    view.browser().navigate(Location::local("/generated-menu"));
    pump();
    let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
    parent.append(&gtk::Label::new(Some("Generated file target")));
    let model = gtk::StringList::new(&["first.bin", "second.bin"]);
    let selection = gtk::MultiSelection::new(Some(model));
    install_item_context_menu(&view.state, parent.upcast_ref(), &selection, Rc::new(|_| Some(0)), Rc::new(|position| Some(position as usize)), 0);
    let window = gtk::Window::builder().default_width(500).default_height(300).child(&parent).build();
    window.present();
    pump();
    let controllers = parent.observe_controllers();
    let click = (0..controllers.n_items()).filter_map(|position| controllers.item(position).and_downcast::<gtk::GestureClick>()).next().unwrap();
    click.emit_by_name::<()>("pressed", &[&1i32, &12.0f64, &12.0f64]);
    pump();
    let copy = menu_button(parent.upcast_ref(), "Copy").expect("Clipboard Copy action");
    assert!(copy.is_visible());
    assert!(!menu_button(parent.upcast_ref(), "Copy to…").unwrap().is_visible());
    copy.emit_clicked();
    let clipboard = gtk::gdk::Display::default().unwrap().clipboard();
    let files = glib::MainContext::default().block_on(clipboard.read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT)).unwrap().get::<gtk::gdk::FileList>().unwrap().files();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path().unwrap(), PathBuf::from("/generated-menu/first.bin"));
    let (stream, mime) = glib::MainContext::default().block_on(clipboard.read_future(&["text/uri-list"], glib::Priority::DEFAULT)).unwrap();
    assert_eq!(mime, "text/uri-list");
    let encoded = glib::MainContext::default().block_on(stream.read_bytes_future(4096, glib::Priority::DEFAULT)).unwrap();
    assert!(String::from_utf8_lossy(&encoded).contains("file:///generated-menu/first.bin"));
    clipboard.set_content(Some(&gtk::gdk::ContentProvider::for_bytes("text/uri-list", &glib::Bytes::from_static(b"file:///generated-menu/from-another-app.bin\r\n")))).unwrap();
    assert!(clipboard_has_files());
    let external = glib::MainContext::default().block_on(clipboard.read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT)).unwrap().get::<gtk::gdk::FileList>().unwrap().files();
    assert_eq!(external[0].path().unwrap(), PathBuf::from("/generated-menu/from-another-app.bin"));
    selection.select_all();
    click.emit_by_name::<()>("pressed", &[&1i32, &12.0f64, &12.0f64]);
    pump();
    let mut child = parent.first_child();
    let mut visible_copy = None;
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(menu) = widget.downcast_ref::<gtk::Popover>() {
            let content = menu.child().unwrap();
            let mut section = content.first_child();
            while let Some(widget) = section {
                section = widget.next_sibling();
                if widget.is_visible() && let Some(button) = menu_button(&widget, "Copy") {
                    visible_copy = Some(button);
                }
            }
        }
    }
    visible_copy.unwrap().emit_clicked();
    let files = glib::MainContext::default().block_on(clipboard.read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT)).unwrap().get::<gtk::gdk::FileList>().unwrap().files();
    assert_eq!(files.len(), 2);
    assert!(view.state.cut_locations.borrow().is_empty());
    std::fs::create_dir_all(actions.parent().unwrap()).unwrap();
    std::fs::write(&actions, "[[actions]]\nname = 'Generated context action'\ncommand = ['/bin/false']\nextensions = ['bin']\n").unwrap();
    let mut options = manager.context_menu_options();
    options.transfer_destinations = true;
    manager.set_context_menu_options(options);
    selection.select_item(0, true);
    click.emit_by_name::<()>("pressed", &[&1i32, &12.0f64, &12.0f64]);
    pump();
    assert!(menu_button(parent.upcast_ref(), "Copy to…").unwrap().get_visible());
    assert!(menu_button(parent.upcast_ref(), "Generated context action").is_some());
    let mut options = manager.context_menu_options();
    options.custom_actions = false;
    manager.set_context_menu_options(options);
    click.emit_by_name::<()>("pressed", &[&1i32, &12.0f64, &12.0f64]);
    pump();
    assert!(menu_button(parent.upcast_ref(), "Generated context action").is_none());
    std::fs::remove_file(actions).unwrap();
    manager.set_context_menu_options(super::super::context_menu::Options::default());
    window.close();
}

#[test]
#[ignore = "Requires an isolated GTK display; generated navigation widget/model lifecycle fixture"]
fn desktop_navigation_releases_discarded_presentations() {
    use crate::ui::browser_modes::tests::{
        LifecycleProbes, append_pane_probes, append_widget_probes,
    };

    struct GeneratedDirectory(PathBuf);
    impl Drop for GeneratedDirectory {
        fn drop(&mut self) {
            let _removed = std::fs::remove_dir_all(&self.0);
        }
    }

    fn pump_until(context: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() && Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(condition(), "Generated lifecycle fixture timed out: {context}");
    }

    fn capture(view: &BrowserView) -> LifecycleProbes {
        let mut probes = Vec::new();
        for column in view.state.columns.borrow().iter() {
            append_widget_probes(column.shell.upcast_ref(), "columns", &mut probes);
            for object in [
                column.model.upcast_ref::<glib::Object>(),
                column.filtered_model.upcast_ref::<glib::Object>(),
                column.selection.upcast_ref::<glib::Object>(),
            ] {
                probes.push((
                    format!("columns {}", object.type_().name()),
                    object.downgrade(),
                ));
            }
            for bound in column.bound_rows.borrow().iter() {
                if let Some(item) = bound.item.upgrade() {
                    probes.push((
                        "columns item".into(),
                        item.upcast_ref::<glib::Object>().downgrade(),
                    ));
                }
            }
        }
        append_pane_probes(&view.state.mode_views.borrow(), &mut probes);
        probes
    }

    fn assert_released(probes: &LifecycleProbes) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while probes.iter().any(|(_, object)| object.upgrade().is_some())
            && Instant::now() < deadline
        {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
        let retained: Vec<_> = probes
            .iter()
            .filter(|(_, object)| object.upgrade().is_some())
            .map(|(label, _)| label.as_str())
            .collect();
        assert!(retained.is_empty(), "Discarded objects retained: {retained:?}");
    }

    gtk::init().expect("Isolated GTK display available");
    crate::assets::prepare().expect("Fixture icons load");
    let root = GeneratedDirectory(std::env::temp_dir().join(format!(
        "hermes-navigation-lifecycle-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    )));
    let first = root.0.join("first");
    let second = root.0.join("second");
    for folder in [&first, &second] {
        std::fs::create_dir_all(folder).unwrap();
        for index in 0..1_000 {
            std::fs::write(folder.join(format!("entry-{index:04}.txt")), b"fixture").unwrap();
        }
    }
    let view = BrowserView::new(Rc::new(crate::adapters::LocalFileSource), PeekBehavior::default());
    let browser = view.browser();
    let window = gtk::Window::builder()
        .default_width(1_000)
        .default_height(700)
        .child(&view.widget())
        .build();
    window.present();
    let finished = Rc::new(Cell::new(false));
    let finished_for_observer = finished.clone();
    browser.observe(move |event| match event {
        BrowserEvent::LoadFinished { depth: 0 } => finished_for_observer.set(true),
        BrowserEvent::LoadFailed { message, .. } => panic!("{message}"),
        _ => {}
    });
    let mut previous = Vec::new();
    for (mode, presentation) in [
        (BrowserMode::Columns, "columns"),
        (BrowserMode::Grid, "grid"),
        (BrowserMode::Explorer, "explorer"),
    ] {
        view.set_view_mode(mode);
        let folders = if browser.active_location() == Some(Location::local(&first)) {
            [&second, &first, &second]
        } else {
            [&first, &second, &first]
        };
        for folder in folders {
            finished.set(false);
            browser.navigate(Location::local(folder));
            pump_until(&format!("load {presentation}: {}", folder.display()), || finished.get());
            assert_eq!(browser.column_entries(0).unwrap().len(), 1_000);
            assert_released(&previous);
            pump_until(&format!("bound {presentation} items: {}", folder.display()), || {
                capture(&view)
                    .iter()
                    .any(|(label, _)| label == &format!("{presentation} item"))
            });
            previous = capture(&view);
            assert!(previous.iter().any(|(label, _)| label == "columns GtkStringList"));
            assert_eq!(previous.iter().any(|(label, _)| label == "grid GtkStringList"), mode == BrowserMode::Grid);
            assert_eq!(previous.iter().any(|(label, _)| label == "explorer GtkStringList"), mode == BrowserMode::Explorer);
        }
    }
    let state = Rc::downgrade(&view.state);
    let weak_browser = Rc::downgrade(&browser);
    browser.clear_observer();
    window.close();
    window.set_child(gtk::Widget::NONE);
    drop((view, browser, window));
    assert_released(&previous);
    pump_until("browser and view destruction", || state.upgrade().is_none() && weak_browser.upgrade().is_none());
}

#[derive(Default)]
struct TransferFixtureOperations {
    requests: RefCell<Vec<(crate::services::OperationRequestId, Rc<dyn Fn(crate::services::OperationEvent)>)>>,
    deletions: RefCell<Vec<bool>>,
    cancellations: Rc<Cell<usize>>,
}

impl TransferFixtureOperations {
    fn start(&self, id: crate::services::OperationRequestId, emit: Rc<dyn Fn(crate::services::OperationEvent)>) -> crate::services::LoadHandle {
        self.requests.borrow_mut().push((id, emit));
        let cancellations = self.cancellations.clone();
        crate::services::LoadHandle::new(move || cancellations.set(cancellations.get() + 1))
    }
}

impl OperationProvider for TransferFixtureOperations {
    fn rename(&self, request: crate::services::RenameRequest, emit: Rc<dyn Fn(crate::services::OperationEvent)>) -> crate::services::LoadHandle {
        self.start(request.id, emit)
    }
    fn create_directory(&self, request: crate::services::CreateDirectoryRequest, emit: Rc<dyn Fn(crate::services::OperationEvent)>) -> crate::services::LoadHandle {
        self.start(request.id, emit)
    }
    fn create_file(&self, request: crate::services::CreateFileRequest, emit: Rc<dyn Fn(crate::services::OperationEvent)>) -> crate::services::LoadHandle {
        self.start(request.id, emit)
    }
    fn paste(&self, request: crate::services::PasteRequest, emit: Rc<dyn Fn(crate::services::OperationEvent)>) -> crate::services::LoadHandle {
        self.start(request.id, emit)
    }
    fn delete(&self, request: crate::services::DeleteRequest, emit: Rc<dyn Fn(crate::services::OperationEvent)>) -> crate::services::LoadHandle {
        self.deletions.borrow_mut().push(request.permanent);
        self.start(request.id, emit)
    }
    fn restore(&self, request: crate::services::RestoreRequest, emit: Rc<dyn Fn(crate::services::OperationEvent)>) -> crate::services::LoadHandle {
        self.start(request.id, emit)
    }
    fn extract_archive(&self, request: crate::services::ExtractArchiveRequest, emit: Rc<dyn Fn(crate::services::OperationEvent)>) -> crate::services::LoadHandle {
        self.start(request.id, emit)
    }
    fn compress_archive(&self, request: crate::services::CompressArchiveRequest, emit: Rc<dyn Fn(crate::services::OperationEvent)>) -> crate::services::LoadHandle {
        self.start(request.id, emit)
    }
}

#[test]
#[ignore = "Requires an isolated desktop display; verifies generated transfer outcomes and clipboard ownership"]
fn desktop_transfer_outcomes_preserve_cut_clipboard() {
    use crate::services::{DirectoryEvent, DirectoryRequest, LoadHandle, OperationEvent};
    struct GeneratedSource;
    impl FileSource for GeneratedSource {
        fn validate_location(&self, _: &Location) -> Result<(), crate::services::LocationValidationError> {
            Ok(())
        }
        fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
            emit(DirectoryEvent::Finished { request_id: request.id });
            LoadHandle::new(|| {})
        }
    }
    gtk::init().expect("Isolated desktop display available");
    crate::assets::prepare().expect("Icons load");
    let view = BrowserView::new(Rc::new(GeneratedSource), PeekBehavior::default());
    let window = gtk::Window::builder().child(&view.state.overlay).build();
    let provider = Rc::new(TransferFixtureOperations::default());
    view.set_operation_provider(provider.clone());
    let entry = |name: &str| FileEntry(Rc::new(crate::model::FileEntryInner {
        location: Location::local(format!("/generated-transfer-fixture/{name}")),
        native_name: name.into(), display_name: name.into(), kind: EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown, music_tags: None,
    }));
    let first = entry("first");
    let second = entry("second");
    let unrelated = entry("unrelated");
    let destination = Location::local("/generated-transfer-fixture/destination");
    let clipboard = gtk::gdk::Display::default().unwrap().clipboard();
    let start = |sources: &[FileEntry], move_sources| {
        view.state.start_transfer(destination.clone(), sources.iter().map(|entry| entry.location.clone()).collect(), move_sources);
        provider.requests.borrow().last().unwrap().clone()
    };
    let complete = |request_id, emit: &Rc<dyn Fn(OperationEvent)>, source: &FileEntry, move_source| {
        emit(OperationEvent::TransferItemCompleted { request_id, source: source.location.clone(), move_source });
    };

    view.state.cut_entries(&[first.clone(), second.clone()]);
    let original = clipboard.content();
    let (request_id, emit) = start(&[first.clone(), second.clone()], true);
    assert_eq!(clipboard.content(), original);
    assert_eq!(view.state.cut_locations.borrow().len(), 2);
    let progress = view.state.delete_progress.borrow().as_ref().unwrap().layer.clone();
    view.state.browser.create_directory(destination.clone(), "overlap".into());
    assert_eq!(provider.requests.borrow().len(), 1);
    assert_eq!(provider.cancellations.get(), 0);
    assert_eq!(view.state.delete_progress.borrow().as_ref().unwrap().layer, progress);
    complete(request_id, &emit, &first, false);
    assert_eq!(view.state.cut_locations.borrow().len(), 2);
    complete(request_id, &emit, &first, true);
    assert_eq!(*view.state.cut_locations.borrow(), vec![second.location.clone()]);
    assert_eq!(view.state.clipboard_indicator.label().as_deref(), Some("Cut · 1"));
    assert_eq!(view.state.clipboard_list.first_child().unwrap().downcast::<gtk::Label>().unwrap().text(), "second");
    assert!(view.state.clipboard_list.first_child().unwrap().next_sibling().is_none());
    emit(OperationEvent::TransferCompletedWithErrors { request_id, message: "second failed".into() });
    assert_eq!(*view.state.cut_locations.borrow(), vec![second.location.clone()]);
    assert!(view.state.delete_progress.borrow().is_none());
    let (request_id, emit) = start(std::slice::from_ref(&second), true);
    complete(request_id, &emit, &second, true);
    emit(OperationEvent::Pasted { request_id });
    assert!(view.state.cut_locations.borrow().is_empty());
    assert!(clipboard.content().is_none());
    assert!(!view.state.clipboard_indicator.is_visible());

    view.state.cut_entries(&[first.clone(), second.clone()]);
    let (request_id, emit) = start(&[first.clone(), second.clone()], true);
    complete(request_id, &emit, &first, true);
    let remaining_content = clipboard.content();
    view.state.browser.cancel_file_operation();
    complete(request_id, &emit, &second, true);
    assert_eq!(*view.state.cut_locations.borrow(), vec![second.location.clone()]);
    assert_eq!(clipboard.content(), remaining_content);

    view.state.cut_entries(std::slice::from_ref(&first));
    let (request_id, emit) = start(std::slice::from_ref(&first), true);
    view.state.copy_entries(std::slice::from_ref(&unrelated));
    let copied = clipboard.content();
    complete(request_id, &emit, &first, true);
    emit(OperationEvent::Pasted { request_id });
    assert_eq!(clipboard.content(), copied);
    assert_eq!(view.state.clipboard_indicator.label().as_deref(), Some("Copied · 1"));

    view.state.cut_entries(std::slice::from_ref(&first));
    let (request_id, emit) = start(std::slice::from_ref(&first), true);
    view.state.cut_entries(std::slice::from_ref(&first));
    let recut = clipboard.content();
    complete(request_id, &emit, &first, true);
    emit(OperationEvent::Pasted { request_id });
    assert_eq!(*view.state.cut_locations.borrow(), vec![first.location.clone()]);
    assert_eq!(clipboard.content(), recut);

    let (request_id, emit) = start(std::slice::from_ref(&first), true);
    clipboard.set_text("External clipboard content");
    let external = clipboard.content();
    complete(request_id, &emit, &first, true);
    emit(OperationEvent::Pasted { request_id });
    assert_eq!(clipboard.content(), external);

    view.state.cut_entries(std::slice::from_ref(&first));
    let cut = clipboard.content();
    let (request_id, emit) = start(std::slice::from_ref(&first), false);
    complete(request_id, &emit, &first, false);
    emit(OperationEvent::Pasted { request_id });
    assert_eq!(*view.state.cut_locations.borrow(), vec![first.location.clone()]);
    assert_eq!(clipboard.content(), cut);

    view.state.cut_entries(std::slice::from_ref(&first));
    let (request_id, emit) = start(std::slice::from_ref(&first), true);
    let unchanged = clipboard.content();
    emit(OperationEvent::Failed { request_id, message: "failed before moving".into() });
    assert_eq!(*view.state.cut_locations.borrow(), vec![first.location.clone()]);
    assert_eq!(clipboard.content(), unchanged);

    struct GeneratedDirectory(PathBuf);
    impl Drop for GeneratedDirectory {
        fn drop(&mut self) {
            let _removed = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = GeneratedDirectory(std::env::temp_dir().join(format!(
        "hermes-transfer-outcomes-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
    )));
    std::fs::create_dir_all(root.0.join("destination")).unwrap();
    std::fs::write(root.0.join("moved.txt"), b"Generated transfer fixture").unwrap();
    let generated_entry = |name: &str| {
        let mut generated = entry(name);
        Rc::make_mut(&mut generated.0).location = Location::local(root.0.join(name));
        generated
    };
    let moved = generated_entry("moved.txt");
    let missing = generated_entry("missing.txt");
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    view.state.cut_entries(&[moved.clone(), missing.clone()]);
    let finished = Rc::new(Cell::new(false));
    let partial_error = Rc::new(Cell::new(false));
    let observed_finished = finished.clone();
    let observed_error = partial_error.clone();
    view.browser().observe(move |event| {
        if matches!(event, BrowserEvent::TransferFinished) {
            observed_finished.set(true);
        }
        if matches!(event, BrowserEvent::OperationCompletedWithErrors { .. }) {
            observed_error.set(true);
        }
    });
    view.state.start_transfer(Location::local(root.0.join("destination")),
        vec![moved.location.clone(), missing.location.clone()], true);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !finished.get() && Instant::now() < deadline {
        for _ in 0..128 {
            if !glib::MainContext::default().pending() || finished.get() {
                break;
            }
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(finished.get(), "Generated provider transfer finishes");
    assert!(partial_error.get(), "Missing generated source reports partial error");
    assert!(!root.0.join("moved.txt").exists());
    assert_eq!(std::fs::read(root.0.join("destination/moved.txt")).unwrap(), b"Generated transfer fixture");
    assert_eq!(*view.state.cut_locations.borrow(), vec![missing.location.clone()]);
    assert_eq!(view.state.clipboard_indicator.label().as_deref(), Some("Cut · 1"));
    window.close();
    view.browser().clear_observer();
}

#[test]
#[ignore = "Requires a desktop display; verifies streaming result selection in all views"]
fn desktop_search_snapshots_preserve_selection_in_every_view() {
    use crate::services::{DirectoryEvent, DirectoryRequest, LoadHandle, RequestId};
    fn mapped_notice(widget: &gtk::Widget) -> Option<gtk::Label> {
        if widget.has_css_class("search-coverage-notice") && widget.is_mapped() {
            return widget.clone().downcast::<gtk::Label>().ok();
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(label) = mapped_notice(&current) {
                return Some(label);
            }
            child = current.next_sibling();
        }
        None
    }
    struct SnapshotSource {
        emit: Rc<RefCell<Option<Rc<dyn Fn(DirectoryEvent)>>>>,
        request: Rc<Cell<RequestId>>,
    }
    impl FileSource for SnapshotSource {
        fn validate_location(&self, _: &Location) -> Result<(), crate::services::LocationValidationError> { Ok(()) }
        fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
            self.request.set(request.id);
            self.emit.replace(Some(emit));
            LoadHandle::new(|| {})
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let captured = Rc::new(RefCell::new(None));
    let request = Rc::new(Cell::new(RequestId(0)));
    let view = BrowserView::new(Rc::new(SnapshotSource { emit: captured.clone(), request: request.clone() }), PeekBehavior::default());
    let window = gtk::Window::builder().default_width(900).default_height(500).child(&view.widget()).build();
    window.present();
    let entry = |name: &str| FileEntry(Rc::new(crate::model::FileEntryInner {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(), display_name: name.into(), kind: EntryKind::File,
        size: crate::model::MetadataValue::Unknown, modified_unix_seconds: crate::model::MetadataValue::Unknown, music_tags: None,
    }));
    for mode in [BrowserMode::Columns, BrowserMode::Explorer, BrowserMode::Grid] {
        view.set_view_mode(mode);
        view.browser().navigate(Location::local("/fixture"));
        let emit = captured.borrow().as_ref().unwrap().clone();
        emit(DirectoryEvent::Snapshot { request_id: request.get(), entries: vec![entry("a.txt"), entry("b.txt"), entry("c.txt")] });
        view.browser().set_selection(0, &[0, 1], Some(1));
        emit(DirectoryEvent::Snapshot { request_id: request.get(), entries: vec![entry("b.txt"), entry("d.txt")] });
        let selected = view.browser().selected_entries();
        assert_eq!(selected.len(), 1, "{mode:?}");
        assert_eq!(selected[0].display_name, "b.txt", "{mode:?}");
        assert_eq!(view.browser().column_snapshot(0).unwrap().selected_positions, vec![0], "{mode:?}");
        emit(DirectoryEvent::Finished { request_id: request.get() });
        emit(DirectoryEvent::Notice { request_id: request.get(), message: Some("Partial search coverage".into()) });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while mapped_notice(&view.widget().upcast()).is_none() && std::time::Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let notice = mapped_notice(&view.widget().upcast()).expect("Active view displays search notice");
        assert_eq!(notice.text(), "Partial search coverage", "{mode:?}");
        assert!(view.browser().column_snapshot(0).unwrap().notice.is_some());
        assert_eq!(view.browser().selected_entries()[0].display_name, "b.txt");
        for switched_mode in [BrowserMode::Columns, BrowserMode::Grid, BrowserMode::Explorer] {
            view.set_view_mode(switched_mode);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while mapped_notice(&view.widget().upcast()).is_none() && std::time::Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert_eq!(mapped_notice(&view.widget().upcast()).unwrap().text(), "Partial search coverage", "{switched_mode:?}");
        }
        let notice = mapped_notice(&view.widget().upcast()).unwrap();
        emit(DirectoryEvent::Notice { request_id: request.get(), message: None });
        assert!(!notice.is_visible(), "{mode:?}");
    }
    window.close();
    view.browser().clear_observer();
    captured.borrow_mut().take();
}

#[test]
#[ignore = "Requires a desktop display; verifies preview versus music activation routing"]
fn desktop_song_activation_uses_the_registered_music_handler() {
    struct SongSource;
    impl FileSource for SongSource {
        fn validate_location(&self, _: &Location) -> Result<(), crate::services::LocationValidationError> { Ok(()) }
        fn enumerate(&self, request: crate::services::DirectoryRequest, emit: Rc<dyn Fn(crate::services::DirectoryEvent)>) -> crate::services::LoadHandle {
            let entry = FileEntry(Rc::new(crate::model::FileEntryInner {
                location: Location::local("/fixture/03.flac"),
                native_name: "03.flac".into(), display_name: "03.flac".into(), kind: EntryKind::File,
                size: crate::model::MetadataValue::Unknown, modified_unix_seconds: crate::model::MetadataValue::Unknown, music_tags: None,
            }));
            emit(crate::services::DirectoryEvent::Batch { request_id: request.id, entries: vec![entry] });
            emit(crate::services::DirectoryEvent::Finished { request_id: request.id });
            crate::services::LoadHandle::new(|| {})
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let view = BrowserView::new(Rc::new(SongSource), PeekBehavior::default());
    let opened = Rc::new(RefCell::new(Vec::new()));
    let captured = opened.clone();
    view.set_open_handler(Rc::new(move |location| { captured.borrow_mut().push(location); true }));
    view.browser().navigate(Location::local("/fixture"));
    view.browser().preview(0, 0);
    assert!(opened.borrow().is_empty(), "Single-click preview must not play a song");
    view.browser().activate(0, 0);
    view.browser().activate_focused();
    assert_eq!(*opened.borrow(), vec![Location::local("/fixture/03.flac"); 2]);
    view.browser().clear_observer();
}

#[test]
fn file_sizes_use_compact_decimal_units() {
    assert_eq!(format_file_size(999), "999 B");
    assert_eq!(format_file_size(1_200), "1.2 kB");
    assert_eq!(format_file_size(1_000_000), "1 MB");
    assert_eq!(format_file_size(2_500_000_000), "2.5 GB");
}

#[test]
fn inline_rename_selects_the_stem_but_keeps_the_extension() {
    assert_eq!(rename_stem_end("report.txt"), 6);
    assert_eq!(rename_stem_end("archive.tar.gz"), 11);
    assert_eq!(rename_stem_end("README"), 6);
    assert_eq!(rename_stem_end(".gitignore"), 10);
}

#[test]
fn delete_confirmation_labels_distinguish_files_and_folders() {
    let file = crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { music_tags: None, location: Location::local("/fixture/file.txt"),
        native_name: "file.txt".into(),
        display_name: "file.txt".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Known(10),
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
    }));
    let folder = crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { music_tags: None, location: Location::local("/fixture/folder"),
        native_name: "folder".into(),
        display_name: "folder".into(),
        kind: crate::model::EntryKind::Directory,
        size: crate::model::MetadataValue::Known(10),
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
    }));

    assert_eq!(item_count_label(1), "1 item");
    assert_eq!(item_count_label(2), "2 items");
    assert_eq!(entry_kind_summary(std::slice::from_ref(&file)), "1 file");
    assert_eq!(entry_kind_summary(&[file.clone(), file.clone()]), "2 files");
    assert_eq!(entry_kind_summary(&[folder.clone()]), "1 folder");
    assert_eq!(entry_kind_summary(&[file, folder]), "2 items");
}

#[test]
fn only_the_trash_root_uses_the_aggregate_properties_size() {
    assert!(is_trash_root(&Location::uri("trash:///")));
    assert!(!is_trash_root(&Location::uri("trash:///photo.png")));
    assert!(!is_trash_root(&Location::local(
        "/home/user/.local/share/Trash"
    )));
}

#[test]
fn quick_preview_is_offered_only_for_supported_files() {
    let entry = |name: &str, kind| crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { music_tags: None, location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
    }));

    assert!(entry_supports_quick_preview(&entry(
        "photo.png",
        crate::model::EntryKind::File,
    )));
    assert!(entry_supports_quick_preview(&entry(
        "notes.txt",
        crate::model::EntryKind::FileSymbolicLink,
    )));
    assert!(entry_supports_quick_preview(&entry(
        "archive.zip",
        crate::model::EntryKind::File,
    )));
    assert!(entry_supports_quick_preview(&entry(
        "letter.docx",
        crate::model::EntryKind::File,
    )));
    assert!(!entry_supports_quick_preview(&entry(
        "photos.png",
        crate::model::EntryKind::Directory,
    )));

    let supported = entry("photo.png", crate::model::EntryKind::File);
    let unsupported = entry("unknown.data", crate::model::EntryKind::File);
    let directory = entry("photos", crate::model::EntryKind::Directory);
    assert!(entry_responds_to_single_click(&supported, true));
    assert!(!entry_responds_to_single_click(&supported, false));
    assert!(!entry_responds_to_single_click(&unsupported, true));
    assert!(entry_responds_to_single_click(&directory, false));
}

#[test]
fn incoming_file_lists_preserve_local_and_remote_locations() {
    let files = gtk::gdk::FileList::from_array(&[
        gio::File::for_path("/fixture/photo.raw"),
        gio::File::for_uri("sftp://host.example/home/user/video.mp4"),
    ]);
    let value = files.to_value();

    assert_eq!(
        locations_from_file_list_value(&value),
        Some(vec![
            Location::local("/fixture/photo.raw"),
            Location::uri("sftp://host.example/home/user/video.mp4"),
        ])
    );
}

#[test]
fn local_file_drops_prefer_move_while_external_drops_prefer_copy() {
    let both = gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE;

    assert_eq!(
        preferred_file_drop_action(both, true),
        gtk::gdk::DragAction::MOVE
    );
    assert_eq!(
        preferred_file_drop_action(both, false),
        gtk::gdk::DragAction::COPY
    );
    assert_eq!(
        preferred_file_drop_action(gtk::gdk::DragAction::MOVE, false),
        gtk::gdk::DragAction::MOVE
    );
}

#[test]
fn multi_selection_summary_lists_at_most_three_names() {
    let entry = |name: &str| crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { music_tags: None, location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
    }));

    assert_eq!(
        selected_items_summary(&[entry("one"), entry("two"), entry("three")]),
        "one, two, three"
    );
    assert_eq!(
        selected_items_summary(&[entry("one"), entry("two"), entry("three"), entry("four")]),
        "one, two, three, …"
    );
}

#[test]
fn trash_locations_include_the_root_and_descendants() {
    assert!(is_trash_location(&Location::uri("trash:///")));
    assert!(is_trash_location(&Location::uri("trash:///folder")));
    assert!(!is_trash_location(&Location::local(
        "/home/example/.local/share/Trash"
    )));
}

#[test]
fn transfer_collisions_detect_existing_destination_items() -> Result<(), Box<dyn std::error::Error>>
{
    let root = std::env::temp_dir().join(format!("strata-collision-test-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&root);
    let source_dir = root.join("source");
    let destination = root.join("destination");
    std::fs::create_dir_all(&source_dir)?;
    std::fs::create_dir_all(&destination)?;
    let source = source_dir.join("photo.jpg");
    std::fs::write(&source, b"new")?;

    assert!(!transfer_has_collision(
        &Location::local(&source),
        &Location::local(&destination)
    ));
    std::fs::write(destination.join("photo.jpg"), b"old")?;
    assert!(transfer_has_collision(
        &Location::local(&source),
        &Location::local(&destination)
    ));

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn destination_paths_expand_home_and_relative_input() {
    let base = std::path::Path::new("/work/current");
    let home = std::path::Path::new("/home/example");

    assert_eq!(resolve_destination_path("~", base, home), home);
    assert_eq!(
        resolve_destination_path("~/Documents", base, home),
        home.join("Documents")
    );
    assert_eq!(
        resolve_destination_path("../Archive", base, home),
        base.join("../Archive")
    );
    assert_eq!(
        resolve_destination_path("/tmp/export", base, home),
        std::path::Path::new("/tmp/export")
    );
}

#[test]
fn directory_autocomplete_suggests_only_matching_folders() -> Result<(), Box<dyn std::error::Error>>
{
    let root = std::env::temp_dir().join(format!("strata-path-suggestions-{}", std::process::id()));
    let _ignored = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("Documents"))?;
    std::fs::create_dir_all(root.join("Downloads"))?;
    std::fs::write(root.join("Document.txt"), b"not a folder")?;

    let suggestions = directory_suggestions(&format!("{}/Doc", root.display()), &root, &root);

    assert_eq!(suggestions, vec![root.join("Documents")]);
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn properties_permissions_are_formatted_symbolically_and_numerically() {
    assert_eq!(format_permissions(0o100774), "-rwxrwxr--  774");
    assert_eq!(format_permissions(0o040755), "drwxr-xr-x  755");
}

#[test]
fn properties_paths_abbreviate_the_home_directory() {
    let home = glib::home_dir();

    assert_eq!(compact_display_path(&Location::local(&home)), "~");
    assert_eq!(
        compact_display_path(&Location::local(home.join("Documents/report.txt"))),
        "~/Documents/report.txt"
    );
    assert_eq!(
        compact_display_path(&Location::uri("trash:///example")),
        "trash:///example"
    );
}

#[test]
fn file_names_map_to_specific_lucide_icons() {
    assert_eq!(icon_for_name("setup.sh"), crate::assets::icons::TERMINAL);
    assert_eq!(icon_for_name("photo.webp"), crate::assets::icons::PICTURES);
    assert_eq!(icon_for_name("movie.mkv"), crate::assets::icons::VIDEOS);
    assert_eq!(icon_for_name("album.flac"), crate::assets::icons::MUSIC);
    assert_eq!(icon_for_name("source.rs"), crate::assets::icons::FILE_CODE);
    assert_eq!(
        icon_for_name("backup.tar"),
        crate::assets::icons::FILE_ARCHIVE
    );
    assert_eq!(icon_for_name("README.md"), crate::assets::icons::DOCUMENTS);
}

#[test]
fn pressing_an_item_in_a_multi_selection_preserves_the_drag_group() {
    assert!(should_preserve_drag_selection(true, 2));
    assert!(should_preserve_drag_selection(true, 8));
    assert!(!should_preserve_drag_selection(true, 1));
    assert!(!should_preserve_drag_selection(false, 4));
}

#[test]
fn paste_prefers_the_hovered_pane_then_the_deepest_pane() {
    assert_eq!(paste_destination_depth(Some(1), 3), Some(1));
    assert_eq!(paste_destination_depth(None, 3), Some(2));
    assert_eq!(paste_destination_depth(Some(4), 3), Some(2));
    assert_eq!(paste_destination_depth(None, 0), None);
}

#[test]
fn new_folder_prefers_the_hovered_pane_then_falls_back_safely() {
    assert_eq!(
        new_folder_destination_depth(Some(1), Some(0), Some(2), 3),
        Some(1)
    );
    assert_eq!(
        new_folder_destination_depth(None, Some(0), Some(2), 3),
        Some(0)
    );
    assert_eq!(
        new_folder_destination_depth(Some(4), Some(5), Some(1), 3),
        Some(1)
    );
    assert_eq!(new_folder_destination_depth(None, None, None, 3), Some(2));
    assert_eq!(new_folder_destination_depth(None, None, None, 0), None);
}

#[test]
fn cut_clipboard_locations_match_regardless_of_order() {
    let first = Location::local("/fixture/first");
    let second = Location::local("/fixture/second");

    assert!(same_locations(
        &[first.clone(), second.clone()],
        &[second, first]
    ));
    assert!(!same_locations(&[], &[]));
    assert!(!same_locations(
        &[Location::local("/fixture/first")],
        &[Location::local("/fixture/other")]
    ));
}

#[test]
fn single_pane_modes_reserve_half_for_preview_sizing() {
    assert_eq!(single_pane_preview_reservation(800), 400);
    assert_eq!(single_pane_preview_reservation(0), 0);
}

#[test]
fn music_context_actions_require_an_audio_only_selection() {
    let entry = |name: &str, kind| FileEntry(Rc::new(crate::model::FileEntryInner {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        kind,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        music_tags: None,
    }));
    let track = entry("track.flac", EntryKind::File);
    let second = entry("song.mp3", EntryKind::FileSymbolicLink);
    assert!(audio_context_actions_visible(std::slice::from_ref(&track)));
    assert!(audio_context_actions_visible(&[track.clone(), second]));
    assert!(!audio_context_actions_visible(&[]));
    assert!(!audio_context_actions_visible(&[entry("album.flac", EntryKind::Directory)]));
    assert!(!audio_context_actions_visible(&[track, entry("cover.jpg", EntryKind::File)]));
}

#[test]
fn pane_resizing_preserves_the_initial_minimum_width() {
    assert_eq!(resized_column_width(COLUMN_WIDTH, -80.0), COLUMN_WIDTH);
    assert_eq!(resized_column_width(COLUMN_WIDTH, 75.0), 375);
    assert_eq!(resized_column_width(420, -20.0), 400);
}

#[test]
fn reveal_target_scrolls_only_enough_to_show_the_new_column() {
    assert_eq!(
        horizontal_reveal_target(0.0, 900.0, 0.0, 1_200.0, 900.0, 1_200.0),
        300.0
    );
}

#[test]
fn reveal_target_is_stable_when_the_column_is_already_visible() {
    assert_eq!(
        horizontal_reveal_target(300.0, 900.0, 0.0, 1_500.0, 900.0, 1_200.0),
        300.0
    );
}

#[test]
fn reveal_target_can_scroll_back_to_an_earlier_column() {
    assert_eq!(
        horizontal_reveal_target(600.0, 900.0, 0.0, 1_500.0, 300.0, 600.0),
        300.0
    );
}

#[test]
#[ignore = "Requires a desktop display; run explicitly"]
fn desktop_hover_peek_lifecycle() {
    use crate::services::{DirectoryEvent, DirectoryRequest, LoadHandle, LocationValidationError};

    struct GeneratedSource {
        requests: Rc<RefCell<Vec<Location>>>,
        cancellations: Rc<Cell<usize>>,
    }
    impl FileSource for GeneratedSource {
        fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
            Ok(())
        }

        fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
            self.requests.borrow_mut().push(request.location.clone());
            let entries = (0..3).map(|index| FileEntry(Rc::new(crate::model::FileEntryInner {
                location: Location::local(format!("{}/folder-{index}", request.location.display_path())),
                native_name: format!("folder-{index}").into(),
                display_name: format!("folder-{index}"),
                kind: EntryKind::Directory,
                size: crate::model::MetadataValue::Unknown,
                modified_unix_seconds: crate::model::MetadataValue::Unknown,
                music_tags: None,
            }))).collect();
            emit(DirectoryEvent::Batch { request_id: request.id, entries });
            emit(DirectoryEvent::Finished { request_id: request.id });
            let cancellations = self.cancellations.clone();
            LoadHandle::new(move || cancellations.set(cancellations.get() + 1))
        }
    }

    gtk::init().expect("Desktop display available");
    crate::assets::prepare().expect("Icons load");
    let requests = Rc::new(RefCell::new(Vec::new()));
    let cancellations = Rc::new(Cell::new(0));
    let view = BrowserView::new(Rc::new(GeneratedSource {
        requests: requests.clone(),
        cancellations: cancellations.clone(),
    }), PeekBehavior {
        open_delay: Duration::from_millis(5),
        close_delay: Duration::from_millis(5),
        fade_duration: Duration::ZERO,
        item_limit: 2,
    });
    let window = gtk::Window::builder().default_width(900).default_height(500)
        .child(&view.state.overlay).build();
    view.state.browser.navigate(Location::local("/generated"));
    window.present();
    let drain = || {
        let until = Instant::now() + Duration::from_millis(100);
        while Instant::now() < until {
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    drain();
    let anchor: gtk::Widget = view.state.columns.borrow()[0].shell.clone().upcast();
    let first = Location::local("/generated/folder-0");
    let second = Location::local("/generated/folder-1");
    view.state.browser.begin_peek(0, first.clone());
    assert_eq!(requests.borrow().len(), 1, "Missing anchors must not start a load");

    view.state.schedule_peek(0, first.clone(), anchor.clone());
    view.state.schedule_peek(0, second.clone(), anchor.clone());
    drain();
    assert_eq!(requests.borrow().len(), 2, "Only the newest timer opens");
    assert_eq!(requests.borrow().last(), Some(&second));
    let peek = view.state.peek.borrow().clone().expect("Valid anchor opens peek");
    assert_eq!(peek.model.n_items(), 2);
    assert_eq!(peek.entry_count.get(), 2);
    assert!(view.state.pending_peek.borrow().is_none());
    view.state.schedule_close_peek();
    view.state.schedule_peek(0, second.clone(), anchor.clone());
    drain();
    assert_eq!(requests.borrow().len(), 2, "Reentering keeps the same peek");
    assert!(view.state.pending_close.borrow().is_none());
    view.state.schedule_close_peek();
    drain();
    assert!(view.state.peek.borrow().is_none());
    assert!(view.state.pending_close.borrow().is_none());
    assert_eq!(cancellations.get(), 1);
    assert!(peek.revealer.parent().is_none());

    view.state.schedule_peek(0, first.clone(), anchor.clone());
    view.set_peek_enabled(false);
    drain();
    assert_eq!(requests.borrow().len(), 2);
    assert!(view.state.pending_peek.borrow().is_none());
    assert!(view.state.peek_anchor.borrow().is_none());
    view.set_peek_enabled(true);

    view.state.schedule_peek(0, first.clone(), anchor.clone());
    view.set_view_mode(BrowserMode::Grid);
    drain();
    assert_eq!(requests.borrow().len(), 2, "Mode changes invalidate pending anchors");
    view.set_view_mode(BrowserMode::Columns);
    drain();

    let column = view.state.columns.borrow()[0].clone();
    let row = column.bound_rows.borrow().iter().find_map(|bound| {
        let item = bound.item.upgrade()?;
        (item.position() == 0).then(|| bound.row.upgrade()).flatten()
    }).expect("Generated first row is bound");
    view.state.schedule_peek(0, first.clone(), row.upcast());
    let replacement = entry_model_value(&view.state.browser.entry_at(0, 1).unwrap());
    column.model.splice(0, 1, &[&replacement]);
    drain();
    assert_eq!(requests.borrow().len(), 2, "Recycled rows cannot open their old folder");
    view.state.handle(BrowserEvent::EntriesReplaced {
        depth: 0,
        entries: view.state.browser.column_entries(0).unwrap(),
    });
    drain();

    view.state.schedule_peek(0, first.clone(), anchor.clone());
    view.state.browser.navigate(Location::local("/replacement"));
    drain();
    assert_eq!(requests.borrow().len(), 3, "Navigation cancels the old hover timer");
    assert!(view.state.peek.borrow().is_none());
    assert!(view.state.peek_anchor.borrow().is_none());
    view.state.schedule_peek(0, Location::local("/replacement/folder-0"), anchor);
    drain();
    assert_eq!(requests.borrow().len(), 3, "Detached anchors cannot reopen a peek");
    view.state.browser.navigate(Location::local("/generated"));
    drain();
    let anchor: gtk::Widget = view.state.columns.borrow()[0].shell.clone().upcast();
    for _ in 0..4 {
        view.state.schedule_peek(0, first.clone(), anchor.clone());
        drain();
        view.state.schedule_close_peek();
        drain();
        assert!(view.state.pending_peek.borrow().is_none());
        assert!(view.state.pending_close.borrow().is_none());
    }

    view.state.schedule_peek(0, first.clone(), anchor.clone());
    drain();
    let finished = view.state.peek.borrow().clone().unwrap();
    finished.spinner.start();
    let weak = Rc::downgrade(&view.state);
    finished.spinner.connect_notify_local(Some("spinning"), move |_, _| {
        weak.upgrade().unwrap().browser.close_peek();
    });
    view.state.handle(BrowserEvent::PeekFinished);
    assert!(view.state.peek.borrow().is_none(), "Finishing may close synchronously");
    drain();

    view.state.schedule_peek(0, first.clone(), anchor.clone());
    drain();
    let failed = view.state.peek.borrow().clone().unwrap();
    let weak = Rc::downgrade(&view.state);
    failed.presentation.message.connect_notify_local(Some("label"), move |_, _| {
        weak.upgrade().unwrap().browser.close_peek();
    });
    view.state.handle(BrowserEvent::PeekFailed { message: "Generated failure".into() });
    assert!(view.state.peek.borrow().is_none(), "Errors may close synchronously");
    drain();

    let children = view.state.overlay.observe_children();
    let weak = Rc::downgrade(&view.state);
    let handler = children.connect_items_changed(move |_, _, _, _| {
        weak.upgrade().unwrap().browser.close_peek();
    });
    let before = requests.borrow().len();
    view.state.schedule_peek(0, first.clone(), anchor.clone());
    drain();
    assert!(view.state.peek.borrow().is_none(), "Reentrant overlay closure stays closed");
    assert_eq!(requests.borrow().len(), before, "Closed overlays must not start loading");
    children.disconnect(handler);

    let connected = Rc::new(Cell::new(false));
    let observed = connected.clone();
    let weak = Rc::downgrade(&view.state);
    view.state.browser.observe(move |event| {
        if matches!(event, BrowserEvent::PeekStarted { .. }) {
            let state = weak.upgrade().unwrap();
            let model = state.peek.borrow().as_ref().expect("Peek attached").model.clone();
            let weak = Rc::downgrade(&state);
            model.connect_items_changed(move |_, _, _, _| {
                weak.upgrade().unwrap().browser.close_peek();
            });
            observed.set(true);
        }
    });
    view.state.schedule_peek(0, Location::local("/generated/folder-0"), anchor);
    drain();
    assert!(connected.get(), "Timer opened peek before synchronous enumeration");
    assert!(view.state.peek.borrow().is_none());
    drain();
    view.state.browser.clear_observer();
    window.close();
}
