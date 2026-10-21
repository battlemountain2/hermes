// SPDX-License-Identifier: GPL-3.0-or-later

use std::cmp::Ordering;

use crate::services::table::{SheetData, SpreadsheetData};

use super::*;

#[test]
fn test_sort_key_comparisons() {
    let n1 = SortKey::new("10");
    let n2 = SortKey::new("2");
    assert_eq!(n2.compare(&n1), Ordering::Less);

    let t1 = SortKey::new("apple");
    let t2 = SortKey::new("banana");
    assert_eq!(t1.compare(&t2), Ordering::Less);

    let empty = SortKey::new("");
    assert_eq!(n1.compare(&empty), Ordering::Less);
    assert_eq!(empty.compare(&n1), Ordering::Greater);
}

#[test]
fn test_column_name_generation() {
    assert_eq!(column_name(0), "A");
    assert_eq!(column_name(25), "Z");
    assert_eq!(column_name(26), "AA");
}

#[test]
#[ignore = "Requires an isolated GTK display"]
fn desktop_spreadsheet_repeated_fullscreen_restores_and_releases_single_and_multi_sheet_views() {
    gtk::init().expect("GTK init");
    crate::assets::prepare().expect("Assets prepare");

    let data = SpreadsheetData {
        sheet_names: vec!["Q1".to_string(), "Q2".to_string()],
        active_sheet: 0,
        headers: vec!["Item".to_string(), "Cost".to_string()],
        rows: vec![vec!["Coffee".to_string(), "4.50".to_string()]],
        total_rows: 2,
        total_cols: 2,
        truncated: false,
        sheets: vec![
            SheetData {
                name: "Q1".to_string(),
                headers: vec!["Item".to_string(), "Cost".to_string()],
                rows: vec![vec!["Coffee".to_string(), "4.50".to_string()]],
                total_rows: 2,
                total_cols: 2,
                truncated: false,
            },
            SheetData {
                name: "Q2".to_string(),
                headers: vec!["Item".to_string(), "Cost".to_string(), "Profit".to_string()],
                rows: vec![vec![
                    "Tea".to_string(),
                    "3.00".to_string(),
                    "1.50".to_string(),
                ]],
                total_rows: 2,
                total_cols: 3,
                truncated: false,
            },
        ],
    };

    let mut single = data.clone();
    single.sheets.truncate(1);
    single.sheet_names.truncate(1);
    let mut legacy = single.clone();
    legacy.sheets.clear();
    for fixture in [&legacy, &single, &data] {
        exercise_fullscreen_lifecycle(fixture);
    }
}

fn observe_table_objects(widget: &gtk::Widget, objects: &mut Vec<glib::WeakRef<glib::Object>>) {
    objects.push(widget.clone().upcast::<glib::Object>().downgrade());
    if let Some(table) = widget.downcast_ref::<gtk::ColumnView>() {
        let selection = table
            .model()
            .expect("selection")
            .downcast::<gtk::NoSelection>()
            .expect("no selection");
        objects.push(selection.clone().upcast::<glib::Object>().downgrade());
        let sorted = selection
            .model()
            .expect("sort model")
            .downcast::<gtk::SortListModel>()
            .expect("sorted rows");
        objects.push(sorted.clone().upcast::<glib::Object>().downgrade());
        let store = sorted.model().expect("row store");
        objects.push(store.clone().upcast::<glib::Object>().downgrade());
        objects.push(store.item(0).expect("first row").downgrade());
        for index in 0..table.columns().n_items() {
            let column = table
                .columns()
                .item(index)
                .expect("column")
                .downcast::<gtk::ColumnViewColumn>()
                .expect("table column");
            objects.push(column.clone().upcast::<glib::Object>().downgrade());
            objects.push(
                column
                    .factory()
                    .expect("factory")
                    .upcast::<glib::Object>()
                    .downgrade(),
            );
            objects.push(
                column
                    .sorter()
                    .expect("sorter")
                    .upcast::<glib::Object>()
                    .downgrade(),
            );
        }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        observe_table_objects(&current, objects);
    }
}

fn exercise_fullscreen_lifecycle(data: &SpreadsheetData) {
    let view = build_spreadsheet_view(data, "Financials.xlsx");
    let weak_slot = Rc::downgrade(&view.window);
    let weak_home = view.widget.downgrade();
    let weak_viewer = view.widget.first_child().expect("viewer").downgrade();
    let mut objects = Vec::new();
    observe_table_objects(view.widget.upcast_ref(), &mut objects);

    if data.sheets.len() > 1 {
        let viewer = weak_viewer.upgrade().expect("viewer");
        let controls = viewer.first_child().expect("header");
        let bar = controls.next_sibling().expect("sheet buttons");
        let stack = bar
            .next_sibling()
            .expect("stack")
            .downcast::<gtk::Stack>()
            .expect("sheet stack");
        let first = bar
            .first_child()
            .expect("first tab")
            .downcast::<gtk::Button>()
            .expect("tab button");
        let second = first
            .next_sibling()
            .expect("second tab")
            .downcast::<gtk::Button>()
            .expect("tab button");
        second.emit_clicked();
        assert_eq!(stack.visible_child_name().as_deref(), Some("sheet_1"));
        assert!(!first.has_css_class("active-sheet-tab"));
        assert!(second.has_css_class("active-sheet-tab"));
        let dimensions = controls
            .first_child()
            .expect("title")
            .next_sibling()
            .expect("dimensions")
            .downcast::<gtk::Label>()
            .expect("dimension label");
        assert_eq!(dimensions.text(), "2 rows · 3 cols · Sheet 2 of 2");
        first.emit_clicked();
        assert_eq!(stack.visible_child_name().as_deref(), Some("sheet_0"));
        assert!(first.has_css_class("active-sheet-tab"));
        assert!(!second.has_css_class("active-sheet-tab"));
    }

    let mut windows = Vec::new();
    for iteration in 0..6 {
        assert!(view.open_fullscreen());
        let weak_window = view
            .window
            .borrow()
            .as_ref()
            .expect("fullscreen window")
            .downgrade();
        assert!(view.widget.first_child().is_none());
        assert_eq!(
            weak_viewer.upgrade().expect("live viewer").parent(),
            weak_window
                .upgrade()
                .map(|window| window.upcast::<gtk::Widget>())
        );
        assert_eq!(
            view.expand.tooltip_text().as_deref(),
            Some("Exit fullscreen (F or Escape)")
        );
        assert!(view.open_fullscreen());
        assert_eq!(
            view.window.borrow().as_ref(),
            weak_window.upgrade().as_ref()
        );

        match iteration {
            0 => view.expand.emit_clicked(),
            1 => weak_window.upgrade().expect("window").close(),
            2 => view.stop(),
            _ => {
                let window = weak_window.upgrade().expect("window");
                let controllers = window.observe_controllers();
                let keys = (0..controllers.n_items())
                    .filter_map(|index| {
                        controllers
                            .item(index)?
                            .downcast::<gtk::EventControllerKey>()
                            .ok()
                    })
                    .find(|controller| {
                        controller.name().as_deref() == Some("hermes-table-fullscreen")
                    })
                    .expect("fullscreen keys");
                let key = match iteration {
                    3 => gtk::gdk::Key::Escape,
                    4 => gtk::gdk::Key::f,
                    _ => gtk::gdk::Key::F,
                };
                assert!(keys.emit_by_name::<bool>(
                    "key-pressed",
                    &[&key, &0_u32, &gtk::gdk::ModifierType::empty()]
                ));
            }
        }
        while glib::MainContext::default().iteration(false) {}
        assert!(view.window.borrow().is_none());
        assert_eq!(view.widget.first_child(), weak_viewer.upgrade());
        assert_eq!(
            weak_viewer
                .upgrade()
                .expect("restored viewer")
                .parent()
                .as_ref(),
            Some(view.widget.upcast_ref())
        );
        assert_eq!(
            view.expand.tooltip_text().as_deref(),
            Some("Fullscreen spreadsheet (F)")
        );
        view.stop();
        assert_eq!(view.widget.first_child(), weak_viewer.upgrade());
        windows.push(weak_window);
    }

    assert!(view.open_fullscreen());
    windows.push(
        view.window
            .borrow()
            .as_ref()
            .expect("final window")
            .downgrade(),
    );
    drop(view);
    while glib::MainContext::default().iteration(false) {}
    assert!(weak_home.upgrade().is_none());
    assert!(weak_viewer.upgrade().is_none());
    assert!(weak_slot.upgrade().is_none());
    assert!(windows.iter().all(|window| window.upgrade().is_none()));
    assert!(objects.iter().all(|object| object.upgrade().is_none()));
}
