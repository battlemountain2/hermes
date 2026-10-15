use std::{
    cell::Cell,
    f64::consts::PI,
    rc::Rc,
    time::Instant,
};

use gtk::{glib, prelude::*};

use crate::ui::theme::ThemeManager;

const TARGET_RPM: f64 = 33.3333333;
const TARGET_ANGULAR_VELOCITY: f64 = TARGET_RPM * 2.0 * PI / 60.0; // ~3.49 rad/s
const ACCELERATION_RATE: f64 = 6.0;
const DECELERATION_RATE: f64 = 4.5;

pub struct VinylDisc {
    widget: gtk::DrawingArea,
    angle: Rc<Cell<f64>>,
    velocity: Rc<Cell<f64>>,
    playing: Rc<Cell<bool>>,
    tick_active: Rc<Cell<bool>>,
    last_tick: Rc<Cell<Option<Instant>>>,
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

        let angle = Rc::new(Cell::new(0.0_f64));
        let velocity = Rc::new(Cell::new(0.0_f64));
        let playing = Rc::new(Cell::new(false));
        let tick_active = Rc::new(Cell::new(false));
        let last_tick = Rc::new(Cell::new(None));

        let draw_angle = angle.clone();
        widget.set_draw_func(move |_, cr, width, height| {
            draw_vinyl(cr, width as f64, height as f64, draw_angle.get());
        });

        let disc = Self {
            widget,
            angle,
            velocity,
            playing,
            tick_active,
            last_tick,
        };

        let weak_widget = disc.widget.downgrade();
        let weak_disc = disc.downgrade();
        disc.widget.connect_map(move |_| {
            if let (Some(widget), Some(disc)) = (weak_widget.upgrade(), weak_disc.upgrade()) {
                if disc.playing.get() {
                    disc.ensure_tick_running(&widget);
                }
            }
        });

        disc
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.widget
    }

    pub fn set_playing(&self, is_playing: bool) {
        self.playing.set(is_playing);
        if is_playing && self.widget.is_mapped() {
            self.ensure_tick_running(&self.widget);
        }
    }

    fn ensure_tick_running(&self, widget: &gtk::DrawingArea) {
        if self.tick_active.get() {
            return;
        }

        // Check if animations are disabled
        let animations_enabled = gtk::Settings::default()
            .map(|s| s.is_gtk_enable_animations())
            .unwrap_or(true);
        if !animations_enabled {
            self.velocity.set(0.0);
            return;
        }

        self.tick_active.set(true);
        self.last_tick.set(Some(Instant::now()));

        let angle = self.angle.clone();
        let velocity = self.velocity.clone();
        let playing = self.playing.clone();
        let tick_active = self.tick_active.clone();
        let last_tick = self.last_tick.clone();

        widget.add_tick_callback(move |widget, _| {
            let now = Instant::now();
            let dt = last_tick
                .get()
                .map(|prev| now.duration_since(prev).as_secs_f64().min(0.1))
                .unwrap_or(0.016);
            last_tick.set(Some(now));

            let current_v = velocity.get();
            let target_v = if playing.get() {
                TARGET_ANGULAR_VELOCITY
            } else {
                0.0
            };

            let rate = if target_v > current_v {
                ACCELERATION_RATE
            } else {
                DECELERATION_RATE
            };
            let new_v = current_v + (target_v - current_v) * (dt * rate).min(1.0);
            velocity.set(new_v);

            if new_v > 0.001 {
                let new_angle = (angle.get() + new_v * dt) % (2.0 * PI);
                angle.set(new_angle);
                widget.queue_draw();
                glib::ControlFlow::Continue
            } else if playing.get() {
                widget.queue_draw();
                glib::ControlFlow::Continue
            } else {
                velocity.set(0.0);
                tick_active.set(false);
                widget.queue_draw();
                glib::ControlFlow::Break
            }
        });
    }

    fn downgrade(&self) -> WeakVinylDisc {
        WeakVinylDisc {
            angle: Rc::downgrade(&self.angle),
            velocity: Rc::downgrade(&self.velocity),
            playing: Rc::downgrade(&self.playing),
            tick_active: Rc::downgrade(&self.tick_active),
            last_tick: Rc::downgrade(&self.last_tick),
        }
    }
}

struct WeakVinylDisc {
    angle: std::rc::Weak<Cell<f64>>,
    velocity: std::rc::Weak<Cell<f64>>,
    playing: std::rc::Weak<Cell<bool>>,
    tick_active: std::rc::Weak<Cell<bool>>,
    last_tick: std::rc::Weak<Cell<Option<Instant>>>,
}

impl WeakVinylDisc {
    fn upgrade(&self) -> Option<VinylDiscHandle> {
        Some(VinylDiscHandle {
            _angle: self.angle.upgrade()?,
            velocity: self.velocity.upgrade()?,
            playing: self.playing.upgrade()?,
            tick_active: self.tick_active.upgrade()?,
            last_tick: self.last_tick.upgrade()?,
        })
    }
}

struct VinylDiscHandle {
    _angle: Rc<Cell<f64>>,
    velocity: Rc<Cell<f64>>,
    playing: Rc<Cell<bool>>,
    tick_active: Rc<Cell<bool>>,
    last_tick: Rc<Cell<Option<Instant>>>,
}

impl VinylDiscHandle {
    fn ensure_tick_running(&self, widget: &gtk::DrawingArea) {
        if self.tick_active.get() {
            return;
        }
        let animations_enabled = gtk::Settings::default()
            .map(|s| s.is_gtk_enable_animations())
            .unwrap_or(true);
        if !animations_enabled {
            self.velocity.set(0.0);
            return;
        }

        self.tick_active.set(true);
        self.last_tick.set(Some(Instant::now()));

        let angle = self._angle.clone();
        let velocity = self.velocity.clone();
        let playing = self.playing.clone();
        let tick_active = self.tick_active.clone();
        let last_tick = self.last_tick.clone();

        widget.add_tick_callback(move |widget, _| {
            let now = Instant::now();
            let dt = last_tick
                .get()
                .map(|prev| now.duration_since(prev).as_secs_f64().min(0.1))
                .unwrap_or(0.016);
            last_tick.set(Some(now));

            let current_v = velocity.get();
            let target_v = if playing.get() {
                TARGET_ANGULAR_VELOCITY
            } else {
                0.0
            };

            let rate = if target_v > current_v {
                ACCELERATION_RATE
            } else {
                DECELERATION_RATE
            };
            let new_v = current_v + (target_v - current_v) * (dt * rate).min(1.0);
            velocity.set(new_v);

            if new_v > 0.001 {
                let new_angle = (angle.get() + new_v * dt) % (2.0 * PI);
                angle.set(new_angle);
                widget.queue_draw();
                glib::ControlFlow::Continue
            } else if playing.get() {
                widget.queue_draw();
                glib::ControlFlow::Continue
            } else {
                velocity.set(0.0);
                tick_active.set(false);
                widget.queue_draw();
                glib::ControlFlow::Break
            }
        });
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
