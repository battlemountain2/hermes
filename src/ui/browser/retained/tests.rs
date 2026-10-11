use super::*;

#[test]
#[ignore = "Requires isolated GTK; retained tab widgets, selection, scroll and live changes"]
fn desktop_retained_tabs_preserve_presentations_and_release_closed_views() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let root = std::env::temp_dir().join(format!("hermes-retained-tabs-{}", std::process::id()));
    let first = root.join("First");
    let second = root.join("Second");
    for folder in [&first, &second] {
        std::fs::create_dir_all(folder).unwrap();
        for index in 0..1000 {
            std::fs::write(folder.join(format!("entry-{index:04}.txt")), b"fixture").unwrap();
        }
    }
    let view = BrowserView::new(
        Rc::new(crate::adapters::RoutedFileSource::new()),
        PeekBehavior::default(),
    );
    let browser = view.browser();
    let mut first_tab = Trail::new(
        TrailId::new("first").unwrap(),
        "First",
        Location::local(&first).breadcrumbs(),
    )
    .unwrap();
    let mut second_tab = Trail::new(
        TrailId::new("second").unwrap(),
        "Second",
        Location::local(&second).breadcrumbs(),
    )
    .unwrap();
    let window = gtk::Window::builder()
        .default_width(1000)
        .default_height(600)
        .child(&view.widget())
        .build();
    window.present();
    let pump = |milliseconds| {
        let until = Instant::now() + Duration::from_millis(milliseconds);
        while Instant::now() < until {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let loaded = || {
        let until = Instant::now() + Duration::from_secs(10);
        while browser
            .column_snapshot(0)
            .is_none_or(|column| column.loading)
            && Instant::now() < until
        {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!browser.column_snapshot(0).unwrap().loading);
    };
    for mode in [
        crate::model::TrailBrowserMode::Columns,
        crate::model::TrailBrowserMode::Grid,
        crate::model::TrailBrowserMode::Explorer,
    ] {
        first_tab.view.browser_mode = mode;
        second_tab.view.browser_mode = mode;
        view.activate_tab(&first_tab);
        view.set_view_mode(match mode {
            crate::model::TrailBrowserMode::Columns => BrowserMode::Columns,
            crate::model::TrailBrowserMode::Grid => BrowserMode::Grid,
            crate::model::TrailBrowserMode::Explorer => BrowserMode::Explorer,
        });
        loaded();
        browser.select(0, 400);
        pump(150);
        let model = view.state.columns.borrow()[0].model.clone();
        let shell = view.state.columns.borrow()[0].shell.clone();
        let mode_widget = view.state.mode_views.borrow().widget();
        view.show_filter();
        view.state.columns.borrow()[0]
            .filter_entry
            .set_text("entry-04");
        let mut before = Vec::new();
        capture_scroll(
            view.state.mode_views.borrow().widget().upcast_ref(),
            &mut before,
        );
        for (adjustment, _) in &before {
            if adjustment.upper() > adjustment.page_size() {
                adjustment.set_value(250.0);
            }
        }
        pump(100);
        before.clear();
        capture_scroll(
            view.state.mode_views.borrow().widget().upcast_ref(),
            &mut before,
        );
        view.activate_tab(&second_tab);
        loaded();
        let began = Instant::now();
        view.activate_tab(&first_tab);
        eprintln!(
            "retained {:?} 1k-entry tab switch: {:?}",
            mode,
            began.elapsed()
        );
        assert_eq!(view.state.columns.borrow()[0].model, model);
        assert_eq!(view.state.columns.borrow()[0].shell, shell);
        assert_eq!(view.state.mode_views.borrow().widget(), mode_widget);
        assert_eq!(
            view.state.columns.borrow()[0].filter_entry.text(),
            "entry-04"
        );
        assert_eq!(
            browser.focused_entry().unwrap().display_name,
            "entry-0400.txt"
        );
        pump(150);
        for (index, (adjustment, value)) in before.iter().enumerate() {
            assert!(
                (adjustment.value() - value).abs() < 2.0,
                "Scroll {index} changed: {} -> {}",
                value,
                adjustment.value()
            );
        }
        view.activate_tab(&second_tab);
        pump(30);
        let inactive = first.join("added.txt");
        std::fs::write(&inactive, b"inactive change").unwrap();
        pump(250);
        view.activate_tab(&first_tab);
        loaded();
        assert_eq!(browser.column_entries(0).unwrap().len(), 1001);
        assert_eq!(
            browser.focused_entry().unwrap().display_name,
            "entry-0400.txt"
        );
        std::fs::remove_file(inactive).unwrap();
        pump(250);
        let weak_second = view
            .state
            .retained_tabs
            .borrow()
            .iter()
            .find(|(id, _)| id == &second_tab.id)
            .unwrap()
            .1
            .columns[0]
            .model
            .downgrade();
        browser.retain_open_tabs(&[first_tab.id.clone()]);
        pump(150);
        assert!(weak_second.upgrade().is_none());
        assert!(view.state.retained_tabs.borrow().is_empty());
    }
    browser.clear_observer();
    window.close();
    drop((window, view, browser));
    pump(200);
    std::fs::remove_dir_all(root).unwrap();
}
