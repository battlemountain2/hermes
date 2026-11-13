use std::{cell::RefCell, rc::Rc, time::Duration};

use gtk::{glib, prelude::*};

use crate::assets::{self, icons};

#[cfg(test)]
mod tests;

pub(super) struct MediaPreview {
    pub widget: gtk::Box,
    pub stream: gtk::MediaStream,
    fullscreen: Rc<RefCell<Option<gtk::Window>>>,
    fullscreen_button: gtk::Button,
    playback_enabled: Rc<std::cell::Cell<bool>>,
    is_animation: bool,
    play_handler: RefCell<Option<glib::SignalHandlerId>>,
}

impl MediaPreview {
    pub fn new(stream: &gtk::MediaFile, is_audio: bool) -> Self {
        Self::build(stream.upcast_ref(), is_audio, false, false)
    }

    pub fn new_full(stream: &impl IsA<gtk::MediaStream>) -> Self {
        Self::build(stream.upcast_ref(), false, false, true)
    }

    pub fn new_animation(stream: &gtk::MediaFile) -> Self {
        stream.set_loop(false);
        stream.set_muted(true);
        let preview = Self::build(stream.upcast_ref(), false, true, false);
        let weak_widget = preview.widget.downgrade();
        let playback_enabled = preview.playback_enabled.clone();
        let restart_queued = Rc::new(std::cell::Cell::new(false));
        let seek_widget = weak_widget.clone();
        let seek_enabled = playback_enabled.clone();
        let seek_restart = restart_queued.clone();
        stream.connect_seeking_notify(move |stream| {
            if !stream.is_seeking() && seek_restart.get() {
                queue_animation_play(
                    stream.upcast_ref(),
                    seek_widget.clone(),
                    seek_enabled.clone(),
                    seek_restart.clone(),
                );
            }
        });
        stream.connect_ended_notify(move |stream| {
            if playback_enabled.get()
                && stream.is_ended()
                && weak_widget
                    .upgrade()
                    .is_some_and(|widget| widget.is_mapped())
                && !restart_queued.replace(true)
            {
                let weak_stream = stream.downgrade();
                let weak_widget = weak_widget.clone();
                let playback_enabled = playback_enabled.clone();
                let restart_queued = restart_queued.clone();
                glib::idle_add_local_once(move || {
                    if playback_enabled.get()
                        && weak_widget
                            .upgrade()
                            .is_some_and(|widget| widget.is_mapped())
                        && let Some(stream) = weak_stream.upgrade()
                        && stream.is_ended()
                    {
                        stream.seek(0);
                        if !stream.is_seeking() {
                            queue_animation_play(
                                stream.upcast_ref(),
                                weak_widget.clone(),
                                playback_enabled.clone(),
                                restart_queued.clone(),
                            );
                        }
                    } else {
                        restart_queued.set(false);
                    }
                });
            }
        });
        preview
    }

    fn build(
        stream: &gtk::MediaStream,
        is_audio: bool,
        is_animation: bool,
        full_length: bool,
    ) -> Self {
        stream.connect_prepared_notify(|stream| {
            tracing::info!(
                backend = stream.type_().name(),
                audio = stream.has_audio(),
                video = stream.has_video(),
                muted = stream.is_muted(),
                volume = stream.volume(),
                duration = stream.duration(),
                "preview playback prepared"
            );
        });
        stream.connect_playing_notify(|stream| {
            tracing::info!(
                playing = stream.is_playing(),
                audio = stream.has_audio(),
                muted = stream.is_muted(),
                volume = stream.volume(),
                "preview playback state"
            );
        });
        stream.connect_error_notify(|stream| {
            if let Some(error) = stream.error() {
                tracing::warn!(%error, "preview playback failed");
            }
        });
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("hermes-media-player");
        widget.set_vexpand(true);
        let fullscreen = Rc::new(RefCell::new(None::<gtk::Window>));
        let playback_enabled = Rc::new(std::cell::Cell::new(true));
        let picture = media_picture(stream);
        if is_animation {
            let started = Rc::new(std::cell::Cell::new(false));
            let map_started = started.clone();
            let map_enabled = playback_enabled.clone();
            let weak_stream = stream.downgrade();
            picture.connect_map(move |_| {
                if let Some(stream) = weak_stream.upgrade()
                    && map_enabled.get()
                    && stream.is_prepared()
                    && !map_started.replace(true)
                {
                    stream.play();
                }
            });
            let weak_picture = picture.downgrade();
            let prepare_enabled = playback_enabled.clone();
            stream.connect_prepared_notify(move |stream| {
                if prepare_enabled.get()
                    && stream.is_prepared()
                    && weak_picture
                        .upgrade()
                        .is_some_and(|picture| picture.is_mapped())
                    && !started.replace(true)
                {
                    stream.play();
                }
            });
        }
        widget.append(&picture);

        let controls = playback_controls(stream);
        let expand = icon_button(
            icons::MAXIMIZE,
            if is_animation {
                "Fullscreen animation (F)"
            } else {
                "Fullscreen video (F)"
            },
        );
        expand.set_visible(!is_audio);
        append_action(&controls, &expand);
        widget.append(&controls);

        let status = gtk::Label::new(Some("Loading playback…"));
        status.add_css_class("preview-note");
        status.set_wrap(true);
        widget.append(&status);
        let weak_status = status.downgrade();
        stream.connect_notify_local(None, move |stream, _| {
            if let Some(status) = weak_status.upgrade() {
                update_status(stream, &status, is_animation, full_length);
            }
        });
        update_status(stream, &status, is_animation, full_length);

        let weak_stream = stream.downgrade();
        let weak_widget = widget.downgrade();
        let fullscreen_slot = fullscreen.clone();
        expand.connect_clicked(move |_| {
            let Some(stream) = weak_stream.upgrade() else {
                return;
            };
            if let Some(window) = fullscreen_slot.borrow().as_ref() {
                window.present();
                return;
            }
            let window = gtk::Window::builder()
                .title(if is_animation {
                    "Hermes — Animated image"
                } else {
                    "Hermes — Video"
                })
                .build();
            if let Some(parent) = weak_widget
                .upgrade()
                .and_then(|widget| widget.root())
                .and_then(|root| root.downcast::<gtk::Window>().ok())
            {
                window.set_transient_for(Some(&parent));
                window.set_application(parent.application().as_ref());
            }
            let overlay = gtk::Overlay::new();
            overlay.add_css_class("hermes-media-player");
            overlay.set_child(Some(&media_picture(&stream)));
            let controls = playback_controls(&stream);
            let exit = icon_button(icons::MINIMIZE, "Exit fullscreen (F or Escape)");
            append_action(&controls, &exit);
            let revealer = gtk::Revealer::builder()
                .child(&controls)
                .reveal_child(true)
                .transition_duration(180)
                .valign(gtk::Align::End)
                .build();
            overlay.add_overlay(&revealer);
            let last_activity = Rc::new(std::cell::Cell::new(std::time::Instant::now()));
            let motion = gtk::EventControllerMotion::new();
            let activity = last_activity.clone();
            let weak_revealer = revealer.downgrade();
            motion.connect_motion(move |_, _, _| {
                activity.set(std::time::Instant::now());
                if let Some(revealer) = weak_revealer.upgrade() {
                    revealer.set_reveal_child(true);
                }
            });
            overlay.add_controller(motion);
            let key_activity = last_activity.clone();
            let weak_controls = controls.downgrade();
            let weak_revealer = revealer.downgrade();
            let weak_stream = stream.downgrade();
            overlay.add_tick_callback(move |_, _| {
                if let (Some(revealer), Some(controls), Some(stream)) = (
                    weak_revealer.upgrade(),
                    weak_controls.upgrade(),
                    weak_stream.upgrade(),
                ) {
                    let focused = controls
                        .root()
                        .and_then(|root| root.focus())
                        .is_some_and(|focus| focus.is_ancestor(&controls));
                    revealer.set_reveal_child(
                        !stream.is_playing()
                            || controls.state_flags().contains(gtk::StateFlags::PRELIGHT)
                            || focused
                            || last_activity.get().elapsed() < Duration::from_secs(3),
                    );
                }
                glib::ControlFlow::Continue
            });
            let weak_window = window.downgrade();
            exit.connect_clicked(move |_| {
                if let Some(window) = weak_window.upgrade() {
                    window.close();
                }
            });
            let keys = gtk::EventControllerKey::new();
            let weak_window = window.downgrade();
            let weak_revealer = revealer.downgrade();
            keys.connect_key_pressed(move |_, key, _, modifiers| {
                key_activity.set(std::time::Instant::now());
                if let Some(revealer) = weak_revealer.upgrade() {
                    revealer.set_reveal_child(true);
                }
                if key == gtk::gdk::Key::Escape || fullscreen_shortcut(key, modifiers) {
                    if let Some(window) = weak_window.upgrade() {
                        window.close();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            window.add_controller(keys);
            let weak_slot = Rc::downgrade(&fullscreen_slot);
            window.connect_close_request(move |_| {
                if let Some(slot) = weak_slot.upgrade() {
                    slot.borrow_mut().take();
                }
                glib::Propagation::Proceed
            });
            window.set_child(Some(&overlay));
            fullscreen_slot.replace(Some(window.clone()));
            window.fullscreen();
            window.present();
        });
        Self {
            widget,
            stream: stream.clone(),
            fullscreen,
            fullscreen_button: expand,
            playback_enabled,
            is_animation,
            play_handler: RefCell::new(None),
        }
    }

    pub fn set_play_handler(&self, on_play: Rc<dyn Fn()>) {
        if let Some(handler) = self.play_handler.borrow_mut().take() {
            self.stream.disconnect(handler);
        }
        if self.is_animation || !self.playback_enabled.get() {
            return;
        }
        let had_focus = std::cell::Cell::new(
            self.stream.is_playing() && self.stream.is_prepared() && self.stream.has_audio(),
        );
        let handler = self
            .stream
            .connect_notify_local(None, move |stream, property| {
                if !matches!(property.name(), "playing" | "prepared" | "has-audio") {
                    return;
                }
                let claims_focus =
                    stream.is_playing() && stream.is_prepared() && stream.has_audio();
                if had_focus.replace(claims_focus) != claims_focus && claims_focus {
                    on_play();
                }
            });
        self.play_handler.replace(Some(handler));
    }

    pub fn pause_for_music(&self) {
        if !self.is_animation && (!self.stream.is_prepared() || self.stream.has_audio()) {
            self.stream.pause();
        }
    }

    pub fn open_fullscreen(&self) -> bool {
        if !self.fullscreen_button.is_visible()
            || !self.stream.is_prepared()
            || !self.stream.has_video()
            || self.stream.error().is_some()
        {
            return false;
        }
        self.fullscreen_button.emit_clicked();
        true
    }

    pub fn stop(&self) {
        self.playback_enabled.set(false);
        if let Some(handler) = self.play_handler.borrow_mut().take() {
            self.stream.disconnect(handler);
        }
        self.stream.pause();
        if let Some(stream) = self.stream.downcast_ref::<super::stream::PlaybackStream>() {
            stream.stop();
        }
        let window = self.fullscreen.borrow_mut().take();
        if let Some(window) = window {
            window.close();
        }
    }
}

impl Drop for MediaPreview {
    fn drop(&mut self) {
        self.stop();
    }
}

fn queue_animation_play(
    stream: &gtk::MediaStream,
    widget: glib::WeakRef<gtk::Box>,
    enabled: Rc<std::cell::Cell<bool>>,
    restart: Rc<std::cell::Cell<bool>>,
) {
    let stream = stream.downgrade();
    glib::idle_add_local_once(move || {
        if enabled.get()
            && widget.upgrade().is_some_and(|widget| widget.is_mapped())
            && let Some(stream) = stream.upgrade()
            && !stream.is_seeking()
            && stream.error().is_none()
            && restart.replace(false)
        {
            stream.play();
        }
    });
}

pub(super) fn fullscreen_shortcut(key: gtk::gdk::Key, modifiers: gtk::gdk::ModifierType) -> bool {
    matches!(key, gtk::gdk::Key::f | gtk::gdk::Key::F)
        && !modifiers.intersects(
            gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK,
        )
}

fn append_action(controls: &gtk::Box, button: &gtk::Button) {
    if let Some(row) = controls
        .last_child()
        .and_then(|child| child.downcast::<gtk::Box>().ok())
    {
        row.append(button);
    }
}

fn media_picture(stream: &gtk::MediaStream) -> gtk::Picture {
    let picture = gtk::Picture::for_paintable(stream);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    let surface = Rc::new(RefCell::new(None));
    let realized_surface = surface.clone();
    let weak_stream = stream.downgrade();
    picture.connect_realize(move |picture| {
        if let (Some(stream), Some(surface)) = (
            weak_stream.upgrade(),
            picture.native().and_then(|native| native.surface()),
        ) {
            stream.realize(&surface);
            realized_surface.replace(Some(surface));
        }
    });
    let weak_stream = stream.downgrade();
    picture.connect_unrealize(move |_| {
        if let (Some(stream), Some(surface)) = (weak_stream.upgrade(), surface.borrow_mut().take())
        {
            stream.unrealize(&surface);
        }
    });
    picture
}

pub(super) fn playback_controls(stream: &gtk::MediaStream) -> gtk::Box {
    let controls = gtk::Box::new(gtk::Orientation::Vertical, 4);
    controls.add_css_class("hermes-media-controls");
    let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
    seek.set_draw_value(false);
    seek.set_tooltip_text(Some("Seek"));
    controls.append(&seek);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.append(&row);
    let play = icon_button(icons::PLAY, "Play");
    row.append(&play);
    let time = gtk::Label::new(Some("0:00 / 0:00"));
    time.add_css_class("hermes-media-time");
    time.set_hexpand(true);
    time.set_xalign(0.0);
    row.append(&time);
    let mute = icon_button(icons::VOLUME, "Mute audio");
    row.append(&mute);
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
    volume.set_draw_value(false);
    volume.set_size_request(72, -1);
    volume.set_tooltip_text(Some("Volume"));
    row.append(&volume);
    let weak_stream = stream.downgrade();
    play.connect_clicked(move |_| {
        if let Some(stream) = weak_stream.upgrade() {
            if stream.is_playing() {
                stream.pause();
            } else {
                if stream.is_ended() && stream.is_seekable() {
                    stream.seek(0);
                }
                stream.play();
            }
        }
    });
    let weak_stream = stream.downgrade();
    mute.connect_clicked(move |_| {
        if let Some(stream) = weak_stream.upgrade() {
            stream.set_muted(!stream.is_muted());
        }
    });
    let source = stream.downgrade();
    let target = stream.downgrade();
    let seek_control = super::super::seek::SeekControl::new(
        &seek,
        move || source.upgrade(),
        move |timestamp| {
            if let Some(stream) = target.upgrade() {
                stream.seek(timestamp);
            }
        },
    );
    let weak_stream = stream.downgrade();
    volume.connect_change_value(move |_, _, value| {
        if let Some(stream) = weak_stream.upgrade() {
            stream.set_volume(value.clamp(0.0, 1.0));
        }
        glib::Propagation::Proceed
    });
    let weak_controls = controls.downgrade();
    let weak_play = play.downgrade();
    let weak_mute = mute.downgrade();
    let weak_time = time.downgrade();
    let weak_volume = volume.downgrade();
    let previous_playing = std::cell::Cell::new(None);
    let previous_muted = std::cell::Cell::new(None);
    let refresh = move |stream: &gtk::MediaStream| {
        let (Some(controls), Some(play), Some(mute), Some(time), Some(volume)) = (
            weak_controls.upgrade(),
            weak_play.upgrade(),
            weak_mute.upgrade(),
            weak_time.upgrade(),
            weak_volume.upgrade(),
        ) else {
            return;
        };
        controls.set_sensitive(stream.is_prepared() && stream.error().is_none());
        let duration = stream.duration().max(0);
        seek_control.sync();
        time.set_text(&format!(
            "{} / {}",
            format_time(stream.timestamp()),
            format_time(duration)
        ));
        if previous_playing.replace(Some(stream.is_playing())) != Some(stream.is_playing()) {
            set_button_icon(
                &play,
                if stream.is_playing() {
                    icons::PAUSE
                } else {
                    icons::PLAY
                },
            );
        }
        play.set_tooltip_text(Some(if stream.is_playing() {
            "Pause"
        } else if stream.is_ended() && stream.is_seekable() {
            "Replay from beginning"
        } else {
            "Play"
        }));
        if previous_muted.replace(Some(stream.is_muted())) != Some(stream.is_muted()) {
            set_button_icon(
                &mute,
                if stream.is_muted() {
                    icons::MUTED
                } else {
                    icons::VOLUME
                },
            );
        }
        mute.set_tooltip_text(Some(if stream.is_muted() {
            "Unmute audio"
        } else {
            "Mute audio"
        }));
        mute.set_sensitive(stream.has_audio());
        volume.set_sensitive(stream.has_audio());
        mute.set_visible(!stream.is_prepared() || stream.has_audio());
        volume.set_visible(!stream.is_prepared() || stream.has_audio());
        volume.set_value(stream.volume());
    };
    refresh(stream);
    stream.connect_notify_local(None, move |stream, _| refresh(stream));
    controls
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&assets::primary_icon(icon, 18)));
    button.add_css_class("hermes-media-action");
    button
}

fn set_button_icon(button: &gtk::Button, icon: &str) {
    if let Some(image) = button
        .child()
        .and_then(|child| child.downcast::<gtk::Image>().ok())
    {
        assets::set_primary_icon(&image, icon);
    }
}

fn update_status(
    stream: &gtk::MediaStream,
    status: &gtk::Label,
    is_animation: bool,
    full_length: bool,
) {
    if let Some(error) = stream.error() {
        status.set_text(&format!(
            "Playback unavailable: {error}. Try opening the original file."
        ));
    } else if stream.is_prepared() {
        if is_animation {
            status.set_text("Animated image · Looping playback · Up to 30 seconds");
        } else if full_length {
            status.set_text(if stream.is_seeking() {
                "Seeking…"
            } else if stream.is_ended() {
                "Video finished · Press Play to replay"
            } else {
                ""
            });
        } else {
            status.set_text(&format!(
                "Preview · {} available · Open original for full playback",
                format_time(stream.duration())
            ));
        }
    } else {
        status.set_text("Loading playback…");
    }
    status.set_visible(!status.text().is_empty());
}

pub(super) fn format_time(microseconds: i64) -> String {
    let seconds = microseconds.max(0) / 1_000_000;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}
