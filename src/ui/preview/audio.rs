use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::{
    assets::{self, icons},
    model::FileEntry,
    sandbox::Cancellation,
    services::{LoadHandle, preview::AudioMetadata},
    ui::seek::SeekControl,
};

mod vinyl;
mod visualizer;
mod layout;

#[cfg(test)]
mod tests;

use vinyl::VinylDisc;
use visualizer::MicroSegmentVisualizer;

pub(super) struct AudioPreview {
    pub widget: gtk::ScrolledWindow,
    pub stream: super::stream::PlaybackStream,
    artwork_load: RefCell<Option<LoadHandle>>,
    vinyl: Rc<VinylDisc>,
    visualizer: Rc<MicroSegmentVisualizer>,
    refresh_handler: RefCell<Option<glib::SignalHandlerId>>,
}

impl AudioPreview {
    pub fn new(entry: &FileEntry, metadata: &AudioMetadata, on_play: Rc<dyn Fn()>) -> Option<Self> {
        let path = entry.location.native_path()?.to_owned();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.add_css_class("preview-audio");
        content.set_margin_start(16);
        content.set_margin_end(16);
        content.set_margin_top(16);
        content.set_margin_bottom(16);

        // Artwork & Spinning Vinyl Record Hero
        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        hero.add_css_class("preview-audio-hero");
        hero.set_halign(gtk::Align::Center);
        hero.set_valign(gtk::Align::Center);

        let hero_overlay = gtk::Overlay::new();
        hero_overlay.add_css_class("preview-audio-hero-overlay");
        hero_overlay.set_size_request(230, 170);

        let vinyl = Rc::new(VinylDisc::new());
        vinyl.widget().set_halign(gtk::Align::End);
        vinyl.widget().set_valign(gtk::Align::Center);
        hero_overlay.set_child(Some(vinyl.widget()));

        let jacket = gtk::Overlay::new();
        jacket.add_css_class("preview-audio-jacket");
        jacket.set_size_request(170, 170);
        jacket.set_halign(gtk::Align::Start);
        jacket.set_valign(gtk::Align::Center);

        let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 8);
        placeholder.set_halign(gtk::Align::Center);
        placeholder.set_valign(gtk::Align::Center);
        placeholder.append(&assets::primary_icon(icons::MUSIC, 64));

        let artwork_status = gtk::Label::new(Some("Loading artwork…"));
        artwork_status.add_css_class("preview-audio-artwork-status");
        placeholder.append(&artwork_status);
        jacket.set_child(Some(&placeholder));

        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_visible(false);
        jacket.add_overlay(&picture);

        hero_overlay.add_overlay(&jacket);
        hero.append(&hero_overlay);

        // Controls Panel (Track info, Visualizer, Scrub bar, Transport)
        let controls_panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
        controls_panel.add_css_class("preview-audio-controls-panel");
        controls_panel.set_hexpand(true);
        controls_panel.set_valign(gtk::Align::Center);

        // Optional Track Index (e.g. Track 6 of 15)
        let track_text = format_track_number(metadata);
        if let Some(track_info) = track_text {
            let track_label = gtk::Label::new(Some(&track_info));
            track_label.add_css_class("preview-audio-track-number");
            track_label.set_xalign(0.0);
            controls_panel.append(&track_label);
        }

        // Song Title
        let title = gtk::Label::new(Some(
            metadata
                .tags
                .title
                .as_deref()
                .unwrap_or(&entry.display_name),
        ));
        title.add_css_class("preview-audio-title");
        title.set_selectable(true);
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        title.set_xalign(0.0);
        controls_panel.append(&title);

        // Subtitle (Artist — Album)
        let subtitle_text = format_subtitle(metadata);
        if !subtitle_text.is_empty() {
            let subtitle = gtk::Label::new(Some(&subtitle_text));
            subtitle.add_css_class("preview-audio-subtitle");
            subtitle.set_selectable(true);
            subtitle.set_wrap(true);
            subtitle.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            subtitle.set_xalign(0.0);
            controls_panel.append(&subtitle);
        }

        // Playback Stream
        let stream = super::stream::PlaybackStream::new_audio(&path);

        // Hi-Fi Micro-Segment Meter Visualizer
        let visualizer = Rc::new(MicroSegmentVisualizer::new(&stream));
        controls_panel.append(visualizer.widget());

        // Scrub Bar & Time Row
        let seek_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        seek_box.add_css_class("preview-audio-seek-box");

        let time_elapsed = gtk::Label::new(Some("0:00"));
        time_elapsed.add_css_class("preview-audio-time");
        time_elapsed.add_css_class("preview-audio-time-elapsed");

        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
        seek.add_css_class("preview-audio-seek");
        seek.set_draw_value(false);
        seek.set_hexpand(true);
        seek.set_tooltip_text(Some("Seek"));

        let time_total = gtk::Label::new(Some("0:00"));
        time_total.add_css_class("preview-audio-time");
        time_total.add_css_class("preview-audio-time-total");

        seek_box.append(&time_elapsed);
        seek_box.append(&seek);
        seek_box.append(&time_total);
        controls_panel.append(&seek_box);

        // Transport Controls Row
        let transport = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        transport.add_css_class("preview-audio-transport");

        let prev_button = icon_button(icons::ARROW_LEFT, "Previous track / Replay");
        prev_button.add_css_class("preview-audio-transport-button");

        let play_button = icon_button(icons::PLAY, "Play");
        play_button.add_css_class("preview-audio-play-button");
        play_button.set_child(Some(&assets::on_accent_icon(icons::PLAY, 18)));

        let next_button = icon_button(icons::ARROW_RIGHT, "Next track");
        next_button.add_css_class("preview-audio-transport-button");

        let transport_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        transport_spacer.set_hexpand(true);

        let mute_button = icon_button(icons::VOLUME, "Mute audio");
        mute_button.add_css_class("preview-audio-transport-button");

        let volume_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        volume_scale.add_css_class("preview-audio-volume-scale");
        volume_scale.set_draw_value(false);
        volume_scale.set_size_request(68, -1);
        volume_scale.set_tooltip_text(Some("Volume"));

        transport.append(&prev_button);
        transport.append(&play_button);
        transport.append(&next_button);
        transport.append(&transport_spacer);
        transport.append(&mute_button);
        transport.append(&volume_scale);
        controls_panel.append(&transport);

        let player_box = layout::PlayerCard::new(&hero, &controls_panel);
        content.append(&player_box);

        // Status description
        let status = gtk::Label::new(None);
        status.add_css_class("preview-audio-status");
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.set_tooltip_text(Some(
            "Play this track without replacing your queue. Other music pauses; closing the preview does not resume it.",
        ));
        content.append(&status);

        // Embedded Metadata Section
        let heading = gtk::Label::new(Some("Embedded metadata · Read-only"));
        heading.add_css_class("preview-audio-heading");
        heading.set_xalign(0.0);
        content.append(&heading);

        let rows = metadata_rows(metadata);
        let primary_count = 14 + usize::from(metadata.properties.bit_depth.is_some());
        let (primary, additional) = rows.split_at(primary_count);
        content.append(&metadata_grid(primary));

        if !additional.is_empty() {
            let expander = gtk::Expander::new(Some(&format!(
                "Additional embedded tags ({})",
                additional.len()
            )));
            expander.add_css_class("preview-audio-extra-tags");
            expander.set_child(Some(&metadata_grid(additional)));
            content.append(&expander);
        }

        // Wire Up Transport Actions
        let weak_stream = stream.downgrade();
        play_button.connect_clicked(move |_| {
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
        prev_button.connect_clicked(move |_| {
            if let Some(stream) = weak_stream.upgrade() {
                if stream.is_seekable() {
                    stream.seek(0);
                }
            }
        });

        let weak_stream = stream.downgrade();
        next_button.connect_clicked(move |_| {
            if let Some(stream) = weak_stream.upgrade() {
                if stream.is_seekable() {
                    let dur = stream.duration();
                    if dur > 0 {
                        stream.seek(dur);
                    }
                }
            }
        });

        let weak_stream = stream.downgrade();
        mute_button.connect_clicked(move |_| {
            if let Some(stream) = weak_stream.upgrade() {
                stream.set_muted(!stream.is_muted());
            }
        });

        let weak_stream = stream.downgrade();
        volume_scale.connect_change_value(move |_, _, val| {
            if let Some(stream) = weak_stream.upgrade() {
                stream.set_volume(val.clamp(0.0, 1.0));
            }
            glib::Propagation::Proceed
        });

        // Seek Control
        let stream_ref_for_seek = stream.downgrade();
        let stream_ref_for_target = stream.downgrade();
        let seek_control = SeekControl::new(
            &seek,
            move || stream_ref_for_seek.upgrade().map(|s| s.upcast::<gtk::MediaStream>()),
            move |target_time| {
                if let Some(stream) = stream_ref_for_target.upgrade() {
                    stream.seek(target_time);
                }
            },
        );

        // Sync State Refresh Loop
        let weak_stream = stream.downgrade();
        let weak_vinyl = Rc::downgrade(&vinyl);
        let weak_play_btn = play_button.downgrade();
        let weak_mute_btn = mute_button.downgrade();
        let weak_vol_scale = volume_scale.downgrade();
        let weak_time_elapsed = time_elapsed.downgrade();
        let weak_time_total = time_total.downgrade();
        let weak_status = status.downgrade();
        let weak_transport = transport.downgrade();
        let weak_visualizer = Rc::downgrade(&visualizer);

        let was_playing_cell = Cell::new(false);
        let prev_playing_icon = Cell::new(None);
        let prev_muted_icon = Cell::new(None);

        let refresh_ui = move || {
            let Some(stream) = weak_stream.upgrade() else {
                return;
            };

            let is_playing = stream.is_playing();
            if is_playing && !was_playing_cell.replace(true) {
                on_play();
            } else if !is_playing {
                was_playing_cell.set(false);
            }

            // Sync vinyl rotation state
            if let Some(vinyl) = weak_vinyl.upgrade() {
                vinyl.set_playing(is_playing);
            }

            // Sync visualizer tick loop
            if is_playing {
                if let Some(vis) = weak_visualizer.upgrade() {
                    vis.ensure_tick_running();
                }
            }

            // Update Play/Pause button
            if let Some(play_btn) = weak_play_btn.upgrade() {
                if prev_playing_icon.replace(Some(is_playing)) != Some(is_playing) {
                    set_button_icon(&play_btn, if is_playing { icons::PAUSE } else { icons::PLAY });
                }
                play_btn.set_tooltip_text(Some(if is_playing {
                    "Pause"
                } else if stream.is_ended() && stream.is_seekable() {
                    "Replay from beginning"
                } else {
                    "Play"
                }));
            }

            // Update Mute button
            let is_muted = stream.is_muted();
            if let Some(mute_btn) = weak_mute_btn.upgrade() {
                if prev_muted_icon.replace(Some(is_muted)) != Some(is_muted) {
                    set_button_icon(&mute_btn, if is_muted { icons::MUTED } else { icons::VOLUME });
                }
                mute_btn.set_tooltip_text(Some(if is_muted { "Unmute audio" } else { "Mute audio" }));
            }

            // Update Volume Slider
            if let Some(vol_scale) = weak_vol_scale.upgrade() {
                vol_scale.set_value(stream.volume());
            }

            // Update Time Labels & Seek
            seek_control.sync();
            let duration = stream.duration().max(0);
            if let Some(time_label) = weak_time_elapsed.upgrade() {
                time_label.set_text(&super::media::format_time(stream.timestamp()));
            }
            if let Some(total_label) = weak_time_total.upgrade() {
                total_label.set_text(&super::media::format_time(duration));
            }

            // Update Sensitivity
            let ready = stream.is_prepared() && stream.error().is_none();
            if let Some(transport) = weak_transport.upgrade() {
                transport.set_sensitive(ready);
            }

            // Update Status text
            if let Some(status_label) = weak_status.upgrade() {
                status_label.set_text(&if let Some(error) = stream.error() {
                    format!("Playback unavailable: {error}. Open the original file to try another player.")
                } else if !stream.is_prepared() {
                    "Loading playback…".into()
                } else if stream.is_seeking() {
                    "Seeking…".into()
                } else if stream.is_ended() {
                    "Track finished · Press Play to replay".into()
                } else {
                    "Track preview · Music pauses while you listen".into()
                });
            }
        };

        refresh_ui();
        let refresh_cb = Rc::new(refresh_ui);
        let cb_clone = refresh_cb.clone();
        let refresh_handler = stream.connect_notify_local(None, move |_, _| {
            cb_clone();
        });

        let widget = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .build();

        // Artwork Background Loader
        let cancellation = Cancellation::default();
        let task_cancellation = cancellation.clone();
        let weak_picture = picture.downgrade();
        let weak_placeholder = placeholder.downgrade();
        let weak_artwork_status = artwork_status.downgrade();

        let task = glib::MainContext::default().spawn_local(async move {
            let worker_cancellation = task_cancellation.clone();
            let result = gio::spawn_blocking(move || {
                crate::adapters::music_metadata::read_audio_artwork(&path, &worker_cancellation)
            })
            .await;
            if task_cancellation.is_cancelled() {
                return;
            }
            if let (Some(picture), Some(placeholder), Some(status)) = (
                weak_picture.upgrade(),
                weak_placeholder.upgrade(),
                weak_artwork_status.upgrade(),
            ) {
                if let Ok(Some(bytes)) = result
                    && let Ok(texture) =
                        gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
                {
                    picture.set_paintable(Some(&texture));
                    picture.set_visible(true);
                    placeholder.set_visible(false);
                } else {
                    status.set_text("No artwork available");
                }
            }
        });

        let artwork_load = LoadHandle::new(move || {
            cancellation.cancel();
            task.abort();
        });

        Some(Self {
            widget,
            stream,
            artwork_load: RefCell::new(Some(artwork_load)),
            vinyl,
            visualizer,
            refresh_handler: RefCell::new(Some(refresh_handler)),
        })
    }

    pub fn stop(&self) {
        if let Some(handler) = self.refresh_handler.borrow_mut().take() {
            self.stream.disconnect(handler);
        }
        self.artwork_load.borrow_mut().take();
        self.vinyl.stop();
        self.visualizer.stop();
        self.stream.stop();
    }
}

impl Drop for AudioPreview {
    fn drop(&mut self) {
        self.stop();
    }
}

fn format_track_number(metadata: &AudioMetadata) -> Option<String> {
    if let Some(track) = metadata.embedded_tags.get("track").or_else(|| metadata.embedded_tags.get("tracknumber")) {
        return Some(format!("Track {track}"));
    }
    if let Some(track) = metadata.tags.track {
        return Some(format!("Track {track}"));
    }
    None
}

fn format_subtitle(metadata: &AudioMetadata) -> String {
    match (&metadata.tags.artist, &metadata.tags.album) {
        (Some(artist), Some(album)) if !artist.is_empty() && !album.is_empty() => {
            format!("{artist} — {album}")
        }
        (Some(artist), _) if !artist.is_empty() => artist.clone(),
        (_, Some(album)) if !album.is_empty() => album.clone(),
        _ => String::new(),
    }
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&assets::primary_icon(icon, 18)));
    button
}

fn set_button_icon(button: &gtk::Button, icon: &str) {
    if let Some(image) = button
        .child()
        .and_then(|child| child.downcast::<gtk::Image>().ok())
    {
        if button.has_css_class("preview-audio-play-button") {
            assets::set_on_accent_icon(&image, icon);
        } else {
            assets::set_primary_icon(&image, icon);
        }
    }
}

fn metadata_grid(rows: &[(String, String)]) -> gtk::Grid {
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(16)
        .hexpand(true)
        .build();
    grid.add_css_class("preview-audio-metadata");
    for (index, (name, text)) in rows.iter().enumerate() {
        let label = gtk::Label::new(Some(name));
        label.add_css_class("preview-audio-tag-name");
        label.set_xalign(0.0);
        label.set_valign(gtk::Align::Start);
        let value = gtk::Label::new(Some(text));
        value.add_css_class("preview-audio-tag-value");
        value.set_selectable(true);
        value.set_xalign(0.0);
        value.set_hexpand(true);
        value.set_wrap(true);
        value.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        value.set_max_width_chars(40);
        grid.attach(&label, 0, index as i32, 1, 1);
        grid.attach(&value, 1, index as i32, 1, 1);
    }
    grid
}

fn metadata_rows(metadata: &AudioMetadata) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut add = |label: &str, value: Option<String>| {
        rows.push((
            label.into(),
            value
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "—".into()),
        ));
    };
    let tag = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| metadata.embedded_tags.get(*name).cloned())
    };
    add("Title", metadata.tags.title.clone());
    add("Artist", metadata.tags.artist.clone());
    add("Album", metadata.tags.album.clone());
    add("Album artist", metadata.tags.album_artist.clone());
    add("Genre", tag(&["genre"]));
    add(
        "Track",
        tag(&["tracknumber", "track"])
            .or_else(|| metadata.tags.track.map(|number| number.to_string())),
    );
    add(
        "Disc",
        tag(&["discnumber", "disc"])
            .or_else(|| metadata.tags.disc.map(|number| number.to_string())),
    );
    add("Year / date", tag(&["date", "year", "originaldate"]));
    let properties = &metadata.properties;
    add(
        "Duration",
        properties
            .duration_seconds
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(|seconds| super::media::format_time((seconds * 1_000_000.0) as i64)),
    );
    add(
        "Sample rate",
        properties.sample_rate.map(|rate| format!("{rate} Hz")),
    );
    add(
        "Bitrate",
        properties
            .bitrate
            .map(|rate| format!("{:.1} kb/s", rate as f64 / 1000.0)),
    );
    add("Codec", properties.codec.clone());
    add("Container", properties.container.clone());
    add(
        "Channels",
        properties.channels.map(|channels| match channels {
            1 => "1 (Mono)".into(),
            2 => "2 (Stereo)".into(),
            _ => channels.to_string(),
        }),
    );
    if let Some(depth) = properties.bit_depth {
        add("Bit depth", Some(format!("{depth}-bit")));
    }
    for (name, value) in &metadata.embedded_tags {
        if !matches!(
            name.as_str(),
            "title"
                | "artist"
                | "album"
                | "album_artist"
                | "albumartist"
                | "genre"
                | "tracknumber"
                | "track"
                | "discnumber"
                | "disc"
                | "date"
                | "year"
                | "originaldate"
        ) {
            rows.push((name.replace('_', " "), value.clone()));
        }
    }
    rows
}
