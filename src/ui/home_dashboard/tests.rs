// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn test_same_day_matching() {
    let now = glib::DateTime::now_local().expect("local time");
    assert!(same_day(&now, &now));

    if let Ok(yesterday) = now.add_days(-1) {
        assert!(!same_day(&now, &yesterday));
    }
}

#[test]
fn test_date_bucket_categorization() {
    let now = glib::DateTime::now_local().expect("local time");
    let now_unix = now.to_unix();

    // Now -> Today
    assert_eq!(
        DateBucket::from_timestamp(now_unix, &now),
        DateBucket::Today
    );

    // 2 hours ago -> Today
    assert_eq!(
        DateBucket::from_timestamp(now_unix - 7200, &now),
        DateBucket::Today
    );

    // Yesterday
    if let Ok(yesterday) = now.add_days(-1) {
        let yesterday_unix = yesterday.to_unix();
        assert_eq!(
            DateBucket::from_timestamp(yesterday_unix, &now),
            DateBucket::Yesterday
        );
    }

    // 4 days ago -> Earlier this week
    if let Ok(four_days_ago) = now.add_days(-4) {
        let four_days_unix = four_days_ago.to_unix();
        assert_eq!(
            DateBucket::from_timestamp(four_days_unix, &now),
            DateBucket::EarlierThisWeek
        );
    }

    // 30 days ago -> Older
    if let Ok(thirty_days_ago) = now.add_days(-30) {
        let thirty_days_unix = thirty_days_ago.to_unix();
        assert_eq!(
            DateBucket::from_timestamp(thirty_days_unix, &now),
            DateBucket::Older
        );
    }
}

#[test]
fn quick_access_has_unique_folders_without_a_duplicate_personal_folder_card() {
    let folders = load_quick_access_folders();
    let mut locations = std::collections::HashSet::new();
    let personal = Location::local(super::super::window::home_directory());
    for folder in folders {
        assert_ne!(folder.location, personal);
        assert!(locations.insert(folder.location));
        if folder.is_pinned {
            assert_eq!(folder.icon, icons::FOLDER);
        }
    }
}

#[test]
#[ignore = "Private GTK display and generated XDG folders; renders the Home shortcut layout"]
fn private_home_shortcuts_have_two_balanced_rows_and_a_personal_folder_action() {
    use gtk::prelude::*;
    use std::time::{Duration, Instant};
    gtk::init().expect("initialize the isolated GTK display");
    crate::assets::prepare().expect("register embedded test assets");
    crate::ui::window::load_styles();
    let theme = crate::ui::theme::ThemeManager::shared();
    theme.select_theme("graphite");
    let browser = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        crate::ui::browser::PeekBehavior::default(),
    );
    let dashboard = HomeDashboard::new(browser.clone(), None);
    let window = gtk::Window::builder()
        .default_width(1050)
        .default_height(800)
        .child(dashboard.widget())
        .build();
    window.present();
    let context = glib::MainContext::default();
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(250) {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut rows = std::collections::BTreeMap::new();
    let mut child = dashboard.state.quick_access_flow.first_child();
    while let Some(card) = child {
        child = card.next_sibling();
        let position = card
            .compute_point(
                &dashboard.state.quick_access_flow,
                &gtk::graphene::Point::zero(),
            )
            .expect("expected test value from card .compute_point( &dashboard.state.quick_access_flow, &gtk::graphene::Point::zero(), )");
        *rows.entry(position.y().round() as i32).or_insert(0) += 1;
        assert!(card.width() > 0);
    }
    assert_eq!(
        rows.values().copied().collect::<Vec<_>>(),
        vec![5, 5],
        "Generated six standard and four pinned folders should balance"
    );
    if let Some(path) = std::env::var_os("HERMES_HOME_SCREENSHOT") {
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
        let node = snapshot
            .to_node()
            .expect("expected test value from snapshot.to_node()");
        let renderer = gtk::gsk::CairoRenderer::new();
        renderer
            .realize(None::<&gtk::gdk::Surface>)
            .expect("expected test value from renderer.realize(None::<&gtk::gdk::Surface>)");
        renderer
            .render_texture(&node, None)
            .save_to_png(path)
            .expect(
                "expected test value from renderer .render_texture(&node, None) .save_to_png(path)",
            );
        renderer.unrealize();
    }
    browser.browser().clear_observer();
    window.destroy();
}
