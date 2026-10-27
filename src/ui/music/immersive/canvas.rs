use std::{cell::Cell, f64::consts::TAU};

use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};

use super::VisualizerStyle;

const MAX_RIBBON_SEGMENTS: usize = 128;
const MAX_DIMENSION: f64 = i32::MAX as f64;
const PRISM_FRAMES: usize = 6;
const ORBIT_ARC_SEGMENTS: usize = 24;
const ORBIT_STARS: usize = 12;
const ORBIT_COLUMNS: usize = 16;

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
        VisualizerStyle::Tunnel => prism(snapshot, width, height, frame),
        VisualizerStyle::Cosmic => orbit(snapshot, width, height, frame),
    }
    snapshot.pop();
}

fn ribbons(snapshot: &gtk::Snapshot, width: f64, height: f64, frame: Frame) {
    let segments = ((width / 20.0).ceil() as usize).clamp(1, MAX_RIBBON_SEGMENTS);
    for ribbon in 0..4 {
        let base_y = height * (0.34 + 0.11 * ribbon as f64);
        let frequency = 0.0032 + 0.0006 * ribbon as f64;
        let speed = 1.05 + 0.22 * ribbon as f64;
        let phase = phase(frame.time, speed) + ribbon as f64 * 1.35;
        let band_idx = match ribbon {
            0 => 0,
            1 => 4,
            2 => 10,
            _ => 18,
        };
        let energy = f64::from(frame.bands[band_idx]);
        let amplitude = 26.0 + 90.0 * energy;
        let thickness = (2.2 + 1.2 * energy) as f32;
        let color = frame.color(ribbon as f64 / 3.0, (0.62 + 0.32 * energy).min(0.96));
        let wave = |horizontal: f64| {
            let h1 = (horizontal * frequency + phase).sin();
            let h2 = 0.35 * (horizontal * frequency * 2.1 - phase * 1.25).sin();
            let h3 = 0.15 * (horizontal * frequency * 0.55 + phase * 0.7).cos();
            base_y + amplitude * (h1 + h2 + h3)
        };
        let mut previous = (0.0, wave(0.0));
        for segment in 1..=segments {
            let horizontal = width * segment as f64 / segments as f64;
            let vertical = wave(horizontal);
            let delta_x = horizontal - previous.0;
            let delta_y = vertical - previous.1;
            snapshot.save();
            snapshot.translate(&graphene::Point::new(previous.0 as f32, previous.1 as f32));
            snapshot.rotate(delta_y.atan2(delta_x).to_degrees() as f32);
            snapshot.append_color(
                &color,
                &graphene::Rect::new(
                    -0.25,
                    -thickness * 0.5,
                    delta_x.hypot(delta_y) as f32 + 0.5,
                    thickness,
                ),
            );
            snapshot.restore();
            previous = (horizontal, vertical);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Stroke {
    start: (f64, f64),
    end: (f64, f64),
    width: f64,
    blend: f64,
    alpha: f64,
}

impl Stroke {
    fn append(self, snapshot: &gtk::Snapshot, frame: Frame) {
        let delta_x = self.end.0 - self.start.0;
        let delta_y = self.end.1 - self.start.1;
        snapshot.save();
        snapshot.translate(&graphene::Point::new(
            self.start.0 as f32,
            self.start.1 as f32,
        ));
        snapshot.rotate(delta_y.atan2(delta_x).to_degrees() as f32);
        snapshot.append_color(
            &frame.color(self.blend, self.alpha),
            &graphene::Rect::new(
                (-self.width * 0.5) as f32,
                (-self.width * 0.5) as f32,
                (delta_x.hypot(delta_y) + self.width) as f32,
                self.width as f32,
            ),
        );
        snapshot.restore();
    }
}

fn band_energy(frame: Frame, start: usize, count: usize) -> f64 {
    frame.bands[start..start + count]
        .iter()
        .map(|value| f64::from(*value))
        .sum::<f64>()
        / count as f64
}

fn detail_scale(width: f64, height: f64) -> f64 {
    (width.min(height) / 720.0).min(2.0)
}

fn rotate_point(point: (f64, f64), angle: f64, center: (f64, f64)) -> (f64, f64) {
    let (sine, cosine) = angle.sin_cos();
    (
        center.0 + point.0 * cosine - point.1 * sine,
        center.1 + point.0 * sine + point.1 * cosine,
    )
}

fn prism_geometry(width: f64, height: f64, frame: Frame) -> [Stroke; PRISM_FRAMES * 4] {
    let scale = detail_scale(width, height);
    let motion = phase(frame.time, 0.12);
    let center = (
        width * 0.5 + width.min(height) * 0.012 * motion.sin(),
        height * 0.5 + width.min(height) * 0.008 * motion.cos(),
    );
    let sub_bass = band_energy(frame, 0, 4);
    let breath = 1.0 + 0.025 * sub_bass;
    std::array::from_fn(|index| {
        let depth = (index / 4) as f64;
        let shrink = 0.77_f64.powf(depth) * breath;
        let half_width = width.min(height * 1.8) * 0.39 * shrink;
        let half_height = height.min(width / 1.25) * 0.34 * shrink;
        let angle = 0.025 * motion.sin() + depth * 0.007 * (motion + 0.6).sin();
        let corners = [
            (-half_width, -half_height),
            (half_width, -half_height),
            (half_width, half_height),
            (-half_width, half_height),
        ];
        let tier_energy = band_energy(frame, (index / 4) * 4, 4);
        Stroke {
            start: rotate_point(corners[index % 4], angle, center),
            end: rotate_point(corners[(index + 1) % 4], angle, center),
            width: scale * (1.35 - depth * 0.10 + 0.25 * tier_energy),
            blend: depth / (PRISM_FRAMES - 1) as f64,
            alpha: (0.72 - depth * 0.045 + 0.22 * tier_energy).clamp(0.15, 0.98),
        }
    })
}

fn prism(snapshot: &gtk::Snapshot, width: f64, height: f64, frame: Frame) {
    for stroke in prism_geometry(width, height, frame) {
        stroke.append(snapshot, frame);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Star {
    center: (f64, f64),
    radius: f64,
    blend: f64,
    alpha: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SpectrumColumn {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    blend: f64,
}

#[derive(Debug, PartialEq)]
struct OrbitGeometry {
    arcs: [Stroke; ORBIT_ARC_SEGMENTS * 2],
    links: [Stroke; ORBIT_STARS / 2],
    stars: [Star; ORBIT_STARS],
    columns: [SpectrumColumn; ORBIT_COLUMNS],
}

fn orbit_point(width: f64, height: f64, track: usize, angle: f64) -> (f64, f64) {
    let radius = if track == 0 { 1.0 } else { 0.71 };
    rotate_point(
        (
            width.min(height * 1.65) * 0.31 * radius * angle.cos(),
            height.min(width / 1.25) * 0.20 * radius * angle.sin(),
        ),
        if track == 0 { -0.18 } else { 0.26 },
        (width * 0.5, height * 0.46),
    )
}

fn orbit_geometry(width: f64, height: f64, frame: Frame) -> OrbitGeometry {
    let scale = detail_scale(width, height);
    let motion = phase(frame.time, 0.025);
    let stars: [Star; ORBIT_STARS] = std::array::from_fn(|index| {
        let seed = ((index * 73 + 19) % 101) as f64 / 101.0;
        let angle = index as f64 * 2.399963229728653 + seed * 0.2 + motion;
        let energy = band_energy(frame, index * 2 + 4, 2);
        Star {
            center: orbit_point(width, height, index % 2, angle),
            radius: (scale * (1.6 + seed * 1.0 + energy * 1.2)).min(7.5),
            blend: seed,
            alpha: (0.65 + energy * 0.32).clamp(0.2, 0.98),
        }
    });
    OrbitGeometry {
        arcs: std::array::from_fn(|index| {
            let track = index / ORBIT_ARC_SEGMENTS;
            let segment = index % ORBIT_ARC_SEGMENTS;
            let start = motion + track as f64 * 2.3;
            let angle = TAU * 0.75 / ORBIT_ARC_SEGMENTS as f64;
            Stroke {
                start: orbit_point(width, height, track, start + segment as f64 * angle),
                end: orbit_point(width, height, track, start + (segment + 1) as f64 * angle),
                width: scale * 1.0,
                blend: track as f64,
                alpha: 0.42,
            }
        }),
        links: std::array::from_fn(|index| {
            let star = index * 2;
            let seed = ((star * 73 + 19) % 101) as f64 / 101.0;
            let angle = star as f64 * 2.399963229728653 + seed * 0.2 + motion;
            Stroke {
                start: stars[star].center,
                end: orbit_point(width, height, star % 2, angle - 0.18),
                width: scale * 0.75,
                blend: index as f64 / (ORBIT_STARS / 2 - 1) as f64,
                alpha: 0.34,
            }
        }),
        stars,
        columns: std::array::from_fn(|index| {
            let spacing = width.min(height * 1.4) * 0.027;
            let energy = band_energy(frame, index * 2, 2);
            let column_height = scale * 2.0 + height * 0.11 * energy;
            SpectrumColumn {
                left: width * 0.5 + (index as f64 - 8.0) * spacing + spacing * 0.3,
                top: height * 0.77 - column_height,
                width: spacing * 0.42,
                height: column_height,
                blend: index as f64 / (ORBIT_COLUMNS - 1) as f64,
            }
        }),
    }
}

fn orbit(snapshot: &gtk::Snapshot, width: f64, height: f64, frame: Frame) {
    let geometry = orbit_geometry(width, height, frame);
    for stroke in geometry.arcs.into_iter().chain(geometry.links) {
        stroke.append(snapshot, frame);
    }
    for star in geometry.stars {
        let bounds = graphene::Rect::new(
            (star.center.0 - star.radius) as f32,
            (star.center.1 - star.radius) as f32,
            (star.radius * 2.0) as f32,
            (star.radius * 2.0) as f32,
        );
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, star.radius as f32));
        snapshot.append_color(&frame.color(star.blend, star.alpha), &bounds);
        snapshot.pop();
    }
    for column in geometry.columns {
        snapshot.append_color(
            &frame.color(column.blend, 0.82),
            &graphene::Rect::new(
                column.left as f32,
                column.top as f32,
                column.width as f32,
                column.height as f32,
            ),
        );
    }
}
