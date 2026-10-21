use gtk::{gio, glib, prelude::*};
use std::{cell::Cell, path::Path, rc::Rc};

use super::formatting::{self, Value};

pub struct StatusBar {
    pub container: gtk::Box,
    item_count: gtk::Label,
    selection_info: gtk::Label,
    free_space: gtk::Label,
    free_space_generation: Rc<Cell<u64>>,
}

impl StatusBar {
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        container.add_css_class("status-bar");

        let left_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        left_box.set_halign(gtk::Align::Start);

        let item_count = gtk::Label::new(Some("0 items"));
        item_count.add_css_class("status-item-count");

        let selection_info = gtk::Label::new(None);
        selection_info.add_css_class("status-selection-info");

        left_box.append(&item_count);
        left_box.append(&selection_info);

        let free_space = gtk::Label::new(None);
        free_space.add_css_class("status-free-space");
        free_space.set_halign(gtk::Align::End);
        free_space.set_hexpand(true);

        container.append(&left_box);
        container.append(&free_space);

        Self {
            container,
            item_count,
            selection_info,
            free_space,
            free_space_generation: Rc::new(Cell::new(0)),
        }
    }

    pub fn update_item_count(&self, count: usize) {
        self.item_count.set_text(&format_item_count(count));
    }

    pub fn update_selection(&self, count: usize, total_bytes: u64) {
        formatting::bind(&self.selection_info, Value::Selection(count, total_bytes));
    }

    pub fn update_free_space(&self, path: &Path) {
        let file = gio::File::for_path(path);
        let free_space_label = self.free_space.downgrade();
        let generation = self.free_space_generation.clone();
        let expected = generation.get().wrapping_add(1);
        generation.set(expected);
        formatting::clear(&self.free_space, "");

        glib::MainContext::default().spawn_local(async move {
            let result = file
                .query_filesystem_info_future("filesystem::free", glib::Priority::DEFAULT)
                .await;
            if generation.get() != expected {
                return;
            }
            let Some(free_space_label) = free_space_label.upgrade() else {
                return;
            };
            if let Ok(info) = result {
                if info.has_attribute("filesystem::free") {
                    let free_bytes = info.attribute_uint64("filesystem::free");
                    formatting::bind(&free_space_label, Value::FreeSpace(free_bytes));
                } else {
                    formatting::clear(&free_space_label, "");
                }
            } else {
                formatting::clear(&free_space_label, "");
            }
        });
    }

    pub fn clear_free_space(&self) {
        self.free_space_generation
            .set(self.free_space_generation.get().wrapping_add(1));
        formatting::clear(&self.free_space, "");
    }
}

pub(crate) fn format_item_count(count: usize) -> String {
    if count == 1 {
        "1 item".into()
    } else {
        format!("{} items", count)
    }
}

#[cfg(test)]
pub(crate) fn format_selection(count: usize, total_bytes: u64) -> String {
    formatting::format_value(Value::Selection(count, total_bytes))
}

#[cfg(test)]
pub(crate) fn format_free_space(free_bytes: u64) -> String {
    formatting::format_value(Value::FreeSpace(free_bytes))
}

#[cfg(test)]
mod tests;
