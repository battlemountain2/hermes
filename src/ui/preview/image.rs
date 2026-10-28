use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

use gtk::{glib, prelude::*};

use crate::assets::{self, icons};
use crate::model::FileEntry;

use super::{ImageContextMenuHandler, ImageMenuActions};

#[cfg(test)]
mod tests;

pub(super) struct ImagePreview {
    pub widget: gtk::Box,
    fullscreen: Rc<RefCell<Option<gtk::Window>>>,
    expand: gtk::Button,
}

#[derive(Clone, Copy)]
struct ZoomTarget {
    scale: f64,
    point: (f64, f64),
    pointer: (f64, f64),
}

#[derive(Clone)]
struct ImagePixels {
    pixels: gtk::gdk_pixbuf::Pixbuf,
    surface: Option<gtk::cairo::ImageSurface>,
}

impl ImagePixels {
    fn new(pixels: gtk::gdk_pixbuf::Pixbuf) -> Self {
        let surface = gtk::cairo::ImageSurface::create(
            gtk::cairo::Format::ARgb32,
            pixels.width(),
            pixels.height(),
        )
        .ok()
        .and_then(|surface| {
            let context = gtk::cairo::Context::new(&surface).ok()?;
            context.set_source_pixbuf(&pixels, 0.0, 0.0);
            context.paint().ok()?;
            Some(surface)
        });
        Self { pixels, surface }
    }
}

impl ImagePreview {
    pub fn new(
        texture: &gtk::gdk::Texture,
        context_menu: Option<(FileEntry, ImageContextMenuHandler)>,
    ) -> Self {
        Self::from_texture(texture, context_menu, false)
    }

    pub(super) fn new_loading(texture: &gtk::gdk::Texture) -> Self {
        Self::from_texture(texture, None, texture.width().max(texture.height()) >= 256)
    }

    fn from_texture(
        texture: &gtk::gdk::Texture,
        context_menu: Option<(FileEntry, ImageContextMenuHandler)>,
        upscale_to_fit: bool,
    ) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.set_vexpand(true);
        widget.add_css_class("hermes-image-player");
        let mut downloader = gtk::gdk::TextureDownloader::new(texture);
        downloader.set_format(gtk::gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let pixels = ImagePixels::new(gtk::gdk_pixbuf::Pixbuf::from_bytes(
            &bytes,
            gtk::gdk_pixbuf::Colorspace::Rgb,
            true,
            8,
            texture.width(),
            texture.height(),
            stride as i32,
        ));
        let (scroll, controls) = image_view(&pixels, upscale_to_fit);
        if let Some((entry, _)) = context_menu.as_ref() {
            let image_drag = gtk::DragSource::builder()
                .actions(gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE)
                .build();
            let drag_entry = entry.clone();
            image_drag.connect_prepare(move |source, _x, _y| {
                crate::ui::browser::setup_drag_icon(source, &drag_entry, 1);
                crate::ui::browser::file_drag_content(&[drag_entry.clone()])
            });
            scroll.add_controller(image_drag);
        }
        widget.append(&scroll);
        let expand = image_action(icons::MAXIMIZE, "Fullscreen image (F)");
        controls.append(&expand);
        install_context_menu(&scroll, &controls, &expand, false, context_menu.clone());
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
            let (scroll, controls) = image_view(&pixels, false);
            content.append(&scroll);
            let exit = image_action(icons::MINIMIZE, "Exit fullscreen (F or Escape)");
            controls.append(&exit);
            install_context_menu(&scroll, &controls, &exit, true, context_menu.clone());
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
        if let Some(viewport) = self.widget.first_child() {
            dismiss_context_menu(&viewport);
        }
        let window = self.fullscreen.borrow_mut().take();
        if let Some(window) = window {
            window.close();
        }
    }
}

impl Drop for ImagePreview {
    fn drop(&mut self) {
        self.stop();
    }
}

fn install_context_menu(
    viewport: &gtk::Overlay,
    controls: &gtk::Box,
    fullscreen: &gtk::Button,
    exiting_fullscreen: bool,
    context_menu: Option<(FileEntry, ImageContextMenuHandler)>,
) {
    let Some((entry, handler)) = context_menu else {
        return;
    };
    let Some(fit) = controls.first_child().and_downcast::<gtk::Button>() else {
        return;
    };
    let Some(actual) = fit.next_sibling().and_downcast::<gtk::Button>() else {
        return;
    };
    let actions = ImageMenuActions {
        fit: fit.downgrade(),
        actual: actual.downgrade(),
        fullscreen: fullscreen.downgrade(),
        exiting_fullscreen,
    };
    let click = gtk::GestureClick::new();
    click.set_name(Some("hermes-image-context-menu"));
    click.set_button(3);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let clicked_entry = entry.clone();
    let clicked_actions = actions.clone();
    let clicked_handler = handler.clone();
    click.connect_pressed(move |gesture, _, horizontal, vertical| {
        if let Some(viewport) = gesture.widget() {
            clicked_handler(
                &viewport,
                clicked_entry.clone(),
                clicked_actions.clone(),
                horizontal,
                vertical,
            );
            gesture.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    viewport.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.set_name(Some("hermes-image-context-keys"));
    keys.connect_key_pressed(move |keys, key, _, modifiers| {
        if key != gtk::gdk::Key::Menu
            && !(key == gtk::gdk::Key::F10
                && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK))
        {
            return glib::Propagation::Proceed;
        }
        if let Some(viewport) = keys.widget() {
            handler(&viewport, entry.clone(), actions.clone(), 24.0, 24.0);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    viewport.add_controller(keys);
    viewport.set_focusable(true);
    viewport.connect_unmap(|viewport| dismiss_context_menu(viewport.upcast_ref()));
    viewport.connect_destroy(|viewport| dismiss_context_menu(viewport.upcast_ref()));
}

pub(super) fn dismiss_context_menu(widget: &gtk::Widget) {
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Ok(menu) = current.downcast::<gtk::Popover>()
            && menu.has_css_class("image-context-menu")
        {
            menu.popdown();
            if menu.parent().is_some() {
                menu.unparent();
            }
        }
    }
}

fn image_action(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text(tooltip).build();
    button.add_css_class("hermes-media-action");
    button.set_child(Some(&assets::primary_icon(icon, 16)));
    button
}

fn image_view(pixels: &ImagePixels, upscale_to_fit: bool) -> (gtk::Overlay, gtk::Box) {
    let width = f64::from(pixels.pixels.width());
    let height = f64::from(pixels.pixels.height());
    let zoom = Rc::new(Cell::new(1.0));
    let fitting = Rc::new(Cell::new(true));
    let pending = Rc::new(Cell::new(None::<(f64, f64)>));
    let target = Rc::new(Cell::new(None::<ZoomTarget>));
    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&area)
        .hexpand(true)
        .vexpand(true)
        .build();
    let canvas = gtk::DrawingArea::new();
    canvas.add_css_class("hermes-image-canvas");
    canvas.set_can_target(false);
    canvas.set_hexpand(true);
    canvas.set_vexpand(true);
    let viewport = gtk::Overlay::new();
    viewport.set_hexpand(true);
    viewport.set_vexpand(true);
    viewport.set_child(Some(&scroll));
    viewport.add_overlay(&canvas);
    let draw_zoom = zoom.clone();
    let draw_fitting = fitting.clone();
    let draw_pending = pending.clone();
    let weak_scroll = scroll.downgrade();
    let pixels = pixels.clone();
    canvas.set_draw_func(move |canvas, context, canvas_width, canvas_height| {
        let Some(scroll) = weak_scroll.upgrade() else {
            return;
        };
        let horizontal = scroll.hadjustment();
        let vertical = scroll.vadjustment();
        let (offset_x, offset_y) = draw_pending
            .get()
            .unwrap_or((horizontal.value(), vertical.value()));
        let viewport = (
            if horizontal.page_size() > 0.0 {
                horizontal.page_size()
            } else {
                f64::from(canvas_width)
            },
            if vertical.page_size() > 0.0 {
                vertical.page_size()
            } else {
                f64::from(canvas_height)
            },
        );
        let scale = if draw_fitting.get() {
            initial_fit(
                f64::from(pixels.pixels.width()),
                f64::from(pixels.pixels.height()),
                viewport.0,
                viewport.1,
                upscale_to_fit,
            )
        } else {
            draw_zoom.get()
        };
        let background = canvas.color();
        paint_image(
            context,
            &pixels,
            scale,
            viewport,
            (offset_x, offset_y),
            &background,
        );
    });
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.add_css_class("hermes-media-controls");
    let fit = gtk::Button::with_label("Fit");
    fit.add_css_class("hermes-media-action");
    fit.set_tooltip_text(Some(
        "Fit image to the available space · Scroll to zoom · Drag to pan",
    ));
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
    let fit_target = target.clone();
    fit.connect_clicked(move |_| {
        fit_target.set(None);
        fit_mode.set(true);
    });
    let actual_zoom = zoom.clone();
    let actual_fit = fitting.clone();
    let actual_pending = pending.clone();
    let actual_target = target.clone();
    let weak_area = area.downgrade();
    let weak_scroll = scroll.downgrade();
    actual.connect_clicked(move |_| {
        if let (Some(area), Some(scroll)) = (weak_area.upgrade(), weak_scroll.upgrade()) {
            actual_fit.set(false);
            actual_target.set(None);
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
    let tick_target = target.clone();
    let last_tick = Cell::new(Instant::now());
    let weak_area = area.downgrade();
    let weak_percentage = percentage.downgrade();
    let weak_canvas = canvas.downgrade();
    let last_draw = Cell::new(None::<(f64, f64, f64, f64, f64, f32, f32, f32)>);
    let tick_scroll = scroll.downgrade();
    viewport.add_tick_callback(move |_, _| {
        let Some(scroll) = tick_scroll.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let Some(area) = weak_area.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let horizontal = scroll.hadjustment();
        let vertical = scroll.vadjustment();
        let view_width = horizontal.page_size();
        let view_height = vertical.page_size();
        let elapsed = last_tick.replace(Instant::now()).elapsed().as_secs_f64();
        if view_width <= 0.0 || view_height <= 0.0 {
            return glib::ControlFlow::Continue;
        }
        if tick_fitting.get() {
            let scale = initial_fit(width, height, view_width, view_height, upscale_to_fit);
            if (scale - tick_zoom.get()).abs() > 0.000001 || area.content_width() == 0 {
                tick_zoom.set(scale);
                resize_image(&area, width, height, scale);
                tick_pending.set(Some((0.0, 0.0)));
            }
        } else if let Some(target) = tick_target.get() {
            let scale = if super::super::motion::animations_enabled() {
                smooth_zoom(tick_zoom.get(), target.scale, elapsed)
            } else {
                target.scale
            };
            tick_zoom.set(scale);
            resize_image(&area, width, height, scale);
            tick_pending.set(Some((
                offset_for_point(target.point.0, target.pointer.0, view_width, width * scale),
                offset_for_point(
                    target.point.1,
                    target.pointer.1,
                    view_height,
                    height * scale,
                ),
            )));
            if scale == target.scale {
                tick_target.set(None);
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
        let (offset_x, offset_y) = tick_pending
            .get()
            .unwrap_or((horizontal.value(), vertical.value()));
        if let Some(canvas) = weak_canvas.upgrade() {
            let color = canvas.color();
            let draw = (
                tick_zoom.get(),
                offset_x,
                offset_y,
                view_width,
                view_height,
                color.red(),
                color.green(),
                color.blue(),
            );
            if last_draw.replace(Some(draw)) != Some(draw) {
                canvas.queue_draw();
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
    let wheel_pending = pending.clone();
    let wheel_target = target.clone();
    let pan_zoom = zoom.clone();
    wheel.connect_scroll(move |_, _, delta| {
        let Some(scroll) = weak_scroll.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let previous = zoom.get();
        let next = scroll_zoom(
            wheel_target.get().map_or(previous, |target| target.scale),
            delta,
        );
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
        wheel_target.set(Some(ZoomTarget {
            scale: next,
            point: (
                point_under_pointer(old_x, pointer_x, horizontal.page_size(), width * previous),
                point_under_pointer(old_y, pointer_y, vertical.page_size(), height * previous),
            ),
            pointer: (pointer_x, pointer_y),
        }));
        fitting.set(false);
        glib::Propagation::Stop
    });
    scroll.add_controller(wheel);
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    let start = Rc::new(Cell::new((0.0, 0.0)));
    let weak_scroll = scroll.downgrade();
    let drag_start = start.clone();
    let drag_pending = pending.clone();
    drag.connect_drag_begin(move |_, _, _| {
        target.set(None);
        if let Some(scroll) = weak_scroll.upgrade() {
            drag_start.set(
                drag_pending
                    .get()
                    .unwrap_or((scroll.hadjustment().value(), scroll.vadjustment().value())),
            );
        }
    });
    let weak_scroll = scroll.downgrade();
    drag.connect_drag_update(move |_, horizontal, vertical| {
        if let Some(scroll) = weak_scroll.upgrade() {
            let (start_x, start_y) = start.get();
            pending.set(Some((
                (start_x - horizontal).clamp(
                    0.0,
                    (width * pan_zoom.get() - scroll.hadjustment().page_size()).max(0.0),
                ),
                (start_y - vertical).clamp(
                    0.0,
                    (height * pan_zoom.get() - scroll.vadjustment().page_size()).max(0.0),
                ),
            )));
        }
    });
    scroll.add_controller(drag);
    (viewport, controls)
}

fn resize_image(area: &gtk::DrawingArea, width: f64, height: f64, scale: f64) {
    area.set_content_width((width * scale).round().max(1.0) as i32);
    area.set_content_height((height * scale).round().max(1.0) as i32);
    area.queue_draw();
}

fn paint_image(
    context: &gtk::cairo::Context,
    pixels: &ImagePixels,
    scale: f64,
    viewport: (f64, f64),
    offset: (f64, f64),
    background: &gtk::gdk::RGBA,
) {
    let _ = context.save();
    context.set_operator(gtk::cairo::Operator::Clear);
    let _ = context.paint();
    context.rectangle(0.0, 0.0, viewport.0, viewport.1);
    context.clip();
    context.set_operator(gtk::cairo::Operator::Source);
    context.set_source_rgb(
        f64::from(background.red()),
        f64::from(background.green()),
        f64::from(background.blue()),
    );
    let _ = context.paint();
    context.set_operator(gtk::cairo::Operator::Over);
    context.translate(
        ((viewport.0 - f64::from(pixels.pixels.width()) * scale) / 2.0).max(0.0) - offset.0,
        ((viewport.1 - f64::from(pixels.pixels.height()) * scale) / 2.0).max(0.0) - offset.1,
    );
    context.scale(scale, scale);
    if let Some(surface) = pixels.surface.as_ref() {
        let _ = context.set_source_surface(surface, 0.0, 0.0);
    } else {
        context.set_source_pixbuf(&pixels.pixels, 0.0, 0.0);
    }
    context.source().set_filter(gtk::cairo::Filter::Bilinear);
    let _ = context.paint();
    let _ = context.restore();
}

fn fit_zoom(width: f64, height: f64, viewport_width: f64, viewport_height: f64) -> f64 {
    (viewport_width / width)
        .min(viewport_height / height)
        .min(1.0)
}

fn initial_fit(
    width: f64,
    height: f64,
    viewport_width: f64,
    viewport_height: f64,
    upscale: bool,
) -> f64 {
    if upscale {
        (viewport_width / width).min(viewport_height / height)
    } else {
        fit_zoom(width, height, viewport_width, viewport_height)
    }
}

fn scroll_zoom(previous: f64, delta: f64) -> f64 {
    if !delta.is_finite() {
        return previous;
    }
    (previous * 1.15_f64.powf(-delta.clamp(-10.0, 10.0))).clamp(0.000001, 10.0)
}

#[cfg(test)]
fn anchored_offset(offset: f64, pointer: f64, viewport: f64, previous: f64, next: f64) -> f64 {
    offset_for_point(
        point_under_pointer(offset, pointer, viewport, previous),
        pointer,
        viewport,
        next,
    )
}

fn point_under_pointer(offset: f64, pointer: f64, viewport: f64, extent: f64) -> f64 {
    let padding = ((viewport - extent) / 2.0).max(0.0);
    ((offset + pointer - padding) / extent).clamp(0.0, 1.0)
}

fn offset_for_point(point: f64, pointer: f64, viewport: f64, next: f64) -> f64 {
    let new_padding = ((viewport - next) / 2.0).max(0.0);
    (point * next + new_padding - pointer).clamp(0.0, (next - viewport).max(0.0))
}

fn smooth_zoom(current: f64, target: f64, elapsed: f64) -> f64 {
    let next = current + (target - current) * (1.0 - (-elapsed.clamp(0.0, 0.05) / 0.035).exp());
    if (next - target).abs() < 0.00001 {
        target
    } else {
        next
    }
}
