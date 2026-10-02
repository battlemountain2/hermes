use gtk::{gio, glib, prelude::*};
use std::path::Path;

use super::browser::format_file_size;

pub struct StatusBar {
    pub container: gtk::Box,
    item_count: gtk::Label,
    selection_info: gtk::Label,
    free_space: gtk::Label,
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
        }
    }
    
    pub fn update_item_count(&self, count: usize) {
        self.item_count.set_text(&format_item_count(count));
    }
    
    pub fn update_selection(&self, count: usize, total_bytes: u64) {
        self.selection_info.set_text(&format_selection(count, total_bytes));
    }
    
    pub fn update_free_space(&self, path: &Path) {
        let file = gio::File::for_path(path);
        let free_space_label = self.free_space.clone();
        
        glib::MainContext::default().spawn_local(async move {
            if let Ok(info) = file.query_filesystem_info_future(
                "filesystem::free",
                glib::Priority::DEFAULT,
            ).await {
                if info.has_attribute("filesystem::free") {
                    let free_bytes = info.attribute_uint64("filesystem::free");
                    free_space_label.set_text(&format_free_space(free_bytes));
                } else {
                    free_space_label.set_text("");
                }
            } else {
                free_space_label.set_text("");
            }
        });
    }

    pub fn clear_free_space(&self) {
        self.free_space.set_text("");
    }
}

pub(crate) fn format_item_count(count: usize) -> String {
    if count == 1 {
        "1 item".into()
    } else {
        format!("{} items", count)
    }
}

pub(crate) fn format_selection(count: usize, total_bytes: u64) -> String {
    if count == 0 {
        String::new()
    } else {
        let size_str = format_file_size(total_bytes);
        format!("{} selected, {}", count, size_str)
    }
}

pub(crate) fn format_free_space(free_bytes: u64) -> String {
    let size_str = format_file_size(free_bytes);
    format!("{} free", size_str)
}

#[cfg(test)]
mod tests;
