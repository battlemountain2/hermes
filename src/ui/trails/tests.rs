use super::visible_tab_scroll;

#[test]
fn file_tab_surfaces_keep_square_indicator_corners() {
    let css = include_str!("../../style.css");
    for selector in [
        ".file-tab {",
        ".file-tab-select {",
        ".file-tab-close {",
        ".file-tab-rename {",
        ".tab-strip-new {",
    ] {
        let rule = css
            .rsplit_once(&format!("\n{selector}"))
            .unwrap()
            .1
            .split_once('}')
            .unwrap()
            .0;
        assert!(
            rule.contains("border-radius: 0;"),
            "Rounded tab surface: {selector}"
        );
    }
}

#[test]
#[ignore = "Requires isolated GTK display; generated folder/tab reuse and live-change fixture"]
fn desktop_warm_tab_directory_reuse_and_live_changes() {
    use crate::{
        app::{BrowserEvent, Trails},
        model::{Location, TrailViewState},
        services::{StoredTrails, TrailStore},
        ui::browser::{BrowserView, PeekBehavior},
    };
    use gtk::prelude::*;
    use std::{cell::Cell, rc::Rc, time::{Duration, Instant}};

    struct MemoryStore;
    impl TrailStore for MemoryStore {
        fn load(&self) -> std::io::Result<StoredTrails> { Ok(StoredTrails::default()) }
        fn save(&self, _: &StoredTrails) -> std::io::Result<()> { Ok(()) }
    }

    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let root = std::env::temp_dir().join(format!("hermes-tab-directories-{}", std::process::id()));
    let first = root.join("First");
    let second = root.join("Second");
    for folder in [&first, &second] {
        std::fs::create_dir_all(folder).unwrap();
        for index in 0..1_000 {
            std::fs::write(folder.join(format!("entry-{index:04}.txt")), b"fixture").unwrap();
        }
    }
    let trails = Trails::empty(Rc::new(MemoryStore));
    let first_tab = trails.create("First", Location::local(&first), TrailViewState::default()).unwrap();
    trails.create("Second", Location::local(&second), TrailViewState::default()).unwrap();
    trails.activate(&first_tab).unwrap();
    let view = BrowserView::new(Rc::new(crate::adapters::RoutedFileSource::new()), PeekBehavior::default());
    match std::env::var("HERMES_TEST_TAB_MODE").as_deref() {
        Ok("grid") => view.set_view_mode(crate::ui::browser_modes::BrowserMode::Grid),
        Ok("explorer") => view.set_view_mode(crate::ui::browser_modes::BrowserMode::Explorer),
        _ => {}
    }
    let browser = view.browser();
    let activated_browser = browser.clone();
    let bar = super::TabBar::new(
        trails, browser.clone(), Rc::new(TrailViewState::default),
        Rc::new(move |trail| {
            activated_browser.activate_tab(trail.id.clone(), trail.active_location().unwrap().clone(), trail.view.preferences);
        }),
    );
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&bar.widget());
    content.append(&view.widget());
    let window = gtk::Window::builder().default_width(1000).default_height(700).child(&content).build();
    window.present();
    let began = Rc::new(Cell::new(Instant::now()));
    let first_batch = Rc::new(Cell::new(None));
    let finished = Rc::new(Cell::new(None));
    let observed_began = began.clone();
    let observed_first = first_batch.clone();
    let observed_finished = finished.clone();
    browser.observe(move |event| match event {
        BrowserEvent::EntriesInserted { depth: 0, .. } if observed_first.get().is_none() => {
            observed_first.set(Some(observed_began.get().elapsed()));
        }
        BrowserEvent::LoadFinished { depth: 0 } => observed_finished.set(Some(observed_began.get().elapsed())),
        BrowserEvent::TabRestored { .. } => observed_finished.set(Some(observed_began.get().elapsed())),
        BrowserEvent::LoadFailed { message, .. } => panic!("{message}"),
        _ => {}
    });
    let pump_until = |condition: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() && Instant::now() < deadline {
            gtk::glib::MainContext::default().iteration(false);
        }
        assert!(condition(), "Folder/tab fixture timed out");
    };
    let reset_timing = || {
        first_batch.set(None);
        finished.set(None);
        began.set(Instant::now());
    };
    for (label, offset) in [("cold-first", 0), ("cold-second", 1), ("warm-first", -1), ("warm-second", 1)] {
        reset_timing();
        if offset == 0 { browser.activate_tab(first_tab.clone(), Location::local(&first), crate::model::ViewPreferences::default()); } else { bar.cycle(offset); }
        pump_until(&|| finished.get().is_some());
        assert_eq!(browser.column_entries(0).unwrap().len(), 1_000);
        eprintln!("generated 1k-entry tab {label}: first GTK insertion={:?}, complete/restored model={:?}", first_batch.get(), finished.get().unwrap());
        if label.starts_with("warm") { assert!(first_batch.get().is_none(), "Unchanged tab must not reload entries"); }
    }
    let added = first.join("added.txt");
    std::fs::write(&added, b"changed while inactive").unwrap();
    let until_notifications = Instant::now() + Duration::from_millis(250);
    while Instant::now() < until_notifications {
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    reset_timing();
    bar.cycle(-1);
    pump_until(&|| browser.column_snapshot(0).is_some_and(|column| !column.loading && column.entries.len() == 1001));
    assert_eq!(browser.column_entries(0).unwrap().len(), 1_001);
    let renamed = first.join("renamed.txt");
    std::fs::rename(&added, &renamed).unwrap();
    pump_until(&|| browser.column_entries(0).unwrap().iter().any(|entry| entry.location == Location::local(&renamed)));
    assert!(!browser.column_entries(0).unwrap().iter().any(|entry| entry.location == Location::local(&added)));
    std::fs::remove_file(&renamed).unwrap();
    pump_until(&|| browser.column_entries(0).unwrap().len() == 1_000);
    view.browser().clear_observer();
    window.close();
    drop((bar, view, browser));
    while gtk::glib::MainContext::default().iteration(false) {}
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tab_drag_payloads_do_not_accept_file_or_bookmark_drags() {
    assert!(super::tab_drag_id("hermes-tab:\"tab-1\"").is_some());
    assert!(super::tab_drag_id("hermes-pin:file:///tmp").is_none());
    assert!(super::tab_drag_id("file:///tmp").is_none());
    assert!(super::tab_drag_id("hermes-tab:broken").is_none());
    assert!(super::tab_drag_id("hermes-tab:\"\"").is_none());
    assert!(super::tab_drag_id("hermes-tab:\"  \"").is_none());
}

#[test]
fn active_tabs_stay_visible_without_unnecessary_scrolling() {
    assert_eq!(visible_tab_scroll(0.0, 400.0, 800.0, 40.0, 180.0), 0.0);
    assert_eq!(visible_tab_scroll(0.0, 400.0, 800.0, 450.0, 180.0), 230.0);
    assert_eq!(visible_tab_scroll(300.0, 400.0, 800.0, 40.0, 180.0), 40.0);
    assert_eq!(visible_tab_scroll(0.0, 100.0, 800.0, 250.0, 180.0), 250.0);
    assert_eq!(visible_tab_scroll(30.0, 0.0, 800.0, 250.0, 180.0), 30.0);
    assert_eq!(visible_tab_scroll(0.0, 400.0, 300.0, 0.0, 500.0), 0.0);
}

#[test]
#[ignore = "Requires GTK display; checks tab reuse during switching and navigation"]
fn desktop_tab_refresh_reuses_unmodified_widgets() {
    use crate::{app::{Browser, Trails}, model::{Location, TrailViewState}, services::{StoredTrails, TrailStore}};
    use gtk::prelude::*;
    use std::rc::Rc;
    struct MemoryStore;
    impl TrailStore for MemoryStore {
        fn load(&self) -> std::io::Result<StoredTrails> { Ok(StoredTrails::default()) }
        fn save(&self, _: &StoredTrails) -> std::io::Result<()> { Ok(()) }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let trails = Trails::empty(Rc::new(MemoryStore));
    let first = trails.create("First", Location::local("/tmp/first"), TrailViewState::default()).unwrap();
    let second = trails.create("Second", Location::local("/tmp/second"), TrailViewState::default()).unwrap();
    let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
    let bar = super::TabBar::new(trails.clone(), browser, Rc::new(TrailViewState::default), Rc::new(|_| {}));
    let first_widget = bar.tabs.first_child().unwrap();
    let second_widget = first_widget.next_sibling().unwrap();
    let started = std::time::Instant::now();
    for _ in 0..100 {
        trails.activate(&first).unwrap();
        bar.refresh();
        trails.activate(&second).unwrap();
        bar.refresh();
    }
    println!("200 warm tab refreshes: {:?}", started.elapsed());
    assert_eq!(bar.tabs.first_child().unwrap(), first_widget);
    assert_eq!(first_widget.next_sibling().unwrap(), second_widget);
    assert!(second_widget.has_css_class("active"));
    assert!(!first_widget.has_css_class("active"));
    let select = first_widget.first_child().unwrap();
    let rename = select.next_sibling().unwrap();
    select.set_visible(false);
    rename.set_visible(true);
    bar.refresh();
    assert!(select.is_visible());
    assert!(!rename.is_visible());
    trails.rename(&second, "Renamed").unwrap();
    bar.refresh();
    assert_eq!(bar.tabs.first_child().unwrap(), first_widget);
    assert_ne!(first_widget.next_sibling().unwrap(), second_widget);
    trails.move_relative(&second, &first, true).unwrap();
    bar.refresh();
    assert_eq!(bar.tabs.first_child().unwrap().next_sibling().unwrap(), first_widget);
    trails.close(&second).unwrap();
    bar.refresh();
    assert_eq!(bar.tabs.first_child().unwrap(), first_widget);
    assert!(!first_widget.last_child().unwrap().is_sensitive());
}

#[test]
#[ignore = "Requires GTK desktop; exercises tab activation hooks with in-memory trails"]
fn desktop_new_switch_and_close_tabs_return_to_files_without_hiding_transport() {
    use crate::{
        app::{Browser, Trails},
        model::{Location, TrailViewState},
        services::{StoredTrails, TrailStore},
    };
    use gtk::prelude::*;
    use std::rc::Rc;
    struct MemoryStore;
    impl TrailStore for MemoryStore {
        fn load(&self) -> std::io::Result<StoredTrails> {
            Ok(StoredTrails::default())
        }
        fn save(&self, _: &StoredTrails) -> std::io::Result<()> {
            Ok(())
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let trails = Trails::empty(Rc::new(MemoryStore));
    trails
        .create("First", Location::local("/tmp"), TrailViewState::default())
        .unwrap();
    let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
    browser.navigate(Location::local("/tmp"));
    let workspace = gtk::Stack::new();
    workspace.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some("files"));
    workspace.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some("music"));
    let toggle = gtk::ToggleButton::new();
    let shown = workspace.clone();
    toggle.connect_toggled(move |button| {
        shown.set_visible_child_name(if button.is_active() { "music" } else { "files" })
    });
    let transport = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    transport.set_visible(true);
    let activated = toggle.clone();
    let bar = super::TabBar::new(
        trails,
        browser,
        Rc::new(TrailViewState::default),
        Rc::new(move |_| activated.set_active(false)),
    );
    toggle.set_active(true);
    bar.new_tab();
    assert_eq!(workspace.visible_child_name().as_deref(), Some("files"));
    assert!(transport.is_visible());
    toggle.set_active(true);
    bar.cycle(-1);
    assert!(!toggle.is_active());
    toggle.set_active(true);
    bar.close_active();
    assert!(!toggle.is_active());
    assert!(transport.is_visible());
}

#[test]
#[ignore = "Requires a display; run explicitly to check themed tab layout"]
fn desktop_tabs_remain_compact_and_aligned() {
    use crate::{
        app::{Browser, Trails},
        model::{Location, TrailViewState},
        services::{StoredTrails, TrailStore},
    };
    use gtk::prelude::*;
    use std::{
        cell::RefCell,
        rc::Rc,
        time::{Duration, Instant},
    };

    struct MemoryStore;
    impl TrailStore for MemoryStore {
        fn load(&self) -> std::io::Result<StoredTrails> {
            Ok(StoredTrails::default())
        }
        fn save(&self, _: &StoredTrails) -> std::io::Result<()> {
            Ok(())
        }
    }
    gtk::init().expect("A desktop display is required");
    crate::assets::prepare().expect("Icons must load");
    let tokens: crate::ui::theme::ThemeTokens =
        toml::from_str(include_str!("../../../data/themes/azure-glow.toml"))
            .expect("Theme fixture must parse");
    let mut css = String::new();
    for (name, value) in [
        ("bg", tokens.background),
        ("surface", tokens.surface),
        ("text", tokens.text),
        ("accent", tokens.accent),
        ("danger", tokens.danger),
        ("muted", tokens.muted),
        ("highlight", tokens.highlight),
        ("border", tokens.border),
        ("dim_text", tokens.dim_text),
    ] {
        css.push_str(&format!("@define-color theme_{name} {value};\n"));
    }
    css.push_str(include_str!("../../style.css"));
    let provider = gtk::CssProvider::new();
    let errors = Rc::new(RefCell::new(Vec::new()));
    let captured = errors.clone();
    provider
        .connect_parsing_error(move |_, _, error| captured.borrow_mut().push(error.to_string()));
    provider.load_from_string(&css);
    assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
    let display = gtk::gdk::Display::default().expect("Display must exist");
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let trails = Trails::empty(Rc::new(MemoryStore));
    for name in ["Videos", "Downloads", "An exceptionally long folder title"] {
        trails
            .create(name, Location::local("/tmp"), TrailViewState::default())
            .expect("Fixture tabs must be created");
    }
    let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
    let bar = super::TabBar::new(
        trails,
        browser,
        Rc::new(TrailViewState::default),
        Rc::new(|_| {}),
    );
    let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layout.append(&bar.widget());
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_vexpand(true);
    layout.append(&content);
    let window = gtk::Window::builder()
        .default_width(1200)
        .default_height(100)
        .child(&layout)
        .build();
    window.present();
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut tab = bar.tabs.first_child().expect("First tab exists");
    let first_height = tab.height();
    assert!(
        (40..=42).contains(&first_height),
        "Tab row unexpectedly grew to {first_height}px"
    );
    let first_bounds = tab.compute_bounds(&bar.root).unwrap();
    assert!(
        first_bounds.x().abs() <= 1.0,
        "The first tab is inset from the strip"
    );
    assert!(
        first_bounds.y().abs() <= 1.0,
        "The indicator is inset from the top edge"
    );
    assert!(
        (first_bounds.height() - bar.root.height() as f32).abs() <= 1.0,
        "The tab does not fill the strip height"
    );
    let mut previous_right: Option<f32> = None;
    for _ in 0..3 {
        assert!(
            tab.width() >= 120 && tab.width() <= 300,
            "Tab stretched to {}px",
            tab.width()
        );
        assert_eq!(tab.height(), first_height);
        let bounds = tab.compute_bounds(&bar.tabs).unwrap();
        if let Some(previous_right) = previous_right {
            assert!(
                (bounds.x() - previous_right).abs() <= 1.0,
                "Gap between adjacent tab surfaces"
            );
        }
        previous_right = Some(bounds.x() + bounds.width());
        let mut child = tab.first_child();
        while let Some(widget) = child {
            if widget.is_visible() {
                let bounds = widget.compute_bounds(&tab).expect("Child bounds exist");
                let center = bounds.y() + bounds.height() / 2.0;
                assert!((center - first_height as f32 / 2.0).abs() <= 1.0);
            }
            child = widget.next_sibling();
        }
        tab = tab.next_sibling().expect("Next tab or add button exists");
    }
    assert!(
        bar.tabs.width() < bar.root.width(),
        "Compact tabs must leave trailing space"
    );
    window.set_child(None::<&gtk::Widget>);
    window.close();
    let narrow_window = gtk::Window::builder()
        .default_width(260)
        .default_height(180)
        .child(&layout)
        .build();
    narrow_window.present();
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(bar.root.hadjustment().upper() > bar.root.hadjustment().page_size());
    assert_eq!(bar.tabs.first_child().unwrap().height(), first_height);
    for _ in 0..2 {
        let last = bar.trails.all().last().unwrap().id.clone();
        bar.close_tab(&last);
    }
    narrow_window.set_child(None::<&gtk::Widget>);
    narrow_window.close();
    let single_window = gtk::Window::builder()
        .default_width(260)
        .default_height(180)
        .child(&layout)
        .build();
    single_window.present();
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(10));
    }
    let remaining = bar.tabs.first_child().unwrap();
    let bounds = remaining.compute_bounds(&bar.root).unwrap();
    assert!(bounds.x().abs() <= 1.0 && bounds.y().abs() <= 1.0);
    assert_eq!(remaining.height(), first_height);
    assert!(
        remaining.width() < bar.root.width(),
        "A lone tab must stay compact"
    );
    assert!(!remaining.last_child().unwrap().is_sensitive());
    single_window.close();
    gtk::style_context_remove_provider_for_display(&display, &provider);
}
