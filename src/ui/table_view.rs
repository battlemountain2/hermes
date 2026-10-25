// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::RefCell,
    cmp::Ordering,
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    assets::{self, icons},
    services::table::{SheetData, SpreadsheetData},
};

#[derive(Clone, Debug, PartialEq)]
enum SortKey {
    Number(f64),
    Text(String),
    Empty,
}

impl SortKey {
    fn new(text: &str) -> Self {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Self::Empty;
        }
        match trimmed.parse::<f64>() {
            Ok(number) if number.is_finite() => Self::Number(number),
            _ => Self::Text(trimmed.to_lowercase()),
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Empty, Self::Empty) => Ordering::Equal,
            (Self::Number(_), _) | (Self::Text(_), Self::Empty) => Ordering::Less,
            _ => Ordering::Greater,
        }
    }
}

pub struct SpreadsheetView {
    pub widget: gtk::Box,
    window: Rc<RefCell<Option<gtk::Window>>>,
    expand: gtk::Button,
}

impl SpreadsheetView {
    pub fn open_fullscreen(&self) -> bool {
        let existing = self.window.borrow().clone();
        if let Some(window) = existing {
            window.present();
            return true;
        }
        self.expand.emit_clicked();
        true
    }

    pub fn stop(&self) {
        let window = self.window.borrow_mut().take();
        if let Some(window) = window {
            window.close();
        }
    }
}

impl Drop for SpreadsheetView {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn build_spreadsheet_view(data: &SpreadsheetData, title: &str) -> SpreadsheetView {
    let home = gtk::Box::new(gtk::Orientation::Vertical, 0);
    home.set_vexpand(true);
    home.set_hexpand(true);

    let viewer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    viewer.add_css_class("spreadsheet-container");
    viewer.set_vexpand(true);
    viewer.set_hexpand(true);

    // Header bar with title, dimensions pill, and fullscreen toggle
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.add_css_class("spreadsheet-header");

    let name_label = gtk::Label::new(Some(title));
    name_label.add_css_class("music-title");
    name_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    name_label.set_xalign(0.0);
    name_label.set_hexpand(true);
    controls.append(&name_label);

    let dim_label = gtk::Label::new(None);
    dim_label.add_css_class("spreadsheet-dimension-pill");
    controls.append(&dim_label);

    let expand = gtk::Button::builder()
        .tooltip_text("Fullscreen spreadsheet (F)")
        .build();
    expand.add_css_class("hermes-media-action");
    expand.set_child(Some(&assets::primary_icon(icons::MAXIMIZE, 16)));
    controls.append(&expand);

    viewer.append(&controls);

    // Sheets stack
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(100)
        .vexpand(true)
        .hexpand(true)
        .build();

    let sheets: Vec<SheetData> = if !data.sheets.is_empty() {
        data.sheets.clone()
    } else {
        vec![SheetData {
            name: data.sheet_names.first().cloned().unwrap_or_else(|| "Sheet1".to_string()),
            headers: data.headers.clone(),
            rows: data.rows.clone(),
            total_rows: data.total_rows,
            total_cols: data.total_cols,
            truncated: data.truncated,
        }]
    };

    let dim_label_for_tabs = dim_label.clone();
    let sheet_buttons: Rc<RefCell<Vec<glib::WeakRef<gtk::Button>>>> = Rc::new(RefCell::new(Vec::new()));

    if sheets.len() > 1 {
        let sheet_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        sheet_bar.add_css_class("spreadsheet-sheet-bar");

        for (idx, sheet) in sheets.iter().enumerate() {
            let btn = gtk::Button::with_label(&sheet.name);
            btn.add_css_class("spreadsheet-sheet-button");
            if idx == 0 {
                btn.add_css_class("active-sheet-tab");
            }

            let stack = stack.clone();
            let sheet_name = format!("sheet_{idx}");
            let buttons = sheet_buttons.clone();
            let dim_lbl = dim_label_for_tabs.clone();
            let s_rows = sheet.total_rows.max(sheet.rows.len() + if sheet.headers.is_empty() { 0 } else { 1 });
            let s_cols = sheet.total_cols.max(sheet.headers.len());
            let sheets_count = sheets.len();

            btn.connect_clicked(move |clicked| {
                for candidate in buttons.borrow().iter().filter_map(glib::WeakRef::upgrade) {
                    candidate.remove_css_class("active-sheet-tab");
                }
                clicked.add_css_class("active-sheet-tab");
                stack.set_visible_child_name(&sheet_name);
                dim_lbl.set_text(&format!(
                    "{} rows · {} cols · Sheet {} of {}",
                    s_rows, s_cols, idx + 1, sheets_count
                ));
            });

            sheet_buttons.borrow_mut().push(btn.downgrade());
            sheet_bar.append(&btn);
        }
        viewer.append(&sheet_bar);
    }

    // Set initial dimension label
    if let Some(first) = sheets.first() {
        let s_rows = first.total_rows.max(first.rows.len() + if first.headers.is_empty() { 0 } else { 1 });
        let s_cols = first.total_cols.max(first.headers.len());
        if sheets.len() > 1 {
            dim_label.set_text(&format!(
                "{} rows · {} cols · Sheet 1 of {}",
                s_rows, s_cols, sheets.len()
            ));
        } else {
            dim_label.set_text(&format!("{} rows · {} cols", s_rows, s_cols));
        }
    }

    // Build each sheet's table page
    for (idx, sheet) in sheets.iter().enumerate() {
        let sheet_page = build_sheet_table_page(sheet);
        stack.add_named(&sheet_page, Some(&format!("sheet_{idx}")));
    }

    viewer.append(&stack);

    if data.truncated {
        let warning = gtk::Label::new(Some(
            "Table preview capped at 200 rows and 256 columns; open file for complete data."
        ));
        warning.add_css_class("dim-label");
        warning.set_margin_top(4);
        warning.set_margin_bottom(4);
        viewer.append(&warning);
    }

    home.append(&viewer);

    // Fullscreen controller wiring
    let slot = Rc::new(RefCell::new(None::<gtk::Window>));
    let shared_slot = slot.clone();
    let weak_home = home.downgrade();
    let weak_viewer = viewer.downgrade();
    let expand_btn = expand.clone();
    let title_string = title.to_string();
    let stack_for_keys = stack.clone();
    let sheet_buttons_for_keys = sheet_buttons.clone();
    let sheets_count = sheets.len();

    expand.connect_clicked(move |button| {
        let existing = shared_slot.borrow().clone();
        if let Some(window) = existing {
            window.close();
            return;
        }
        let (Some(home), Some(viewer)) = (weak_home.upgrade(), weak_viewer.upgrade()) else {
            return;
        };
        let window = gtk::Window::builder()
            .title(&format!("Hermes — {}", title_string))
            .destroy_with_parent(true)
            .build();
        if let Some(parent) = home.root().and_downcast::<gtk::Window>() {
            window.set_transient_for(Some(&parent));
            window.set_application(parent.application().as_ref());
        }
        home.remove(&viewer);
        window.set_child(Some(&viewer));
        button.set_child(Some(&assets::primary_icon(icons::MINIMIZE, 16)));
        button.set_tooltip_text(Some("Exit fullscreen (F or Escape)"));

        let weak_slot = Rc::downgrade(&shared_slot);
        let weak_home = home.downgrade();
        let weak_viewer = viewer.downgrade();
        let weak_button = button.downgrade();

        window.connect_close_request(move |window| {
            let viewer = weak_viewer.upgrade();
            window.set_child(None::<&gtk::Widget>);
            if let (Some(home), Some(viewer)) = (weak_home.upgrade(), viewer) {
                if viewer.parent().is_none() {
                    home.append(&viewer);
                }
            }
            if let Some(button) = weak_button.upgrade() {
                button.set_child(Some(&assets::primary_icon(icons::MAXIMIZE, 16)));
                button.set_tooltip_text(Some("Fullscreen spreadsheet (F)"));
            }
            if let Some(slot) = weak_slot.upgrade() {
                slot.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });

        attach_table_key_controller(&window, &stack_for_keys, &sheet_buttons_for_keys, sheets_count, Some(window.downgrade()));
        shared_slot.replace(Some(window.clone()));
        window.fullscreen();
        window.present();
    });

    viewer.set_focusable(true);
    attach_table_key_controller(&viewer, &stack, &sheet_buttons, sheets.len(), None);

    SpreadsheetView {
        widget: home,
        window: slot,
        expand: expand_btn,
    }
}

fn build_sheet_table_page(sheet: &SheetData) -> gtk::Widget {
    let rows_rc = Rc::new(sheet.rows.clone());
    let cols = sheet.headers.len().max(
        rows_rc.iter().map(Vec::len).max().unwrap_or(0)
    );

    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for i in 0..rows_rc.len() {
        store.append(&glib::BoxedAnyObject::new(i));
    }

    let sort_model = gtk::SortListModel::new(Some(store), None::<gtk::Sorter>);
    let no_selection = gtk::NoSelection::new(Some(sort_model.clone()));
    let column_view = gtk::ColumnView::new(Some(no_selection));
    column_view.set_show_row_separators(true);
    column_view.set_show_column_separators(true);

    for col_idx in 0..cols {
        let title = if col_idx < sheet.headers.len() && !sheet.headers[col_idx].is_empty() {
            sheet.headers[col_idx].clone()
        } else {
            column_name(col_idx)
        };

        let factory = gtk::SignalListItemFactory::new();
        let rows_for_factory = rows_rc.clone();
        factory.connect_setup(move |_, item| {
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_margin_start(8);
            label.set_margin_end(8);
            label.set_margin_top(4);
            label.set_margin_bottom(4);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            if let Some(list_item) = item.downcast_ref::<gtk::ColumnViewCell>() {
                list_item.set_child(Some(&label));
            }
        });

        factory.connect_bind(move |_, item| {
            if let Some(cell) = item.downcast_ref::<gtk::ColumnViewCell>()
                && let Some(label) = cell.child().and_then(|c| c.downcast::<gtk::Label>().ok())
                && let Some(row_obj) = cell.item().and_then(|it| it.downcast::<glib::BoxedAnyObject>().ok())
            {
                let row_idx = *row_obj.borrow::<usize>();
                if let Some(row) = rows_for_factory.get(row_idx) {
                    let text = row.get(col_idx).map(String::as_str).unwrap_or("");
                    label.set_text(text);
                }
            }
        });

        let column = gtk::ColumnViewColumn::new(Some(&title), Some(factory));
        column.set_resizable(true);

        // Custom Sorter
        let rows_for_sorter = rows_rc.clone();
        let custom_sorter = gtk::CustomSorter::new(move |a, b| {
            let idx_a = a.downcast_ref::<glib::BoxedAnyObject>().map(|o| *o.borrow::<usize>()).unwrap_or(0);
            let idx_b = b.downcast_ref::<glib::BoxedAnyObject>().map(|o| *o.borrow::<usize>()).unwrap_or(0);

            let val_a = rows_for_sorter.get(idx_a).and_then(|r| r.get(col_idx)).map(String::as_str).unwrap_or("");
            let val_b = rows_for_sorter.get(idx_b).and_then(|r| r.get(col_idx)).map(String::as_str).unwrap_or("");

            let key_a = SortKey::new(val_a);
            let key_b = SortKey::new(val_b);
            match key_a.compare(&key_b) {
                Ordering::Less => gtk::Ordering::Smaller,
                Ordering::Equal => gtk::Ordering::Equal,
                Ordering::Greater => gtk::Ordering::Larger,
            }
        });

        column.set_sorter(Some(&custom_sorter));
        column_view.append_column(&column);
    }

    if let Some(sorter) = column_view.sorter() {
        sort_model.set_sorter(Some(&sorter));
    }

    let scroll = gtk::ScrolledWindow::builder()
        .child(&column_view)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .hexpand(true)
        .build();

    scroll.upcast()
}

fn column_name(mut idx: usize) -> String {
    let mut name = String::new();
    loop {
        let rem = idx % 26;
        name.insert(0, (b'A' + rem as u8) as char);
        if idx < 26 {
            break;
        }
        idx = (idx / 26) - 1;
    }
    name
}

fn attach_table_key_controller(
    target: &impl glib::object::IsA<gtk::Widget>,
    stack: &gtk::Stack,
    sheet_buttons: &Rc<RefCell<Vec<glib::WeakRef<gtk::Button>>>>,
    sheets_count: usize,
    window_to_close: Option<glib::WeakRef<gtk::Window>>,
) {
    let controller = gtk::EventControllerKey::new();
    if window_to_close.is_some() {
        controller.set_name(Some("hermes-table-fullscreen"));
    } else {
        controller.set_name(Some("hermes-table-viewer"));
    }

    let stack = stack.clone();
    let sheet_buttons = sheet_buttons.clone();

    controller.connect_key_pressed(move |_, keyval, _keycode, state| {
        // Fullscreen exit shortcuts: Escape, F/f (without Ctrl/Alt)
        if let Some(ref weak_window) = window_to_close {
            if keyval == gtk::gdk::Key::Escape
                || (!state.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                    && !state.contains(gtk::gdk::ModifierType::ALT_MASK)
                    && (keyval == gtk::gdk::Key::f || keyval == gtk::gdk::Key::F))
            {
                if let Some(window) = weak_window.upgrade() {
                    window.close();
                    return glib::Propagation::Stop;
                }
            }
        }

        // Sheet tab navigation
        if sheets_count > 1 {
            let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let shift = state.contains(gtk::gdk::ModifierType::SHIFT_MASK);

            let next_tab = (ctrl && keyval == gtk::gdk::Key::Page_Down)
                || (ctrl && !shift && keyval == gtk::gdk::Key::Tab);
            let prev_tab = (ctrl && keyval == gtk::gdk::Key::Page_Up)
                || (ctrl && (keyval == gtk::gdk::Key::ISO_Left_Tab || (shift && keyval == gtk::gdk::Key::Tab)));

            if next_tab || prev_tab {
                if let Some(current_name) = stack.visible_child_name() {
                    if let Some(idx_str) = current_name.strip_prefix("sheet_") {
                        if let Ok(idx) = idx_str.parse::<usize>() {
                            let target_idx = if next_tab {
                                (idx + 1) % sheets_count
                            } else if idx == 0 {
                                sheets_count - 1
                            } else {
                                idx - 1
                            };
                            let buttons = sheet_buttons.borrow();
                            if let Some(Some(btn)) = buttons.get(target_idx).map(glib::WeakRef::upgrade) {
                                btn.emit_clicked();
                                return glib::Propagation::Stop;
                            }
                        }
                    }
                }
            }
        }

        // Table scrolling with arrow keys / PageUp / PageDown / Home / End
        if let Some(child) = stack.visible_child() {
            if let Some(scrolled) = child.downcast_ref::<gtk::ScrolledWindow>() {
                let vadj = scrolled.vadjustment();
                let hadj = scrolled.hadjustment();

                let step_v = vadj.step_increment().max(32.0);
                let page_v = vadj.page_increment().max(200.0);
                let step_h = hadj.step_increment().max(40.0);

                match keyval {
                    gtk::gdk::Key::Up => {
                        vadj.set_value((vadj.value() - step_v).max(vadj.lower()));
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Down => {
                        vadj.set_value((vadj.value() + step_v).min(vadj.upper() - vadj.page_size()));
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Left => {
                        hadj.set_value((hadj.value() - step_h).max(hadj.lower()));
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Right => {
                        hadj.set_value((hadj.value() + step_h).min(hadj.upper() - hadj.page_size()));
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Page_Up => {
                        vadj.set_value((vadj.value() - page_v).max(vadj.lower()));
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Page_Down => {
                        vadj.set_value((vadj.value() + page_v).min(vadj.upper() - vadj.page_size()));
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::Home => {
                        vadj.set_value(vadj.lower());
                        return glib::Propagation::Stop;
                    }
                    gtk::gdk::Key::End => {
                        vadj.set_value((vadj.upper() - vadj.page_size()).max(vadj.lower()));
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
        }

        glib::Propagation::Proceed
    });

    target.add_controller(controller);
}

#[cfg(test)]
mod tests;
