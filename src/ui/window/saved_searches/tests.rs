use super::*;
use crate::services::{
    DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
    ScopedSearch, SearchScope,
    saved_searches::{Command, Store},
};

struct EmptySource;

impl FileSource for EmptySource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }
    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Finished {
            request_id: request.id,
        });
        LoadHandle::new(|| {})
    }
}

fn sidebar_children(sidebar: &SidebarView) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut child = sidebar.state.widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        children.push(current);
    }
    children
}

fn wait_for_sidebar_revision(sidebar: &SidebarView, store: &Store) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while sidebar.state.saved_revision.get() != store.revision() {
        assert!(Instant::now() < deadline, "Missing sidebar revision update");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn assert_sidebar_idle(sidebar: &SidebarView) {
    let children = sidebar_children(sidebar);
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        assert_eq!(sidebar_children(sidebar), children);
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn smart_folder_label(sidebar: &SidebarView, location: &Location) -> Option<String> {
    sidebar
        .state
        .place_rows
        .borrow()
        .iter()
        .find(|(entry, _)| entry == location)
        .map(|(_, row)| {
            row.child()
                .unwrap()
                .last_child()
                .unwrap()
                .downcast::<gtk::Label>()
                .unwrap()
                .text()
                .to_string()
        })
}

#[test]
#[ignore = "Private GTK/XDG; hidden Smart Folders acknowledge store changes without rebuilding idle rows"]
fn desktop_hidden_smart_folders_acknowledge_updates_without_rebuilding_rows() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let theme_manager = ThemeManager::shared();
    theme_manager.set_show_smart_folders(false);
    let root = std::env::temp_dir().join(format!("hermes-sidebar-hidden-{}", std::process::id()));
    let store = Store::start(root.join("private/queries.json"));
    assert!(store.flush(Duration::from_secs(3)));
    let view = BrowserView::new(Rc::new(EmptySource), PeekBehavior::default());
    let sidebar = build_sidebar_with_searches(view, theme_manager, store.clone());
    assert!(store.revision() > 0);
    assert_eq!(sidebar.state.saved_revision.get(), store.revision());
    let children = sidebar_children(&sidebar);
    assert!(!children.is_empty());
    assert_sidebar_idle(&sidebar);
    let request = ScopedSearch::new(
        SearchScope::ThisFolder,
        std::path::Path::new("/synthetic/photos"),
        "type:image",
    );
    let location = Location::uri(request.to_uri());
    store
        .send(Command::Create {
            name: "Photos".into(),
            uri: request.to_uri(),
        })
        .unwrap();
    assert!(store.flush(Duration::from_secs(3)));
    let previous = store.snapshot().entries[0].clone();
    for command in [
        Command::Update {
            previous: previous.clone(),
            name: previous.name.clone(),
            uri: previous.uri.clone(),
        },
        Command::Reload,
        Command::Delete(previous),
    ] {
        assert_ne!(sidebar.state.saved_revision.get(), store.revision());
        wait_for_sidebar_revision(&sidebar, &store);
        assert_eq!(sidebar_children(&sidebar), children);
        assert_eq!(smart_folder_label(&sidebar, &location), None);
        assert_sidebar_idle(&sidebar);
        store.send(command).unwrap();
        assert!(store.flush(Duration::from_secs(3)));
    }
    wait_for_sidebar_revision(&sidebar, &store);
    assert_eq!(sidebar_children(&sidebar), children);
    assert_sidebar_idle(&sidebar);
    sidebar.disconnect();
    drop(sidebar);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Private GTK/XDG; Smart Folder toggles render current snapshots and retain concurrent updates for polling"]
fn desktop_smart_folders_toggle_visibility_and_poll_snapshot_revisions() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let theme_manager = ThemeManager::shared();
    theme_manager.set_show_smart_folders(false);
    let root = std::env::temp_dir().join(format!("hermes-sidebar-toggle-{}", std::process::id()));
    let store = Store::start(root.join("private/queries.json"));
    assert!(store.flush(Duration::from_secs(3)));
    let request = ScopedSearch::new(
        SearchScope::ThisFolder,
        std::path::Path::new("/synthetic/photos"),
        "type:image",
    );
    store
        .send(Command::Create {
            name: "Photos".into(),
            uri: request.to_uri(),
        })
        .unwrap();
    assert!(store.flush(Duration::from_secs(3)));
    let location = Location::uri(request.to_uri());
    let view = BrowserView::new(Rc::new(EmptySource), PeekBehavior::default());
    let sidebar = build_sidebar_with_searches(view, theme_manager.clone(), store.clone());
    assert_eq!(smart_folder_label(&sidebar, &location), None);
    let rendered_revision = store.revision();
    let previous = store.snapshot().entries[0].clone();
    let children = sidebar.state.widget.observe_children();
    let changed_during_render = Rc::new(Cell::new(false));
    let changed = changed_during_render.clone();
    let concurrent_store = store.clone();
    let handler = children.connect_items_changed(move |children, position, _, added| {
        for offset in 0..added {
            if let Some(heading) = children
                .item(position + offset)
                .and_downcast::<gtk::Label>()
                && heading.text() == "SMART FOLDERS"
                && !changed.replace(true)
            {
                concurrent_store
                    .send(Command::Update {
                        previous: previous.clone(),
                        name: "New photos".into(),
                        uri: previous.uri.clone(),
                    })
                    .unwrap();
                assert!(concurrent_store.flush(Duration::from_secs(3)));
            }
        }
    });
    theme_manager.set_show_smart_folders(true);
    sidebar.state.rebuild();
    children.disconnect(handler);
    assert!(changed_during_render.get());
    assert_eq!(
        smart_folder_label(&sidebar, &location).as_deref(),
        Some("Photos")
    );
    assert_eq!(sidebar.state.saved_revision.get(), rendered_revision);
    assert_ne!(sidebar.state.saved_revision.get(), store.revision());
    wait_for_sidebar_revision(&sidebar, &store);
    assert_eq!(
        smart_folder_label(&sidebar, &location).as_deref(),
        Some("New photos")
    );
    assert_sidebar_idle(&sidebar);

    theme_manager.set_show_smart_folders(false);
    sidebar.state.rebuild();
    assert_eq!(sidebar.state.saved_revision.get(), store.revision());
    assert_eq!(smart_folder_label(&sidebar, &location), None);
    let hidden_children = sidebar_children(&sidebar);
    let previous = store.snapshot().entries[0].clone();
    store
        .send(Command::Update {
            previous: previous.clone(),
            name: "Latest photos".into(),
            uri: previous.uri,
        })
        .unwrap();
    assert!(store.flush(Duration::from_secs(3)));
    wait_for_sidebar_revision(&sidebar, &store);
    assert_eq!(sidebar_children(&sidebar), hidden_children);
    assert_sidebar_idle(&sidebar);

    theme_manager.set_show_smart_folders(true);
    sidebar.state.rebuild();
    assert_eq!(
        smart_folder_label(&sidebar, &location).as_deref(),
        Some("Latest photos")
    );
    assert_eq!(sidebar.state.saved_revision.get(), store.revision());
    assert_sidebar_idle(&sidebar);
    sidebar.disconnect();
    drop(sidebar);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Private GTK/XDG; search Save switches modal layers without indexing user folders"]
fn desktop_search_save_opens_manager_and_dismisses_without_stacked_modals() {
    fn find_button(widget: &gtk::Widget, text: &str) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && (button.label().as_deref() == Some(text)
                || button.tooltip_text().as_deref() == Some(text))
        {
            return Some(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if let Some(button) = find_button(&current, text) {
                return Some(button);
            }
        }
        None
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let theme_manager = ThemeManager::shared();
    theme_manager.set_show_smart_folders(true);
    let root = std::env::temp_dir().join(format!("hermes-saved-layers-{}", std::process::id()));
    let store = Store::start(root.join("private/queries.json"));
    assert!(store.flush(Duration::from_secs(3)));
    let view = BrowserView::new(Rc::new(EmptySource), PeekBehavior::default());
    let sidebar = build_sidebar_with_searches(view.clone(), theme_manager, store.clone());
    let contents = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    contents.append(&sidebar.widget);
    contents.append(&view.widget());
    let blurred = BlurBin::new(&contents);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&blurred));
    let dismissed_root = blurred.clone();
    let search = SearchDialog::new(
        Rc::new(|_| {}),
        Rc::new(move || dismissed_root.set_blurred(false)),
    );
    overlay.add_overlay(&search.widget());
    let manager = install(&overlay, &blurred, &search, &sidebar);
    let window = gtk::Window::builder()
        .child(&overlay)
        .default_width(1000)
        .default_height(900)
        .build();
    window.present();
    search.widget().set_visible(true);
    find_button(&search.widget(), "Save search")
        .unwrap()
        .emit_clicked();
    assert!(!search.is_visible());
    assert!(manager.is_visible());
    assert_eq!(store.snapshot().entries.len(), 0);
    find_button(manager.upcast_ref(), "Close saved searches (Esc)")
        .unwrap()
        .emit_clicked();
    assert!(!manager.is_visible());
    assert!(!search.is_visible());
    assert_eq!(store.snapshot().entries.len(), 0);
    sidebar.disconnect();
    window.close();
    drop(sidebar);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Private GTK/XDG; Smart Folder rows open current queries, update, reject file drops and clean up"]
fn desktop_smart_folders_follow_store_updates_and_open_virtual_queries() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let theme_manager = ThemeManager::shared();
    theme_manager.set_show_smart_folders(true);
    let root = std::env::temp_dir().join(format!("hermes-sidebar-saved-{}", std::process::id()));
    let store = Store::start(root.join("private/queries.json"));
    assert!(store.flush(Duration::from_secs(3)));
    let request = ScopedSearch::new(
        SearchScope::ThisFolder,
        std::path::Path::new("/synthetic/photos"),
        "type:image",
    );
    store
        .send(Command::Create {
            name: "Photos".into(),
            uri: request.to_uri(),
        })
        .unwrap();
    assert!(store.flush(Duration::from_secs(3)));
    let view = BrowserView::new(Rc::new(EmptySource), PeekBehavior::default());
    let sidebar = build_sidebar_with_searches(view.clone(), theme_manager, store.clone());
    let widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    widget.append(&sidebar.widget);
    widget.append(&view.widget());
    let window = gtk::Window::builder()
        .child(&widget)
        .default_width(1000)
        .default_height(800)
        .build();
    window.present();
    let location = Location::uri(request.to_uri());
    let row = sidebar
        .state
        .place_rows
        .borrow()
        .iter()
        .find(|(entry, _)| *entry == location)
        .unwrap()
        .1
        .clone();
    let controllers = row.observe_controllers();
    assert!(
        (0..controllers.n_items())
            .all(|position| !controllers.item(position).unwrap().is::<gtk::DropTarget>())
    );
    row.emit_clicked();
    assert_eq!(view.browser().active_location(), Some(location.clone()));
    let previous = store.snapshot().entries[0].clone();
    store
        .send(Command::Update {
            previous: previous.clone(),
            name: "New photos".into(),
            uri: previous.uri.clone(),
        })
        .unwrap();
    assert!(store.flush(Duration::from_secs(3)));
    let wait_for = |name: &str| {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if sidebar.state.saved_revision.get() == store.revision() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Missing sidebar update for {name}"
            );
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    wait_for("rename");
    let row = sidebar
        .state
        .place_rows
        .borrow()
        .iter()
        .find(|(entry, _)| *entry == location)
        .unwrap()
        .1
        .clone();
    assert!(row.has_css_class("active"));
    let label = row
        .child()
        .unwrap()
        .last_child()
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    assert_eq!(label.text(), "New photos");
    store
        .send(Command::Delete(store.snapshot().entries[0].clone()))
        .unwrap();
    assert!(store.flush(Duration::from_secs(3)));
    wait_for("delete");
    assert!(
        !sidebar
            .state
            .place_rows
            .borrow()
            .iter()
            .any(|(entry, _)| *entry == location)
    );
    assert_eq!(view.browser().active_location(), Some(location));
    sidebar.disconnect();
    assert!(sidebar.saved_updates.borrow().is_none());
    window.close();
    drop(sidebar);
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
