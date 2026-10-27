use super::*;

pub(in crate::ui) fn load_probe(
    views: &ModeViews,
) -> (gtk::Stack, gtk::Spinner, gtk::StringList, gtk::Label) {
    let pane = match views.mode {
        BrowserMode::Grid => views.grid_panes.last().unwrap(),
        BrowserMode::Explorer => views.explorer_pane.as_ref().unwrap(),
        BrowserMode::Columns => panic!("Columns use their own load presentation"),
    };
    (
        pane.stack.clone(),
        pane.spinner.clone(),
        pane.model.clone(),
        pane.status.clone(),
    )
}

#[test]
fn explorer_initial_layout_keeps_all_details_in_a_compact_name_column() {
    assert_eq!(EXPLORER_COLUMN_WIDTHS[0], 320);
    assert_eq!(explorer_preview_reservation(), 684);
}

#[test]
#[ignore = "Requires an isolated GTK display; exercises explorer pane header layout"]
fn explorer_pane_header_remains_outside_horizontal_scroll_to_keep_actions_visible() {
    let _ = gtk::init();
    let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
    let pane = build_explorer_pane(
        browser,
        Rc::new(Cell::new(false)),
        Rc::new(RefCell::new(None)),
        Rc::new(RefCell::new(Vec::new())),
        Rc::new(RefCell::new(None)),
        0,
        "Very Long Folder Name That Might Otherwise Overflow",
    );
    let header = pane.shell.first_child().unwrap().downcast::<gtk::Box>().unwrap();
    assert!(header.has_css_class("mode-pane-header"));

    let heading = header
        .first_child()
        .unwrap()
        .next_sibling()
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    assert_eq!(heading.ellipsize(), gtk::pango::EllipsizeMode::End);

    let actions = header.last_child().unwrap().downcast::<gtk::Box>().unwrap();
    assert!(actions.has_css_class("grid-header-actions"));

    let scroller = pane.shell.last_child().unwrap().downcast::<gtk::ScrolledWindow>().unwrap();
    assert_eq!(scroller.hscrollbar_policy(), gtk::PolicyType::Automatic);
    let viewport = scroller.child().unwrap().downcast::<gtk::Viewport>().unwrap();
    assert_eq!(viewport.child().unwrap(), pane.stack.upcast::<gtk::Widget>());
}

#[test]
#[ignore = "Requires an isolated GTK display; real grid slider and Explorer layout fixture"]
fn desktop_thumbnail_slider_updates_mounted_cards_and_explorer_details_fit() {
    use std::time::{Duration, Instant};
    fn pump_until(condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(6);
        while !condition() && Instant::now() < deadline {
            gtk::glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(condition(), "GTK thumbnail/layout condition timed out");
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let _theme = crate::ui::theme::ThemeManager::shared();
    let styles = gtk::CssProvider::new();
    styles.load_from_string(include_str!("../../style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let root = std::env::temp_dir().join(format!("hermes-grid-resize-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("game.desktop"),
        "[Desktop Entry]\nType=Application\nName=Generated game\nIcon=folder\nExec=/bin/false\n",
    )
    .unwrap();
    let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
    let views = Rc::new(RefCell::new(ModeViews::new(
        &gtk::ScrolledWindow::new(),
        browser.clone(),
    )));
    views.borrow_mut().set_mode(BrowserMode::Grid);
    let weak_views = Rc::downgrade(&views);
    browser.observe(move |event| {
        if let Some(views) = weak_views.upgrade() {
            views.borrow_mut().handle(&event);
        }
    });
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(500)
        .child(&views.borrow().widget())
        .build();
    window.present();
    browser.navigate(Location::local(&root));
    pump_until(|| {
        views
            .borrow()
            .grid_panes
            .first()
            .is_some_and(|pane| pane.model.n_items() == 1 && !pane.bound_items.borrow().is_empty())
    });
    let pane = views.borrow().grid_panes[0].clone();
    let overlay = pane.bound_items.borrow()[0]
        .widget
        .upgrade()
        .unwrap()
        .downcast::<gtk::Overlay>()
        .unwrap();
    let card = overlay.child().unwrap();
    let center = card
        .first_child()
        .unwrap()
        .downcast::<gtk::CenterBox>()
        .unwrap();
    let icon = center
        .center_widget()
        .unwrap()
        .first_child()
        .unwrap()
        .downcast::<gtk::Image>()
        .unwrap();
    pump_until(|| icon.pixel_size() == 64);
    let menu = pane
        .shell
        .first_child()
        .unwrap()
        .last_child()
        .unwrap()
        .first_child()
        .unwrap()
        .downcast::<gtk::MenuButton>()
        .unwrap();
    let content = menu.popover().unwrap().child().unwrap();
    let scale = content
        .last_child()
        .unwrap()
        .prev_sibling()
        .unwrap()
        .downcast::<gtk::Scale>()
        .unwrap();
    scale.set_value(7.0);
    pump_until(|| icon.pixel_size() == 256);
    assert_eq!(views.borrow().grid_thumbnail_size.get(), 256);
    scale.set_value(0.0);
    pump_until(|| icon.pixel_size() == 48);
    assert_eq!(views.borrow().grid_thumbnail_size.get(), 48);
    views.borrow_mut().set_mode(BrowserMode::Explorer);
    let explorer = views.borrow().explorer_pane.as_ref().unwrap().clone();
    pump_until(|| explorer.view.width() > 0);
    let content = explorer.stack.child_by_name("content").unwrap();
    let headings = content.first_child().unwrap().next_sibling().unwrap();
    let bounds = headings
        .last_child()
        .unwrap()
        .compute_bounds(&headings)
        .unwrap();
    assert!(
        bounds.x() + bounds.width() <= explorer_preview_reservation() as f32,
        "Details edge {} exceeds reservation",
        bounds.x() + bounds.width()
    );
    assert!(explorer.view.width() >= explorer_preview_reservation());
    window.close();
    browser.clear_observer();
    std::fs::remove_dir_all(root).unwrap();
}

pub(in crate::ui) type LifecycleProbes = Vec<(String, glib::WeakRef<glib::Object>)>;

pub(in crate::ui) fn append_widget_probes(
    widget: &gtk::Widget,
    presentation: &str,
    probes: &mut LifecycleProbes,
) {
    probes.push((
        format!("{presentation} {}", widget.type_().name()),
        widget.upcast_ref::<glib::Object>().downgrade(),
    ));
    let mut child = widget.first_child();
    while let Some(widget) = child {
        append_widget_probes(&widget, presentation, probes);
        child = widget.next_sibling();
    }
}

pub(in crate::ui) fn append_pane_probes(views: &ModeViews, probes: &mut LifecycleProbes) {
    for (presentation, pane) in views
        .grid_panes
        .iter()
        .map(|pane| ("grid", pane))
        .chain(views.explorer_pane.iter().map(|pane| ("explorer", pane)))
    {
        append_widget_probes(pane.shell.upcast_ref(), presentation, probes);
        for object in [
            pane.model.upcast_ref::<glib::Object>(),
            pane.selection.upcast_ref::<glib::Object>(),
        ] {
            probes.push((
                format!("{presentation} {}", object.type_().name()),
                object.downgrade(),
            ));
        }
        if let Some(model) = &pane.filtered_model {
            probes.push((
                format!("{presentation} filtered model"),
                model.upcast_ref::<glib::Object>().downgrade(),
            ));
        }
        for bound in pane.bound_items.borrow().iter() {
            if let Some(item) = bound.item.upgrade() {
                probes.push((
                    format!("{presentation} item"),
                    item.upcast_ref::<glib::Object>().downgrade(),
                ));
            }
        }
    }
}
