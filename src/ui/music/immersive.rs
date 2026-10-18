// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use crate::{
    assets::{self, icons},
    music::Track,
    services::lyrics::{find_lyrics_for_audio_file, Lyrics},
    ui::{
        music::{controller::Controller, file_entry, time_label},
        theme::ThemeManager,
    },
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualizerStyle {
    Ribbons,
    Tunnel,
    Cosmic,
}

#[derive(Clone)]
pub struct ImmersiveView {
    overlay: gtk::Overlay,
    state: Rc<ImmersiveState>,
}

struct ImmersiveState {
    controller: Rc<Controller>,
    _observer: RefCell<Option<Rc<dyn Fn()>>>,
    on_open_library: Rc<dyn Fn()>,
    visualizer_canvas: gtk::DrawingArea,
    visualizer_active: Cell<bool>,
    visualizer_style: Cell<VisualizerStyle>,
    lyrics_active: Cell<bool>,
    current_track: RefCell<Option<Track>>,
    current_lyrics: RefCell<Option<Lyrics>>,
    active_lyric_index: Cell<Option<usize>>,
    lyric_labels: RefCell<Vec<gtk::Label>>,
    lyrics_box: gtk::Box,
    lyrics_scroll: gtk::ScrolledWindow,
    stage_box: gtk::Box,
    art_image: gtk::Image,
    title_label: gtk::Label,
    artist_label: gtk::Label,
    scrub_scale: gtk::Scale,
    elapsed_label: gtk::Label,
    remaining_label: gtk::Label,
    hover_controls: gtk::Box,
    play_button: gtk::Button,
    speaker_button: gtk::Button,
    volume_scale: gtk::Scale,
    shuffle_button: gtk::ToggleButton,
    repeat_button: gtk::Button,
    lyrics_btn: gtk::ToggleButton,
    visualizer_btn: gtk::ToggleButton,
    style_selector: gtk::Box,
    opt_ribbons: gtk::ToggleButton,
    opt_tunnel: gtk::ToggleButton,
    opt_cosmic: gtk::ToggleButton,
    floating_dock: gtk::Box,
    floating_title: gtk::Label,
    floating_artist: gtk::Label,
    floating_play: gtk::Button,
    floating_time: gtk::Label,
    vis_t: Cell<f64>,
    playing_icon: Cell<bool>,
    muted_icon: Cell<bool>,
    syncing: Cell<bool>,
}

impl ImmersiveView {
    pub fn new(
        controller: Rc<Controller>,
        _on_close: Rc<dyn Fn()>,
        on_open_library: Rc<dyn Fn()>,
    ) -> Self {
        let overlay = gtk::Overlay::new();
        overlay.add_css_class("immersive-root");

        // 1. Ambient Aura Background
        let ambient_aura = gtk::DrawingArea::new();
        ambient_aura.set_hexpand(true);
        ambient_aura.set_vexpand(true);
        ambient_aura.add_css_class("immersive-ambient-aura");
        ambient_aura.set_draw_func(move |_, cr, width, height| {
            let (accent, highlight) = current_theme_palette();
            draw_ambient_aura(cr, width as f64, height as f64, accent, highlight);
        });
        overlay.set_child(Some(&ambient_aura));

        // 2. Full-Screen Visualizer Canvas
        let visualizer_canvas = gtk::DrawingArea::new();
        visualizer_canvas.set_hexpand(true);
        visualizer_canvas.set_vexpand(true);
        visualizer_canvas.set_visible(false);
        visualizer_canvas.add_css_class("immersive-visualizer-canvas");
        overlay.add_overlay(&visualizer_canvas);

        // 3. Main Content Layer
        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content_box.add_css_class("immersive-content");
        content_box.set_hexpand(true);
        content_box.set_vexpand(true);

        // --- Top Bar ---
        let top_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        top_bar.add_css_class("immersive-top-bar");

        let back_btn = gtk::Button::builder()
            .tooltip_text("Back to Library · Esc")
            .build();
        back_btn.add_css_class("immersive-pill-btn");
        let back_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        back_content.append(&assets::primary_icon(icons::ARROW_LEFT, 16));
        back_content.append(&gtk::Label::new(Some("Library")));
        back_btn.set_child(Some(&back_content));
        top_bar.append(&back_btn);

        let lyrics_btn = gtk::ToggleButton::builder()
            .active(true)
            .tooltip_text("Toggle lyrics panel")
            .build();
        lyrics_btn.add_css_class("immersive-pill-btn");
        let lyrics_btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        lyrics_btn_box.append(&assets::primary_icon(icons::DOCUMENTS, 15));
        lyrics_btn_box.append(&gtk::Label::new(Some("Lyrics")));
        lyrics_btn.set_child(Some(&lyrics_btn_box));
        top_bar.append(&lyrics_btn);

        let visualizer_btn = gtk::ToggleButton::builder()
            .active(false)
            .tooltip_text("Full-screen aesthetic visualizer")
            .build();
        visualizer_btn.add_css_class("immersive-pill-btn");
        visualizer_btn.add_css_class("immersive-visualizer-btn");
        let visualizer_btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        visualizer_btn_box.append(&assets::primary_icon(icons::MONITOR, 15));
        visualizer_btn_box.append(&gtk::Label::new(Some("Visualizer")));
        visualizer_btn.set_child(Some(&visualizer_btn_box));
        top_bar.append(&visualizer_btn);

        let top_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        top_spacer.set_hexpand(true);
        top_bar.append(&top_spacer);

        // Style Selector for Visualizer
        let style_selector = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        style_selector.add_css_class("immersive-style-selector");
        style_selector.set_visible(false);

        let opt_ribbons = gtk::ToggleButton::with_label("Ribbons");
        opt_ribbons.add_css_class("immersive-style-opt");
        opt_ribbons.set_active(true);

        let opt_tunnel = gtk::ToggleButton::with_label("Tunnel");
        opt_tunnel.add_css_class("immersive-style-opt");
        opt_tunnel.set_group(Some(&opt_ribbons));

        let opt_cosmic = gtk::ToggleButton::with_label("Cosmic");
        opt_cosmic.add_css_class("immersive-style-opt");
        opt_cosmic.set_group(Some(&opt_ribbons));

        style_selector.append(&opt_ribbons);
        style_selector.append(&opt_tunnel);
        style_selector.append(&opt_cosmic);
        top_bar.append(&style_selector);

        content_box.append(&top_bar);

        // --- Main Stage ---
        let stage_box = gtk::Box::new(gtk::Orientation::Horizontal, 48);
        stage_box.add_css_class("immersive-stage");
        stage_box.set_hexpand(true);
        stage_box.set_vexpand(true);
        stage_box.set_halign(gtk::Align::Center);
        stage_box.set_valign(gtk::Align::Center);

        // Deck: Artwork + Scrub + Meta + Hover Controls
        let deck_container = gtk::Box::new(gtk::Orientation::Vertical, 16);
        deck_container.add_css_class("immersive-deck");
        deck_container.set_valign(gtk::Align::Center);

        // Artwork Frame
        let art_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        art_box.add_css_class("immersive-art-box");
        let art_image = assets::primary_icon(icons::MUSIC, 160);
        art_image.set_size_request(320, 320);
        art_image.add_css_class("immersive-art-img");
        art_box.append(&art_image);
        deck_container.append(&art_box);

        // Scrubber Scale & Times
        let scrub_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let scrub_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
        scrub_scale.set_draw_value(false);
        scrub_scale.set_hexpand(true);
        scrub_scale.add_css_class("immersive-scrub-scale");
        scrub_box.append(&scrub_scale);

        let times_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let elapsed_label = gtk::Label::new(Some("0:00"));
        elapsed_label.add_css_class("immersive-time");
        let time_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        time_spacer.set_hexpand(true);
        let remaining_label = gtk::Label::new(Some("0:00"));
        remaining_label.add_css_class("immersive-time");
        times_box.append(&elapsed_label);
        times_box.append(&time_spacer);
        times_box.append(&remaining_label);
        scrub_box.append(&times_box);
        deck_container.append(&scrub_box);

        // Resting Meta Info
        let meta_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let title_label = gtk::Label::new(Some("Nothing playing"));
        title_label.add_css_class("immersive-title");
        title_label.set_xalign(0.0);
        title_label.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let artist_label = gtk::Label::new(Some("Choose a track in Music"));
        artist_label.add_css_class("immersive-artist");
        artist_label.set_xalign(0.0);
        artist_label.set_ellipsize(gtk::pango::EllipsizeMode::End);

        meta_box.append(&title_label);
        meta_box.append(&artist_label);
        deck_container.append(&meta_box);

        // Hover-Revealed Playback & Volume Controls
        let hover_controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        hover_controls.add_css_class("immersive-hover-controls");
        hover_controls.set_opacity(0.0);

        let shuffle_button = gtk::ToggleButton::new();
        shuffle_button.set_child(Some(&assets::primary_icon(icons::SHUFFLE, 16)));
        shuffle_button.add_css_class("immersive-control-btn");
        shuffle_button.set_tooltip_text(Some("Shuffle"));

        let prev_button = gtk::Button::builder().tooltip_text("Previous").build();
        prev_button.set_child(Some(&assets::primary_icon(icons::ARROW_LEFT, 16)));
        prev_button.add_css_class("immersive-control-btn");

        let play_button = gtk::Button::builder().tooltip_text("Play / Pause").build();
        play_button.set_child(Some(&assets::primary_icon(icons::PLAY, 20)));
        play_button.add_css_class("immersive-play-btn");

        let next_button = gtk::Button::builder().tooltip_text("Next").build();
        next_button.set_child(Some(&assets::primary_icon(icons::ARROW_RIGHT, 16)));
        next_button.add_css_class("immersive-control-btn");

        let repeat_button = gtk::Button::builder().tooltip_text("Repeat").build();
        repeat_button.set_child(Some(&assets::primary_icon(icons::REPEAT, 16)));
        repeat_button.add_css_class("immersive-control-btn");

        hover_controls.append(&shuffle_button);
        hover_controls.append(&prev_button);
        hover_controls.append(&play_button);
        hover_controls.append(&next_button);
        hover_controls.append(&repeat_button);

        let hover_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hover_spacer.set_hexpand(true);
        hover_controls.append(&hover_spacer);

        let speaker_button = gtk::Button::builder().tooltip_text("Mute / Unmute").build();
        speaker_button.set_child(Some(&assets::primary_icon(icons::VOLUME, 16)));
        speaker_button.add_css_class("immersive-control-btn");
        hover_controls.append(&speaker_button);

        let volume_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        volume_scale.set_draw_value(false);
        volume_scale.set_value(0.75);
        volume_scale.set_size_request(80, -1);
        volume_scale.add_css_class("immersive-vol-scale");
        hover_controls.append(&volume_scale);

        deck_container.append(&hover_controls);
        stage_box.append(&deck_container);

        // Synced Lyrics Scrollable Panel
        let lyrics_box = gtk::Box::new(gtk::Orientation::Vertical, 20);
        lyrics_box.add_css_class("immersive-lyrics-box");
        let lyrics_scroll = gtk::ScrolledWindow::builder()
            .child(&lyrics_box)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .build();
        lyrics_scroll.add_css_class("immersive-lyrics-scroll");
        lyrics_scroll.set_size_request(420, 480);
        stage_box.append(&lyrics_scroll);

        content_box.append(&stage_box);

        // Floating Pill Dock (Shown during Visualizer mode)
        let floating_dock = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        floating_dock.add_css_class("immersive-floating-dock");
        floating_dock.set_halign(gtk::Align::Center);
        floating_dock.set_valign(gtk::Align::End);
        floating_dock.set_margin_bottom(24);
        floating_dock.set_visible(false);

        let floating_meta = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let floating_title = gtk::Label::new(Some(""));
        floating_title.add_css_class("immersive-floating-title");
        floating_title.set_xalign(0.0);
        let floating_artist = gtk::Label::new(Some(""));
        floating_artist.add_css_class("immersive-floating-artist");
        floating_artist.set_xalign(0.0);
        floating_meta.append(&floating_title);
        floating_meta.append(&floating_artist);
        floating_dock.append(&floating_meta);

        let floating_play = gtk::Button::builder().build();
        floating_play.set_child(Some(&assets::primary_icon(icons::PLAY, 16)));
        floating_play.add_css_class("immersive-control-btn");
        floating_dock.append(&floating_play);

        let floating_time = gtk::Label::new(Some("0:00 / 0:00"));
        floating_time.add_css_class("immersive-time");
        floating_dock.append(&floating_time);

        content_box.append(&floating_dock);
        overlay.add_overlay(&content_box);

        let state = Rc::new(ImmersiveState {
            controller: controller.clone(),
            _observer: RefCell::new(None),
            on_open_library: on_open_library.clone(),
            visualizer_canvas: visualizer_canvas.clone(),
            visualizer_active: Cell::new(false),
            visualizer_style: Cell::new(VisualizerStyle::Ribbons),
            lyrics_active: Cell::new(true),
            current_track: RefCell::new(None),
            current_lyrics: RefCell::new(None),
            active_lyric_index: Cell::new(None),
            lyric_labels: RefCell::new(Vec::new()),
            lyrics_box,
            lyrics_scroll,
            stage_box,
            art_image,
            title_label,
            artist_label,
            scrub_scale,
            elapsed_label,
            remaining_label,
            hover_controls,
            play_button,
            speaker_button,
            volume_scale,
            shuffle_button,
            repeat_button,
            lyrics_btn,
            visualizer_btn,
            style_selector,
            opt_ribbons,
            opt_tunnel,
            opt_cosmic,
            floating_dock,
            floating_title,
            floating_artist,
            floating_play,
            floating_time,
            vis_t: Cell::new(0.0),
            playing_icon: Cell::new(false),
            muted_icon: Cell::new(false),
            syncing: Cell::new(false),
        });

        // Motion Controller for Hover Reveal (Controls fade in smoothly without layout jumping)
        let motion = gtk::EventControllerMotion::new();
        let hover_state = Rc::downgrade(&state);
        motion.connect_enter(move |_, _, _| {
            if let Some(state) = hover_state.upgrade() {
                state.hover_controls.set_opacity(1.0);
            }
        });
        let leave_state = Rc::downgrade(&state);
        motion.connect_leave(move |_| {
            if let Some(state) = leave_state.upgrade() {
                state.hover_controls.set_opacity(0.0);
            }
        });
        overlay.add_controller(motion);

        // Visualizer Canvas Drawing
        let vis_state = Rc::downgrade(&state);
        visualizer_canvas.set_draw_func(move |_, cr, width, height| {
            if let Some(state) = vis_state.upgrade()
                && state.visualizer_active.get()
            {
                let bands = state.controller.spectrum_bands();
                let (accent, highlight) = current_theme_palette();
                draw_generative_visualizer(
                    cr,
                    width as f64,
                    height as f64,
                    state.vis_t.get(),
                    state.visualizer_style.get(),
                    &bands,
                    accent,
                    highlight,
                );
            }
        });

        // Tick Callback for Visualizer and Smooth Scrubbing
        let tick_state = Rc::downgrade(&state);
        overlay.add_tick_callback(move |_, clock| {
            let Some(state) = tick_state.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !state.visualizer_active.get() && !state.lyrics_active.get() {
                return glib::ControlFlow::Continue;
            }
            if state.visualizer_active.get() {
                let frame_time = clock.frame_time();
                let seconds = frame_time as f64 / 1_000_000.0;
                state.vis_t.set(seconds);
                state.visualizer_canvas.queue_draw();
            }
            state.sync_lyrics_and_time();
            glib::ControlFlow::Continue
        });

        // Top Bar Button Handlers
        let back_state = Rc::downgrade(&state);
        back_btn.connect_clicked(move |_| {
            if let Some(state) = back_state.upgrade() {
                (state.on_open_library)();
            }
        });

        let lyrics_state = Rc::downgrade(&state);
        state.lyrics_btn.connect_toggled(move |btn| {
            if let Some(state) = lyrics_state.upgrade() {
                state.lyrics_active.set(btn.is_active());
                state.update_stage_layout();
            }
        });

        let vis_state = Rc::downgrade(&state);
        state.visualizer_btn.connect_toggled(move |btn| {
            if let Some(state) = vis_state.upgrade() {
                let active = btn.is_active();
                state.visualizer_active.set(active);
                state.visualizer_canvas.set_visible(active);
                state.style_selector.set_visible(active);
                state.stage_box.set_visible(!active);
                state.floating_dock.set_visible(active);
            }
        });

        let style_state = Rc::downgrade(&state);
        state.opt_ribbons.connect_toggled(move |btn| {
            if btn.is_active()
                && let Some(state) = style_state.upgrade()
            {
                state.visualizer_style.set(VisualizerStyle::Ribbons);
            }
        });

        let style_state = Rc::downgrade(&state);
        state.opt_tunnel.connect_toggled(move |btn| {
            if btn.is_active()
                && let Some(state) = style_state.upgrade()
            {
                state.visualizer_style.set(VisualizerStyle::Tunnel);
            }
        });

        let style_state = Rc::downgrade(&state);
        state.opt_cosmic.connect_toggled(move |btn| {
            if btn.is_active()
                && let Some(state) = style_state.upgrade()
            {
                state.visualizer_style.set(VisualizerStyle::Cosmic);
            }
        });

        // Transport Controls Handlers
        let ctrl = controller.clone();
        for btn in [&state.play_button, &state.floating_play] {
            let ctrl = ctrl.clone();
            btn.connect_clicked(move |_| {
                ctrl.toggle();
            });
        }

        let ctrl = controller.clone();
        next_button.connect_clicked(move |_| {
            ctrl.next();
        });

        let ctrl = controller.clone();
        prev_button.connect_clicked(move |_| {
            ctrl.previous();
        });

        let ctrl = controller.clone();
        state.speaker_button.connect_clicked(move |_| {
            if let Some(stream) = ctrl.stream() {
                stream.set_muted(!stream.is_muted());
            }
        });

        let ctrl = controller.clone();
        let vol_state = Rc::downgrade(&state);
        state.volume_scale.connect_value_changed(move |scale| {
            if let Some(state) = vol_state.upgrade()
                && !state.syncing.get()
            {
                ctrl.set_volume(scale.value());
            }
        });

        let ctrl = controller.clone();
        let scrub_state = Rc::downgrade(&state);
        state.scrub_scale.connect_change_value(move |_, _, value| {
            if let Some(state) = scrub_state.upgrade()
                && !state.syncing.get()
            {
                let duration = state.controller.duration();
                if duration > 0 {
                    let target_pos = (value * duration as f64) as i64;
                    ctrl.seek(target_pos);
                }
            }
            glib::Propagation::Proceed
        });

        let ctrl = controller.clone();
        let shuff_state = Rc::downgrade(&state);
        state.shuffle_button.connect_toggled(move |btn| {
            if let Some(state) = shuff_state.upgrade()
                && !state.syncing.get()
            {
                ctrl.set_shuffle(btn.is_active());
            }
        });

        let ctrl = controller.clone();
        state.repeat_button.connect_clicked(move |_| {
            let next_repeat = match ctrl.repeat() {
                crate::music::Repeat::Off => crate::music::Repeat::All,
                crate::music::Repeat::All => crate::music::Repeat::One,
                crate::music::Repeat::One => crate::music::Repeat::Off,
            };
            ctrl.set_repeat(next_repeat);
        });

        // Controller Observation Handler
        let obs_state = Rc::downgrade(&state);
        let obs_cb: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(state) = obs_state.upgrade() {
                state.sync();
            }
        });
        controller.observe(&obs_cb);
        state._observer.replace(Some(obs_cb));

        state.sync();

        Self { overlay, state }
    }

    pub fn widget(&self) -> &gtk::Overlay {
        &self.overlay
    }

    pub fn sync(&self) {
        self.state.sync();
    }
}

impl ImmersiveState {
    fn sync(&self) {
        let track = self.controller.current_track();
        if *self.current_track.borrow() != track {
            self.current_track.replace(track.clone());
            if let Some(ref track) = track {
                self.title_label.set_text(&track.title());
                self.floating_title.set_text(&track.title());
                let subtitle = format!("{} · {}", track.artist(), track.album());
                self.artist_label.set_text(&subtitle);
                self.floating_artist.set_text(&track.artist());
                crate::ui::thumbnail::set_thumbnail_or_icon(
                    &self.art_image,
                    &file_entry(track),
                    icons::MUSIC,
                    384,
                    384,
                );
                // Load lyrics
                let lyrics = find_lyrics_for_audio_file(&track.path);
                self.current_lyrics.replace(lyrics);
                self.rebuild_lyrics_ui();
            } else {
                self.title_label.set_text("Nothing playing");
                self.artist_label.set_text("Choose a track in Music");
                self.floating_title.set_text("");
                self.floating_artist.set_text("");
                assets::set_primary_icon(&self.art_image, icons::MUSIC);
                self.current_lyrics.replace(None);
                self.rebuild_lyrics_ui();
            }
            self.update_stage_layout();
        }

        self.syncing.set(true);
        self.volume_scale.set_value(self.controller.volume());
        self.shuffle_button.set_active(self.controller.shuffle());

        let playing = self.controller.status() == super::controller::PlaybackStatus::Playing;
        if self.playing_icon.replace(playing) != playing {
            let icon = if playing { icons::PAUSE } else { icons::PLAY };
            self.play_button.set_child(Some(&assets::primary_icon(icon, 20)));
            self.floating_play.set_child(Some(&assets::primary_icon(icon, 16)));
        }

        let muted = self
            .controller
            .stream()
            .is_some_and(|stream| stream.is_muted());
        if self.muted_icon.replace(muted) != muted {
            self.speaker_button.set_child(Some(&assets::primary_icon(
                if muted { icons::MUTED } else { icons::VOLUME },
                16,
            )));
        }

        let repeat = self.controller.repeat();
        self.repeat_button.set_child(Some(&assets::primary_icon(
            if repeat == crate::music::Repeat::One {
                icons::REPEAT_ONE
            } else {
                icons::REPEAT
            },
            16,
        )));
        if repeat == crate::music::Repeat::Off {
            self.repeat_button.remove_css_class("repeat-enabled");
        } else {
            self.repeat_button.add_css_class("repeat-enabled");
        }

        self.syncing.set(false);
        self.sync_lyrics_and_time();
    }

    fn rebuild_lyrics_ui(&self) {
        // Clear previous lyrics
        while let Some(child) = self.lyrics_box.first_child() {
            self.lyrics_box.remove(&child);
        }
        self.lyric_labels.borrow_mut().clear();
        self.active_lyric_index.set(None);

        let lyrics = self.current_lyrics.borrow();
        if let Some(ref lyrics) = *lyrics {
            let ctrl = self.controller.clone();
            for line in &lyrics.lines {
                let label = gtk::Label::new(Some(&line.text));
                label.add_css_class("immersive-lyric-line");
                label.set_xalign(0.0);
                label.set_wrap(true);
                label.set_cursor_from_name(Some("pointer"));

                let line_timestamp = line.timestamp.as_micros() as i64;
                let ctrl = ctrl.clone();
                let click = gtk::GestureClick::new();
                click.connect_pressed(move |_, _, _, _| {
                    ctrl.seek(line_timestamp);
                });
                label.add_controller(click);

                self.lyrics_box.append(&label);
                self.lyric_labels.borrow_mut().push(label);
            }
        }
    }

    fn sync_lyrics_and_time(&self) {
        let position = self.controller.position();
        let duration = self.controller.duration();

        let fraction = if duration > 0 {
            (position as f64 / duration as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };

        self.syncing.set(true);
        self.scrub_scale.set_value(fraction);
        self.syncing.set(false);

        self.elapsed_label.set_text(&time_label(position));
        let rem = duration - position;
        self.remaining_label.set_text(&format!("-{}", time_label(rem.max(0))));
        self.floating_time
            .set_text(&format!("{} / {}", time_label(position), time_label(duration)));

        // Update active lyric line
        if let Some(ref lyrics) = *self.current_lyrics.borrow() {
            let active_idx = lyrics.active_line_at(Duration::from_micros(position as u64));
            if active_idx != self.active_lyric_index.get() {
                let labels = self.lyric_labels.borrow();
                // Deactivate prior
                if let Some(prev) = self.active_lyric_index.get()
                    && let Some(label) = labels.get(prev)
                {
                    label.remove_css_class("active-lyric");
                }
                // Activate next
                if let Some(next) = active_idx
                    && let Some(label) = labels.get(next)
                {
                    label.add_css_class("active-lyric");
                    // Auto-scroll into center view
                    let adj = self.lyrics_scroll.vadjustment();
                    if let Some(bounds) = label.compute_bounds(&self.lyrics_box) {
                        let target = bounds.y() as f64 - (self.lyrics_scroll.height() as f64 / 2.0)
                            + (bounds.height() as f64 / 2.0);
                        adj.set_value(target.clamp(adj.lower(), (adj.upper() - adj.page_size()).max(0.0)));
                    }
                }
                self.active_lyric_index.set(active_idx);
            }
        }
    }

    fn update_stage_layout(&self) {
        let has_lyrics = self.current_lyrics.borrow().is_some();
        let show_lyrics = self.lyrics_active.get() && has_lyrics;

        self.lyrics_scroll.set_visible(show_lyrics);
        if show_lyrics {
            self.stage_box.remove_css_class("immersive-stage-solo");
            self.art_image.set_size_request(320, 320);
        } else {
            self.stage_box.add_css_class("immersive-stage-solo");
            self.art_image.set_size_request(380, 380);
        }
    }
}

fn current_theme_palette() -> ((f64, f64, f64), (f64, f64, f64)) {
    let tokens = ThemeManager::shared().applied_tokens();
    let accent = tokens
        .as_ref()
        .and_then(|t| gdk::RGBA::parse(&t.accent).ok())
        .map_or((0.85, 0.45, 0.2), |c| {
            (c.red() as f64, c.green() as f64, c.blue() as f64)
        });
    let highlight = tokens
        .as_ref()
        .and_then(|t| gdk::RGBA::parse(&t.highlight).ok())
        .map_or((0.2, 0.6, 0.8), |c| {
            (c.red() as f64, c.green() as f64, c.blue() as f64)
        });
    (accent, highlight)
}

pub fn draw_ambient_aura(
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    (r, g, b): (f64, f64, f64),
    (hr, hg, hb): (f64, f64, f64),
) {
    // Left aura lobe
    let pat1 = gtk::cairo::RadialGradient::new(
        width * 0.28,
        height * 0.42,
        20.0,
        width * 0.28,
        height * 0.42,
        width * 0.45,
    );
    pat1.add_color_stop_rgba(0.0, r, g, b, 0.28);
    pat1.add_color_stop_rgba(1.0, r, g, b, 0.0);
    let _ = cr.set_source(&pat1);
    let _ = cr.paint();

    // Right aura lobe
    let pat2 = gtk::cairo::RadialGradient::new(
        width * 0.72,
        height * 0.48,
        30.0,
        width * 0.72,
        height * 0.48,
        width * 0.48,
    );
    pat2.add_color_stop_rgba(0.0, hr, hg, hb, 0.22);
    pat2.add_color_stop_rgba(1.0, hr, hg, hb, 0.0);
    let _ = cr.set_source(&pat2);
    let _ = cr.paint();
}

pub fn draw_generative_visualizer(
    cr: &gtk::cairo::Context,
    width: f64,
    height: f64,
    t: f64,
    style: VisualizerStyle,
    spectrum: &[f32; crate::playback::SPECTRUM_BANDS],
    (ar, ag, ab): (f64, f64, f64),
    (hr, hg, hb): (f64, f64, f64),
) {

    match style {
        VisualizerStyle::Ribbons => {
            // Neon Ribbons
            for i in 0..4 {
                let base_y = height * (0.35 + 0.1 * i as f64);
                let freq = 0.0035 + 0.0006 * i as f64;
                let phase = t * (1.1 + 0.2 * i as f64) + i as f64 * 1.4;
                let band_energy = spectrum[(i * 4).min(31)] as f64;
                let amp = 24.0 + 85.0 * band_energy;

                let blend = (i as f64) / 3.0;
                let r = ar * (1.0 - blend) + hr * blend;
                let g = ag * (1.0 - blend) + hg * blend;
                let b = ab * (1.0 - blend) + hb * blend;

                cr.set_source_rgba(r, g, b, 0.65);
                cr.set_line_width(2.5);

                let mut x = 0.0;
                let step = 10.0;
                cr.new_path();
                let y0 = base_y + amp * (x * freq + phase).sin();
                cr.move_to(x, y0);
                while x <= width {
                    x += step;
                    let y = base_y + amp * (x * freq + phase).sin()
                        + (amp * 0.3) * (x * freq * 1.8 - phase).cos();
                    cr.line_to(x, y);
                }
                let _ = cr.stroke();
            }
        }
        VisualizerStyle::Tunnel => {
            // Warp Tunnel
            let cx = width / 2.0;
            let cy = height / 2.0;
            let bass = (spectrum[0] + spectrum[1] + spectrum[2] + spectrum[3]) as f64 / 4.0;

            for k in 0..12 {
                let p = (t * 0.75 + k as f64 / 12.0) % 1.0;
                let r = p * p * (width.max(height) * 0.68) + 12.0 * bass;
                let alpha = (p * std::f64::consts::PI).sin() * (0.28 + 0.6 * bass);

                let blend = p;
                let red = ar * (1.0 - blend) + hr * blend;
                let green = ag * (1.0 - blend) + hg * blend;
                let blue = ab * (1.0 - blend) + hb * blend;

                cr.set_source_rgba(red, green, blue, alpha.clamp(0.0, 1.0));
                cr.set_line_width(1.5 + 3.0 * p);
                cr.arc(cx, cy, r, 0.0, 2.0 * std::f64::consts::PI);
                let _ = cr.stroke();
            }
        }
        VisualizerStyle::Cosmic => {
            // Cosmic Dust
            let cx = width / 2.0;
            let cy = height / 2.0;
            let num_particles = 48;

            for j in 0..num_particles {
                let theta_base = (j as f64) * (2.0 * std::f64::consts::PI / num_particles as f64);
                let dist_ratio = ((j * 17) % 100) as f64 / 100.0;
                let base_r = dist_ratio * (width.min(height) * 0.42) + 24.0;
                let speed = 0.25 + 0.04 * ((j % 5) as f64);
                let theta = theta_base + t * speed;

                let energy = spectrum[(j % 16) + 8] as f64;
                let r = base_r + 40.0 * energy;
                let px = cx + r * theta.cos();
                let py = cy + r * theta.sin() * 0.65; // ellipse orbit
                let dot_size = 2.0 + 4.0 * energy;

                let alpha = (0.35 + 0.65 * energy).clamp(0.0, 1.0);
                cr.set_source_rgba(ar, ag, ab, alpha);
                cr.arc(px, py, dot_size, 0.0, 2.0 * std::f64::consts::PI);
                let _ = cr.fill();
            }
        }
    }
}
