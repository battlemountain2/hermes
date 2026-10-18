// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use crate::{
    assets::{self, icons},
    music::Track,
    ui::music::{controller::Controller, file_entry, time_label},
};

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub struct MiniPlayer {
    window: gtk::Window,
    state: Rc<MiniPlayerState>,
}

struct MiniPlayerState {
    controller: Rc<Controller>,
    cover: gtk::Image,
    title: gtk::Label,
    artist: gtk::Label,
    badge: gtk::Label,
    seek: gtk::Scale,
    elapsed: gtk::Label,
    remaining: gtk::Label,
    play: gtk::Button,
    speaker: gtk::Button,
    volume: gtk::Scale,
    playing_icon: Cell<bool>,
    muted_icon: Cell<bool>,
    syncing: Cell<bool>,
    displayed_track: RefCell<Option<Track>>,
}

impl MiniPlayer {
    pub fn new(controller: Rc<Controller>, on_expand_immersive: Rc<dyn Fn()>) -> Self {
        let window = gtk::Window::builder()
            .title("Hermes Mini")
            .default_width(380)
            .default_height(128)
            .resizable(false)
            .build();
        window.add_css_class("hermes-mini-window");

        let root_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root_box.add_css_class("mini-player-root");

        // Top Row: Art + Info + Expand Button
        let top_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let cover = assets::primary_icon(icons::MUSIC, 42);
        cover.set_size_request(48, 48);
        cover.add_css_class("mini-player-cover");
        top_row.append(&cover);

        let meta_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        meta_box.set_hexpand(true);
        let title = gtk::Label::new(Some("Nothing playing"));
        title.add_css_class("mini-player-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let artist_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let artist = gtk::Label::new(Some("Choose a track in Music"));
        artist.add_css_class("mini-player-artist");
        artist.set_xalign(0.0);
        artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
        artist_row.append(&artist);

        let badge = gtk::Label::new(Some("AUDIO"));
        badge.add_css_class("mini-subtle-badge");
        artist_row.append(&badge);

        meta_box.append(&title);
        meta_box.append(&artist_row);
        top_row.append(&meta_box);

        let expand_btn = gtk::Button::builder()
            .tooltip_text("Expand to Immersive Mode · Ctrl+I")
            .build();
        expand_btn.set_child(Some(&assets::primary_icon(icons::MAXIMIZE, 16)));
        expand_btn.add_css_class("mini-expand-btn");
        let expand_cb = on_expand_immersive.clone();
        expand_btn.connect_clicked(move |_| {
            expand_cb();
        });
        top_row.append(&expand_btn);
        root_box.append(&top_row);

        // Middle Row: Scrubber
        let seek_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
        seek.set_draw_value(false);
        seek.set_hexpand(true);
        seek.add_css_class("mini-scrub-scale");
        seek_box.append(&seek);

        let times_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let elapsed = gtk::Label::new(Some("0:00"));
        elapsed.add_css_class("mini-player-time");
        let time_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        time_spacer.set_hexpand(true);
        let remaining = gtk::Label::new(Some("0:00"));
        remaining.add_css_class("mini-player-time");
        times_box.append(&elapsed);
        times_box.append(&time_spacer);
        times_box.append(&remaining);
        seek_box.append(&times_box);
        root_box.append(&seek_box);

        // Bottom Row: Controls & Volume
        let bottom_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let prev = gtk::Button::builder().tooltip_text("Previous track").build();
        prev.set_child(Some(&assets::primary_icon(icons::ARROW_LEFT, 15)));
        prev.add_css_class("mini-control-btn");

        let play = gtk::Button::builder().tooltip_text("Play / Pause").build();
        play.set_child(Some(&assets::primary_icon(icons::PLAY, 16)));
        play.add_css_class("mini-play-btn");

        let next = gtk::Button::builder().tooltip_text("Next track").build();
        next.set_child(Some(&assets::primary_icon(icons::ARROW_RIGHT, 15)));
        next.add_css_class("mini-control-btn");

        bottom_row.append(&prev);
        bottom_row.append(&play);
        bottom_row.append(&next);

        let bottom_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bottom_spacer.set_hexpand(true);
        bottom_row.append(&bottom_spacer);

        let speaker = gtk::Button::builder().tooltip_text("Mute / Unmute").build();
        speaker.set_child(Some(&assets::primary_icon(icons::VOLUME, 15)));
        speaker.add_css_class("mini-control-btn");
        bottom_row.append(&speaker);

        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        volume.set_draw_value(false);
        volume.set_value(0.75);
        volume.set_size_request(65, -1);
        volume.add_css_class("mini-vol-scale");
        bottom_row.append(&volume);

        root_box.append(&bottom_row);
        window.set_child(Some(&root_box));

        let state = Rc::new(MiniPlayerState {
            controller: controller.clone(),
            cover,
            title,
            artist,
            badge,
            seek,
            elapsed,
            remaining,
            play,
            speaker,
            volume,
            playing_icon: Cell::new(false),
            muted_icon: Cell::new(false),
            syncing: Cell::new(false),
            displayed_track: RefCell::new(None),
        });

        // Event Listeners
        let ctrl = controller.clone();
        state.play.connect_clicked(move |_| {
            ctrl.toggle();
        });

        let ctrl = controller.clone();
        prev.connect_clicked(move |_| {
            ctrl.previous();
        });

        let ctrl = controller.clone();
        next.connect_clicked(move |_| {
            ctrl.next();
        });

        let ctrl = controller.clone();
        state.speaker.connect_clicked(move |_| {
            if let Some(stream) = ctrl.stream() {
                stream.set_muted(!stream.is_muted());
            }
        });

        let ctrl = controller.clone();
        let vol_state = Rc::downgrade(&state);
        state.volume.connect_value_changed(move |scale| {
            if let Some(state) = vol_state.upgrade()
                && !state.syncing.get()
            {
                ctrl.set_volume(scale.value());
            }
        });

        let ctrl = controller.clone();
        let seek_state = Rc::downgrade(&state);
        state.seek.connect_change_value(move |_, _, value| {
            if let Some(state) = seek_state.upgrade()
                && !state.syncing.get()
            {
                let duration = state.controller.duration();
                if duration > 0 {
                    ctrl.seek((value * duration as f64) as i64);
                }
            }
            glib::Propagation::Proceed
        });

        // Hide on close instead of destroying
        window.connect_close_request(|win| {
            win.set_visible(false);
            glib::Propagation::Stop
        });

        // Sync on tick
        let tick_state = Rc::downgrade(&state);
        window.add_tick_callback(move |win, _| {
            let Some(state) = tick_state.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !win.is_visible() {
                return glib::ControlFlow::Continue;
            }
            state.sync_progress();
            glib::ControlFlow::Continue
        });

        let obs_state = Rc::downgrade(&state);
        let obs_cb: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(state) = obs_state.upgrade() {
                state.sync();
            }
        });
        controller.observe(&obs_cb);

        state.sync();

        Self { window, state }
    }

    #[cfg(test)]
    pub fn window(&self) -> &gtk::Window {
        &self.window
    }

    pub fn toggle(&self) {
        if self.window.is_visible() {
            self.window.set_visible(false);
        } else {
            self.state.sync();
            self.window.present();
        }
    }

    #[cfg(test)]
    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }
}

impl MiniPlayerState {
    fn sync(&self) {
        let track = self.controller.current_track();
        if *self.displayed_track.borrow() != track {
            self.displayed_track.replace(track.clone());
            if let Some(ref track) = track {
                self.title.set_text(&track.title());
                self.artist.set_text(&format!("{} · {}", track.artist(), track.album()));
                let ext = track
                    .path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("AUDIO")
                    .to_uppercase();
                self.badge.set_text(&ext);
                crate::ui::thumbnail::set_thumbnail_or_icon(
                    &self.cover,
                    &file_entry(track),
                    icons::MUSIC,
                    42,
                    48,
                );
            } else {
                self.title.set_text("Nothing playing");
                self.artist.set_text("Choose a track in Music");
                self.badge.set_text("AUDIO");
                assets::set_primary_icon(&self.cover, icons::MUSIC);
            }
        }

        self.syncing.set(true);
        self.volume.set_value(self.controller.volume());

        let playing = self.controller.status() == super::controller::PlaybackStatus::Playing;
        if self.playing_icon.replace(playing) != playing {
            let icon = if playing { icons::PAUSE } else { icons::PLAY };
            self.play.set_child(Some(&assets::primary_icon(icon, 16)));
        }

        let muted = self
            .controller
            .stream()
            .is_some_and(|stream| stream.is_muted());
        if self.muted_icon.replace(muted) != muted {
            self.speaker.set_child(Some(&assets::primary_icon(
                if muted { icons::MUTED } else { icons::VOLUME },
                15,
            )));
        }
        self.syncing.set(false);

        self.sync_progress();
    }

    fn sync_progress(&self) {
        let position = self.controller.position();
        let duration = self.controller.duration();

        let fraction = if duration > 0 {
            (position as f64 / duration as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };

        self.syncing.set(true);
        self.seek.set_value(fraction);
        self.syncing.set(false);

        self.elapsed.set_text(&time_label(position));
        let rem = duration - position;
        self.remaining.set_text(&format!("-{}", time_label(rem.max(0))));
    }
}
