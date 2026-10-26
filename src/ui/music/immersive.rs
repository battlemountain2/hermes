// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};

use crate::{
    assets::{self, icons},
    music::Track,
    sandbox::{Cancellation, ParseOperation},
    services::lyrics::{Lyrics, find_lyrics_for_audio_file, online},
    ui::{
        music::{controller::Controller, time_label},
        theme::ThemeManager,
    },
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod performance_tests;

mod canvas;
mod artwork;
mod backdrop;

struct AssetRequest {
    identity: Rc<()>,
    cancellation: Cancellation,
}

impl AssetRequest {
    fn new() -> Self {
        Self {
            identity: Rc::new(()),
            cancellation: Cancellation::default(),
        }
    }

    fn accepts(&self, identity: &Rc<()>) -> bool {
        Rc::ptr_eq(&self.identity, identity) && !self.cancellation.is_cancelled()
    }
}

impl Drop for AssetRequest {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[derive(Default)]
struct TrackAssets {
    artwork: Option<Vec<u8>>,
    lyrics: Option<Lyrics>,
    backdrop: Option<Vec<u8>>,
    palette: Option<artwork::Palette>,
}

fn read_track_assets(path: &Path, cancellation: &Cancellation) -> TrackAssets {
    let mut result = TrackAssets::default();
    if cancellation.is_cancelled() {
        return result;
    }
    result.lyrics = find_lyrics_for_audio_file(path);
    if cancellation.is_cancelled() {
        return result;
    }
    let folder_art = path
        .parent()
        .and_then(crate::ui::thumbnail::folder_album_art);
    if cancellation.is_cancelled() {
        return result;
    }
    if let Some((cover, ParseOperation::ThumbnailImage)) = folder_art {
        result.artwork =
            crate::sandbox::parse(&cover, ParseOperation::PreviewImage, 1400, cancellation)
                .ok()
                .map(|output| output.data);
    }
    if result.artwork.is_none() && !cancellation.is_cancelled() {
        result.artwork =
            crate::sandbox::parse(path, ParseOperation::PreviewAudioArtwork, 1024, cancellation)
                .ok()
                .map(|output| output.data);
    }
    if !cancellation.is_cancelled()
        && let Some((backdrop, palette)) = result.artwork.as_deref().and_then(artwork::presentation)
    {
        result.backdrop = Some(backdrop);
        result.palette = Some(palette);
    }
    result
}

fn lyrics_metadata(track: &Track) -> Option<(&str, &str)> {
    fn nonempty(value: Option<&str>) -> Option<&str> {
        value.map(str::trim).filter(|value| !value.is_empty())
    }
    let title = nonempty(track.tags.title.as_deref())?;
    let artist = nonempty(track.tags.artist.as_deref())
        .or_else(|| nonempty(track.tags.album_artist.as_deref()))?;
    Some((title, artist))
}

fn same_lyrics_track(first: &Track, second: &Track) -> bool {
    first.path == second.path && lyrics_metadata(first) == lyrics_metadata(second)
}

fn needs_visualizer_frames(
    mapped: bool,
    stopped: bool,
    active: bool,
    playing: bool,
    motion: bool,
) -> bool {
    mapped && !stopped && active && playing && motion
}

fn visualizer_frame_delta(
    previous: Option<i64>,
    deadline: Option<i64>,
    now: i64,
) -> Option<(f64, i64)> {
    const INTERVAL: i64 = 1_000_000 / 60;
    let Some(previous) = previous else {
        return Some((0.0, now.saturating_add(INTERVAL)));
    };
    let deadline = deadline.unwrap_or_else(|| previous.saturating_add(INTERVAL));
    let elapsed = now.saturating_sub(previous);
    if now < deadline || elapsed <= 0 {
        return None;
    }
    let intervals = now.saturating_sub(deadline) / INTERVAL + 1;
    Some((
        elapsed.min(100_000) as f64 / 1_000_000.0,
        deadline.saturating_add(intervals.saturating_mul(INTERVAL)),
    ))
}

fn smooth_spectrum(previous: [f32; 32], target: [f32; 32], seconds: f64) -> [f32; 32] {
    let seconds = if seconds.is_finite() {
        seconds.clamp(0.0, 0.1)
    } else {
        0.0
    };
    let attack = (1.0 - (-seconds / 0.045).exp()) as f32;
    let release = (1.0 - (-seconds / 0.18).exp()) as f32;
    std::array::from_fn(|index| {
        let clean = |value: f32| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let current = clean(previous[index]);
        let target = clean(target[index]);
        let factor = if target > current { attack } else { release };
        current + (target - current) * factor
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LyricsStatus {
    NoTrack,
    LoadingLocal,
    LocalMissing,
    LoadingOnline,
    OnlineMissing,
    Instrumental,
    MissingMetadata,
    Failed(String),
    Local,
    Online,
}

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
    overlay: glib::WeakRef<gtk::Overlay>,
    controller: Rc<Controller>,
    _observer: RefCell<Option<Rc<dyn Fn()>>>,
    lyrics_observer: RefCell<Option<Rc<dyn Fn()>>>,
    mode_syncing: Cell<bool>,
    on_open_library: Rc<dyn Fn()>,
    ambient_aura: backdrop::ArtworkBackdrop,
    artwork_palette: Cell<Option<artwork::Palette>>,
    palette: Cell<((f64, f64, f64), (f64, f64, f64))>,
    visualizer_canvas: canvas::VisualizerCanvas,
    visualizer_bands: Cell<[f32; crate::playback::SPECTRUM_BANDS]>,
    visualizer_active: Cell<bool>,
    visualizer_style: Cell<VisualizerStyle>,
    lyrics_active: Cell<bool>,
    current_track: RefCell<Option<Track>>,
    current_generation: Cell<u64>,
    track_dirty: Cell<bool>,
    stopped: Cell<bool>,
    asset_request: RefCell<Option<AssetRequest>>,
    asset_task: RefCell<Option<glib::JoinHandle<()>>>,
    lyrics_request: RefCell<Option<AssetRequest>>,
    lyrics_task: RefCell<Option<glib::JoinHandle<()>>>,
    lyrics_attempted: Cell<bool>,
    lyrics_status: RefCell<LyricsStatus>,
    lyrics_built: Cell<bool>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    current_lyrics: RefCell<Option<Lyrics>>,
    active_lyric_index: Cell<Option<usize>>,
    lyric_labels: RefCell<Vec<gtk::Label>>,
    lyrics_box: gtk::Box,
    lyrics_scroll: gtk::ScrolledWindow,
    stage_box: gtk::Box,
    art_box: gtk::Box,
    art_stack: gtk::Stack,
    art_picture: gtk::Picture,
    card_visualizer: canvas::VisualizerCanvas,
    title_label: gtk::Label,
    artist_label: gtk::Label,
    scrub_scale: gtk::Scale,
    elapsed_label: gtk::Label,
    remaining_label: gtk::Label,
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
    floating_shuffle: gtk::ToggleButton,
    floating_prev: gtk::Button,
    floating_play: gtk::Button,
    floating_next: gtk::Button,
    floating_repeat: gtk::Button,
    floating_speaker: gtk::Button,
    floating_volume: gtk::Scale,
    floating_scrub: gtk::Scale,
    floating_time: gtk::Label,
    vis_t: Cell<f64>,
    last_frame_us: Cell<Option<i64>>,
    next_frame_us: Cell<Option<i64>>,
    playing_icon: Cell<bool>,
    muted_icon: Cell<bool>,
    repeat_icon: Cell<Option<crate::music::Repeat>>,
    syncing: Cell<bool>,
    last_second: Cell<i64>,
    last_duration: Cell<i64>,
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
        let ambient_aura = backdrop::ArtworkBackdrop::new();
        ambient_aura.set_hexpand(true);
        ambient_aura.set_vexpand(true);
        ambient_aura.add_css_class("immersive-ambient-aura");
        overlay.set_child(Some(&ambient_aura));

        // 2. Full-Screen Visualizer Canvas
        let visualizer_canvas = canvas::VisualizerCanvas::new();
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
            .tooltip_text("Lyrics")
            .build();
        lyrics_btn.add_css_class("immersive-pill-btn");
        let lyrics_btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        lyrics_btn_box.append(&assets::primary_icon(icons::DOCUMENTS, 15));
        lyrics_btn_box.append(&gtk::Label::new(Some("Lyrics")));
        lyrics_btn.set_child(Some(&lyrics_btn_box));
        top_bar.append(&lyrics_btn);

        let visualizer_btn = gtk::ToggleButton::builder()
            .active(false)
            .tooltip_text("Visualizer")
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

        let opt_tunnel = gtk::ToggleButton::with_label("Prism");
        opt_tunnel.add_css_class("immersive-style-opt");
        opt_tunnel.set_group(Some(&opt_ribbons));

        let opt_cosmic = gtk::ToggleButton::with_label("Orbit");
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
        // Deck: Artwork + Meta + Scrub + Transport Controls + Volume
        let deck_container = gtk::Box::new(gtk::Orientation::Vertical, 14);
        deck_container.add_css_class("immersive-deck");
        deck_container.set_valign(gtk::Align::Center);

        // Artwork Frame
        let art_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        art_box.set_overflow(gtk::Overflow::Hidden);
        art_box.add_css_class("immersive-art-box");
        art_box.set_size_request(480, 480);

        let art_stack = gtk::Stack::new();
        art_stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        art_stack.set_transition_duration(200);

        let art_picture = gtk::Picture::new();
        art_picture.set_can_shrink(true);
        art_picture.set_content_fit(gtk::ContentFit::Cover);
        art_picture.add_css_class("immersive-art-pic");
        art_picture.set_size_request(480, 480);
        art_stack.add_named(&art_picture, Some("picture"));

        let card_visualizer = canvas::VisualizerCanvas::new();
        card_visualizer.add_css_class("immersive-card-visualizer");
        card_visualizer.set_size_request(480, 480);
        art_stack.add_named(&card_visualizer, Some("visualizer"));

        let art_fallback = assets::primary_icon(icons::MUSIC, 160);
        art_fallback.add_css_class("immersive-art-img");
        art_stack.add_named(&art_fallback, Some("fallback"));
        art_stack.set_visible_child_name("fallback");

        art_box.append(&art_stack);
        deck_container.append(&art_box);

        // Track Title & Artist (Directly below Artwork)
        let meta_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        meta_box.add_css_class("immersive-meta-box");
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

        // Scrubber Scale & Times (Horizontal row: elapsed + scale + remaining)
        let scrub_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        scrub_box.add_css_class("immersive-scrub-region");
        let elapsed_label = gtk::Label::new(Some("0:00"));
        elapsed_label.add_css_class("immersive-time");
        let scrub_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
        scrub_scale.set_draw_value(false);
        scrub_scale.set_hexpand(true);
        scrub_scale.add_css_class("immersive-scrub-scale");
        let remaining_label = gtk::Label::new(Some("0:00"));
        remaining_label.add_css_class("immersive-time");
        scrub_box.append(&elapsed_label);
        scrub_box.append(&scrub_scale);
        scrub_box.append(&remaining_label);
        deck_container.append(&scrub_box);

        // Centered Transport Controls
        let controls_row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        controls_row.set_halign(gtk::Align::Center);
        controls_row.add_css_class("immersive-controls");
        controls_row.set_tooltip_text(Some("Playback controls · Tab to focus"));

        let shuffle_button = gtk::ToggleButton::new();
        shuffle_button.set_child(Some(&assets::primary_icon(icons::SHUFFLE, 16)));
        shuffle_button.add_css_class("immersive-control-btn");
        shuffle_button.set_tooltip_text(Some("Shuffle"));

        let prev_button = gtk::Button::builder().tooltip_text("Previous").build();
        prev_button.set_child(Some(&assets::primary_icon(icons::ARROW_LEFT, 18)));
        prev_button.add_css_class("immersive-control-btn");

        let play_button = gtk::Button::builder().tooltip_text("Play / Pause").build();
        play_button.set_child(Some(&assets::on_accent_icon(icons::PLAY, 22)));
        play_button.add_css_class("immersive-play-btn");

        let next_button = gtk::Button::builder().tooltip_text("Next").build();
        next_button.set_child(Some(&assets::primary_icon(icons::ARROW_RIGHT, 18)));
        next_button.add_css_class("immersive-control-btn");

        let repeat_button = gtk::Button::builder().tooltip_text("Repeat").build();
        repeat_button.set_child(Some(&assets::primary_icon(icons::REPEAT, 16)));
        repeat_button.add_css_class("immersive-control-btn");

        controls_row.append(&shuffle_button);
        controls_row.append(&prev_button);
        controls_row.append(&play_button);
        controls_row.append(&next_button);
        controls_row.append(&repeat_button);
        deck_container.append(&controls_row);

        // Centered Volume Slider Row
        let vol_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        vol_row.set_halign(gtk::Align::Center);
        vol_row.add_css_class("immersive-vol-row");
        let speaker_button = gtk::Button::builder().tooltip_text("Mute / Unmute").build();
        speaker_button.set_child(Some(&assets::primary_icon(icons::VOLUME, 16)));
        speaker_button.add_css_class("immersive-control-btn");
        vol_row.append(&speaker_button);

        let volume_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        volume_scale.set_draw_value(false);
        volume_scale.set_value(0.75);
        volume_scale.set_size_request(160, -1);
        volume_scale.add_css_class("immersive-vol-scale");
        vol_row.append(&volume_scale);
        deck_container.append(&vol_row);

        install_control_focus(&deck_container);
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
        lyrics_scroll.set_size_request(460, 540);
        stage_box.append(&lyrics_scroll);

        content_box.append(&stage_box);

        // Floating Pill Dock (Shown during Visualizer mode)
        let floating_dock = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        floating_dock.add_css_class("immersive-floating-dock");
        install_control_focus(&floating_dock);
        floating_dock.set_halign(gtk::Align::Center);
        floating_dock.set_valign(gtk::Align::End);
        floating_dock.set_margin_bottom(24);
        floating_dock.set_visible(false);

        let floating_meta = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let floating_title = gtk::Label::new(Some(""));
        floating_title.add_css_class("immersive-floating-title");
        floating_title.set_xalign(0.0);
        floating_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let floating_artist = gtk::Label::new(Some(""));
        floating_artist.add_css_class("immersive-floating-artist");
        floating_artist.set_xalign(0.0);
        floating_artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
        floating_meta.append(&floating_title);
        floating_meta.append(&floating_artist);
        floating_dock.append(&floating_meta);

        let floating_scrub = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
        floating_scrub.set_draw_value(false);
        floating_scrub.set_size_request(160, -1);
        floating_scrub.add_css_class("immersive-floating-scrub");
        floating_dock.append(&floating_scrub);

        let floating_time = gtk::Label::new(Some("0:00 / 0:00"));
        floating_time.add_css_class("immersive-time");
        floating_dock.append(&floating_time);

        let floating_shuffle = gtk::ToggleButton::new();
        floating_shuffle.set_child(Some(&assets::primary_icon(icons::SHUFFLE, 14)));
        floating_shuffle.add_css_class("immersive-control-btn");
        floating_shuffle.set_tooltip_text(Some("Shuffle"));
        floating_dock.append(&floating_shuffle);

        let floating_prev = gtk::Button::builder().tooltip_text("Previous").build();
        floating_prev.set_child(Some(&assets::primary_icon(icons::ARROW_LEFT, 14)));
        floating_prev.add_css_class("immersive-control-btn");
        floating_dock.append(&floating_prev);

        let floating_play = gtk::Button::builder().tooltip_text("Play / Pause").build();
        floating_play.set_child(Some(&assets::on_accent_icon(icons::PLAY, 16)));
        floating_play.add_css_class("immersive-play-btn");
        floating_dock.append(&floating_play);

        let floating_next = gtk::Button::builder().tooltip_text("Next").build();
        floating_next.set_child(Some(&assets::primary_icon(icons::ARROW_RIGHT, 14)));
        floating_next.add_css_class("immersive-control-btn");
        floating_dock.append(&floating_next);

        let floating_repeat = gtk::Button::builder().tooltip_text("Repeat").build();
        floating_repeat.set_child(Some(&assets::primary_icon(icons::REPEAT, 14)));
        floating_repeat.add_css_class("immersive-control-btn");
        floating_dock.append(&floating_repeat);

        let floating_speaker = gtk::Button::builder().tooltip_text("Mute / Unmute").build();
        floating_speaker.set_child(Some(&assets::primary_icon(icons::VOLUME, 14)));
        floating_speaker.add_css_class("immersive-control-btn");
        floating_dock.append(&floating_speaker);

        let floating_volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        floating_volume.set_draw_value(false);
        floating_volume.set_value(0.75);
        floating_volume.set_size_request(70, -1);
        floating_volume.add_css_class("immersive-vol-scale");
        floating_dock.append(&floating_volume);

        overlay.add_overlay(&content_box);
        overlay.add_overlay(&floating_dock);

        let state = Rc::new(ImmersiveState {
            overlay: overlay.downgrade(),
            controller: controller.clone(),
            _observer: RefCell::new(None),
            lyrics_observer: RefCell::new(None),
            mode_syncing: Cell::new(false),
            on_open_library: on_open_library.clone(),
            ambient_aura: ambient_aura.clone(),
            artwork_palette: Cell::new(None),
            palette: Cell::new(current_theme_palette()),
            visualizer_canvas: visualizer_canvas.clone(),
            visualizer_bands: Cell::new([0.0; crate::playback::SPECTRUM_BANDS]),
            visualizer_active: Cell::new(false),
            visualizer_style: Cell::new(VisualizerStyle::Ribbons),
            lyrics_active: Cell::new(true),
            current_track: RefCell::new(None),
            current_generation: Cell::new(0),
            track_dirty: Cell::new(true),
            stopped: Cell::new(false),
            asset_request: RefCell::new(None),
            asset_task: RefCell::new(None),
            lyrics_request: RefCell::new(None),
            lyrics_task: RefCell::new(None),
            lyrics_attempted: Cell::new(false),
            lyrics_status: RefCell::new(LyricsStatus::LoadingLocal),
            lyrics_built: Cell::new(false),
            tick: RefCell::new(None),
            current_lyrics: RefCell::new(None),
            active_lyric_index: Cell::new(None),
            lyric_labels: RefCell::new(Vec::new()),
            lyrics_box,
            lyrics_scroll,
            stage_box,
            art_box,
            art_stack,
            art_picture,
            card_visualizer,
            title_label,
            artist_label,
            scrub_scale,
            elapsed_label,
            remaining_label,
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
            floating_shuffle,
            floating_prev,
            floating_play,
            floating_next,
            floating_repeat,
            floating_speaker,
            floating_volume,
            floating_scrub,
            floating_time,
            vis_t: Cell::new(0.0),
            last_frame_us: Cell::new(None),
            next_frame_us: Cell::new(None),
            playing_icon: Cell::new(false),
            muted_icon: Cell::new(false),
            repeat_icon: Cell::new(None),
            syncing: Cell::new(false),
            last_second: Cell::new(-1),
            last_duration: Cell::new(-1),
        });

        let mapped_state = Rc::downgrade(&state);
        overlay.connect_map(move |_| {
            if let Some(state) = mapped_state.upgrade() {
                state.sync();
            }
        });
        let unmapped_state = Rc::downgrade(&state);
        overlay.connect_unmap(move |_| {
            if let Some(state) = unmapped_state.upgrade() {
                state.suspend();
            }
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
            if let Some(state) = lyrics_state.upgrade()
                && !state.mode_syncing.get()
            {
                state.set_lyrics_mode(btn.is_active());
            }
        });

        let vis_state = Rc::downgrade(&state);
        state.visualizer_btn.connect_toggled(move |btn| {
            if let Some(state) = vis_state.upgrade()
                && !state.mode_syncing.get()
            {
                state.set_visualizer_mode(btn.is_active());
            }
        });

        let lyrics_state = Rc::downgrade(&state);
        let lyrics_observer: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(state) = lyrics_state.upgrade()
                && !state.stopped.get()
            {
                state.cancel_online_lyrics();
                state.lyrics_attempted.set(false);
                if state.current_lyrics.borrow().is_none() && state.asset_request.borrow().is_none()
                {
                    state
                        .lyrics_status
                        .replace(if state.current_track.borrow().is_some() {
                            LyricsStatus::LocalMissing
                        } else {
                            LyricsStatus::NoTrack
                        });
                    state.rebuild_lyrics_ui();
                }
                state.sync();
            }
        });
        ThemeManager::shared().observe_online_lyrics(&lyrics_observer);
        state.lyrics_observer.replace(Some(lyrics_observer));

        let style_state = Rc::downgrade(&state);
        state.opt_ribbons.connect_toggled(move |btn| {
            if btn.is_active()
                && let Some(state) = style_state.upgrade()
            {
                state.visualizer_style.set(VisualizerStyle::Ribbons);
                state.redraw_visualizer();
            }
        });

        let style_state = Rc::downgrade(&state);
        state.opt_tunnel.connect_toggled(move |btn| {
            if btn.is_active()
                && let Some(state) = style_state.upgrade()
            {
                state.visualizer_style.set(VisualizerStyle::Tunnel);
                state.redraw_visualizer();
            }
        });

        let style_state = Rc::downgrade(&state);
        state.opt_cosmic.connect_toggled(move |btn| {
            if btn.is_active()
                && let Some(state) = style_state.upgrade()
            {
                state.visualizer_style.set(VisualizerStyle::Cosmic);
                state.redraw_visualizer();
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
        for btn in [&prev_button, &state.floating_prev] {
            let ctrl = ctrl.clone();
            btn.connect_clicked(move |_| {
                ctrl.previous();
            });
        }

        let ctrl = controller.clone();
        for btn in [&next_button, &state.floating_next] {
            let ctrl = ctrl.clone();
            btn.connect_clicked(move |_| {
                ctrl.next();
            });
        }

        let ctrl = controller.clone();
        for btn in [&state.speaker_button, &state.floating_speaker] {
            let ctrl = ctrl.clone();
            btn.connect_clicked(move |_| {
                if let Some(stream) = ctrl.stream() {
                    stream.set_muted(!stream.is_muted());
                }
            });
        }

        let ctrl = controller.clone();
        let vol_state = Rc::downgrade(&state);
        for scale in [&state.volume_scale, &state.floating_volume] {
            let ctrl = ctrl.clone();
            let vol_state = vol_state.clone();
            scale.connect_value_changed(move |scale| {
                if let Some(state) = vol_state.upgrade()
                    && !state.syncing.get()
                {
                    ctrl.set_volume(scale.value());
                }
            });
        }

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
        let scrub_state = Rc::downgrade(&state);
        state.floating_scrub.connect_change_value(move |_, _, value| {
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
        for btn in [&state.shuffle_button, &state.floating_shuffle] {
            let ctrl = ctrl.clone();
            let shuff_state = shuff_state.clone();
            btn.connect_toggled(move |btn| {
                if let Some(state) = shuff_state.upgrade()
                    && !state.syncing.get()
                {
                    ctrl.set_shuffle(btn.is_active());
                }
            });
        }

        let ctrl = controller.clone();
        for btn in [&state.repeat_button, &state.floating_repeat] {
            let ctrl = ctrl.clone();
            btn.connect_clicked(move |_| {
                let next_repeat = match ctrl.repeat() {
                    crate::music::Repeat::Off => crate::music::Repeat::All,
                    crate::music::Repeat::All => crate::music::Repeat::One,
                    crate::music::Repeat::One => crate::music::Repeat::Off,
                };
                ctrl.set_repeat(next_repeat);
            });
        }

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

    pub fn request_stop(&self) {
        self.state.request_stop();
    }
}

impl Drop for ImmersiveView {
    fn drop(&mut self) {
        if Rc::strong_count(&self.state) == 1 {
            self.request_stop();
        }
    }
}

impl ImmersiveState {
    fn set_lyrics_mode(self: &Rc<Self>, lyrics: bool) {
        if self.stopped.get() || self.mode_syncing.replace(true) {
            return;
        }
        if !lyrics {
            self.cancel_online_lyrics();
        }
        self.lyrics_active.set(lyrics);
        self.lyrics_btn.set_active(lyrics);
        self.mode_syncing.set(false);
        self.sync();
    }

    fn set_visualizer_mode(self: &Rc<Self>, visualizer: bool) {
        if self.stopped.get() || self.mode_syncing.replace(true) {
            return;
        }
        self.visualizer_active.set(visualizer);
        self.visualizer_btn.set_active(visualizer);
        self.mode_syncing.set(false);
        self.sync();
    }

    #[cfg(test)]
    fn set_mode(self: &Rc<Self>, lyrics: bool, visualizer: bool) {
        if self.stopped.get() || self.mode_syncing.replace(true) {
            return;
        }
        if !lyrics {
            self.cancel_online_lyrics();
        }
        self.lyrics_active.set(lyrics);
        self.visualizer_active.set(visualizer);
        self.lyrics_btn.set_active(lyrics);
        self.visualizer_btn.set_active(visualizer);
        self.mode_syncing.set(false);
        self.sync();
    }

    fn is_mapped(&self) -> bool {
        self.overlay
            .upgrade()
            .is_some_and(|overlay| overlay.is_mapped())
    }

    fn cancel_request(&self) {
        self.asset_request.borrow_mut().take();
        if let Some(task) = self.asset_task.borrow_mut().take() {
            task.abort();
        }
    }

    fn cancel_online_lyrics(&self) {
        self.lyrics_request.borrow_mut().take();
        if let Some(task) = self.lyrics_task.borrow_mut().take() {
            task.abort();
        }
        if *self.lyrics_status.borrow() == LyricsStatus::LoadingOnline {
            self.lyrics_status.replace(LyricsStatus::LocalMissing);
            self.lyrics_attempted.set(false);
        }
    }

    fn cancel_tick(&self) {
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
        self.last_frame_us.set(None);
        self.next_frame_us.set(None);
    }

    fn suspend(&self) {
        self.cancel_request();
        self.cancel_online_lyrics();
        self.cancel_tick();
        self.track_dirty.set(true);
        self.current_lyrics.replace(None);
        self.visualizer_bands
            .set([0.0; crate::playback::SPECTRUM_BANDS]);
        self.visualizer_canvas.clear();
        self.card_visualizer.clear();
        self.art_picture
            .set_paintable(Option::<&gdk::Paintable>::None);
        self.art_stack.set_visible_child_name("fallback");
        self.artwork_palette.set(None);
        self.ambient_aura.set_artwork(None);
        self.clear_lyrics_ui();
    }

    fn request_stop(&self) {
        if !self.stopped.replace(true) {
            self.suspend();
            self._observer.borrow_mut().take();
            self.lyrics_observer.borrow_mut().take();
        }
    }

    fn should_animate(&self) -> bool {
        needs_visualizer_frames(
            self.is_mapped(),
            self.stopped.get() || self.controller.closed.get(),
            self.visualizer_active.get(),
            self.controller.status() == super::controller::PlaybackStatus::Playing,
            crate::ui::motion::animations_enabled(),
        )
    }

    fn sync_tick(self: &Rc<Self>) {
        if !self.should_animate() {
            self.cancel_tick();
            return;
        }
        if self.tick.borrow().is_some() {
            return;
        }
        let Some(overlay) = self.overlay.upgrade() else {
            return;
        };
        self.last_frame_us.set(None);
        self.next_frame_us.set(None);
        let weak = Rc::downgrade(self);
        let tick = overlay.add_tick_callback(move |_, clock| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !state.should_animate() {
                state.tick.borrow_mut().take();
                state.last_frame_us.set(None);
                state.next_frame_us.set(None);
                return glib::ControlFlow::Break;
            }
            let now = clock.frame_time();
            let Some((delta, next)) =
                visualizer_frame_delta(state.last_frame_us.get(), state.next_frame_us.get(), now)
            else {
                return glib::ControlFlow::Continue;
            };
            state.last_frame_us.set(Some(now));
            state.next_frame_us.set(Some(next));
            state.vis_t.set(state.vis_t.get() + delta);
            state.visualizer_bands.set(smooth_spectrum(
                state.visualizer_bands.get(),
                state.controller.spectrum_bands(),
                delta,
            ));
            state.redraw_visualizer();
            glib::ControlFlow::Continue
        });
        self.tick.replace(Some(tick));
    }

    fn redraw_visualizer(&self) {
        if self.is_mapped() && !self.stopped.get() && self.visualizer_active.get() {
            let style = self.visualizer_style.get();
            let time = self.vis_t.get();
            let bands = self.visualizer_bands.get();
            let palette = self.palette.get();
            if self.lyrics_active.get() {
                self.card_visualizer.set_frame(style, time, bands, palette);
            } else {
                self.visualizer_canvas.set_frame(style, time, bands, palette);
            }
        }
    }

    fn sync(self: &Rc<Self>) {
        if self.controller.closed.get() {
            self.request_stop();
        }
        if self.stopped.get() || !self.is_mapped() {
            return;
        }
        let palette = visible_artwork_palette(self.artwork_palette.get());
        if let Some(color) = ThemeManager::shared().applied_tokens()
            .and_then(|tokens| gdk::RGBA::parse(&tokens.background).ok())
        {
            let color = self.artwork_palette.get().map_or(color, |palette| {
                let background = artwork::background(palette, (f64::from(color.red()), f64::from(color.green()), f64::from(color.blue())));
                gdk::RGBA::new(background.0 as f32, background.1 as f32, background.2 as f32, 1.0)
            });
            self.ambient_aura.set_base(color);
        }
        if let Some(overlay) = self.overlay.upgrade() {
            if crate::ui::motion::animations_enabled() {
                overlay.remove_css_class("reduced-motion");
            } else {
                overlay.add_css_class("reduced-motion");
            }
        }
        if self.palette.replace(palette) != palette {
            self.ambient_aura.queue_draw();
            self.redraw_visualizer();
        }
        self.art_stack
            .set_transition_duration(if crate::ui::motion::animations_enabled() {
                200
            } else {
                0
            });
        let lyrics = self.lyrics_active.get();
        let visualizer = self.visualizer_active.get();
        let full_stage_vis = visualizer && !lyrics;
        let card_vis = visualizer && lyrics;

        self.visualizer_canvas.set_visible(full_stage_vis);
        self.floating_dock.set_visible(full_stage_vis);
        self.stage_box.set_visible(!full_stage_vis);
        self.style_selector.set_visible(visualizer);

        if card_vis {
            self.art_stack.set_visible_child_name("visualizer");
        } else if self.art_picture.paintable().is_some() {
            self.art_stack.set_visible_child_name("picture");
        } else {
            self.art_stack.set_visible_child_name("fallback");
        }

        if !visualizer {
            self.visualizer_canvas.clear();
            self.card_visualizer.clear();
        } else if full_stage_vis {
            self.card_visualizer.clear();
        } else {
            self.visualizer_canvas.clear();
        }
        let track = self.controller.current_track();
        let generation = self.controller.generation.get();
        let previous_track = self.current_track.borrow().clone();
        if self.track_dirty.replace(false)
            || previous_track.as_ref().map(|track| &track.path)
                != track.as_ref().map(|track| &track.path)
            || self.current_generation.get() != generation
        {
            self.cancel_request();
            self.cancel_online_lyrics();
            self.lyrics_attempted.set(false);
            self.lyrics_status.replace(LyricsStatus::LoadingLocal);
            self.visualizer_bands
                .set([0.0; crate::playback::SPECTRUM_BANDS]);
            self.current_generation.set(generation);
            self.current_track.replace(track.clone());
            self.artwork_palette.set(None);
            self.palette.set(current_theme_palette());
            self.ambient_aura.set_artwork(None);
            self.last_second.set(-1);
            self.last_duration.set(-1);
            self.current_lyrics.replace(None);
            self.rebuild_lyrics_ui();
            self.art_picture
                .set_paintable(Option::<&gdk::Paintable>::None);
            self.art_stack.set_visible_child_name("fallback");
            if let Some(ref track) = track {
                self.update_track_labels(track);
                self.load_track_assets(track.clone(), generation);
            } else {
                self.lyrics_status.replace(LyricsStatus::NoTrack);
                self.rebuild_lyrics_ui();
                self.title_label.set_text("Nothing playing");
                self.artist_label.set_text("Choose a track in Music");
                self.floating_title.set_text("Nothing playing");
                self.floating_artist.set_text("Choose a track in Music");
            }
        } else if previous_track != track
            && let (Some(previous), Some(track)) = (previous_track.as_ref(), track.as_ref())
        {
            self.refresh_track_metadata(previous, track);
        }
        self.update_stage_layout();
        self.maybe_lookup_lyrics();

        self.syncing.set(true);
        self.volume_scale.set_value(self.controller.volume());
        self.floating_volume.set_value(self.controller.volume());

        self.shuffle_button.set_active(self.controller.shuffle());
        self.floating_shuffle.set_active(self.controller.shuffle());

        let playing = self.controller.status() == super::controller::PlaybackStatus::Playing;
        if self.playing_icon.replace(playing) != playing {
            let icon = if playing { icons::PAUSE } else { icons::PLAY };
            self.play_button
                .set_child(Some(&assets::on_accent_icon(icon, 20)));
            self.floating_play
                .set_child(Some(&assets::on_accent_icon(icon, 16)));
        }

        let muted = self
            .controller
            .stream()
            .is_some_and(|stream| stream.is_muted());
        if self.muted_icon.replace(muted) != muted {
            let icon = if muted { icons::MUTED } else { icons::VOLUME };
            self.speaker_button
                .set_child(Some(&assets::primary_icon(icon, 16)));
            self.floating_speaker
                .set_child(Some(&assets::primary_icon(icon, 16)));
        }

        let repeat = self.controller.repeat();
        if self.repeat_icon.replace(Some(repeat)) != Some(repeat) {
            let rep_icon = if repeat == crate::music::Repeat::One {
                icons::REPEAT_ONE
            } else {
                icons::REPEAT
            };
            self.repeat_button
                .set_child(Some(&assets::primary_icon(rep_icon, 16)));
            self.floating_repeat
                .set_child(Some(&assets::primary_icon(rep_icon, 16)));
            if repeat == crate::music::Repeat::Off {
                self.repeat_button.remove_css_class("repeat-enabled");
                self.floating_repeat.remove_css_class("repeat-enabled");
            } else {
                self.repeat_button.add_css_class("repeat-enabled");
                self.floating_repeat.add_css_class("repeat-enabled");
            }
        }

        self.syncing.set(false);
        self.sync_lyrics_and_time();
        self.sync_tick();
        self.redraw_visualizer();
    }

    fn update_track_labels(&self, track: &Track) {
        self.title_label.set_text(&track.title());
        self.floating_title.set_text(&track.title());
        let subtitle = format!("{} · {}", track.artist(), track.album());
        self.artist_label.set_text(&subtitle);
        self.floating_artist.set_text(track.artist());
    }

    fn refresh_track_metadata(self: &Rc<Self>, previous: &Track, track: &Track) {
        self.current_track.replace(Some(track.clone()));
        self.update_track_labels(track);
        if !same_lyrics_track(previous, track) {
            self.cancel_online_lyrics();
            self.lyrics_attempted.set(false);
            if *self.lyrics_status.borrow() != LyricsStatus::Local {
                self.current_lyrics.replace(None);
                self.lyrics_status
                    .replace(if self.asset_request.borrow().is_some() {
                        LyricsStatus::LoadingLocal
                    } else {
                        LyricsStatus::LocalMissing
                    });
                self.rebuild_lyrics_ui();
            }
        }
    }

    fn load_track_assets(self: &Rc<Self>, track: Track, generation: u64) {
        let request = AssetRequest::new();
        let identity = request.identity.clone();
        let cancellation = request.cancellation.clone();
        self.asset_request.replace(Some(request));
        let weak = Rc::downgrade(self);
        let path = track.path.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let output = gio::spawn_blocking(move || read_track_assets(&path, &cancellation)).await;
            if let Some(state) = weak.upgrade() {
                state.finish_track_assets(&identity, &track, generation, output.ok());
            }
        });
        self.asset_task.replace(Some(task));
    }

    fn finish_track_assets(
        self: &Rc<Self>,
        identity: &Rc<()>,
        track: &Track,
        generation: u64,
        output: Option<TrackAssets>,
    ) {
        if self.stopped.get()
            || !self.is_mapped()
            || self.controller.closed.get()
            || self.controller.generation.get() != generation
            || self
                .controller
                .current_track()
                .is_none_or(|current| current.path != track.path)
            || self
                .current_track
                .borrow()
                .as_ref()
                .is_none_or(|current| current.path != track.path)
            || !self
                .asset_request
                .borrow()
                .as_ref()
                .is_some_and(|request| request.accepts(identity))
        {
            return;
        }
        self.asset_task.borrow_mut().take();
        self.asset_request.borrow_mut().take();
        if let Some(output) = output {
            self.artwork_palette.set(output.palette);
            self.palette.set(visible_artwork_palette(output.palette));
            self.ambient_aura.set_artwork(output.backdrop.and_then(|bytes| {
                gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)).ok()
            }));
            if let Some(bytes) = output.artwork
                && let Ok(texture) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
            {
                self.art_picture.set_paintable(Some(&texture));
                if !(self.visualizer_active.get() && self.lyrics_active.get()) {
                    self.art_stack.set_visible_child_name("picture");
                }
            }
            self.current_lyrics.replace(output.lyrics);
        }
        self.lyrics_status
            .replace(if self.current_lyrics.borrow().is_some() {
                LyricsStatus::Local
            } else {
                LyricsStatus::LocalMissing
            });
        self.rebuild_lyrics_ui();
        self.update_stage_layout();
        self.sync_lyrics_and_time();
        self.maybe_lookup_lyrics();
        self.redraw_visualizer();
    }

    fn maybe_lookup_lyrics(self: &Rc<Self>) {
        if self.stopped.get()
            || !self.is_mapped()
            || !self.lyrics_active.get()
            || self.current_lyrics.borrow().is_some()
            || self.lyrics_attempted.get()
            || *self.lyrics_status.borrow() != LyricsStatus::LocalMissing
            || !ThemeManager::shared().online_lyrics()
        {
            return;
        }
        let Some(track) = self.current_track.borrow().clone() else {
            return;
        };
        let Some((title, artist)) = lyrics_metadata(&track) else {
            self.lyrics_attempted.set(true);
            self.lyrics_status.replace(LyricsStatus::MissingMetadata);
            self.rebuild_lyrics_ui();
            return;
        };
        let duration = self.controller.duration();
        if duration <= 0 {
            return;
        }
        let Ok(query) = online::Query::new(title, artist, duration as f64 / 1_000_000.0) else {
            self.lyrics_attempted.set(true);
            self.lyrics_status.replace(LyricsStatus::MissingMetadata);
            self.rebuild_lyrics_ui();
            return;
        };
        let request = AssetRequest::new();
        let identity = request.identity.clone();
        let cancellation = request.cancellation.clone();
        let generation = self.current_generation.get();
        self.lyrics_request.replace(Some(request));
        self.lyrics_attempted.set(true);
        self.lyrics_status.replace(LyricsStatus::LoadingOnline);
        self.rebuild_lyrics_ui();
        let weak = Rc::downgrade(self);
        let task = glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || online::lookup(&query, &cancellation))
                .await
                .unwrap_or_else(|_| {
                    Err("The lyrics lookup could not finish. Try again.".to_owned())
                });
            if let Some(state) = weak.upgrade() {
                state.finish_online_lyrics(&identity, &track, generation, result);
            }
        });
        self.lyrics_task.replace(Some(task));
    }

    fn finish_online_lyrics(
        self: &Rc<Self>,
        identity: &Rc<()>,
        track: &Track,
        generation: u64,
        result: Result<online::Lookup, String>,
    ) {
        if self.stopped.get()
            || !self.is_mapped()
            || !self.lyrics_active.get()
            || !ThemeManager::shared().online_lyrics()
            || self.controller.closed.get()
            || self.controller.generation.get() != generation
            || self
                .controller
                .current_track()
                .is_none_or(|current| !same_lyrics_track(&current, track))
            || self
                .current_track
                .borrow()
                .as_ref()
                .is_none_or(|current| !same_lyrics_track(current, track))
            || !self
                .lyrics_request
                .borrow()
                .as_ref()
                .is_some_and(|request| request.accepts(identity))
        {
            return;
        }
        self.lyrics_task.borrow_mut().take();
        self.lyrics_request.borrow_mut().take();
        let status = match result {
            Ok(online::Lookup::Found(lyrics)) => {
                self.current_lyrics.replace(Some(lyrics));
                LyricsStatus::Online
            }
            Ok(online::Lookup::Instrumental) => LyricsStatus::Instrumental,
            Ok(online::Lookup::NotFound) => LyricsStatus::OnlineMissing,
            Err(message) => LyricsStatus::Failed(message),
        };
        self.lyrics_status.replace(status);
        self.rebuild_lyrics_ui();
        self.update_stage_layout();
        self.sync_lyrics_and_time();
    }

    fn clear_lyrics_ui(&self) {
        while let Some(child) = self.lyrics_box.first_child() {
            self.lyrics_box.remove(&child);
        }
        self.lyric_labels.borrow_mut().clear();
        self.active_lyric_index.set(None);
        self.lyrics_built.set(false);
    }

    fn rebuild_lyrics_ui(self: &Rc<Self>) {
        self.clear_lyrics_ui();
        if !self.is_mapped()
            || self.stopped.get()
            || !self.lyrics_active.get()
        {
            return;
        }
        let lyrics = self.current_lyrics.borrow();
        if let Some(ref lyrics) = *lyrics {
            let source = gtk::Label::new(Some(
                if *self.lyrics_status.borrow() == LyricsStatus::Online {
                    "Lyrics · LRCLIB"
                } else {
                    "Lyrics · Local file"
                },
            ));
            source.set_xalign(0.0);
            source.add_css_class("music-detail");
            self.lyrics_box.append(&source);
            let ctrl = self.controller.clone();
            for line in &lyrics.lines {
                let label = gtk::Label::new(Some(&line.text));
                label.add_css_class("immersive-lyric-line");
                label.set_xalign(0.0);
                label.set_wrap(true);
                label.set_selectable(!lyrics.synced);
                label.set_cursor_from_name(Some(if lyrics.synced { "pointer" } else { "text" }));

                let line_timestamp = line.timestamp.as_micros().min(i64::MAX as u128) as i64;
                let ctrl = ctrl.clone();
                let click = gtk::GestureClick::new();
                click.connect_pressed(move |_, _, _, _| {
                    ctrl.seek(line_timestamp);
                });
                if lyrics.synced {
                    label.add_controller(click);
                }

                self.lyrics_box.append(&label);
                self.lyric_labels.borrow_mut().push(label);
            }
        } else {
            let status = self.lyrics_status.borrow().clone();
            let (title, description) = match &status {
                LyricsStatus::NoTrack => ("Choose a song", "Lyrics will appear here when a song is playing.".to_owned()),
                LyricsStatus::LoadingLocal => ("Checking local lyrics…", "A matching .lrc file is used first.".to_owned()),
                LyricsStatus::LoadingOnline => ("Finding lyrics…", "Looking up this song on LRCLIB.".to_owned()),
                LyricsStatus::OnlineMissing => ("No matching lyrics found", "You can add a matching .lrc file next to this song.".to_owned()),
                LyricsStatus::Instrumental => ("Instrumental track", "LRCLIB lists this recording as instrumental.".to_owned()),
                LyricsStatus::MissingMetadata => ("Song details needed", "Online matching needs title, artist and duration. Filenames are never sent instead.".to_owned()),
                LyricsStatus::Failed(message) => ("Lyrics lookup unavailable", message.clone()),
                _ => ("No local lyrics", if ThemeManager::shared().online_lyrics() {
                    "Waiting for the song's duration before looking online.".to_owned()
                } else {
                    "Enable online lyrics to find missing words. LRCLIB receives only this song's title, artist and duration; your files stay unchanged.".to_owned()
                }),
            };
            let heading = gtk::Label::new(Some(title));
            heading.add_css_class("immersive-title");
            heading.set_xalign(0.0);
            heading.set_wrap(true);
            let detail = gtk::Label::new(Some(&description));
            detail.add_css_class("music-detail");
            detail.set_xalign(0.0);
            detail.set_wrap(true);
            self.lyrics_box.append(&heading);
            self.lyrics_box.append(&detail);
            let online_enabled = ThemeManager::shared().online_lyrics();
            if (!online_enabled && status == LyricsStatus::LocalMissing)
                || matches!(status, LyricsStatus::Failed(_))
            {
                let action = gtk::Button::with_label(if online_enabled {
                    "Try again"
                } else {
                    "Enable online lyrics"
                });
                action.set_halign(gtk::Align::Start);
                let weak = Rc::downgrade(self);
                action.connect_clicked(move |_| {
                    if let Some(state) = weak.upgrade() {
                        state.lyrics_attempted.set(false);
                        state.lyrics_status.replace(LyricsStatus::LocalMissing);
                        let manager = ThemeManager::shared();
                        if !manager.online_lyrics() {
                            manager.set_online_lyrics(true);
                        } else {
                            state.maybe_lookup_lyrics();
                        }
                    }
                });
                self.lyrics_box.append(&action);
            }
        }
        self.lyrics_built.set(true);
    }

    fn sync_lyrics_and_time(&self) {
        let position = self.controller.position().max(0);
        let duration = self.controller.duration().max(0);

        let fraction = if duration > 0 {
            (position as f64 / duration as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };

        self.syncing.set(true);
        self.scrub_scale.set_value(fraction);
        self.floating_scrub.set_value(fraction);
        self.syncing.set(false);

        let current_sec = position / 1_000_000;
        let second_changed = self.last_second.replace(current_sec) != current_sec;
        let duration_changed = self.last_duration.replace(duration) != duration;
        if second_changed || duration_changed {
            self.elapsed_label.set_text(&time_label(position));
            let rem = duration.saturating_sub(position);
            self.remaining_label
                .set_text(&format!("-{}", time_label(rem.max(0))));
            self.floating_time.set_text(&format!(
                "{} / {}",
                time_label(position),
                time_label(duration)
            ));
        }

        if !self.lyrics_scroll.is_mapped() {
            return;
        }
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
                        adj.set_value(
                            target.clamp(adj.lower(), (adj.upper() - adj.page_size()).max(0.0)),
                        );
                    }
                }
                self.active_lyric_index.set(active_idx);
            }
        }
    }

    fn update_stage_layout(self: &Rc<Self>) {
        let show_lyrics = self.lyrics_active.get();
        if show_lyrics && !self.lyrics_built.get() {
            self.rebuild_lyrics_ui();
        }

        self.lyrics_scroll.set_visible(show_lyrics);
        if show_lyrics {
            self.stage_box.remove_css_class("immersive-stage-solo");
            self.art_box.set_size_request(480, 480);
            self.art_picture.set_size_request(480, 480);
            self.card_visualizer.set_size_request(480, 480);
        } else {
            self.stage_box.add_css_class("immersive-stage-solo");
            self.art_box.set_size_request(580, 580);
            self.art_picture.set_size_request(580, 580);
            self.card_visualizer.set_size_request(580, 580);
        }
    }
}

impl Drop for ImmersiveState {
    fn drop(&mut self) {
        self.cancel_request();
        self.cancel_online_lyrics();
        self.cancel_tick();
    }
}

fn install_control_focus(widget: &gtk::Box) {
    let focus = gtk::EventControllerFocus::new();
    let weak = widget.downgrade();
    focus.connect_enter(move |_| {
        if let Some(widget) = weak.upgrade() {
            widget.add_css_class("controls-focused");
        }
    });
    let weak = widget.downgrade();
    focus.connect_leave(move |_| {
        if let Some(widget) = weak.upgrade() {
            widget.remove_css_class("controls-focused");
        }
    });
    widget.add_controller(focus);
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

fn visible_artwork_palette(palette: Option<artwork::Palette>) -> artwork::Palette {
    let Some(palette) = palette else {
        return current_theme_palette();
    };
    let Some(tokens) = ThemeManager::shared().applied_tokens() else {
        return current_theme_palette();
    };
    let color = |value: &str| gdk::RGBA::parse(value).ok().map(|color| {
        (f64::from(color.red()), f64::from(color.green()), f64::from(color.blue()))
    });
    match (color(&tokens.background), color(&tokens.text)) {
        (Some(background), Some(foreground)) => artwork::legible_palette(palette, artwork::background(palette, background), foreground),
        _ => current_theme_palette(),
    }
}

#[cfg(test)]
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

#[cfg(test)]
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
                let step = 20.0;
                cr.new_path();
                let y0 = base_y + amp * (x * freq + phase).sin();
                cr.move_to(x, y0);
                while x <= width {
                    x += step;
                    let y = base_y
                        + amp * (x * freq + phase).sin()
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

            for k in 0..8 {
                let p = (t * 0.75 + k as f64 / 8.0) % 1.0;
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
            let num_particles = 32;

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
