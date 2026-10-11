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
};

#[cfg(test)]
mod tests;

pub(super) struct AudioPreview {
    pub widget: gtk::ScrolledWindow,
    pub stream: super::stream::PlaybackStream,
    artwork_load: RefCell<Option<LoadHandle>>,
}

impl AudioPreview {
    pub fn new(entry: &FileEntry, metadata: &AudioMetadata, on_play: Rc<dyn Fn()>) -> Option<Self> {
        let path = entry.location.native_path()?.to_owned();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.add_css_class("preview-audio");
        content.set_margin_start(16);
        content.set_margin_end(16);
        content.set_margin_top(16);
        content.set_margin_bottom(16);
        let artwork = gtk::Overlay::new();
        artwork.add_css_class("preview-audio-artwork");
        artwork.set_size_request(-1, 220);
        let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 8);
        placeholder.set_halign(gtk::Align::Center);
        placeholder.set_valign(gtk::Align::Center);
        placeholder.append(&assets::primary_icon(icons::MUSIC, 72));
        let artwork_status = gtk::Label::new(Some("Loading artwork…"));
        artwork_status.add_css_class("preview-audio-artwork-status");
        placeholder.append(&artwork_status);
        artwork.set_child(Some(&placeholder));
        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_visible(false);
        artwork.add_overlay(&picture);
        content.append(&artwork);

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
        title.set_xalign(0.0);
        content.append(&title);
        let stream = super::stream::PlaybackStream::new_audio(&path);
        let was_playing = Cell::new(false);
        stream.connect_playing_notify(move |stream| {
            if stream.is_playing() && !was_playing.replace(true) {
                on_play();
            } else if !stream.is_playing() {
                was_playing.set(false);
            }
        });
        content.append(&super::media::playback_controls(stream.upcast_ref()));
        let status = gtk::Label::new(None);
        status.add_css_class("preview-audio-status");
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.set_tooltip_text(Some(
            "Play this track without replacing your queue. Other music pauses; closing the preview does not resume it.",
        ));
        let update_status = move |stream: &gtk::MediaStream, status: &gtk::Label| {
            status.set_text(&if let Some(error) = stream.error() {
                format!(
                    "Playback unavailable: {error}. Open the original file to try another player."
                )
            } else if !stream.is_prepared() {
                "Loading playback…".into()
            } else if stream.is_seeking() {
                "Seeking…".into()
            } else if stream.is_ended() {
                "Track finished · Press Play to replay".into()
            } else {
                "Track preview · Music pauses while you listen".into()
            });
        };
        update_status(stream.upcast_ref(), &status);
        let weak_status = status.downgrade();
        stream.connect_notify_local(None, move |stream, _| {
            if let Some(status) = weak_status.upgrade() {
                update_status(stream.upcast_ref(), &status);
            }
        });
        content.append(&status);

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
        let widget = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .build();
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
        })
    }

    pub fn stop(&self) {
        self.artwork_load.borrow_mut().take();
        self.stream.stop();
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

impl Drop for AudioPreview {
    fn drop(&mut self) {
        self.stop();
    }
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
