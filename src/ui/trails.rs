// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, rc::Rc};

use gtk::prelude::*;

use crate::{
    app::{Browser, Trails},
    model::{Location, Trail, TrailId, TrailViewState},
};

#[cfg(test)]
mod tests;

pub struct TabBar {
    root: gtk::ScrolledWindow,
    tabs: gtk::Box,
    new_button: gtk::Button,
    tab_new_button: gtk::Button,
    trails: Rc<Trails>,
    browser: Rc<Browser>,
    capture_view: Rc<dyn Fn() -> TrailViewState>,
    activate_tab: Rc<dyn Fn(Trail)>,
    rendered: RefCell<Vec<(TabPresentation, gtk::Box)>>,
    rendered_active: RefCell<Option<TrailId>>,
}

struct TabPresentation {
    id: TrailId,
    name: String,
    location: Option<Location>,
}

impl TabBar {
    pub fn new(
        trails: Rc<Trails>,
        browser: Rc<Browser>,
        capture_view: Rc<dyn Fn() -> TrailViewState>,
        activate_tab: Rc<dyn Fn(Trail)>,
    ) -> Rc<Self> {
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tabs.add_css_class("tab-strip-content");
        tabs.set_valign(gtk::Align::Fill);
        tabs.set_halign(gtk::Align::Start);
        let root = gtk::ScrolledWindow::builder()
            .child(&tabs)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(false)
            .height_request(40)
            .overlay_scrolling(true)
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
            rendered: RefCell::new(Vec::new()),
            rendered_active: RefCell::new(None),
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
        match self
            .trails
            .create(location.display_name(), location, (self.capture_view)())
        {
            Ok(id) => {
                if let Some(tab) = self.trails.all().into_iter().find(|tab| tab.id == id) {
                    (self.activate_tab)(tab);
                }
            }
            Err(error) => tracing::warn!(%error, "unable to create tab"),
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
        let started = std::time::Instant::now();
        let tabs = self.trails.all();
        self.browser.retain_open_tabs(&tabs.iter().map(|tab| tab.id.clone()).collect::<Vec<_>>());
        let active = self.trails.active_id();
        let active_changed = self.rendered_active.replace(active.clone()) != active;
        let mut previous = self.rendered.take();
        let old_order: Vec<_> = previous.iter().map(|(_, item)| item.clone()).collect();
        let mut rendered = Vec::with_capacity(tabs.len());
        let mut rebuilt = 0usize;
        self.root.set_visible(!tabs.is_empty());
        for tab in &tabs {
            let existing = previous.iter().position(|(old, _)| old.id == tab.id && old.name == tab.name && old.location.as_ref() == tab.active_location());
            let item = if let Some(position) = existing {
                previous.remove(position).1
            } else {
                rebuilt += 1;
                self.tab(tab, tabs.len(), active.as_ref())
            };
            let is_active = active.as_ref() == Some(&tab.id);
            if is_active {
                item.add_css_class("active");
            } else {
                item.remove_css_class("active");
            }
            if let Some(close) = item.last_child() {
                close.set_sensitive(tabs.len() > 1);
            }
            if let Some(select) = item.first_child() {
                select.set_visible(true);
                if let Some(rename) = select.next_sibling().and_downcast::<gtk::Entry>() {
                    rename.set_visible(false);
                    if rename.text().as_str() != tab.name {
                        rename.set_text(&tab.name);
                    }
                }
            }
            if is_active && (active_changed || rebuilt > 0) {
                let weak_item = item.downgrade();
                let weak_tabs = self.tabs.downgrade();
                let weak_root = self.root.downgrade();
                item.add_tick_callback(move |_, _| {
                    let (Some(item), Some(tabs), Some(root)) = (
                        weak_item.upgrade(),
                        weak_tabs.upgrade(),
                        weak_root.upgrade(),
                    ) else {
                        return gtk::glib::ControlFlow::Break;
                    };
                    if item.width() == 0 || root.hadjustment().page_size() <= 0.0 {
                        return gtk::glib::ControlFlow::Continue;
                    }
                    if let Some(bounds) = item.compute_bounds(&tabs) {
                        let adjustment = root.hadjustment();
                        adjustment.set_value(visible_tab_scroll(
                            adjustment.value(),
                            adjustment.page_size(),
                            adjustment.upper(),
                            bounds.x() as f64,
                            bounds.width() as f64,
                        ));
                    }
                    gtk::glib::ControlFlow::Break
                });
            }
            rendered.push((TabPresentation {
                id: tab.id.clone(),
                name: tab.name.clone(),
                location: tab.active_location().cloned(),
            }, item));
        }
        if old_order.len() != rendered.len() || old_order.iter().zip(&rendered).any(|(old, (_, item))| old != item) {
            while let Some(child) = self.tabs.first_child() {
                self.tabs.remove(&child);
            }
            for (_, item) in &rendered {
                self.tabs.append(item);
            }
            self.tabs.append(&self.tab_new_button);
        }
        self.rendered.replace(rendered);
        tracing::debug!(tabs = tabs.len(), rebuilt, refresh_micros = started.elapsed().as_micros() as u64, "tab strip refreshed");
    }

    fn tab(self: &Rc<Self>, tab: &Trail, count: usize, active: Option<&TrailId>) -> gtk::Box {
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        item.add_css_class("file-tab");
        item.set_valign(gtk::Align::Fill);
        item.set_hexpand(false);
        if active == Some(&tab.id) {
            item.add_css_class("active");
        }

        let select = gtk::Button::with_label(&tab.name);
        select.add_css_class("file-tab-select");
        select.set_valign(gtk::Align::Fill);
        select.set_hexpand(false);
        if let Some(label) = select
            .child()
            .and_then(|child| child.downcast::<gtk::Label>().ok())
        {
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_width_chars(10);
            label.set_max_width_chars(18);
            label.set_xalign(0.0);
        }
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
            .max_width_chars(14)
            .valign(gtk::Align::Center)
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
        self.install_reordering(&item, &select, &tab.id);

        let close = gtk::Button::builder()
            .tooltip_text("Close Tab (Ctrl+W)")
            .build();
        close.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            14,
        )));
        close.set_valign(gtk::Align::Fill);
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

    fn install_reordering(self: &Rc<Self>, item: &gtk::Box, select: &gtk::Button, id: &TrailId) {
        let source = gtk::DragSource::new();
        source.set_actions(gtk::gdk::DragAction::MOVE);
        let payload = format!(
            "hermes-tab:{}",
            serde_json::to_string(id).expect("Tab ids serialize")
        );
        source.connect_prepare(move |_, _, _| {
            Some(gtk::gdk::ContentProvider::for_value(&payload.to_value()))
        });
        let weak_item = item.downgrade();
        source.connect_drag_begin(move |_, _| {
            if let Some(item) = weak_item.upgrade() {
                item.add_css_class("dragging");
            }
        });
        let weak_item = item.downgrade();
        source.connect_drag_end(move |_, _, _| {
            if let Some(item) = weak_item.upgrade() {
                item.remove_css_class("dragging");
            }
        });
        select.add_controller(source);
        let target = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        target.set_preload(true);
        target.connect_accept(|_, drop| {
            drop.drag().is_some()
                && drop.formats().contains_type(String::static_type())
                && !drop
                    .formats()
                    .contains_type(gtk::gdk::FileList::static_type())
                && !drop.formats().contain_mime_type("text/uri-list")
        });
        let weak_item = item.downgrade();
        let weak = Rc::downgrade(self);
        target.connect_motion(move |target, x, _| {
            let Some(bar) = weak.upgrade() else {
                return gtk::gdk::DragAction::empty();
            };
            let moving = target
                .value()
                .and_then(|value| value.get::<String>().ok())
                .and_then(|value| tab_drag_id(&value));
            if !moving.is_some_and(|id| bar.trails.all().iter().any(|tab| tab.id == id)) {
                return gtk::gdk::DragAction::empty();
            }
            if let Some(item) = weak_item.upgrade() {
                item.remove_css_class("drop-before");
                item.remove_css_class("drop-after");
                item.add_css_class(if x < f64::from(item.width()) / 2.0 {
                    "drop-before"
                } else {
                    "drop-after"
                });
            }
            gtk::gdk::DragAction::MOVE
        });
        let weak_item = item.downgrade();
        target.connect_leave(move |_| {
            if let Some(item) = weak_item.upgrade() {
                item.remove_css_class("drop-before");
                item.remove_css_class("drop-after");
            }
        });
        let destination = id.clone();
        let weak_item = item.downgrade();
        let weak = Rc::downgrade(self);
        target.connect_drop(move |_, value, x, _| {
            let (Some(bar), Some(item)) = (weak.upgrade(), weak_item.upgrade()) else {
                return false;
            };
            item.remove_css_class("drop-before");
            item.remove_css_class("drop-after");
            let Some(moving) = value
                .get::<String>()
                .ok()
                .and_then(|value| tab_drag_id(&value))
            else {
                return false;
            };
            match bar
                .trails
                .move_relative(&moving, &destination, x < f64::from(item.width()) / 2.0)
            {
                Ok(changed) => {
                    if changed {
                        bar.refresh();
                    }
                    true
                }
                Err(error) => {
                    tracing::warn!(%error, "Unable to reorder tabs");
                    false
                }
            }
        });
        item.add_controller(target);
    }
}

fn tab_drag_id(value: &str) -> Option<TrailId> {
    let id = serde_json::from_str::<String>(value.strip_prefix("hermes-tab:")?).ok()?;
    TrailId::new(id)
}

fn visible_tab_scroll(current: f64, page: f64, upper: f64, left: f64, width: f64) -> f64 {
    if page <= 0.0 {
        return current;
    }
    let target = if left < current || width > page {
        left
    } else if left + width > current + page {
        left + width - page
    } else {
        current
    };
    target.clamp(0.0, (upper - page).max(0.0))
}
