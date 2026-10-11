use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::assets::{self, icons};

#[cfg(test)]
mod tests;

pub(super) struct ImagePreview {
    pub widget: gtk::Box,
    fullscreen: Rc<RefCell<Option<gtk::Window>>>,
    expand: gtk::Button,
}

impl ImagePreview {
    pub fn new(texture: &gtk::gdk::Texture) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.set_vexpand(true);
        widget.add_css_class("hermes-image-player");
        let mut downloader = gtk::gdk::TextureDownloader::new(texture);
        downloader.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let pixels = gtk::gdk_pixbuf::Pixbuf::from_bytes(
            &bytes,
            gtk::gdk_pixbuf::Colorspace::Rgb,
            true,
            8,
            texture.width(),
            texture.height(),
            stride as i32,
        );
        let (scroll, controls) = image_view(&pixels);
        widget.append(&scroll);
        let expand = image_action(icons::MAXIMIZE, "Fullscreen image (F)");
        controls.append(&expand);
        widget.append(&controls);
        let fullscreen = Rc::new(RefCell::new(None::<gtk::Window>));
        let slot = fullscreen.clone();
        let weak_widget = widget.downgrade();
        expand.connect_clicked(move |_| {
            if let Some(window) = slot.borrow().as_ref() {
                window.present();
                return;
            }
            let window = gtk::Window::builder()
                .title("Hermes — Image preview")
                .build();
            if let Some(parent) = weak_widget
                .upgrade()
                .and_then(|widget| widget.root())
                .and_then(|root| root.downcast::<gtk::Window>().ok())
            {
                window.set_transient_for(Some(&parent));
                window.set_application(parent.application().as_ref());
            }
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            content.add_css_class("hermes-image-player");
            let (scroll, controls) = image_view(&pixels);
            content.append(&scroll);
            let exit = image_action(icons::MINIMIZE, "Exit fullscreen (F or Escape)");
            controls.append(&exit);
            content.append(&controls);
            let weak_window = window.downgrade();
            exit.connect_clicked(move |_| {
                if let Some(window) = weak_window.upgrade() {
                    window.close();
                }
            });
            let keys = gtk::EventControllerKey::new();
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
            let weak_slot = Rc::downgrade(&slot);
            window.connect_close_request(move |_| {
                if let Some(slot) = weak_slot.upgrade() {
                    slot.borrow_mut().take();
                }
                glib::Propagation::Proceed
            });
            window.set_child(Some(&content));
            slot.replace(Some(window.clone()));
            window.fullscreen();
            window.present();
        });
        Self {
            widget,
            fullscreen,
            expand,
        }
    }

    pub fn open_fullscreen(&self) -> bool {
        self.expand.emit_clicked();
        true
    }

    pub fn stop(&self) {
        let window = self.fullscreen.borrow_mut().take();
        if let Some(window) = window {
            window.close();
        }
    }
}

fn image_action(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text(tooltip).build();
    button.add_css_class("hermes-media-action");
    button.set_child(Some(&assets::primary_icon(icon, 16)));
    button
}

fn image_view(pixels: &gtk::gdk_pixbuf::Pixbuf) -> (gtk::ScrolledWindow, gtk::Box) {
    let width = f64::from(pixels.width());
    let height = f64::from(pixels.height());
    let zoom = Rc::new(Cell::new(1.0));
    let fitting = Rc::new(Cell::new(true));
    let pending = Rc::new(Cell::new(None::<(f64, f64)>));
    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    let draw_zoom = zoom.clone();
    let pixels = pixels.clone();
    area.set_draw_func(move |_, context, allocated_width, allocated_height| {
        let scale = draw_zoom.get();
        context.translate(
            ((f64::from(allocated_width) - width * scale) / 2.0).max(0.0),
            ((f64::from(allocated_height) - height * scale) / 2.0).max(0.0),
        );
        context.scale(scale, scale);
        context.set_source_pixbuf(&pixels, 0.0, 0.0);
        let _ = context.paint();
    });
    let scroll = gtk::ScrolledWindow::builder()
        .child(&area)
        .hexpand(true)
        .vexpand(true)
        .build();
    scroll.set_tooltip_text(Some("Scroll to zoom · Drag to pan · Fit resets the view"));
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.add_css_class("hermes-media-controls");
    let fit = gtk::Button::with_label("Fit");
    fit.add_css_class("hermes-media-action");
    fit.set_tooltip_text(Some("Fit image to the available space"));
    let actual = gtk::Button::with_label("100%");
    actual.add_css_class("hermes-media-action");
    actual.set_tooltip_text(Some("Show the preview at its rendered size"));
    let percentage = gtk::Label::new(None);
    percentage.add_css_class("hermes-image-zoom-label");
    percentage.set_hexpand(true);
    percentage.set_xalign(0.0);
    controls.append(&fit);
    controls.append(&actual);
    controls.append(&percentage);
    let fit_mode = fitting.clone();
    fit.connect_clicked(move |_| {
        fit_mode.set(true);
    });
    let actual_zoom = zoom.clone();
    let actual_fit = fitting.clone();
    let actual_pending = pending.clone();
    let weak_area = area.downgrade();
    let weak_scroll = scroll.downgrade();
    actual.connect_clicked(move |_| {
        if let (Some(area), Some(scroll)) = (weak_area.upgrade(), weak_scroll.upgrade()) {
            actual_fit.set(false);
            actual_zoom.set(1.0);
            resize_image(&area, width, height, 1.0);
            actual_pending.set(Some((
                (width - scroll.hadjustment().page_size()) / 2.0,
                (height - scroll.vadjustment().page_size()) / 2.0,
            )));
        }
    });
    let tick_zoom = zoom.clone();
    let tick_fitting = fitting.clone();
    let tick_pending = pending.clone();
    let weak_area = area.downgrade();
    let weak_percentage = percentage.downgrade();
    scroll.add_tick_callback(move |scroll, _| {
        let Some(area) = weak_area.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let horizontal = scroll.hadjustment();
        let vertical = scroll.vadjustment();
        let view_width = horizontal.page_size();
        let view_height = vertical.page_size();
        if view_width <= 0.0 || view_height <= 0.0 {
            return glib::ControlFlow::Continue;
        }
        if tick_fitting.get() {
            let scale = fit_zoom(width, height, view_width, view_height);
            if (scale - tick_zoom.get()).abs() > 0.000001 || area.content_width() == 0 {
                tick_zoom.set(scale);
                resize_image(&area, width, height, scale);
                tick_pending.set(Some((0.0, 0.0)));
            }
        }
        if let Some((horizontal_value, vertical_value)) = tick_pending.get() {
            let expected_width = (width * tick_zoom.get()).round().max(view_width);
            let expected_height = (height * tick_zoom.get()).round().max(view_height);
            if (horizontal.upper() - expected_width).abs() <= 1.0
                && (vertical.upper() - expected_height).abs() <= 1.0
            {
                super::set_adjustment_value(&horizontal, horizontal_value);
                super::set_adjustment_value(&vertical, vertical_value);
                tick_pending.set(None);
            }
        }
        if let Some(percentage) = weak_percentage.upgrade() {
            let text = format!("{:.0}%", tick_zoom.get() * 100.0);
            if percentage.text() != text {
                percentage.set_text(&text);
            }
        }
        glib::ControlFlow::Continue
    });
    let pointer = Rc::new(Cell::new(None::<(f64, f64)>));
    let motion = gtk::EventControllerMotion::new();
    motion.set_name(Some("hermes-image-pointer"));
    let motion_pointer = pointer.clone();
    motion.connect_motion(move |_, horizontal, vertical| {
        motion_pointer.set(Some((horizontal, vertical)));
    });
    let leave_pointer = pointer.clone();
    motion.connect_leave(move |_| {
        leave_pointer.set(None);
    });
    scroll.add_controller(motion);
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    wheel.set_name(Some("hermes-image-zoom"));
    let weak_scroll = scroll.downgrade();
    let weak_area = area.downgrade();
    let wheel_pending = pending.clone();
    wheel.connect_scroll(move |_, _, delta| {
        let (Some(scroll), Some(area)) = (weak_scroll.upgrade(), weak_area.upgrade()) else {
            return glib::Propagation::Proceed;
        };
        let previous = zoom.get();
        let next = scroll_zoom(previous, delta);
        if previous == next {
            return glib::Propagation::Stop;
        }
        let horizontal = scroll.hadjustment();
        let vertical = scroll.vadjustment();
        let (pointer_x, pointer_y) = pointer
            .get()
            .unwrap_or((horizontal.page_size() / 2.0, vertical.page_size() / 2.0));
        let (old_x, old_y) = wheel_pending
            .get()
            .unwrap_or((horizontal.value(), vertical.value()));
        wheel_pending.set(Some((
            anchored_offset(
                old_x,
                pointer_x,
                horizontal.page_size(),
                width * previous,
                width * next,
            ),
            anchored_offset(
                old_y,
                pointer_y,
                vertical.page_size(),
                height * previous,
                height * next,
            ),
        )));
        fitting.set(false);
        zoom.set(next);
        resize_image(&area, width, height, next);
        glib::Propagation::Stop
    });
    scroll.add_controller(wheel);
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    let start = Rc::new(Cell::new((0.0, 0.0)));
    let weak_scroll = scroll.downgrade();
    let drag_start = start.clone();
    drag.connect_drag_begin(move |_, _, _| {
        if let Some(scroll) = weak_scroll.upgrade() {
            drag_start.set((scroll.hadjustment().value(), scroll.vadjustment().value()));
        }
    });
    let weak_scroll = scroll.downgrade();
    drag.connect_drag_update(move |_, horizontal, vertical| {
        if let Some(scroll) = weak_scroll.upgrade() {
            pending.set(None);
            let (start_x, start_y) = start.get();
            super::set_adjustment_value(&scroll.hadjustment(), start_x - horizontal);
            super::set_adjustment_value(&scroll.vadjustment(), start_y - vertical);
        }
    });
    scroll.add_controller(drag);
    (scroll, controls)
}

fn resize_image(area: &gtk::DrawingArea, width: f64, height: f64, scale: f64) {
    area.set_content_width((width * scale).round().max(1.0) as i32);
    area.set_content_height((height * scale).round().max(1.0) as i32);
    area.queue_draw();
}

fn fit_zoom(width: f64, height: f64, viewport_width: f64, viewport_height: f64) -> f64 {
    (viewport_width / width)
        .min(viewport_height / height)
        .min(1.0)
}

fn scroll_zoom(previous: f64, delta: f64) -> f64 {
    if !delta.is_finite() {
        return previous;
    }
    (previous * 1.15_f64.powf(-delta.clamp(-10.0, 10.0))).clamp(0.000001, 10.0)
}

fn anchored_offset(offset: f64, pointer: f64, viewport: f64, previous: f64, next: f64) -> f64 {
    let old_padding = ((viewport - previous) / 2.0).max(0.0);
    let new_padding = ((viewport - next) / 2.0).max(0.0);
    let image_point = ((offset + pointer - old_padding) / previous).clamp(0.0, 1.0);
    (image_point * next + new_padding - pointer).clamp(0.0, (next - viewport).max(0.0))
}
