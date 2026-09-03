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
        if count == 1 {
            self.item_count.set_text("1 item");
        } else {
            self.item_count.set_text(&format!("{} items", count));
        }
    }
    
    pub fn update_selection(&self, count: usize, total_bytes: u64) {
        if count == 0 {
            self.selection_info.set_text("");
        } else {
            let size_str = format_file_size(total_bytes);
            self.selection_info.set_text(&format!("{} selected, {}", count, size_str));
        }
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
                    let size_str = format_file_size(free_bytes);
                    free_space_label.set_text(&format!("{} free", size_str));
                } else {
                    free_space_label.set_text("");
                }
            } else {
                free_space_label.set_text("");
            }
        });
    }
}

pub fn build_status_bar() -> StatusBar {
    StatusBar::new()
}
