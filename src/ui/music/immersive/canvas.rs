use std::{cell::Cell, f64::consts::TAU};

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

use super::VisualizerStyle;

const MAX_RIBBON_SEGMENTS: usize = 128;
const MAX_DIMENSION: f64 = i32::MAX as f64;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq)]
struct Frame {
    style: VisualizerStyle,
    time: f64,
    bands: [f32; 32],
    palette: ((f64, f64, f64), (f64, f64, f64)),
}

impl Frame {
    fn new(
        style: VisualizerStyle,
        time: f64,
        bands: [f32; 32],
        palette: ((f64, f64, f64), (f64, f64, f64)),
    ) -> Self {
        let channel = |value: f64| finite(value).clamp(0.0, 1.0);
        let color = |(red, green, blue)| (channel(red), channel(green), channel(blue));
        Self {
            style,
            time: finite(time),
            bands: bands.map(|value| finite(f64::from(value)).clamp(0.0, 1.0) as f32),
            palette: (color(palette.0), color(palette.1)),
        }
    }

    fn color(self, blend: f64, alpha: f64) -> gdk::RGBA {
        let (accent, highlight) = self.palette;
        gdk::RGBA::new(
            (accent.0 * (1.0 - blend) + highlight.0 * blend) as f32,
            (accent.1 * (1.0 - blend) + highlight.1 * blend) as f32,
            (accent.2 * (1.0 - blend) + highlight.2 * blend) as f32,
            alpha.clamp(0.0, 1.0) as f32,
        )
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct VisualizerCanvas {
        pub(super) frame: Cell<Option<Frame>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VisualizerCanvas {
        const NAME: &'static str = "HermesVisualizerCanvas";
        type Type = super::VisualizerCanvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for VisualizerCanvas {}

    impl WidgetImpl for VisualizerCanvas {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(frame) = self.frame.get() {
                let widget = self.obj();
                render_frame(
                    snapshot,
                    f64::from(widget.width()),
                    f64::from(widget.height()),
                    frame,
                );
            }
        }
    }
}

glib::wrapper! {
    pub struct VisualizerCanvas(ObjectSubclass<imp::VisualizerCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl VisualizerCanvas {
    pub(super) fn new() -> Self {
        glib::Object::builder()
            .property("hexpand", true)
            .property("vexpand", true)
            .property("overflow", gtk::Overflow::Hidden)
            .build()
    }

    pub(super) fn set_frame(
        &self,
        style: VisualizerStyle,
        time: f64,
        bands: [f32; 32],
        palette: ((f64, f64, f64), (f64, f64, f64)),
    ) {
        let frame = Some(Frame::new(style, time, bands, palette));
        if self.imp().frame.replace(frame) != frame {
            self.queue_draw();
        }
    }

    pub(super) fn clear(&self) {
        if self.imp().frame.take().is_some() {
            self.queue_draw();
        }
    }
}

#[cfg(test)]
pub(super) fn snapshot_frame(
    snapshot: &gtk::Snapshot,
    width: f64,
    height: f64,
    style: VisualizerStyle,
    time: f64,
    bands: [f32; 32],
    palette: ((f64, f64, f64), (f64, f64, f64)),
) {
    render_frame(
        snapshot,
        width,
        height,
        Frame::new(style, time, bands, palette),
    );
}

fn finite(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

fn phase(time: f64, speed: f64) -> f64 {
    time.rem_euclid(TAU / speed) * speed
}

fn render_frame(snapshot: &gtk::Snapshot, width: f64, height: f64, frame: Frame) {
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || width as f32 <= 0.0
        || height as f32 <= 0.0
    {
        return;
    }
    snapshot.push_clip(&graphene::Rect::new(0.0, 0.0, width as f32, height as f32));
    match frame.style {
        VisualizerStyle::Ribbons => ribbons(snapshot, width, height, frame),
        VisualizerStyle::Tunnel => tunnel(snapshot, width, height, frame),
        VisualizerStyle::Cosmic => cosmic(snapshot, width, height, frame),
    }
    snapshot.pop();
}

fn ribbons(snapshot: &gtk::Snapshot, width: f64, height: f64, frame: Frame) {
    let segments = ((width / 20.0).ceil() as usize).clamp(1, MAX_RIBBON_SEGMENTS);
    for ribbon in 0..4 {
        let base_y = height * (0.35 + 0.1 * ribbon as f64);
        let frequency = 0.0035 + 0.0006 * ribbon as f64;
        let phase = phase(frame.time, 1.1 + 0.2 * ribbon as f64) + ribbon as f64 * 1.4;
        let amplitude = 24.0 + 85.0 * f64::from(frame.bands[ribbon * 4]);
        let color = frame.color(ribbon as f64 / 3.0, 0.65);
        let mut previous = (
            0.0,
            base_y + amplitude * phase.sin() + amplitude * 0.3 * (-phase).cos(),
        );
        for segment in 1..=segments {
            let horizontal = width * segment as f64 / segments as f64;
            let vertical = base_y
                + amplitude * (horizontal * frequency + phase).sin()
                + amplitude * 0.3 * (horizontal * frequency * 1.8 - phase).cos();
            let delta_x = horizontal - previous.0;
            let delta_y = vertical - previous.1;
            snapshot.save();
            snapshot.translate(&graphene::Point::new(previous.0 as f32, previous.1 as f32));
            snapshot.rotate(delta_y.atan2(delta_x).to_degrees() as f32);
            snapshot.append_color(
                &color,
                &graphene::Rect::new(-0.25, -1.25, delta_x.hypot(delta_y) as f32 + 0.5, 2.5),
            );
            snapshot.restore();
            previous = (horizontal, vertical);
        }
    }
}

fn tunnel(snapshot: &gtk::Snapshot, width: f64, height: f64, frame: Frame) {
    let bass = frame.bands[..4]
        .iter()
        .map(|value| f64::from(*value))
        .sum::<f64>()
        / 4.0;
    for ring in 0..8 {
        let progress =
            (frame.time.rem_euclid(4.0 / 3.0) * 0.75 + ring as f64 / 8.0).rem_euclid(1.0);
        let radius = progress * progress * width.max(height) * 0.68 + 12.0 * bass;
        let alpha = (progress * std::f64::consts::PI).sin() * (0.28 + 0.6 * bass);
        if radius <= 0.0 || alpha <= 0.0 {
            continue;
        }
        let stroke = 1.5 + 3.0 * progress;
        let outer_radius = radius + stroke / 2.0;
        let outline = gsk::RoundedRect::from_rect(
            graphene::Rect::new(
                (width / 2.0 - outer_radius) as f32,
                (height / 2.0 - outer_radius) as f32,
                (outer_radius * 2.0) as f32,
                (outer_radius * 2.0) as f32,
            ),
            outer_radius as f32,
        );
        snapshot.append_border(
            &outline,
            &[stroke as f32; 4],
            &[frame.color(progress, alpha); 4],
        );
    }
}

fn cosmic(snapshot: &gtk::Snapshot, width: f64, height: f64, frame: Frame) {
    for particle in 0..32 {
        let theta =
            particle as f64 * TAU / 32.0 + phase(frame.time, 0.25 + 0.04 * (particle % 5) as f64);
        let distance = ((particle * 17) % 100) as f64 / 100.0;
        let energy = f64::from(frame.bands[particle % 16 + 8]);
        let radius = distance * width.min(height) * 0.42 + 24.0 + 40.0 * energy;
        let center_x = width / 2.0 + radius * theta.cos();
        let center_y = height / 2.0 + radius * theta.sin() * 0.65;
        let dot_radius = 2.0 + 4.0 * energy;
        let bounds = graphene::Rect::new(
            (center_x - dot_radius) as f32,
            (center_y - dot_radius) as f32,
            (dot_radius * 2.0) as f32,
            (dot_radius * 2.0) as f32,
        );
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, dot_radius as f32));
        snapshot.append_color(&frame.color(0.0, 0.35 + 0.65 * energy), &bounds);
        snapshot.pop();
    }
}
