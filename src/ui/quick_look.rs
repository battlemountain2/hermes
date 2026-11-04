// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{
    adapters::LocalPreviewProvider,
    model::FileEntry,
    ui::preview::PreviewDrawer,
};

#[cfg(test)]
mod tests;

pub struct QuickLookModal {
    window: gtk::Window,
    drawer: PreviewDrawer,
    is_open: Cell<bool>,
    on_next: RefCell<Option<Rc<dyn Fn() -> Option<FileEntry>>>>,
    on_prev: RefCell<Option<Rc<dyn Fn() -> Option<FileEntry>>>>,
}

impl QuickLookModal {
    pub fn new() -> Rc<Self> {
        let window = gtk::Window::builder()
            .title("Quick Look")
            .default_width(850)
            .default_height(600)
            .destroy_with_parent(true)
            .build();
        window.add_css_class("quick-look-window");

        let drawer = PreviewDrawer::new(Rc::new(LocalPreviewProvider));
        let drawer_widget = drawer.widget();
        drawer_widget.set_vexpand(true);
        drawer_widget.set_hexpand(true);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.add_css_class("quick-look-footer");
        footer.set_halign(gtk::Align::Center);
        footer.set_valign(gtk::Align::Center);

        let hint = gtk::Label::new(Some("← / →  navigate    Space / Esc  close"));
        hint.add_css_class("quick-look-hint");
        footer.append(&hint);

        let layout = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layout.append(&drawer_widget);
        layout.append(&footer);

        window.set_child(Some(&layout));

        let this = Rc::new(Self {
            window: window.clone(),
            drawer: drawer.clone(),
            is_open: Cell::new(false),
            on_next: RefCell::new(None),
            on_prev: RefCell::new(None),
        });

        // Window close handler
        let weak = Rc::downgrade(&this);
        window.connect_close_request(move |win| {
            if let Some(modal) = weak.upgrade() {
                modal.is_open.set(false);
                modal.drawer.close();
            }
            win.set_visible(false);
            glib::Propagation::Stop
        });

        // Keyboard navigation controller
        let weak_keys = Rc::downgrade(&this);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(modal) = weak_keys.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if key == gtk::gdk::Key::Escape || key == gtk::gdk::Key::space {
                modal.close();
                return glib::Propagation::Stop;
            }
            if matches!(key, gtk::gdk::Key::Right | gtk::gdk::Key::Down) {
                if let Some(next_fn) = modal.on_next.borrow().as_ref() {
                    if let Some(entry) = next_fn() {
                        modal.window.set_title(Some(&format!("Quick Look — {}", entry.display_name)));
                        modal.drawer.show(entry);
                        return glib::Propagation::Stop;
                    }
                }
            }
            if matches!(key, gtk::gdk::Key::Left | gtk::gdk::Key::Up) {
                if let Some(prev_fn) = modal.on_prev.borrow().as_ref() {
                    if let Some(entry) = prev_fn() {
                        modal.window.set_title(Some(&format!("Quick Look — {}", entry.display_name)));
                        modal.drawer.show(entry);
                        return glib::Propagation::Stop;
                    }
                }
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);

        this
    }

    pub fn is_open(&self) -> bool {
        self.is_open.get()
    }

    pub fn show(&self, entry: FileEntry, parent: &impl IsA<gtk::Window>) {
        self.window.set_transient_for(Some(parent));
        self.window.set_title(Some(&format!("Quick Look — {}", entry.display_name)));
        self.window.set_visible(true);
        self.window.present();
        self.drawer.show(entry);
        self.is_open.set(true);
    }

    pub fn update_entry(&self, entry: FileEntry) {
        if self.is_open.get() {
            self.window.set_title(Some(&format!("Quick Look — {}", entry.display_name)));
            self.drawer.show(entry);
        }
    }

    pub fn close(&self) {
        self.window.set_visible(false);
        self.drawer.close();
        self.is_open.set(false);
    }

    pub fn toggle(&self, entry: Option<FileEntry>, parent: &impl IsA<gtk::Window>) {
        if self.is_open() {
            self.close();
        } else if let Some(entry) = entry {
            self.show(entry, parent);
        }
    }

    pub fn set_navigation_handlers<N, P>(&self, next: N, prev: P)
    where
        N: Fn() -> Option<FileEntry> + 'static,
        P: Fn() -> Option<FileEntry> + 'static,
    {
        self.on_next.replace(Some(Rc::new(next)));
        self.on_prev.replace(Some(Rc::new(prev)));
    }
}
