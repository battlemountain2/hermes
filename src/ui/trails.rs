// SPDX-License-Identifier: GPL-3.0-or-later

use std::rc::Rc;

use gtk::prelude::*;

use crate::{
    app::{Browser, Trails},
    model::{Trail, TrailId, TrailViewState},
};

pub struct TabBar {
    root: gtk::ScrolledWindow,
    tabs: gtk::Box,
    new_button: gtk::Button,
    tab_new_button: gtk::Button,
    trails: Rc<Trails>,
    browser: Rc<Browser>,
    capture_view: Rc<dyn Fn() -> TrailViewState>,
    activate_tab: Rc<dyn Fn(Trail)>,
}

impl TabBar {
    pub fn new(
        trails: Rc<Trails>,
        browser: Rc<Browser>,
        capture_view: Rc<dyn Fn() -> TrailViewState>,
        activate_tab: Rc<dyn Fn(Trail)>,
    ) -> Rc<Self> {
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        tabs.add_css_class("tab-strip-content");
        let root = gtk::ScrolledWindow::builder()
            .child(&tabs)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .build();
        root.add_css_class("tab-strip");

        let new_button = gtk::Button::builder()
            .tooltip_text("New Tab (Ctrl+T)")
            .build();
        new_button.set_child(Some(&crate::assets::text_icon(
            crate::assets::icons::PLUS,
            18,
        )));
        new_button.add_css_class("header-action");
        let tab_new_button = gtk::Button::builder()
            .tooltip_text("New Tab (Ctrl+T)")
            .build();
        tab_new_button.set_child(Some(&crate::assets::text_icon(
            crate::assets::icons::PLUS,
            16,
        )));
        tab_new_button.add_css_class("tab-strip-new");

        let tab_bar = Rc::new(Self {
            root,
            tabs,
            new_button,
            tab_new_button,
            trails,
            browser,
            capture_view,
            activate_tab,
        });
        let weak = Rc::downgrade(&tab_bar);
        tab_bar.new_button.connect_clicked(move |_| {
            if let Some(tab_bar) = weak.upgrade() {
                tab_bar.new_tab();
            }
        });
        let weak = Rc::downgrade(&tab_bar);
        tab_bar.tab_new_button.connect_clicked(move |_| {
            if let Some(tab_bar) = weak.upgrade() {
                tab_bar.new_tab();
            }
        });
        tab_bar.refresh();
        tab_bar
    }

    pub fn widget(&self) -> gtk::ScrolledWindow {
        self.root.clone()
    }

    pub fn new_button(&self) -> gtk::Button {
        self.new_button.clone()
    }

    pub fn new_tab(self: &Rc<Self>) {
        let Some(location) = self.browser.active_location() else {
            return;
        };
        if let Err(error) =
            self.trails
                .create(location.display_name(), location, (self.capture_view)())
        {
            tracing::warn!(%error, "unable to create tab");
        }
        self.refresh();
    }

    pub fn close_active(self: &Rc<Self>) {
        let Some(active) = self.trails.active_id() else {
            return;
        };
        self.close_tab(&active);
    }

    pub fn cycle(self: &Rc<Self>, offset: isize) {
        match self.trails.cycle(offset) {
            Ok(Some(tab)) => (self.activate_tab)(tab),
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "unable to cycle tabs"),
        }
        self.refresh();
    }

    pub fn refresh(self: &Rc<Self>) {
        while let Some(child) = self.tabs.first_child() {
            self.tabs.remove(&child);
        }
        let tabs = self.trails.all();
        let active = self.trails.active_id();
        self.root.set_visible(!tabs.is_empty());
        for tab in &tabs {
            self.tabs
                .append(&self.tab(tab, tabs.len(), active.as_ref()));
        }
        self.tabs.append(&self.tab_new_button);
    }

    fn tab(self: &Rc<Self>, tab: &Trail, count: usize, active: Option<&TrailId>) -> gtk::Box {
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 1);
        item.add_css_class("file-tab");
        if active == Some(&tab.id) {
            item.add_css_class("active");
        }

        let select = gtk::Button::with_label(&tab.name);
        select.add_css_class("file-tab-select");
        select.set_tooltip_text(
            tab.active_location()
                .map(|location| location.display_path())
                .as_deref(),
        );
        let id = tab.id.clone();
        let weak = Rc::downgrade(self);
        select.connect_clicked(move |_| {
            let Some(tab_bar) = weak.upgrade() else {
                return;
            };
            match tab_bar.trails.activate(&id) {
                Ok(Some(tab)) => (tab_bar.activate_tab)(tab),
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "unable to activate tab"),
            }
            tab_bar.refresh();
        });

        let rename = gtk::Entry::builder()
            .text(&tab.name)
            .width_chars(14)
            .visible(false)
            .build();
        rename.add_css_class("file-tab-rename");
        let rename_id = tab.id.clone();
        let weak = Rc::downgrade(self);
        rename.connect_activate(move |entry| {
            let Some(tab_bar) = weak.upgrade() else {
                return;
            };
            if let Err(error) = tab_bar.trails.rename(&rename_id, entry.text()) {
                tracing::warn!(%error, "unable to rename tab");
            }
            tab_bar.refresh();
        });
        let rename_keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        rename_keys.connect_key_pressed(move |_, key, _, _| {
            if key != gtk::gdk::Key::Escape {
                return gtk::glib::Propagation::Proceed;
            }
            if let Some(tab_bar) = weak.upgrade() {
                tab_bar.refresh();
            }
            gtk::glib::Propagation::Stop
        });
        rename.add_controller(rename_keys);

        let rename_click = gtk::GestureClick::new();
        rename_click.set_button(1);
        let shown_select = select.clone();
        let shown_rename = rename.clone();
        rename_click.connect_pressed(move |gesture, presses, _, _| {
            if presses != 2 {
                return;
            }
            let _claimed = gesture.set_state(gtk::EventSequenceState::Claimed);
            shown_select.set_visible(false);
            shown_rename.set_visible(true);
            shown_rename.grab_focus();
            shown_rename.select_region(0, -1);
        });
        select.add_controller(rename_click);

        let pin = gtk::Button::builder()
            .tooltip_text(if tab.pinned { "Unpin Tab" } else { "Pin Tab" })
            .build();
        pin.set_child(Some(&crate::assets::text_icon(
            crate::assets::icons::PIN,
            12,
        )));
        pin.add_css_class("file-tab-pin");
        if tab.pinned {
            pin.add_css_class("active");
        }
        let pin_id = tab.id.clone();
        let weak = Rc::downgrade(self);
        pin.connect_clicked(move |_| {
            let Some(tab_bar) = weak.upgrade() else {
                return;
            };
            if let Err(error) = tab_bar.trails.toggle_pinned(&pin_id) {
                tracing::warn!(%error, "unable to update pinned tab");
            }
            tab_bar.refresh();
        });

        let close = gtk::Button::builder()
            .tooltip_text("Close Tab (Ctrl+W)")
            .build();
        close.set_child(Some(&crate::assets::text_icon(crate::assets::icons::X, 13)));
        close.add_css_class("file-tab-close");
        close.set_sensitive(count > 1);
        let close_id = tab.id.clone();
        let weak = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(tab_bar) = weak.upgrade() {
                tab_bar.close_tab(&close_id);
            }
        });

        let middle_click = gtk::GestureClick::new();
        middle_click.set_button(2);
        let middle_id = tab.id.clone();
        let weak = Rc::downgrade(self);
        middle_click.connect_pressed(move |gesture, _, _, _| {
            if let Some(tab_bar) = weak.upgrade() {
                tab_bar.close_tab(&middle_id);
                let _claimed = gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        item.add_controller(middle_click);

        item.append(&select);
        item.append(&rename);
        item.append(&pin);
        item.append(&close);
        item
    }

    fn close_tab(self: &Rc<Self>, id: &TrailId) {
        match self.trails.close(id) {
            Ok(Some(tab)) => (self.activate_tab)(tab),
            Ok(None) => {}
            Err(error) => tracing::warn!(%error, "unable to close tab"),
        }
        self.refresh();
    }
}
