use super::*;

#[test]
fn metadata_rows_show_required_fields_and_unknowns_honestly() {
    let rows = metadata_rows(&AudioMetadata::default());
    for name in [
        "Title",
        "Artist",
        "Album",
        "Album artist",
        "Genre",
        "Track",
        "Disc",
        "Year / date",
        "Duration",
        "Sample rate",
        "Bitrate",
        "Codec",
        "Container",
        "Channels",
    ] {
        assert!(rows.contains(&(name.into(), "—".into())), "Missing {name}");
    }
}

#[test]
fn metadata_rows_include_useful_embedded_tags_and_unabridged_track_numbers() {
    let metadata = AudioMetadata {
        tags: crate::model::MusicTags {
            title: Some("<Plain text>".into()),
            track: Some(3),
            ..Default::default()
        },
        embedded_tags: [
            ("track", "3/17"),
            ("genre", "Ambient"),
            ("date", "2025"),
            ("comment", "Embedded notes"),
            ("composer", "Composer"),
        ]
        .into_iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect(),
        properties: crate::sandbox_helper::audio::AudioProperties {
            duration_seconds: Some(3661.2),
            sample_rate: Some(96000),
            bitrate: Some(1500000),
            codec: Some("flac".into()),
            bit_depth: Some(24),
            ..Default::default()
        },
    };
    let rows = metadata_rows(&metadata);
    for (name, value) in [
        ("Title", "<Plain text>"),
        ("Track", "3/17"),
        ("Genre", "Ambient"),
        ("Year / date", "2025"),
        ("Duration", "1:01:01"),
        ("Sample rate", "96000 Hz"),
        ("Bitrate", "1500.0 kb/s"),
        ("Codec", "flac"),
        ("Bit depth", "24-bit"),
        ("comment", "Embedded notes"),
    ] {
        assert!(
            rows.contains(&(name.into(), value.into())),
            "Missing {name}: {value}"
        );
    }
    assert!(
        !rows
            .iter()
            .any(|(name, _)| name == "track" || name == "date")
    );
}

#[test]
#[ignore = "Requires a private desktop display, FFmpeg and a built target/debug/strata worker"]
fn desktop_audio_preview_playback_metadata_artwork_and_cancellation() {
    use crate::ui::music::controller::PlaybackStatus;
    use crate::ui::preview::PreviewDrawer;
    assert_eq!(
        std::env::var("HERMES_AUDIO_FIXTURE_ISOLATED").as_deref(),
        Ok("1"),
        "Run in a private HOME/XDG, D-Bus and Broadway session"
    );
    use std::time::{Duration, Instant};
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    crate::assets::register_icon_theme();
    let _theme = crate::ui::theme::ThemeManager::shared();
    let styles = gtk::CssProvider::new();
    styles.load_from_string(include_str!("../../../style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let directory =
        std::env::temp_dir().join(format!("hermes-audio-preview-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let track = directory.join("track.flac");
    let cover = directory.join("cover.png");
    assert!(
        std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=64x64",
                "-frames:v",
                "1"
            ])
            .arg(&cover)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=stereo",
                "-t",
                "16",
                "-c:a",
                "flac",
                "-metadata",
                "title=Fixture track",
                "-metadata",
                "artist=Fixture artist",
                "-metadata",
                "album=Fixture album",
                "-metadata",
                "album_artist=Fixture album artist",
                "-metadata",
                "genre=Ambient",
                "-metadata",
                "track=3/17",
                "-metadata",
                "disc=2/2",
                "-metadata",
                "date=2025",
                "-metadata",
                "comment=Embedded notes"
            ])
            .arg(&track)
            .status()
            .unwrap()
            .success()
    );
    let entry = crate::model::FileEntry(Rc::new(crate::model::FileEntryInner {
        location: crate::model::Location::local(&track),
        native_name: "track.flac".into(),
        display_name: "track.flac".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        music_tags: None,
    }));
    let drawer = PreviewDrawer::new(Rc::new(crate::adapters::LocalPreviewProvider));
    let application = gtk::Application::builder()
        .application_id("io.github.hermes.AudioPreviewFixture")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.register(None::<&gio::Cancellable>).unwrap();
    let music_player = crate::ui::music::MusicPlayer::for_application(&application);
    let music = music_player.controller();
    music.set_volume(0.0);
    let play_count = Rc::new(Cell::new(0));
    let count = play_count.clone();
    let window = gtk::ApplicationWindow::builder()
        .application(&application)
        .default_width(500)
        .default_height(800)
        .child(&drawer.widget())
        .build();
    crate::ui::window::install_audio_playback_coordination(&window, &drawer, music.clone());
    let production_handler = drawer.state.audio_play_handler.borrow().clone().unwrap();
    drawer.set_audio_play_handler(Rc::new(move || {
        count.set(count.get() + 1);
        production_handler();
    }));
    window.present();
    let pump_until = |condition: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !condition() && Instant::now() < deadline {
            for _ in 0..128 {
                if !glib::MainContext::default().iteration(false) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(condition(), "Audio preview condition timed out");
    };
    music_player.play_folder(track.clone());
    pump_until(&|| music.duration() >= 15_000_000 && music.position() > 100_000);
    let music_id = music.track_id();
    let music_track = music.current_track();
    drawer.show(entry.clone());
    pump_until(&|| drawer.state.audio.borrow().is_some());
    let stream = drawer.state.audio.borrow().as_ref().unwrap().stream.clone();
    pump_until(&|| stream.is_prepared() || stream.error().is_some());
    assert!(stream.error().is_none(), "{:?}", stream.error());
    stream.set_muted(true);
    assert!(!stream.is_playing());
    assert_eq!(play_count.get(), 0);
    assert_eq!(music.status(), PlaybackStatus::Playing);
    assert!(stream.duration() >= 15_000_000);
    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut found = vec![widget.clone()];
        let mut child = widget.first_child();
        while let Some(current) = child {
            found.extend(descendants(&current));
            child = current.next_sibling();
        }
        found
    }
    let widgets = descendants(&drawer.widget());
    let extras = widgets
        .iter()
        .find_map(|widget| widget.downcast_ref::<gtk::Expander>())
        .unwrap();
    assert!(!extras.is_expanded());
    extras.set_expanded(true);
    pump_until(&|| extras.child().is_some_and(|child| child.is_mapped()));
    let widgets = descendants(&drawer.widget());
    let labels = widgets
        .iter()
        .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
        .map(|label| label.text().to_string())
        .collect::<Vec<_>>();
    for value in [
        "Fixture track",
        "Fixture artist",
        "Fixture album",
        "Ambient",
        "3/17",
        "2/2",
        "2025",
        "Embedded notes",
        "flac",
        "48000 Hz",
    ] {
        assert!(
            labels.iter().any(|label| label == value),
            "Missing {value}: {labels:?}"
        );
    }
    let picture = widgets
        .iter()
        .find_map(|widget| widget.downcast_ref::<gtk::Picture>())
        .unwrap()
        .clone();
    pump_until(&|| picture.paintable().is_some());
    extras.set_expanded(false);
    if let Some(directory) = std::env::var_os("HERMES_TEST_SCREENSHOT_DIR") {
        use gtk::gsk::prelude::*;
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let widget = drawer.widget();
        pump_until(&|| widget.is_mapped() && widget.width() > 0 && widget.height() > 0);
        let paintable = gtk::WidgetPaintable::new(Some(&widget));
        widget.queue_draw();
        let deadline = Instant::now() + Duration::from_secs(2);
        let node = loop {
            for _ in 0..128 {
                if !glib::MainContext::default().iteration(false) {
                    break;
                }
            }
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(
                &snapshot,
                f64::from(widget.width()),
                f64::from(widget.height()),
            );
            if let Some(node) = snapshot.to_node() {
                break Some(node);
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        if let Some(node) = node {
            let renderer = gtk::gsk::CairoRenderer::new();
            renderer.realize(None::<&gtk::gdk::Surface>).unwrap();
            let texture = renderer.render_texture(
                &node,
                Some(&gtk::graphene::Rect::new(
                    0.0,
                    0.0,
                    widget.width() as f32,
                    widget.height() as f32,
                )),
            );
            texture
                .save_to_png(directory.join("audio-preview.png"))
                .unwrap();
            renderer.unrealize();
        } else {
            eprintln!(
                "Screenshot skipped: no rendered frame; connect a client to the private Broadway display"
            );
        }
    }
    let play = widgets
        .iter()
        .find_map(|widget| {
            widget
                .downcast_ref::<gtk::Button>()
                .filter(|button| button.tooltip_text().as_deref() == Some("Play"))
        })
        .unwrap()
        .clone();
    play.emit_clicked();
    pump_until(&|| stream.timestamp() > 100_000);
    assert_eq!(play_count.get(), 1);
    assert_eq!(music.status(), PlaybackStatus::Paused);
    assert_eq!(music.track_id(), music_id);
    assert_eq!(music.current_track(), music_track);
    drawer.show(entry.clone());
    assert_eq!(drawer.state.audio.borrow().as_ref().unwrap().stream, stream);
    drawer.pause_audio_preview();
    assert!(!stream.is_playing());
    assert_eq!(play_count.get(), 1);
    music.play();
    pump_until(&|| music.status() == PlaybackStatus::Playing);
    assert!(!stream.is_playing());
    assert_eq!(play_count.get(), 1);
    stream.seek(9_000_000);
    pump_until(&|| !stream.is_seeking() && stream.timestamp() >= 8_900_000);
    play.emit_clicked();
    pump_until(&|| stream.timestamp() > 9_100_000);
    assert_eq!(play_count.get(), 2);
    assert_eq!(music.status(), PlaybackStatus::Paused);
    drawer.close();
    assert!(!stream.is_playing());
    assert!(drawer.state.audio.borrow().is_none());
    assert_eq!(play_count.get(), 2);
    assert_eq!(
        music.status(),
        PlaybackStatus::Paused,
        "Closing must not resume persistent music"
    );
    assert_eq!(music.track_id(), music_id);
    assert_eq!(music.current_track(), music_track);
    drawer.show(entry.clone());
    drawer.close();
    music.shutdown();
    for _ in 0..128 {
        if !glib::MainContext::default().iteration(false) {
            break;
        }
    }
    assert!(drawer.state.content.first_child().is_none());
    drawer.show(entry);
    pump_until(&|| drawer.state.audio.borrow().is_some());
    assert!(
        !drawer
            .state
            .audio
            .borrow()
            .as_ref()
            .unwrap()
            .stream
            .is_playing()
    );
    drawer.close();
    window.close();
    std::fs::remove_dir_all(directory).unwrap();
}
