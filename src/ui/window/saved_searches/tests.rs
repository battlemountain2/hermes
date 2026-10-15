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
