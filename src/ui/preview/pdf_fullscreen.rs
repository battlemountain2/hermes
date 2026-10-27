use crate::assets::{self, icons};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

pub(super) struct PdfFullscreen {
    pub widget: gtk::Box,
    window: Rc<RefCell<Option<gtk::Window>>>,
    expand: gtk::Button,
}

impl PdfFullscreen {
    pub fn new(scroll: &gtk::ScrolledWindow, title: &str) -> Self {
        let home = gtk::Box::new(gtk::Orientation::Vertical, 0);
        home.set_vexpand(true);
        home.set_hexpand(true);
        let viewer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        viewer.add_css_class("hermes-pdf-reader");
        viewer.set_vexpand(true);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        controls.add_css_class("hermes-media-controls");
        let name = gtk::Label::new(Some(title));
        name.add_css_class("music-title");
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        name.set_xalign(0.0);
        name.set_hexpand(true);
        controls.append(&name);
        let hint = gtk::Label::new(Some("Ctrl+scroll: zoom · Ctrl+0: fit"));
        hint.add_css_class("music-detail");
        controls.append(&hint);
        let expand = gtk::Button::builder()
            .tooltip_text("Fullscreen document (F)")
            .build();
        expand.add_css_class("hermes-media-action");
        expand.set_child(Some(&assets::primary_icon(icons::MAXIMIZE, 16)));
        controls.append(&expand);
        viewer.append(&controls);
        viewer.append(scroll);
        home.append(&viewer);
        let slot = Rc::new(RefCell::new(None::<gtk::Window>));
        let shared_slot = slot.clone();
        let weak_home = home.downgrade();
        let weak_viewer = viewer.downgrade();
        let weak_scroll = scroll.downgrade();
        expand.connect_clicked(move |button| {
            let existing = shared_slot.borrow().clone();
            if let Some(window) = existing {
                window.close();
                return;
            }
            let (Some(home), Some(viewer), Some(scroll)) = (
                weak_home.upgrade(),
                weak_viewer.upgrade(),
                weak_scroll.upgrade(),
            ) else {
                return;
            };
            let window = gtk::Window::builder()
                .title("Hermes — Document reader")
                .destroy_with_parent(true)
                .build();
            if let Some(parent) = home.root().and_downcast::<gtk::Window>() {
                window.set_transient_for(Some(&parent));
                window.set_application(parent.application().as_ref());
            }
            let fraction = scroll_fraction(&scroll);
            home.remove(&viewer);
            window.set_child(Some(&viewer));
            button.set_child(Some(&assets::primary_icon(icons::MINIMIZE, 16)));
            button.set_tooltip_text(Some("Exit fullscreen (F or Escape)"));
            let weak_slot = Rc::downgrade(&shared_slot);
            let weak_home = home.downgrade();
            let weak_viewer = viewer.downgrade();
            let weak_scroll = scroll.downgrade();
            let weak_button = button.downgrade();
            window.connect_close_request(move |window| {
                let scroll = weak_scroll.upgrade();
                let fraction = scroll.as_ref().map(scroll_fraction);
                window.set_child(None::<&gtk::Widget>);
                if let (Some(home), Some(viewer)) = (weak_home.upgrade(), weak_viewer.upgrade()) {
                    if viewer.parent().is_none() {
                        home.append(&viewer);
                    }
                }
                if let Some(button) = weak_button.upgrade() {
                    button.set_child(Some(&assets::primary_icon(icons::MAXIMIZE, 16)));
                    button.set_tooltip_text(Some("Fullscreen PDF (F)"));
                }
                if let Some(slot) = weak_slot.upgrade() {
                    slot.borrow_mut().take();
                }
                if let (Some(scroll), Some(fraction)) = (scroll, fraction) {
                    restore_fraction(&scroll, fraction);
                }
                glib::Propagation::Proceed
            });
            let keys = gtk::EventControllerKey::new();
            keys.set_name(Some("hermes-pdf-fullscreen"));
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let weak_window = window.downgrade();
            keys.connect_key_pressed(move |_, key, _, modifiers| {
                if key == gtk::gdk::Key::Escape || super::media::fullscreen_shortcut(key, modifiers)
                {
                    if let Some(window) = weak_window.upgrade() {
                        window.close();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(keys);
            shared_slot.replace(Some(window.clone()));
            window.fullscreen();
            window.present();
            scroll.grab_focus();
            restore_fraction(&scroll, fraction);
        });
        Self {
            widget: home,
            window: slot,
            expand,
        }
    }

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

fn scroll_fraction(scroll: &gtk::ScrolledWindow) -> f64 {
    let adjustment = scroll.vadjustment();
    adjustment.value() / adjustment.upper().max(1.0)
}

impl Drop for PdfFullscreen {
    fn drop(&mut self) {
        self.stop();
    }
}

fn restore_fraction(scroll: &gtk::ScrolledWindow, fraction: f64) {
    let frames = Cell::new(0);
    scroll.add_tick_callback(move |scroll, _| {
        frames.set(frames.get() + 1);
        if frames.get() < 3 {
            return glib::ControlFlow::Continue;
        }
        let adjustment = scroll.vadjustment();
        adjustment.set_value((fraction * adjustment.upper()).clamp(
            adjustment.lower(),
            (adjustment.upper() - adjustment.page_size()).max(adjustment.lower()),
        ));
        glib::ControlFlow::Break
    });
}

#[cfg(test)]
mod tests;
