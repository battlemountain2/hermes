use std::{
    cell::{Cell, RefCell},
    f64::consts::PI,
    rc::Rc,
    time::Instant,
};

use gtk::{glib, prelude::*};

use crate::ui::{motion::animations_enabled, theme::ThemeManager};

const TARGET_RPM: f64 = 33.3333333;
const TARGET_ANGULAR_VELOCITY: f64 = TARGET_RPM * 2.0 * PI / 60.0; // ~3.49 rad/s
const ACCELERATION_RATE: f64 = 6.0;
const DECELERATION_RATE: f64 = 4.5;

pub struct VinylDisc {
    widget: gtk::DrawingArea,
    animation: Rc<VinylAnimation>,
}

#[derive(Clone, Copy, Default)]
struct Rotation {
    angle: f64,
    velocity: f64,
}

impl Rotation {
    fn advance(&mut self, playing: bool, elapsed: f64) -> bool {
        let elapsed = elapsed.min(0.1);
        let target_velocity = if playing {
            TARGET_ANGULAR_VELOCITY
        } else {
            0.0
        };
        let rate = if target_velocity > self.velocity {
            ACCELERATION_RATE
        } else {
            DECELERATION_RATE
        };
        self.velocity += (target_velocity - self.velocity) * (elapsed * rate).min(1.0);

        if self.velocity > 0.001 {
            self.angle = (self.angle + self.velocity * elapsed) % (2.0 * PI);
            true
        } else if playing {
            true
        } else {
            self.velocity = 0.0;
            false
        }
    }
}

#[derive(Default)]
struct VinylAnimation {
    rotation: Cell<Rotation>,
    playing: Cell<bool>,
    stopped: Cell<bool>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    last_tick: Cell<Option<Instant>>,
}

impl VinylDisc {
    pub fn new() -> Self {
        let widget = gtk::DrawingArea::new();
        widget.add_css_class("preview-audio-vinyl");
        widget.set_content_width(180);
        widget.set_content_height(180);
        widget.set_size_request(180, 180);
        widget.set_hexpand(false);
        widget.set_vexpand(false);
        widget.set_halign(gtk::Align::Center);
        widget.set_valign(gtk::Align::Center);

        let animation = Rc::new(VinylAnimation::default());

        let draw_animation = Rc::downgrade(&animation);
        widget.set_draw_func(move |_, cr, width, height| {
            let angle = draw_animation
                .upgrade()
                .map(|animation| animation.rotation.get().angle)
                .unwrap_or_default();
            draw_vinyl(cr, width as f64, height as f64, angle);
        });

        let weak_animation = Rc::downgrade(&animation);
        widget.connect_map(move |widget| {
            if let Some(animation) = weak_animation.upgrade() {
                animation.ensure_tick_running(widget);
            }
        });

        let weak_animation = Rc::downgrade(&animation);
        widget.connect_unmap(move |_| {
            if let Some(animation) = weak_animation.upgrade() {
                animation.cancel_tick();
            }
        });

        Self { widget, animation }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.widget
    }

    pub fn set_playing(&self, is_playing: bool) {
        if self.animation.stopped.get() {
            return;
        }
        self.animation.playing.set(is_playing);
        self.animation.ensure_tick_running(&self.widget);
    }

    pub(super) fn stop(&self) {
        self.animation.stopped.set(true);
        self.animation.playing.set(false);
        self.animation.cancel_tick();
    }
}

impl Drop for VinylDisc {
    fn drop(&mut self) {
        self.stop();
    }
}

impl VinylAnimation {
    fn should_animate(&self, mapped: bool, enabled: bool) -> bool {
        !self.stopped.get()
            && mapped
            && enabled
            && (self.playing.get() || self.rotation.get().velocity > 0.001)
    }

    fn reset_motion(&self) {
        let mut rotation = self.rotation.get();
        rotation.velocity = 0.0;
        self.rotation.set(rotation);
        self.last_tick.set(None);
    }

    fn cancel_tick(&self) {
        let tick = self.tick.borrow_mut().take();
        if let Some(tick) = tick {
            tick.remove();
        }
        self.reset_motion();
    }

    fn ensure_tick_running(self: &Rc<Self>, widget: &gtk::DrawingArea) {
        if !self.should_animate(widget.is_mapped(), animations_enabled()) {
            self.cancel_tick();
            return;
        }
        if self.tick.borrow().is_some() {
            return;
        }

        let weak_animation = Rc::downgrade(self);
        self.last_tick.set(Some(Instant::now()));
        let tick = widget.add_tick_callback(move |widget, _| {
            let Some(animation) = weak_animation.upgrade() else {
                return glib::ControlFlow::Break;
            };
            animation.advance(widget)
        });
        *self.tick.borrow_mut() = Some(tick);
    }

    fn advance(&self, widget: &gtk::DrawingArea) -> glib::ControlFlow {
        if !self.should_animate(widget.is_mapped(), animations_enabled()) {
            drop(self.tick.borrow_mut().take());
            self.reset_motion();
            return glib::ControlFlow::Break;
        }

        let now = Instant::now();
        let elapsed = self
            .last_tick
            .replace(Some(now))
            .map(|previous| now.duration_since(previous).as_secs_f64())
            .unwrap_or(0.016);
        let mut rotation = self.rotation.get();
        let running = rotation.advance(self.playing.get(), elapsed);
        self.rotation.set(rotation);
        widget.queue_draw();

        if running {
            glib::ControlFlow::Continue
        } else {
            drop(self.tick.borrow_mut().take());
            self.reset_motion();
            glib::ControlFlow::Break
        }
    }
}

fn draw_vinyl(cr: &cairo::Context, width: f64, height: f64, angle: f64) {
    let cx = width / 2.0;
    let cy = height / 2.0;
    let radius = (width.min(height) / 2.0 - 2.0).max(10.0);

    let tokens = ThemeManager::shared().current_tokens();
    let accent = tokens
        .as_ref()
        .and_then(|t| gtk::gdk::RGBA::parse(&t.accent).ok())
        .unwrap_or_else(|| gtk::gdk::RGBA::new(0.48, 0.64, 0.97, 1.0));
    let surface = tokens
        .as_ref()
        .and_then(|t| gtk::gdk::RGBA::parse(&t.surface).ok())
        .unwrap_or_else(|| gtk::gdk::RGBA::new(0.12, 0.13, 0.18, 1.0));

    // Outer disc body (deep charcoal vinyl)
    cr.arc(cx, cy, radius, 0.0, 2.0 * PI);
    cr.set_source_rgba(0.07, 0.08, 0.10, 1.0);
    let _ = cr.fill();

    // Outer rim edge highlight
    cr.arc(cx, cy, radius - 1.0, 0.0, 2.0 * PI);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.08);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    // Concentric micro-grooves
    let groove_steps = [
        0.44, 0.48, 0.52, 0.56, 0.60, 0.63, 0.67, 0.70, 0.74, 0.78, 0.81, 0.85, 0.89, 0.92, 0.96,
    ];
    for &factor in &groove_steps {
        cr.arc(cx, cy, radius * factor, 0.0, 2.0 * PI);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.04);
        cr.set_line_width(0.75);
        let _ = cr.stroke();

        cr.arc(cx, cy, radius * (factor + 0.01), 0.0, 2.0 * PI);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
        cr.set_line_width(0.75);
        let _ = cr.stroke();
    }

    // Realistic anisotropic light sheen (two butterfly lobes that reflect light)
    // The sheen highlights rotate slightly or stay aligned with the light source
    let sheen_angle = angle * 0.25; // Subtle movement with rotation
    for lobe in [0.0, PI] {
        let base_a = sheen_angle + lobe;
        let p = cairo::RadialGradient::new(cx, cy, radius * 0.38, cx, cy, radius * 0.98);
        p.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.0);
        p.add_color_stop_rgba(0.5, 1.0, 1.0, 1.0, 0.05);
        p.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);

        cr.save().ok();
        cr.move_to(cx, cy);
        cr.arc(cx, cy, radius * 0.98, base_a - 0.4, base_a + 0.4);
        cr.close_path();
        let _ = cr.set_source(&p);
        let _ = cr.fill();
        cr.restore().ok();
    }

    // Center record label
    let label_radius = radius * 0.38;
    cr.arc(cx, cy, label_radius, 0.0, 2.0 * PI);
    cr.set_source_rgba(
        f64::from(surface.red()),
        f64::from(surface.green()),
        f64::from(surface.blue()),
        1.0,
    );
    let _ = cr.fill();

    // Center label accent border ring
    cr.arc(cx, cy, label_radius, 0.0, 2.0 * PI);
    cr.set_source_rgba(
        f64::from(accent.red()),
        f64::from(accent.green()),
        f64::from(accent.blue()),
        0.75,
    );
    cr.set_line_width(2.0);
    let _ = cr.stroke();

    // Center label stylized track lines (rotating with record)
    cr.save().ok();
    cr.translate(cx, cy);
    cr.rotate(angle);
    for i in 0..4 {
        let rot = i as f64 * (PI / 2.0);
        cr.save().ok();
        cr.rotate(rot);
        cr.move_to(label_radius * 0.28, 0.0);
        cr.line_to(label_radius * 0.72, 0.0);
        cr.set_source_rgba(
            f64::from(accent.red()),
            f64::from(accent.green()),
            f64::from(accent.blue()),
            0.4,
        );
        cr.set_line_width(1.5);
        let _ = cr.stroke();
        cr.restore().ok();
    }
    cr.restore().ok();

    // Center spindle hole
    let hole_radius = (radius * 0.07).max(3.0);
    cr.arc(cx, cy, hole_radius, 0.0, 2.0 * PI);
    cr.set_source_rgba(0.04, 0.04, 0.06, 1.0);
    let _ = cr.fill();

    cr.arc(cx, cy, hole_radius, 0.0, 2.0 * PI);
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.2);
    cr.set_line_width(0.75);
    let _ = cr.stroke();
}

#[cfg(test)]
mod tests;
