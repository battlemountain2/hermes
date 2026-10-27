use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{
    playback::SPECTRUM_BANDS,
    ui::{
        audio_preferences::{self, Decay, Options},
        preview::stream::PlaybackStream,
        theme::ThemeManager,
    },
};

const NUM_SEGMENTS: usize = 14;

#[cfg(test)]
mod tests;

pub struct MicroSegmentVisualizer {
    widget: gtk::DrawingArea,
    state: Rc<VisualizerState>,
}

struct VisualizerState {
    widget: glib::WeakRef<gtk::DrawingArea>,
    stream: glib::WeakRef<PlaybackStream>,
    bands: Cell<[f32; SPECTRUM_BANDS]>,
    peaks: Cell<[f32; SPECTRUM_BANDS]>,
    peak_hold: Cell<[u8; SPECTRUM_BANDS]>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    playing_handler: RefCell<Option<glib::SignalHandlerId>>,
    options: Cell<Options>,
    preferences_observer: RefCell<Option<Rc<dyn Fn()>>>,
    stopped: Cell<bool>,
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

        let state = Rc::new(VisualizerState {
            widget: widget.downgrade(),
            stream: stream.downgrade(),
            bands: Cell::new([0.0; SPECTRUM_BANDS]),
            peaks: Cell::new([0.0; SPECTRUM_BANDS]),
            peak_hold: Cell::new([0; SPECTRUM_BANDS]),
            tick: RefCell::new(None),
            playing_handler: RefCell::new(None),
            options: Cell::new(audio_preferences::current()),
            preferences_observer: RefCell::new(None),
            stopped: Cell::new(false),
        });
        let draw_state = Rc::downgrade(&state);
        widget.set_draw_func(move |_, cr, width, height| {
            if let Some(state) = draw_state.upgrade() {
                draw_micro_segments(
                    cr,
                    width as f64,
                    height as f64,
                    &state.bands.get(),
                    &state.peaks.get(),
                    state.options.get().columns,
                );
            }
        });
        let mapped_state = Rc::downgrade(&state);
        widget.connect_map(move |_| {
            if let Some(state) = mapped_state.upgrade() {
                state.ensure_tick_running();
            }
        });
        let unmapped_state = Rc::downgrade(&state);
        widget.connect_unmap(move |_| {
            if let Some(state) = unmapped_state.upgrade() {
                state.cancel_tick();
                state.clear();
            }
        });
        let playing_state = Rc::downgrade(&state);
        let handler = stream.connect_playing_notify(move |_| {
            if let Some(state) = playing_state.upgrade() {
                state.ensure_tick_running();
            }
        });
        state.playing_handler.replace(Some(handler));
        let preferences_state = Rc::downgrade(&state);
        let observer: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(state) = preferences_state.upgrade()
                && !state.stopped.get()
            {
                let options = audio_preferences::current();
                let previous = state.options.replace(options);
                if previous.decay != options.decay {
                    let mut hold = state.peak_hold.get();
                    clamp_peak_hold(&mut hold, options.decay);
                    state.peak_hold.set(hold);
                }
                if let Some(widget) = state.widget.upgrade() {
                    widget.queue_draw();
                }
                state.ensure_tick_running();
            }
        });
        audio_preferences::observe(&observer);
        state.preferences_observer.replace(Some(observer));
        Self { widget, state }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.widget
    }

    pub fn ensure_tick_running(&self) {
        self.state.ensure_tick_running();
    }

    pub fn stop(&self) {
        self.state.stopped.set(true);
        self.state.preferences_observer.borrow_mut().take();
        self.state.cancel_tick();
        self.state.clear();
        if let Some(handler) = self.state.playing_handler.borrow_mut().take()
            && let Some(stream) = self.state.stream.upgrade()
        {
            stream.disconnect(handler);
        }
    }
}

impl Drop for MicroSegmentVisualizer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl VisualizerState {
    fn cancel_tick(&self) {
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
    }

    fn clear(&self) {
        let bands_changed = self.bands.replace([0.0; SPECTRUM_BANDS]) != [0.0; SPECTRUM_BANDS];
        let peaks_changed = self.peaks.replace([0.0; SPECTRUM_BANDS]) != [0.0; SPECTRUM_BANDS];
        self.peak_hold.set([0; SPECTRUM_BANDS]);
        if (bands_changed || peaks_changed)
            && let Some(widget) = self.widget.upgrade()
        {
            widget.queue_draw();
        }
    }

    fn ensure_tick_running(self: &Rc<Self>) {
        let Some(widget) = self.widget.upgrade() else {
            return;
        };
        if self.stopped.get() || !widget.is_mapped() || !crate::ui::motion::animations_enabled() {
            self.cancel_tick();
            self.clear();
            return;
        }
        let playing = self
            .stream
            .upgrade()
            .is_some_and(|stream| stream.is_playing());
        if self.tick.borrow().is_some()
            || (!playing && !self.peaks.get().iter().any(|peak| *peak > 0.02))
        {
            return;
        }
        let weak = Rc::downgrade(self);
        let tick = widget.add_tick_callback(move |widget, _| {
            weak.upgrade()
                .map_or(glib::ControlFlow::Break, |state| state.frame(widget))
        });
        self.tick.replace(Some(tick));
    }

    fn frame(&self, widget: &gtk::DrawingArea) -> glib::ControlFlow {
        if self.stopped.get() || !widget.is_mapped() || !crate::ui::motion::animations_enabled() {
            self.tick.borrow_mut().take();
            self.clear();
            return glib::ControlFlow::Break;
        }
        let Some(stream) = self.stream.upgrade() else {
            self.tick.borrow_mut().take();
            self.clear();
            return glib::ControlFlow::Break;
        };
        let playing = stream.is_playing();
        let bands = if playing {
            stream.spectrum_bands()
        } else {
            [0.0; SPECTRUM_BANDS]
        };
        self.bands.set(bands);
        let mut peaks = self.peaks.get();
        let mut hold = self.peak_hold.get();
        update_peaks(&bands, &mut peaks, &mut hold, self.options.get().decay);
        self.peaks.set(peaks);
        self.peak_hold.set(hold);
        widget.queue_draw();
        if !playing && peaks.iter().all(|peak| *peak <= 0.02) {
            self.tick.borrow_mut().take();
            self.clear();
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    }
}

fn update_peaks(
    bands: &[f32; SPECTRUM_BANDS],
    peaks: &mut [f32; SPECTRUM_BANDS],
    hold: &mut [u8; SPECTRUM_BANDS],
    decay: Decay,
) {
    for band in 0..SPECTRUM_BANDS {
        if bands[band] >= peaks[band] {
            peaks[band] = bands[band];
            hold[band] = decay.hold_frames();
        } else if hold[band] > 0 {
            hold[band] -= 1;
        } else {
            peaks[band] = (peaks[band] * decay.factor()).max(0.0);
        }
    }
}

fn clamp_peak_hold(hold: &mut [u8; SPECTRUM_BANDS], decay: Decay) {
    for remaining in hold {
        *remaining = (*remaining).min(decay.hold_frames());
    }
}

fn group_columns(levels: &[f32; SPECTRUM_BANDS], columns: usize) -> [f32; SPECTRUM_BANDS] {
    if columns == SPECTRUM_BANDS {
        return *levels;
    }
    let mut grouped = [0.0; SPECTRUM_BANDS];
    for (column, pair) in levels.chunks_exact(2).enumerate() {
        grouped[column] = pair[0].max(pair[1]);
    }
    grouped
}

fn draw_micro_segments(
    cr: &cairo::Context,
    width: f64,
    height: f64,
    bands: &[f32; SPECTRUM_BANDS],
    peaks: &[f32; SPECTRUM_BANDS],
    columns: usize,
) {
    if width <= 10.0 || height <= 10.0 {
        return;
    }
    let tokens = ThemeManager::shared()
        .applied_tokens()
        .or_else(|| ThemeManager::shared().current_tokens());

    let (accent, danger, text, dim_text) = if let Some(t) = tokens.as_ref() {
        (
            gtk::gdk::RGBA::parse(&t.accent)
                .unwrap_or_else(|_| gtk::gdk::RGBA::new(0.48, 0.64, 0.97, 1.0)),
            gtk::gdk::RGBA::parse(&t.danger)
                .unwrap_or_else(|_| gtk::gdk::RGBA::new(0.97, 0.46, 0.56, 1.0)),
            gtk::gdk::RGBA::parse(&t.text)
                .unwrap_or_else(|_| gtk::gdk::RGBA::new(0.95, 0.96, 0.98, 1.0)),
            gtk::gdk::RGBA::parse(&t.dim_text)
                .unwrap_or_else(|_| gtk::gdk::RGBA::new(0.60, 0.65, 0.72, 1.0)),
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
    let total_gaps = col_gap * (columns - 1) as f64;
    let col_width = ((width - total_gaps) / columns as f64).max(1.0);
    let bands = group_columns(bands, columns);
    let peaks = group_columns(peaks, columns);

    let total_seg_gaps = seg_gap * (NUM_SEGMENTS - 1) as f64;
    let seg_height = ((height - total_seg_gaps - 2.0) / NUM_SEGMENTS as f64).max(1.5);

    for col in 0..columns {
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
