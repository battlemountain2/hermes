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
#[ignore = "Private GTK display; generated wide/long sheets and real scroll adjustments"]
fn desktop_spreadsheet_scrollbars_reach_both_axes_and_lazy_sheets_keep_position() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let sheet = SheetData {
        name: "Wide sheet".into(),
        headers: (0..40).map(|column| format!("Column {column}")).collect(),
        rows: (0..200).map(|row| (0..40).map(|column| format!("Row {row} column {column}")).collect()).collect(),
        total_rows: 201,
        total_cols: 40,
        truncated: false,
    };
    let data = SpreadsheetData {
        sheets: vec![sheet.clone(), SheetData { name: "Other sheet".into(), ..sheet }],
        ..SpreadsheetData::default()
    };
    let view = build_spreadsheet_view(&data, "Scroll controls.xlsx");
    let window = gtk::Window::builder().default_width(420).default_height(360).child(&view.widget).build();
    window.present();
    let root = view.widget.upcast_ref::<gtk::Widget>();
    let scroll = find_spreadsheet_widget(root, "spreadsheet-grid-scroll").downcast::<gtk::ScrolledWindow>().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        glib::MainContext::default().iteration(false);
        if scroll.is_mapped() && scroll.hadjustment().upper() > scroll.hadjustment().page_size()
            && scroll.vadjustment().upper() > scroll.vadjustment().page_size() {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "Both axes must overflow the viewport");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(scroll.hscrollbar().is_mapped());
    assert!(scroll.vscrollbar().is_mapped());
    assert!(!scroll.is_overlay_scrolling());
    for adjustment in [scroll.hadjustment(), scroll.vadjustment()] {
        let end = adjustment.upper() - adjustment.page_size();
        adjustment.set_value(end);
        assert!((adjustment.value() - end).abs() < 1.0);
    }
    let position = (scroll.hadjustment().value(), scroll.vadjustment().value());
    let bar = find_spreadsheet_widget(root, "spreadsheet-sheet-bar");
    bar.last_child().unwrap().downcast::<gtk::Button>().unwrap().emit_clicked();
    bar.first_child().unwrap().downcast::<gtk::Button>().unwrap().emit_clicked();
    assert_eq!(position, (scroll.hadjustment().value(), scroll.vadjustment().value()));
    window.close();
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
        let sheet_scroll = controls.next_sibling().expect("sheet scroller");
        let bar = find_spreadsheet_widget(&sheet_scroll, "spreadsheet-sheet-bar");
        let stack = sheet_scroll
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
            .expect("metadata")
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

fn find_spreadsheet_widget(root: &gtk::Widget, class: &str) -> gtk::Widget {
    fn find(root: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
        if root.has_css_class(class) {
            return Some(root.clone());
        }
        let mut child = root.first_child();
        while let Some(current) = child {
            if let Some(found) = find(&current, class) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    find(root, class).unwrap_or_else(|| panic!("Missing spreadsheet widget: {class}"))
}

#[test]
fn spreadsheet_styles_stay_scoped_semantic_square_and_font_inheriting() {
    let css = include_str!("../../style.css");
    let section = css
        .split_once("/* Spreadsheet viewer & multi-sheet tabs */")
        .expect("spreadsheet section")
        .1
        .split_once("/* -------------------------------------------------------------")
        .expect("next section")
        .0;
    for rule in section.split('}').filter(|rule| !rule.trim().is_empty()) {
        let (selectors, declarations) = rule.split_once('{').expect("CSS rule");
        for selector in selectors.split(',') {
            assert!(selector.trim().starts_with(".spreadsheet-"), "{selector}");
        }
        for declaration in declarations.split(';').filter(|value| value.contains(':')) {
            let (property, value) = declaration.split_once(':').expect("declaration");
            match property.trim() {
                "font-family" => assert_eq!(value.trim(), "inherit"),
                "border-radius" => assert_eq!(value.trim(), "0"),
                "transition" | "box-shadow" => assert_eq!(value.trim(), "none"),
                "color"
                | "background"
                | "background-image"
                | "border"
                | "border-right"
                | "border-left"
                | "border-top"
                | "border-bottom"
                | "border-color"
                | "border-bottom-color"
                | "outline"
                | "outline-color" => {
                    assert!(
                        value.contains("@theme_") || matches!(value.trim(), "0" | "none"),
                        "Non-semantic spreadsheet declaration: {declaration}"
                    );
                }
                _ => {}
            }
        }
    }
    assert!(!section.contains("dimension-pill"));
    assert!(section.contains(".spreadsheet-sheet-button.active-sheet-tab:hover"));
    assert!(section.contains(".spreadsheet-grid row:selected"));
    assert!(section.contains(".spreadsheet-grid cell:focus-visible"));
    assert!(section.contains(".spreadsheet-grid.column-separators > listview > row > cell"));
}

#[test]
#[ignore = "Requires isolated GTK display; synthetic sheets, live CSS/font changes and narrow geometry only"]
fn desktop_spreadsheet_compact_layout_live_styles_and_sorting() {
    fn mapped_labels(root: &gtk::Widget) -> Vec<gtk::Label> {
        let mut labels = Vec::new();
        if !root.is_mapped() {
            return labels;
        }
        if let Some(label) = root.downcast_ref::<gtk::Label>() {
            labels.push(label.clone());
        }
        let mut child = root.first_child();
        while let Some(current) = child {
            labels.extend(mapped_labels(&current));
            child = current.next_sibling();
        }
        labels
    }

    fn wait_for_live_font(root: &gtk::Widget, font: &str) {
        let ticks = Rc::new(std::cell::Cell::new(0_u32));
        let observed_ticks = ticks.clone();
        let callback = root.add_tick_callback(move |_, _| {
            observed_ticks.set(observed_ticks.get() + 1);
            glib::ControlFlow::Continue
        });
        root.queue_draw();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let ready = loop {
            glib::MainContext::default().iteration(false);
            let labels = mapped_labels(root);
            let fonts_match = labels
                .iter()
                .any(|label| label.has_css_class("spreadsheet-cell"))
                && labels.iter().all(|label| {
                    [label.pango_context(), label.layout().context()]
                        .iter()
                        .all(|context| {
                            context
                                .font_description()
                                .expect("label font")
                                .family()
                                .as_deref()
                                == Some(font)
                        })
                });
            if ticks.get() >= 2 && fonts_match {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        callback.remove();
        assert!(
            ready,
            "Spreadsheet font did not reach {font} after {} GTK ticks: {:?}",
            ticks.get(),
            mapped_labels(root)
                .iter()
                .map(|label| (
                    label.css_classes(),
                    label.pango_context().font_description(),
                    label.layout().context().font_description(),
                ))
                .collect::<Vec<_>>()
        );
    }

    gtk::init().expect("Isolated GTK display");
    crate::assets::prepare().expect("Assets prepare");
    let display = gtk::gdk::Display::default().expect("display");
    let styles = gtk::CssProvider::new();
    let errors = Rc::new(RefCell::new(Vec::new()));
    let captured = errors.clone();
    styles.connect_parsing_error(move |_, _, error| captured.borrow_mut().push(error.to_string()));
    styles.load_from_string(include_str!("../../style.css"));
    gtk::style_context_add_provider_for_display(
        &display,
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let live_styles = gtk::CssProvider::new();
    gtk::style_context_add_provider_for_display(
        &display,
        &live_styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    let data = SpreadsheetData {
        sheets: (0..16)
            .map(|index| SheetData {
                name: format!("Sheet {index} {}", "Long worksheet name ".repeat(20)),
                headers: vec!["Item".into(), "Cost".into()],
                rows: vec![
                    vec!["Coffee".into(), "10".into()],
                    vec!["Tea".into(), "2".into()],
                ],
                total_rows: 1_000_000 + index,
                total_cols: 256,
                truncated: true,
            })
            .collect(),
        truncated: true,
        ..SpreadsheetData::default()
    };
    let title = format!("{}.xlsx", "Long financial workbook name ".repeat(20));
    let view = build_spreadsheet_view(&data, &title);
    let root = view.widget.upcast_ref::<gtk::Widget>();
    let title_label = find_spreadsheet_widget(root, "spreadsheet-title")
        .downcast::<gtk::Label>()
        .expect("title");
    let dimensions = find_spreadsheet_widget(root, "spreadsheet-dimensions")
        .downcast::<gtk::Label>()
        .expect("dimensions");
    let sheet_scroll = find_spreadsheet_widget(root, "spreadsheet-sheet-scroll")
        .downcast::<gtk::ScrolledWindow>()
        .expect("sheet scroll");
    let bar = find_spreadsheet_widget(root, "spreadsheet-sheet-bar");
    let stack = sheet_scroll
        .next_sibling()
        .expect("stack")
        .downcast::<gtk::Stack>()
        .expect("stack");
    let table = find_spreadsheet_widget(root, "spreadsheet-grid")
        .downcast::<gtk::ColumnView>()
        .expect("grid");
    assert_eq!(stack.pages().n_items(), 1);
    assert!(!stack.is_hhomogeneous());
    assert!(!stack.is_vhomogeneous());
    let grid_scroll = find_spreadsheet_widget(root, "spreadsheet-grid-scroll")
        .downcast::<gtk::ScrolledWindow>()
        .expect("grid scroller");
    assert_eq!(grid_scroll.hscrollbar_policy(), gtk::PolicyType::Always);
    assert_eq!(grid_scroll.vscrollbar_policy(), gtk::PolicyType::Always);
    assert!(!grid_scroll.is_overlay_scrolling());
    let window = gtk::Window::builder()
        .default_width(240)
        .default_height(360)
        .child(&view.widget)
        .build();
    window.present();

    for (font, size, foreground, background) in [
        ("Sans", 13, "black", "white"),
        ("Monospace", 22, "white", "black"),
        ("Sans", 13, "black", "white"),
    ] {
        live_styles.load_from_string(&format!(
            "@define-color theme_bg {background}; @define-color theme_surface {background};
             @define-color theme_text {foreground}; @define-color theme_dim_text {foreground};
             @define-color theme_accent {foreground}; @define-color theme_on_accent {background};
             @define-color theme_border {foreground}; @define-color theme_highlight {background};
             @define-color theme_muted {background};
             window {{ font-family: '{font}'; font-size: {size}px; }}"
        ));
        wait_for_live_font(root, font);
        assert!(view.widget.measure(gtk::Orientation::Horizontal, -1).0 <= 240);
        let header = find_spreadsheet_widget(root, "spreadsheet-header");
        let action_bounds = view.expand.compute_bounds(&header).expect("action bounds");
        assert!(action_bounds.x() >= 0.0);
        assert!(action_bounds.x() + action_bounds.width() <= header.width() as f32);
        assert!(dimensions.width() > 0);
        assert!(title_label.width() > 0);
        assert_eq!(title_label.tooltip_text().as_deref(), Some(title.as_str()));
        assert_eq!(
            dimensions.tooltip_text().as_deref(),
            Some(dimensions.text().as_str())
        );
        assert_eq!(
            title_label
                .pango_context()
                .font_description()
                .expect("title font")
                .family()
                .as_deref(),
            Some(font)
        );
        for cell in mapped_labels(root)
            .iter()
            .filter(|label| label.has_css_class("spreadsheet-cell"))
        {
            for context in [cell.pango_context(), cell.layout().context()] {
                let description = context.font_description().expect("cell font");
                assert_eq!(description.family().as_deref(), Some(font));
                assert_eq!(description.size(), size * gtk::pango::SCALE);
            }
        }
        assert_eq!(
            title_label.color(),
            gtk::gdk::RGBA::parse(foreground).expect("color")
        );
        assert!(sheet_scroll.hadjustment().upper() > sheet_scroll.hadjustment().page_size());
    }
    let last = bar
        .last_child()
        .expect("last sheet tab")
        .downcast::<gtk::Button>()
        .expect("tab");
    last.emit_clicked();
    wait_for_live_font(root, "Sans");
    assert_eq!(stack.visible_child_name().as_deref(), Some("sheet_15"));
    assert_eq!(stack.pages().n_items(), 2);
    assert_eq!(
        dimensions.text(),
        "1000015 rows · 256 cols · Sheet 16 of 16"
    );
    bar.first_child()
        .expect("first sheet tab")
        .downcast::<gtk::Button>()
        .expect("tab")
        .emit_clicked();
    wait_for_live_font(root, "Sans");
    assert!(errors.borrow().is_empty(), "{:?}", errors.borrow());
    assert!(!title_label.has_css_class("music-title"));
    assert!(!view.expand.has_css_class("hermes-media-action"));
    assert_eq!(stack.transition_type(), gtk::StackTransitionType::None);
    assert_eq!(stack.transition_duration(), 0);
    let first = bar
        .first_child()
        .expect("first tab")
        .downcast::<gtk::Button>()
        .expect("tab");
    let second = first
        .next_sibling()
        .expect("second tab")
        .downcast::<gtk::Button>()
        .expect("tab");
    let first_height = first.measure(gtk::Orientation::Vertical, -1).0;
    second.emit_clicked();
    assert_eq!(stack.visible_child_name().as_deref(), Some("sheet_1"));
    assert_eq!(stack.pages().n_items(), 3);
    assert!(second.has_css_class("active-sheet-tab"));
    assert!(!first.has_css_class("active-sheet-tab"));
    assert_eq!(dimensions.text(), "1000001 rows · 256 cols · Sheet 2 of 16");
    assert_eq!(
        second.measure(gtk::Orientation::Vertical, -1).0,
        first_height
    );
    first.emit_clicked();
    let cost = table
        .columns()
        .item(1)
        .expect("cost column")
        .downcast::<gtk::ColumnViewColumn>()
        .expect("column");
    table.sort_by_column(Some(&cost), gtk::SortType::Ascending);
    let model = table.model().expect("selection model");
    assert_eq!(model.n_items(), 2);
    assert_eq!(
        *model
            .item(0)
            .expect("first row")
            .downcast::<glib::BoxedAnyObject>()
            .expect("row index")
            .borrow::<usize>(),
        1
    );
    assert_eq!(data.sheets[0].rows[0][1], "10");
    window.close();
    gtk::style_context_remove_provider_for_display(&display, &live_styles);
    gtk::style_context_remove_provider_for_display(&display, &styles);
}
