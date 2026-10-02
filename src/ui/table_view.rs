// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cmp::Ordering,
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::services::table::SpreadsheetData;

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

pub fn build_spreadsheet_view(data: &SpreadsheetData) -> gtk::Widget {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
    container.set_vexpand(true);
    container.set_hexpand(true);

    // Sheet tabs or active sheet title if multi-sheet
    if data.sheet_names.len() > 1 {
        let sheet_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        sheet_bar.add_css_class("spreadsheet-sheet-bar");
        for (idx, name) in data.sheet_names.iter().enumerate() {
            let label = gtk::Label::new(Some(name));
            if idx == data.active_sheet {
                label.add_css_class("active-sheet-tab");
            } else {
                label.add_css_class("sheet-tab");
            }
            sheet_bar.append(&label);
        }
        container.append(&sheet_bar);
    }

    let rows_rc = Rc::new(data.rows.clone());
    let cols = data.headers.len().max(
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
        let title = if col_idx < data.headers.len() && !data.headers[col_idx].is_empty() {
            data.headers[col_idx].clone()
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

    container.append(&scroll);

    if data.truncated {
        let warning = gtk::Label::new(Some(
            "Table preview capped at 200 rows and 256 columns; open file for complete data."
        ));
        warning.add_css_class("dim-label");
        warning.set_margin_top(4);
        warning.set_margin_bottom(4);
        container.append(&warning);
    }

    container.upcast::<gtk::Widget>()
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

#[cfg(test)]
mod tests {
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
}
