use gtk::{gio, glib, prelude::*};
use std::{cell::Cell, path::Path, rc::Rc};

use super::formatting::{self, Value};

pub struct StatusBar {
    pub container: gtk::Box,
    item_count: gtk::Label,
    selection_info: gtk::Label,
    free_space: gtk::Label,
    free_space_generation: Rc<Cell<u64>>,
    columns_btn: gtk::Button,
    grid_btn: gtk::Button,
    list_btn: gtk::Button,
    density_label: gtk::Label,
    density_btn: gtk::Button,
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

        let right_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        right_box.set_halign(gtk::Align::End);
        right_box.set_hexpand(true);

        let free_space = gtk::Label::new(None);
        free_space.add_css_class("status-free-space");

        let view_switcher = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        view_switcher.add_css_class("status-view-switcher");

        let columns_btn = gtk::Button::builder()
            .has_frame(false)
            .tooltip_text("Columns view (Ctrl+1)")
            .build();
        columns_btn.add_css_class("status-mode-btn");
        columns_btn.add_css_class("active");
        columns_btn.set_child(Some(&crate::assets::primary_icon(crate::assets::icons::LIST, 14)));

        let grid_btn = gtk::Button::builder()
            .has_frame(false)
            .tooltip_text("Grid view (Ctrl+2)")
            .build();
        grid_btn.add_css_class("status-mode-btn");
        grid_btn.set_child(Some(&crate::assets::primary_icon(crate::assets::icons::GRID, 14)));

        let list_btn = gtk::Button::builder()
            .has_frame(false)
            .tooltip_text("List view (Ctrl+3)")
            .build();
        list_btn.add_css_class("status-mode-btn");
        list_btn.set_child(Some(&crate::assets::primary_icon(crate::assets::icons::ROWS, 14)));

        view_switcher.append(&columns_btn);
        view_switcher.append(&grid_btn);
        view_switcher.append(&list_btn);

        let density_btn = gtk::Button::builder()
            .has_frame(false)
            .tooltip_text("Zoom & Density (Ctrl++, Ctrl+-)")
            .build();
        density_btn.add_css_class("status-density-pill");
        let density_label = gtk::Label::new(Some("Compact"));
        density_label.add_css_class("status-density-label");
        density_btn.set_child(Some(&density_label));

        right_box.append(&free_space);
        right_box.append(&view_switcher);
        right_box.append(&density_btn);

        container.append(&left_box);
        container.append(&right_box);

        Self {
            container,
            item_count,
            selection_info,
            free_space,
            free_space_generation: Rc::new(Cell::new(0)),
            columns_btn,
            grid_btn,
            list_btn,
            density_label,
            density_btn,
        }
    }

    #[allow(dead_code)]
    pub fn update_item_count(&self, count: usize) {
        self.update_items(count, 0);
    }

    pub fn update_items(&self, count: usize, total_bytes: u64) {
        self.item_count.set_text(&format_item_stats(count, total_bytes));
    }

    pub fn update_selection(&self, count: usize, total_bytes: u64) {
        formatting::bind(&self.selection_info, Value::Selection(count, total_bytes));
    }

    pub fn update_view_mode(&self, mode: crate::ui::browser_modes::BrowserMode) {
        use crate::ui::browser_modes::BrowserMode;
        self.columns_btn.remove_css_class("active");
        self.grid_btn.remove_css_class("active");
        self.list_btn.remove_css_class("active");
        match mode {
            BrowserMode::Columns => self.columns_btn.add_css_class("active"),
            BrowserMode::Grid => self.grid_btn.add_css_class("active"),
            BrowserMode::Explorer => self.list_btn.add_css_class("active"),
        }
    }

    pub fn update_density(&self, label: &str) {
        self.density_label.set_text(label);
    }

    pub fn set_mode_handlers(
        &self,
        on_columns: impl Fn() + 'static,
        on_grid: impl Fn() + 'static,
        on_list: impl Fn() + 'static,
    ) {
        self.columns_btn.connect_clicked(move |_| on_columns());
        self.grid_btn.connect_clicked(move |_| on_grid());
        self.list_btn.connect_clicked(move |_| on_list());
    }

    pub fn set_zoom_handler(&self, on_zoom_step: impl Fn() + 'static) {
        self.density_btn.connect_clicked(move |_| on_zoom_step());
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

pub(crate) fn format_item_stats(count: usize, total_bytes: u64) -> String {
    let items_str = format_item_count(count);
    if count > 0 && total_bytes > 0 {
        format!("{} ({})", items_str, formatting::format_size(total_bytes))
    } else {
        items_str
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
