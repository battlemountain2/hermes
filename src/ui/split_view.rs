// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::prelude::*;

use crate::ui::browser::BrowserView;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivePane {
    Left,
    Right,
}

type PaneChanged = Rc<dyn Fn(ActivePane)>;
type SplitToggled = Rc<dyn Fn(bool)>;

pub struct SplitView {
    container: gtk::Box,
    paned: gtk::Paned,
    left_wrapper: gtk::Box,
    right_wrapper: gtk::Box,
    is_split: Cell<bool>,
    active_pane: Cell<ActivePane>,
    browser_left: BrowserView,
    browser_right: BrowserView,
    active_pane_handlers: RefCell<Vec<PaneChanged>>,
    split_toggle_handlers: RefCell<Vec<SplitToggled>>,
}

impl SplitView {
    pub fn new(browser_left: BrowserView, browser_right: BrowserView) -> Rc<Self> {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
        paned.set_hexpand(true);
        paned.set_vexpand(true);
        paned.set_wide_handle(true);
        paned.add_css_class("split-paned");

        let left_wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
        left_wrapper.set_hexpand(true);
        left_wrapper.set_vexpand(true);
        left_wrapper.append(&browser_left.widget());

        let right_wrapper = gtk::Box::new(gtk::Orientation::Vertical, 0);
        right_wrapper.set_hexpand(true);
        right_wrapper.set_vexpand(true);
        right_wrapper.append(&browser_right.widget());

        // Single pane by default
        container.append(&left_wrapper);

        let this = Rc::new(Self {
            container,
            paned,
            left_wrapper: left_wrapper.clone(),
            right_wrapper: right_wrapper.clone(),
            is_split: Cell::new(false),
            active_pane: Cell::new(ActivePane::Left),
            browser_left,
            browser_right,
            active_pane_handlers: RefCell::new(Vec::new()),
            split_toggle_handlers: RefCell::new(Vec::new()),
        });

        // Click on left pane activates it
        let split_left = Rc::downgrade(&this);
        let click_left = gtk::GestureClick::new();
        click_left.connect_pressed(move |_, _, _, _| {
            if let Some(split) = split_left.upgrade() {
                split.set_active_pane(ActivePane::Left);
            }
        });
        left_wrapper.add_controller(click_left);

        // Click on right pane activates it
        let split_right = Rc::downgrade(&this);
        let click_right = gtk::GestureClick::new();
        click_right.connect_pressed(move |_, _, _, _| {
            if let Some(split) = split_right.upgrade() {
                split.set_active_pane(ActivePane::Right);
            }
        });
        right_wrapper.add_controller(click_right);

        this
    }

    pub fn widget(&self) -> gtk::Widget {
        self.container.clone().upcast()
    }

    pub fn is_split(&self) -> bool {
        self.is_split.get()
    }

    pub fn active_pane(&self) -> ActivePane {
        self.active_pane.get()
    }

    pub fn active_browser(&self) -> &BrowserView {
        match self.active_pane.get() {
            ActivePane::Left => &self.browser_left,
            ActivePane::Right => &self.browser_right,
        }
    }

    pub fn inactive_browser(&self) -> &BrowserView {
        match self.active_pane.get() {
            ActivePane::Left => &self.browser_right,
            ActivePane::Right => &self.browser_left,
        }
    }

    #[expect(dead_code, reason = "Accessor for left pane browser")]
    pub fn left_browser(&self) -> &BrowserView {
        &self.browser_left
    }

    #[expect(dead_code, reason = "Accessor for right pane browser")]
    pub fn right_browser(&self) -> &BrowserView {
        &self.browser_right
    }

    pub fn set_active_pane(&self, pane: ActivePane) {
        if self.active_pane.get() == pane {
            return;
        }
        self.active_pane.set(pane);
        self.update_active_classes();
        let handlers = self.active_pane_handlers.borrow().clone();
        for handler in handlers {
            handler(pane);
        }
    }

    pub fn switch_pane(&self) {
        if !self.is_split.get() {
            return;
        }
        let next = match self.active_pane.get() {
            ActivePane::Left => ActivePane::Right,
            ActivePane::Right => ActivePane::Left,
        };
        self.set_active_pane(next);
        self.active_browser().widget().grab_focus();
    }

    pub fn toggle_split(&self) {
        self.set_split(!self.is_split.get());
    }

    pub fn set_split(&self, split: bool) {
        if self.is_split.get() == split {
            return;
        }
        self.is_split.set(split);
        if split {
            if let Some(child) = self.container.first_child() {
                self.container.remove(&child);
            }
            self.paned.set_start_child(Some(&self.left_wrapper));
            self.paned.set_end_child(Some(&self.right_wrapper));
            self.container.append(&self.paned);

            self.left_wrapper.add_css_class("split-pane");
            self.right_wrapper.add_css_class("split-pane");
            self.update_active_classes();

            let width = self.container.width();
            let pos = if width > 200 { width / 2 } else { 500 };
            self.paned.set_position(pos);

            // Navigate right pane to left pane's current directory if empty
            if self.browser_right.browser().active_location().is_none()
                && let Some(loc) = self.browser_left.browser().active_location()
            {
                self.browser_right.browser().navigate(loc);
            }
        } else {
            self.paned.set_start_child(None::<&gtk::Widget>);
            self.paned.set_end_child(None::<&gtk::Widget>);
            if let Some(child) = self.container.first_child() {
                self.container.remove(&child);
            }

            // Keep the active pane's location when collapsing back to single pane
            if self.active_pane.get() == ActivePane::Right
                && let Some(loc) = self.browser_right.browser().active_location()
            {
                self.browser_left.browser().navigate(loc);
            }

            self.active_pane.set(ActivePane::Left);
            self.left_wrapper.remove_css_class("split-pane");
            self.left_wrapper.remove_css_class("active-pane");
            self.right_wrapper.remove_css_class("split-pane");
            self.right_wrapper.remove_css_class("active-pane");
            self.container.append(&self.left_wrapper);
        }

        let toggle_handlers = self.split_toggle_handlers.borrow().clone();
        for handler in toggle_handlers {
            handler(split);
        }
        let pane_handlers = self.active_pane_handlers.borrow().clone();
        for handler in pane_handlers {
            handler(self.active_pane.get());
        }
    }

    fn update_active_classes(&self) {
        if !self.is_split.get() {
            self.left_wrapper.remove_css_class("active-pane");
            self.right_wrapper.remove_css_class("active-pane");
            return;
        }
        match self.active_pane.get() {
            ActivePane::Left => {
                self.left_wrapper.add_css_class("active-pane");
                self.right_wrapper.remove_css_class("active-pane");
            }
            ActivePane::Right => {
                self.left_wrapper.remove_css_class("active-pane");
                self.right_wrapper.add_css_class("active-pane");
            }
        }
    }

    pub fn connect_active_pane_changed<F: Fn(ActivePane) + 'static>(&self, handler: F) {
        self.active_pane_handlers
            .borrow_mut()
            .push(Rc::new(handler));
    }

    pub fn connect_split_toggled<F: Fn(bool) + 'static>(&self, handler: F) {
        self.split_toggle_handlers
            .borrow_mut()
            .push(Rc::new(handler));
    }

    pub fn copy_to_opposite_pane(&self) -> bool {
        if !self.is_split.get() {
            return false;
        }
        let active = self.active_browser();
        let inactive = self.inactive_browser();
        let Some(dest) = inactive.browser().active_location() else {
            return false;
        };
        if active.copy_selection() {
            inactive.paste_into(dest);
            true
        } else {
            false
        }
    }

    pub fn move_to_opposite_pane(&self) -> bool {
        if !self.is_split.get() {
            return false;
        }
        let active = self.active_browser();
        let inactive = self.inactive_browser();
        let Some(dest) = inactive.browser().active_location() else {
            return false;
        };
        if active.cut_selection() {
            inactive.paste_into(dest);
            true
        } else {
            false
        }
    }
}
