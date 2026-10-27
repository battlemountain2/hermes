use super::*;
use crate::model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue};
use std::{ffi::OsString, rc::Rc};

fn fixture_entry(name: &str) -> FileEntry {
    FileEntry(Rc::new(FileEntryInner {
        music_tags: None,
        location: Location::local(format!("/fixture/{name}")),
        native_name: OsString::from(name),
        display_name: name.to_string(),
        kind: EntryKind::File,
        size: MetadataValue::Known(1024),
        modified_unix_seconds: MetadataValue::Known(0),
    }))
}

#[test]
fn test_kind_mappings() {
    assert_eq!(kind(&fixture_entry("song.mp3")), "Song");
    assert_eq!(kind(&fixture_entry("song.flac")), "Song");
    assert_eq!(kind(&fixture_entry("video.mp4")), "Video");
    assert_eq!(kind(&fixture_entry("video.mkv")), "Video");
    assert_eq!(kind(&fixture_entry("photo.png")), "Photo");
    assert_eq!(kind(&fixture_entry("photo.jpg")), "Photo");
    assert_eq!(kind(&fixture_entry("doc.pdf")), "PDF");
    assert_eq!(kind(&fixture_entry("sheet.xlsx")), "Spreadsheet");
    assert_eq!(kind(&fixture_entry("doc.docx")), "Document");
    assert_eq!(kind(&fixture_entry("text.txt")), "Text");
    assert_eq!(kind(&fixture_entry("archive.zip")), "Archive");
}

#[test]
fn test_operation_family() {
    assert_eq!(
        operation(FormatFamily::Audio),
        Some(ParseOperation::MediaMetadata)
    );
    assert_eq!(
        operation(FormatFamily::Video),
        Some(ParseOperation::MediaMetadata)
    );
    assert_eq!(
        operation(FormatFamily::Image),
        Some(ParseOperation::ImageMetadata)
    );
    assert_eq!(operation(FormatFamily::PlainText), None);
}

#[test]
fn test_detail_text_formatting() {
    let details = Details {
        duration_seconds: Some(125.0),
        width: Some(1920),
        height: Some(1080),
        audio_codec: Some("flac".into()),
        video_codec: Some("h264".into()),
        ..Default::default()
    };
    let formatted = detail_text(&details, FormatFamily::Video);
    assert!(formatted.contains("2:05"));
    assert!(formatted.contains("1920 × 1080 px"));
    assert!(formatted.contains("H264"));
}

#[test]
fn expensive_document_decoders_are_never_started_for_hover() {
    for name in [
        "deck.pptx",
        "sheet.xlsx",
        "document.docx",
        "document.pdf",
        "model.stl",
        "archive.zip",
    ] {
        let entry = fixture_entry(name);
        assert!(!cold_thumbnail(&entry), "{name}");
        assert_eq!(
            operation(classify_by_name(&entry.native_name)),
            None,
            "{name}"
        );
    }
    for name in ["photo.png", "song.flac", "clip.mp4"] {
        assert!(cold_thumbnail(&fixture_entry(name)), "{name}");
    }
}

#[test]
fn durations_are_bounded_and_audio_does_not_report_cover_dimensions() {
    let mut details = Details {
        duration_seconds: Some(3661.0),
        width: Some(600),
        height: Some(600),
        ..Details::default()
    };
    assert_eq!(detail_text(&details, FormatFamily::Audio), "1:01:01");
    for duration in [f64::NAN, f64::INFINITY, -1.0, 315_360_000.0] {
        details.duration_seconds = Some(duration);
        assert_eq!(detail_text(&details, FormatFamily::Audio), "");
    }
}

#[test]
#[ignore = "Requires private GTK/XDG roots and a built sandbox helper; generated media only, never plays audio"]
fn private_hover_cards_load_only_when_visible_cancel_and_keep_their_size() {
    use std::{fs, path::PathBuf, time::Instant};
    let root = PathBuf::from(std::env::var_os("HERMES_PICKER_TEST_ROOT").unwrap());
    assert_eq!(glib::user_cache_dir(), root.join("cache"));
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let theme = super::super::theme::ThemeManager::shared();
    theme.set_hover_previews(true);
    let styles = gtk::CssProvider::new();
    let errors = Rc::new(RefCell::new(Vec::new()));
    let parsing_errors = errors.clone();
    styles.connect_parsing_error(move |_, _, error| {
        parsing_errors.borrow_mut().push(error.to_string())
    });
    styles.load_from_string(include_str!("../../style.css"));
    assert!(
        errors.borrow().is_empty(),
        "GTK stylesheet errors: {:?}",
        errors.borrow()
    );
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    fs::create_dir_all(root.join("hover-fixtures")).unwrap();
    let path = root.join("hover-fixtures/photo.png");
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 320, 240).unwrap();
    pixels.fill(0x376f8aff);
    fs::write(&path, pixels.save_to_bufferv("png", &[]).unwrap()).unwrap();
    let audio_path = root.join("hover-fixtures/silence.wav");
    let mut wave = Vec::new();
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&16_036u32.to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16u32.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&8_000u32.to_le_bytes());
    wave.extend_from_slice(&16_000u32.to_le_bytes());
    wave.extend_from_slice(&2u16.to_le_bytes());
    wave.extend_from_slice(&16u16.to_le_bytes());
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&16_000u32.to_le_bytes());
    wave.resize(16_044, 0);
    fs::write(&audio_path, wave).unwrap();
    let audio = crate::sandbox::parse(
        &audio_path,
        ParseOperation::MediaMetadata,
        0,
        &Cancellation::default(),
    )
    .unwrap();
    let audio: Details = serde_json::from_slice(&audio.data).unwrap();
    assert_eq!(audio.duration_seconds, Some(1.0));
    assert_eq!(audio.audio_codec.as_deref(), Some("pcm_s16le"));
    let mut photo = fixture_entry("photo.png");
    Rc::make_mut(&mut photo.0).location = Location::local(&path);
    let card = Card::new();
    card.bind(photo.clone());
    assert!(!card.widget.is_mapped());
    assert!(card.task.borrow().is_none());
    let window = gtk::Window::new();
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_child(Some(&card.widget));
    window.present();
    allocate(&window);
    let wait = |condition: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(12);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "Timed out: {} / {}",
                card.filename.text(),
                card.details.text()
            );
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(3));
        }
    };
    wait(&|| card.widget.is_mapped() && card.widget.width() > 0 && card.widget.height() > 100);
    let initial = (card.widget.width(), card.widget.height());
    let title_left = card.title.compute_bounds(&card.widget).unwrap().x();
    wait(&|| card.details.text().contains("320 × 240") && card.image.paintable().is_some());
    settle();
    assert_eq!(initial, (card.widget.width(), card.widget.height()));
    assert_eq!(
        title_left,
        card.title.compute_bounds(&card.widget).unwrap().x()
    );
    snapshot(&window, &root.join("hover-photo.png"));
    assert!(card.current(card.generation.get()));
    theme.set_preview_images(false);
    assert!(!card.current(card.generation.get()));
    theme.set_preview_images(true);
    let generation = card.generation.get();
    card.cancel();
    assert!(!card.current(generation));
    assert!(card.task.borrow().is_none());
    card.bind(photo);
    card.start();
    let pending = card.generation.get();
    card.bind(fixture_entry("new-document.docx"));
    assert!(!card.current(pending));
    assert_eq!(card.filename.text(), "new-document.docx");
    assert!(!card.details.text().contains("320 × 240"));
    card.cancel();
    let mut song = fixture_entry("song.flac");
    Rc::make_mut(&mut song.0).music_tags = Some(crate::model::MusicTags {
        title: Some("Song with a long but readable title".into()),
        album_artist: Some("Album artist fallback".into()),
        album: Some("An album".into()),
        ..Default::default()
    });
    card.bind(song);
    assert_eq!(card.artist.text(), "Album artist fallback");
    settle();
    for theme_id in ["azure-glow", "paper"] {
        theme.select_theme(theme_id);
        settle();
        snapshot(&window, &root.join(format!("hover-{theme_id}.png")));
    }
    let normal_height = card.title.measure(gtk::Orientation::Vertical, -1).1;
    theme.set_interface_font_size(22);
    wait(&|| {
        card.title
            .pango_context()
            .font_description()
            .unwrap()
            .size()
            == 22 * gtk::pango::SCALE
    });
    settle();
    allocate(&window);
    assert!(card.title.measure(gtk::Orientation::Vertical, -1).1 > normal_height);
    let scaled = (card.widget.width(), card.widget.height());
    card.apply_details(
        &Details {
            title: Some("Short".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            duration_seconds: Some(125.0),
            audio_codec: Some("flac".into()),
            ..Default::default()
        },
        FormatFamily::Audio,
    );
    settle();
    assert_eq!(scaled, (card.widget.width(), card.widget.height()));
    snapshot(&window, &root.join("hover-large-font.png"));
    theme.set_hover_previews(false);
    assert!(!card.current(card.generation.get()));
    let saved: toml::Value =
        toml::from_str(&fs::read_to_string(crate::paths::settings_file()).unwrap())
            .unwrap();
    assert_eq!(saved["hover_previews"].as_bool(), Some(false));
    theme.set_hover_previews(true);
    window.set_visible(false);
    assert!(card.task.borrow().is_none());
    assert!(card.cancellation.borrow().is_cancelled());
    window.close();
}

fn settle() {
    let deadline = std::time::Instant::now() + Duration::from_millis(150);
    while std::time::Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(3));
    }
}

fn snapshot(window: &gtk::Window, path: &std::path::Path) {
    allocate(window);
    let snapshot = gtk::Snapshot::new();
    let mut child = window.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        invalidate(&widget);
        window.snapshot_child(&widget, &snapshot);
    }
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer.realize(None::<&gtk::gdk::Surface>).unwrap();
    renderer
        .render_texture(&snapshot.to_node().unwrap(), None)
        .save_to_png(path)
        .unwrap();
    renderer.unrealize();
}

fn allocate(window: &gtk::Window) {
    invalidate(window.upcast_ref());
    let width = window.measure(gtk::Orientation::Horizontal, -1).1.max(300);
    let height = window.measure(gtk::Orientation::Vertical, width).1.max(216);
    window.allocate(width, height, -1, None);
}

fn invalidate(widget: &gtk::Widget) {
    let _color = widget.color();
    widget.queue_draw();
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        invalidate(&widget);
    }
}
