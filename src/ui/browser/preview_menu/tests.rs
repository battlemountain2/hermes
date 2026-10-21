use super::*;
use crate::services::{
    LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
};

fn find(widget: &gtk::Widget, predicate: &impl Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if predicate(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(found) = find(&widget, predicate) {
            return Some(found);
        }
    }
    None
}

fn button(widget: &gtk::Widget, label: &str) -> gtk::Button {
    find(widget, &|widget| {
        widget.downcast_ref::<gtk::Button>().is_some_and(|button| {
            button.label().is_some_and(|title| title == label)
                || button
                    .child()
                    .and_then(|row| row.first_child())
                    .and_then(|icon| icon.next_sibling())
                    .and_downcast::<gtk::Label>()
                    .is_some_and(|title| title.text() == label)
        })
    })
    .unwrap_or_else(|| panic!("Missing button {label}"))
    .downcast()
    .unwrap()
}

fn pump(window: &gtk::Window) {
    let deadline = Instant::now() + Duration::from_millis(220);
    while Instant::now() < deadline {
        for _ in 0..128 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        if let Some(clock) = window.frame_clock() {
            clock.emit_by_name::<()>("update", &[]);
            clock.emit_by_name::<()>("layout", &[]);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[track_caller]
fn released(menu: &glib::WeakRef<gtk::Popover>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while menu.upgrade().is_some() && Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        menu.upgrade().is_none(),
        "Closed popover must be released: {:?}",
        menu.upgrade()
            .map(|menu| (menu.ref_count(), menu.parent(), menu.root()))
    );
}

fn viewport(root: &gtk::Widget) -> gtk::Widget {
    find(root, &|widget| widget.has_css_class("hermes-image-player"))
        .unwrap()
        .first_child()
        .unwrap()
}

fn popup(viewport: &gtk::Widget, window: &gtk::Window) -> gtk::Popover {
    let controllers = viewport.observe_controllers();
    let click = (0..controllers.n_items())
        .filter_map(|position| {
            controllers
                .item(position)
                .and_downcast::<gtk::GestureClick>()
        })
        .find(|click| click.name().as_deref() == Some("hermes-image-context-menu"))
        .unwrap();
    assert_eq!(click.button(), 3);
    assert_eq!(click.propagation_phase(), gtk::PropagationPhase::Capture);
    click.emit_by_name::<()>("pressed", &[&1_i32, &24.0_f64, &24.0_f64]);
    window.queue_draw();
    find(viewport, &|widget| {
        widget.has_css_class("image-context-menu")
    })
    .unwrap()
    .downcast()
    .unwrap()
}

struct GeneratedPhoto(Vec<u8>);

impl PreviewProvider for GeneratedPhoto {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        emit(PreviewEvent::Ready(Preview {
            request_id: request.id,
            entry: request.entry,
            content_type: "image/png".into(),
            content: PreviewContent::Rasterized {
                png: self.0.clone(),
            },
        }));
        LoadHandle::new(|| {})
    }
}

#[test]
#[ignore = "Requires a private GTK display/session and private XDG directories; generated photos only"]
#[expect(deprecated, reason = "Exercise the application's GTK rename dialog")]
fn desktop_photo_menu_targets_snapshot_and_cleans_up_drawer_and_fullscreen() {
    let config = PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap());
    assert!(
        config.starts_with(std::env::temp_dir()),
        "Use a private XDG config"
    );
    let data = PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap());
    assert!(
        data.starts_with(std::env::temp_dir()),
        "Use private trash storage"
    );
    let root = config
        .parent()
        .unwrap()
        .join(format!("preview-menu-{}", std::process::id()));
    assert!(!root.exists());
    std::fs::create_dir_all(&root).unwrap();
    gtk::init().unwrap();
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(false);
    crate::assets::prepare().unwrap();
    let manager = crate::ui::theme::ThemeManager::shared();
    manager.set_context_menu_options(crate::ui::context_menu::Options::default());
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 1600, 1000)
            .unwrap();
    pixels.fill(0x40a0c0ff);
    let png = pixels.save_to_bufferv("png", &[]).unwrap();
    let photo = root.join("a-preview.png");
    let other = root.join("z-selection.png");
    std::fs::write(&photo, &png).unwrap();
    std::fs::write(&other, &png).unwrap();
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    let drawer = crate::ui::preview::PreviewDrawer::new(Rc::new(GeneratedPhoto(png)));
    drawer.set_image_context_menu_handler(view.image_context_menu_handler());
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    content.append(&view.widget());
    content.append(&drawer.widget());
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&content));
    let window = gtk::Window::builder()
        .default_width(1100)
        .default_height(650)
        .child(&overlay)
        .build();
    window.present();
    view.browser().navigate(Location::local(&root));
    pump(&window);
    let target = view.browser().entry_at(0, 0).unwrap();
    assert_eq!(target.location, Location::local(&photo));
    view.browser().select(0, 1);
    let selection = view.browser().selected_entries();
    drawer.show(target);
    pump(&window);
    let image = viewport(&drawer.widget());
    let menu = popup(&image, &window);
    for label in [
        "Open",
        "Open With…",
        "Cut",
        "Copy",
        "Copy location",
        "Rename",
        "Move to Trash",
        "Properties",
        "Fullscreen",
        "Fit",
        "100%",
    ] {
        assert!(button(menu.upcast_ref(), label).is_visible());
    }
    assert_eq!(
        view.browser().selected_entries(),
        selection,
        "Right-click must not change selection"
    );
    let weak_menu = menu.downgrade();
    button(menu.upcast_ref(), "Copy").emit_clicked();
    assert!(menu.parent().is_none(), "Closed menu must be unparented");
    drop(menu);
    pump(&window);
    released(&weak_menu);
    let clipboard = gtk::gdk::Display::default().unwrap().clipboard();
    let files = glib::MainContext::default()
        .block_on(
            clipboard.read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT),
        )
        .unwrap()
        .get::<gtk::gdk::FileList>()
        .unwrap()
        .files();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path().unwrap(), photo);
    button(popup(&image, &window).upcast_ref(), "Cut").emit_clicked();
    assert_eq!(
        *view.state.cut_locations.borrow(),
        vec![Location::local(&photo)]
    );
    button(popup(&image, &window).upcast_ref(), "Copy location").emit_clicked();
    assert_eq!(
        glib::MainContext::default()
            .block_on(clipboard.read_text_future())
            .unwrap()
            .unwrap()
            .as_str(),
        photo.to_str().unwrap()
    );
    assert_eq!(view.browser().selected_entries(), selection);
    let mut options = manager.context_menu_options();
    options.open_with = false;
    manager.set_context_menu_options(options);
    let menu = popup(&image, &window);
    assert!(
        find(menu.upcast_ref(), &|widget| widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|title| title.text() == "Open With…"))
        .is_none()
    );
    button(menu.upcast_ref(), "100%").emit_clicked();
    pump(&window);
    let scroll = image
        .clone()
        .downcast::<gtk::Overlay>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    let area = scroll
        .child()
        .unwrap()
        .downcast::<gtk::Viewport>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::DrawingArea>()
        .unwrap();
    assert_eq!(area.content_width(), 1600);
    let drag = (0..scroll.observe_controllers().n_items())
        .find_map(|position| {
            scroll
                .observe_controllers()
                .item(position)
                .and_downcast::<gtk::GestureDrag>()
        })
        .unwrap();
    assert_eq!(
        drag.button(),
        1,
        "Secondary click must not claim the pan gesture"
    );
    button(popup(&image, &window).upcast_ref(), "Fit").emit_clicked();
    pump(&window);
    assert!(area.content_width() < 1600);
    let keys = (0..image.observe_controllers().n_items())
        .find_map(|position| {
            image
                .observe_controllers()
                .item(position)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .unwrap();
    assert!(!keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::F10,
            &0_u32,
            &gtk::gdk::ModifierType::empty()
        ]
    ));
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::F10,
            &0_u32,
            &gtk::gdk::ModifierType::SHIFT_MASK
        ]
    ));
    let keyboard_menu = find(&image, &|widget| widget.has_css_class("image-context-menu")).unwrap();
    assert!(button(&keyboard_menu, "Copy").is_visible());
    let replaced_menu = keyboard_menu.downgrade();
    button(popup(&image, &window).upcast_ref(), "Fullscreen").emit_clicked();
    assert!(
        keyboard_menu.parent().is_none(),
        "Opening another menu dismisses the previous one"
    );
    drop(keyboard_menu);
    assert!(
        replaced_menu.upgrade().is_none() || replaced_menu.upgrade().unwrap().parent().is_none()
    );
    pump(&window);
    let fullscreen = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| window.title().as_deref() == Some("Hermes — Image preview"))
        .unwrap();
    let fullscreen_image = viewport(&fullscreen.child().unwrap());
    let menu = popup(&fullscreen_image, &fullscreen);
    button(menu.upcast_ref(), "Copy location").emit_clicked();
    assert_eq!(
        glib::MainContext::default()
            .block_on(clipboard.read_text_future())
            .unwrap()
            .unwrap()
            .as_str(),
        photo.to_str().unwrap()
    );
    let menu = popup(&fullscreen_image, &fullscreen);
    let weak_fullscreen_menu = menu.downgrade();
    button(menu.upcast_ref(), "Exit fullscreen").emit_clicked();
    drop(menu);
    drop(fullscreen_image);
    drop(fullscreen);
    pump(&window);
    released(&weak_fullscreen_menu);
    button(popup(&image, &window).upcast_ref(), "Rename").emit_clicked();
    pump(&window);
    let dialog = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Dialog>().ok())
        .find(|dialog| dialog.title().as_deref() == Some("Rename"))
        .unwrap();
    let field = find(dialog.upcast_ref(), &|widget| widget.is::<gtk::Entry>())
        .unwrap()
        .downcast::<gtk::Entry>()
        .unwrap();
    assert_eq!(field.text(), "a-preview.png");
    field.set_text("../invalid.png");
    dialog.response(gtk::ResponseType::Accept);
    assert!(field.has_css_class("error") && photo.exists());
    field.set_text("z-selection.png");
    dialog.response(gtk::ResponseType::Accept);
    pump(&window);
    assert!(dialog.is_visible() && field.is_sensitive() && field.has_css_class("error"));
    assert!(
        photo.exists() && other.exists(),
        "Backend rename errors must keep the target and dialog intact"
    );
    field.set_text("renamed.png");
    view.browser().select(0, 1);
    dialog.response(gtk::ResponseType::Accept);
    pump(&window);
    assert!(root.join("renamed.png").exists() && !photo.exists() && other.exists());
    let target = view
        .browser()
        .column_entries(0)
        .unwrap()
        .into_iter()
        .find(|entry| entry.display_name == "renamed.png")
        .unwrap();
    drawer.show(target);
    pump(&window);
    let image = viewport(&drawer.widget());
    drawer.open_fullscreen();
    pump(&window);
    let fullscreen = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| window.title().as_deref() == Some("Hermes — Image preview"))
        .unwrap();
    let fullscreen_image = viewport(&fullscreen.child().unwrap());
    button(
        popup(&fullscreen_image, &fullscreen).upcast_ref(),
        "Properties",
    )
    .emit_clicked();
    assert!(
        !fullscreen.is_visible(),
        "Fullscreen must exit to reveal the standard properties dialog"
    );
    drop(fullscreen_image);
    drop(fullscreen);
    pump(&window);
    assert!(
        find(overlay.upcast_ref(), &|widget| widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(
                |label| label.text() == root.join("renamed.png").to_str().unwrap()
            ))
        .is_some()
    );
    find(overlay.upcast_ref(), &|widget| {
        widget.has_css_class("properties-close")
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap()
    .emit_clicked();
    button(popup(&image, &window).upcast_ref(), "Move to Trash").emit_clicked();
    pump(&window);
    let confirm = find(overlay.upcast_ref(), &|widget| {
        widget.has_css_class("delete-confirmation-delete")
    })
    .unwrap()
    .downcast::<gtk::Button>()
    .unwrap();
    view.browser().select(0, 1);
    confirm.emit_clicked();
    pump(&window);
    assert!(!root.join("renamed.png").exists() && other.exists());
    let menu = popup(&image, &window);
    let weak = menu.downgrade();
    drawer.close();
    assert!(menu.parent().is_none());
    drop(menu);
    pump(&window);
    released(&weak);
    manager.set_context_menu_options(crate::ui::context_menu::Options::default());
    view.browser().clear_observer();
    window.close();
    window.set_child(gtk::Widget::NONE);
    let weak_state = Rc::downgrade(&view.state);
    drop(view);
    assert!(
        weak_state.upgrade().is_none(),
        "Menu handlers must not retain the browser state"
    );
    std::fs::remove_dir_all(root).unwrap();
}
