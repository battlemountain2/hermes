use std::{
    cell::Cell,
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{
    playback::SPECTRUM_BANDS,
    ui::{preview::stream::PlaybackStream, theme::ThemeManager},
};

const NUM_SEGMENTS: usize = 14;
const PEAK_DECAY: f32 = 0.92;
const PEAK_HOLD_FRAMES: u8 = 10;

pub struct MicroSegmentVisualizer {
    widget: gtk::DrawingArea,
    peaks: Rc<Cell<[f32; SPECTRUM_BANDS]>>,
    peak_hold: Rc<Cell<[u8; SPECTRUM_BANDS]>>,
    tick_active: Rc<Cell<bool>>,
    stream: PlaybackStream,
}

impl MicroSegmentVisualizer {
    pub fn new(stream: &PlaybackStream) -> Self {
        let widget = gtk::DrawingArea::new();
        widget.add_css_class("preview-audio-visualizer");
        widget.set_content_height(54);
        widget.set_content_width(220);
        widget.set_size_request(-1, 54);
        widget.set_hexpand(true);
        widget.set_vexpand(false);

        let peaks = Rc::new(Cell::new([0.0_f32; SPECTRUM_BANDS]));
        let peak_hold = Rc::new(Cell::new([0_u8; SPECTRUM_BANDS]));
        let tick_active = Rc::new(Cell::new(false));

        let draw_stream = stream.clone();
        let draw_peaks = peaks.clone();
        let draw_hold = peak_hold.clone();

        widget.set_draw_func(move |_, cr, width, height| {
            let bands = draw_stream.spectrum_bands();
            let mut current_peaks = draw_peaks.get();
            let mut current_hold = draw_hold.get();

            // Update peak hold logic
            for i in 0..SPECTRUM_BANDS {
                if bands[i] >= current_peaks[i] {
                    current_peaks[i] = bands[i];
                    current_hold[i] = PEAK_HOLD_FRAMES;
                } else if current_hold[i] > 0 {
                    current_hold[i] -= 1;
                } else {
                    current_peaks[i] = (current_peaks[i] * PEAK_DECAY).max(0.0);
                }
            }
            draw_peaks.set(current_peaks);
            draw_hold.set(current_hold);

            draw_micro_segments(cr, width as f64, height as f64, &bands, &current_peaks);
        });

        let visualizer = Self {
            widget,
            peaks,
            peak_hold,
            tick_active,
            stream: stream.clone(),
        };

        let weak_visualizer = visualizer.downgrade();
        visualizer.widget.connect_map(move |_| {
            if let Some(vis) = weak_visualizer.upgrade() {
                if vis.stream.is_playing() {
                    vis.ensure_tick_running();
                }
            }
        });

        let weak_vis = visualizer.downgrade();
        stream.connect_playing_notify(move |s| {
            if let Some(vis) = weak_vis.upgrade() {
                if s.is_playing() {
                    vis.ensure_tick_running();
                }
            }
        });

        visualizer
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.widget
    }

    pub fn ensure_tick_running(&self) {
        Self::start_tick(&self.widget, &self.stream, &self.peaks, &self.tick_active);
    }

    pub fn stop(&self) {
        self.tick_active.set(false);
    }

    fn start_tick(
        widget: &gtk::DrawingArea,
        stream: &PlaybackStream,
        peaks: &Rc<Cell<[f32; SPECTRUM_BANDS]>>,
        tick_active: &Rc<Cell<bool>>,
    ) {
        if tick_active.get() {
            return;
        }
        tick_active.set(true);

        let w = widget.clone();
        let s = stream.clone();
        let p = peaks.clone();
        let ta = tick_active.clone();

        widget.add_tick_callback(move |_, _| {
            w.queue_draw();

            if s.is_playing() {
                glib::ControlFlow::Continue
            } else {
                let current_peaks = p.get();
                let has_residual_energy = current_peaks.iter().any(|&p| p > 0.02);
                if has_residual_energy {
                    glib::ControlFlow::Continue
                } else {
                    ta.set(false);
                    glib::ControlFlow::Break
                }
            }
        });
    }

    fn downgrade(&self) -> WeakVisualizer {
        WeakVisualizer {
            widget: self.widget.downgrade(),
            stream: self.stream.clone(),
            peaks: Rc::downgrade(&self.peaks),
            peak_hold: Rc::downgrade(&self.peak_hold),
            tick_active: Rc::downgrade(&self.tick_active),
        }
    }
}

struct WeakVisualizer {
    widget: glib::WeakRef<gtk::DrawingArea>,
    stream: PlaybackStream,
    peaks: std::rc::Weak<Cell<[f32; SPECTRUM_BANDS]>>,
    peak_hold: std::rc::Weak<Cell<[u8; SPECTRUM_BANDS]>>,
    tick_active: std::rc::Weak<Cell<bool>>,
}

impl WeakVisualizer {
    fn upgrade(&self) -> Option<VisualizerHandle> {
        Some(VisualizerHandle {
            widget: self.widget.upgrade()?,
            stream: self.stream.clone(),
            peaks: self.peaks.upgrade()?,
            _peak_hold: self.peak_hold.upgrade()?,
            tick_active: self.tick_active.upgrade()?,
        })
    }
}

struct VisualizerHandle {
    widget: gtk::DrawingArea,
    stream: PlaybackStream,
    peaks: Rc<Cell<[f32; SPECTRUM_BANDS]>>,
    _peak_hold: Rc<Cell<[u8; SPECTRUM_BANDS]>>,
    tick_active: Rc<Cell<bool>>,
}

impl VisualizerHandle {
    fn ensure_tick_running(&self) {
        MicroSegmentVisualizer::start_tick(&self.widget, &self.stream, &self.peaks, &self.tick_active);
    }
}

fn draw_micro_segments(
    cr: &cairo::Context,
    width: f64,
    height: f64,
    bands: &[f32; SPECTRUM_BANDS],
    peaks: &[f32; SPECTRUM_BANDS],
) {
    if width <= 10.0 || height <= 10.0 {
        return;
    }
    let tokens = ThemeManager::shared()
        .applied_tokens()
        .or_else(|| ThemeManager::shared().current_tokens());

    let (accent, danger, text, dim_text) = if let Some(t) = tokens.as_ref() {
        (
            gtk::gdk::RGBA::parse(&t.accent).unwrap_or_else(|_| gtk::gdk::RGBA::new(0.48, 0.64, 0.97, 1.0)),
            gtk::gdk::RGBA::parse(&t.danger).unwrap_or_else(|_| gtk::gdk::RGBA::new(0.97, 0.46, 0.56, 1.0)),
            gtk::gdk::RGBA::parse(&t.text).unwrap_or_else(|_| gtk::gdk::RGBA::new(0.95, 0.96, 0.98, 1.0)),
            gtk::gdk::RGBA::parse(&t.dim_text).unwrap_or_else(|_| gtk::gdk::RGBA::new(0.60, 0.65, 0.72, 1.0)),
        )
    } else {
        (
            gtk::gdk::RGBA::new(0.48, 0.64, 0.97, 1.0),
            gtk::gdk::RGBA::new(0.97, 0.46, 0.56, 1.0),
            gtk::gdk::RGBA::new(0.95, 0.96, 0.98, 1.0),
            gtk::gdk::RGBA::new(0.60, 0.65, 0.72, 1.0),
        )
    };

    let col_gap = if width < 220.0 { 1.0_f64 } else { 2.0_f64 };
    let seg_gap = 1.0_f64;
    let total_gaps = col_gap * (SPECTRUM_BANDS - 1) as f64;
    let col_width = ((width - total_gaps) / SPECTRUM_BANDS as f64).max(1.0);

    let total_seg_gaps = seg_gap * (NUM_SEGMENTS - 1) as f64;
    let seg_height = ((height - total_seg_gaps - 2.0) / NUM_SEGMENTS as f64).max(1.5);

    for col in 0..SPECTRUM_BANDS {
        let x = col as f64 * (col_width + col_gap);
        let level = bands[col].clamp(0.0, 1.0);
        let peak_level = peaks[col].clamp(0.0, 1.0);

        let lit_count = (level * NUM_SEGMENTS as f32).round() as usize;
        let peak_seg = if peak_level > 0.04 {
            Some(((peak_level * (NUM_SEGMENTS - 1) as f32).round() as usize).min(NUM_SEGMENTS - 1))
        } else {
            None
        };

        for seg in 0..NUM_SEGMENTS {
            let y = height - 1.0 - (seg + 1) as f64 * (seg_height + seg_gap);
            let is_lit = seg < lit_count;
            let is_peak = peak_seg == Some(seg);

            if is_lit {
                if seg >= 12 {
                    // Overload / clipping tier: Danger red
                    cr.set_source_rgba(
                        f64::from(danger.red()),
                        f64::from(danger.green()),
                        f64::from(danger.blue()),
                        0.98,
                    );
                } else if seg >= 10 {
                    // High energy tier: Crisp foreground text highlight
                    cr.set_source_rgba(
                        f64::from(text.red()),
                        f64::from(text.green()),
                        f64::from(text.blue()),
                        0.95,
                    );
                } else {
                    // Normal operational tier: Primary theme accent
                    cr.set_source_rgba(
                        f64::from(accent.red()),
                        f64::from(accent.green()),
                        f64::from(accent.blue()),
                        0.92,
                    );
                }
            } else if is_peak {
                // Peak hold indicator: crisp floating bar
                if seg >= 12 {
                    cr.set_source_rgba(
                        f64::from(danger.red()),
                        f64::from(danger.green()),
                        f64::from(danger.blue()),
                        1.0,
                    );
                } else {
                    cr.set_source_rgba(
                        f64::from(text.red()),
                        f64::from(text.green()),
                        f64::from(text.blue()),
                        1.0,
                    );
                }
            } else {
                // Unlit background micro-segment tick (faint studio LED matrix)
                cr.set_source_rgba(
                    f64::from(dim_text.red()),
                    f64::from(dim_text.green()),
                    f64::from(dim_text.blue()),
                    0.18,
                );
            }

            // Zero border-radius rectangular studio tick
            cr.rectangle(x, y, col_width, seg_height);
            let _ = cr.fill();
        }
    }
}
