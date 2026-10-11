use super::format_time;

#[test]
#[ignore = "Requires a desktop display and FFmpeg; run explicitly"]
fn desktop_animation_loops_and_stops_cleanly() {
    use gtk::{glib, prelude::*};
    use std::time::{Duration, Instant};

    gtk::init().expect("Desktop display available");
    crate::assets::prepare().expect("Icons load");
    let directory =
        std::env::temp_dir().join(format!("hermes-animation-smoke-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let gif = directory.join("input.gif");
    let output = directory.join("result.webm");
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=10",
            "-t",
            "0.6",
        ])
        .arg(&gif)
        .status()
        .expect("FFmpeg available");
    assert!(status.success());
    crate::sandbox_helper::run(&[
        "preview-animation".into(),
        gif.to_string_lossy().into_owned(),
        output.to_string_lossy().into_owned(),
        "0".into(),
    ])
    .expect("Animation conversion succeeds");
    let bytes = glib::Bytes::from_owned(std::fs::read(&output).unwrap());
    let input = gtk::gio::MemoryInputStream::from_bytes(&bytes);
    let stream = gtk::MediaFile::for_input_stream(&input);
    let preview = super::MediaPreview::new_animation(&stream);
    let window = gtk::Window::builder()
        .default_width(400)
        .default_height(300)
        .child(&preview.widget)
        .build();
    window.present();
    let mut previous = 0;
    let mut wraps = 0;
    let mut maximum_timestamp = 0;
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        while glib::MainContext::default().iteration(false) {}
        let timestamp = stream.timestamp();
        maximum_timestamp = maximum_timestamp.max(timestamp);
        if previous > 200_000 && timestamp < previous - 100_000 {
            wraps += 1;
        }
        previous = timestamp;
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(stream.error().is_none(), "{:?}", stream.error());
    assert!(stream.is_prepared());
    assert!(stream.has_video());
    assert!(!stream.has_audio());
    assert!(stream.is_muted());
    assert!(
        !stream.is_loop(),
        "Animation uses explicit end-of-cycle restart"
    );
    let restart_deadline = Instant::now() + Duration::from_millis(300);
    while !stream.is_playing() && stream.error().is_none() && Instant::now() < restart_deadline {
        for _ in 0..128 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        stream.is_playing(),
        "GIF should autoplay or finish its queued loop restart; ended={}, seeking={}, wraps={wraps}",
        stream.is_ended(),
        stream.is_seeking()
    );
    assert!(
        wraps >= 2,
        "Animation must actually loop: {wraps} wraps, max {maximum_timestamp}, duration {}",
        stream.duration()
    );
    let controls = preview
        .widget
        .first_child()
        .unwrap()
        .next_sibling()
        .unwrap();
    let row = controls.last_child().unwrap();
    let play = row
        .first_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    let mute = play.next_sibling().unwrap().next_sibling().unwrap();
    let volume = mute.next_sibling().unwrap();
    assert!(!mute.is_visible());
    assert!(!volume.is_visible());
    play.emit_clicked();
    assert!(!stream.is_playing());
    window.set_visible(false);
    window.present();
    while glib::MainContext::default().iteration(false) {}
    assert!(
        !stream.is_playing(),
        "Remapping must preserve a paused animation"
    );
    play.emit_clicked();
    assert!(stream.is_playing());
    assert!(preview.open_fullscreen());
    preview.stop();
    assert!(preview.fullscreen.borrow().is_none());
    assert!(!stream.is_playing());
    window.close();
    while glib::MainContext::default().iteration(false) {}
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn formats_playback_times() {
    assert_eq!(format_time(-1), "0:00");
    assert_eq!(format_time(65_900_000), "1:05");
    assert_eq!(format_time(3_661_000_000), "1:01:01");
}

#[test]
fn fullscreen_key_does_not_override_modified_shortcuts() {
    use gtk::gdk::{Key, ModifierType};
    assert!(super::fullscreen_shortcut(Key::f, ModifierType::empty()));
    assert!(super::fullscreen_shortcut(Key::F, ModifierType::SHIFT_MASK));
    assert!(!super::fullscreen_shortcut(
        Key::f,
        ModifierType::CONTROL_MASK
    ));
    assert!(!super::fullscreen_shortcut(Key::f, ModifierType::ALT_MASK));
    assert!(!super::fullscreen_shortcut(
        Key::f,
        ModifierType::SUPER_MASK
    ));
    assert!(!super::fullscreen_shortcut(Key::a, ModifierType::empty()));
}

#[test]
#[ignore = "Requires a display and FFmpeg; run explicitly as a desktop playback smoke test"]
fn desktop_playback_smoke() {
    use gtk::prelude::*;
    use std::time::{Duration, Instant};

    gtk::init().expect("Desktop display must be available");
    crate::assets::prepare().expect("Bundled icons must load");
    let path =
        std::env::temp_dir().join(format!("hermes-player-smoke-{}.webm", std::process::id()));
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "2",
            "-c:v",
            "libvpx",
            "-c:a",
            "libvorbis",
        ])
        .arg(&path)
        .status()
        .expect("FFmpeg must be installed");
    assert!(status.success());
    let bytes = gtk::glib::Bytes::from_owned(
        std::fs::read(&path).expect("Generated clip must be readable"),
    );
    let input = gtk::gio::MemoryInputStream::from_bytes(&bytes);
    let stream = gtk::MediaFile::for_input_stream(&input);
    eprintln!("Playback backend: {}", stream.type_().name());
    let preview = super::MediaPreview::new(&stream, false);
    let window = gtk::Window::builder()
        .default_width(520)
        .default_height(360)
        .child(&preview.widget)
        .build();
    window.present();
    let pump = |duration: Duration| {
        let until = Instant::now() + duration;
        while Instant::now() < until {
            while gtk::glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    pump(Duration::from_secs(2));
    assert!(stream.error().is_none(), "{:?}", stream.error());
    assert!(stream.is_prepared());
    assert!(stream.has_audio(), "Generated clip audio must be detected");
    assert!(!stream.is_muted(), "Playback must start unmuted");
    assert!(stream.volume() > 0.0, "Playback volume must be audible");
    assert!(stream.duration() > 0);
    let controls = preview
        .widget
        .first_child()
        .and_then(|child| child.next_sibling())
        .expect("Controls exist");
    let row = controls.last_child().expect("Button row exists");
    let play = row
        .first_child()
        .and_then(|child| child.downcast::<gtk::Button>().ok())
        .expect("Play action exists");
    let seek = controls
        .first_child()
        .and_then(|child| child.downcast::<gtk::Scale>().ok())
        .expect("Seek slider exists");
    play.emit_clicked();
    let playback_deadline = Instant::now() + Duration::from_secs(5);
    while stream.timestamp() == 0 && stream.error().is_none() && Instant::now() < playback_deadline
    {
        pump(Duration::from_millis(50));
    }
    assert!(stream.timestamp() > 0);
    play.emit_clicked();
    assert!(!stream.is_playing());
    assert!(stream.is_seekable());
    seek.emit_by_name::<bool>("change-value", &[&gtk::ScrollType::Jump, &1.0_f64]);
    pump(Duration::from_millis(300));
    assert!(stream.timestamp() >= 900_000);
    let mute = play
        .next_sibling()
        .and_then(|child| child.next_sibling())
        .and_then(|child| child.downcast::<gtk::Button>().ok())
        .expect("Mute action exists");
    mute.emit_clicked();
    assert!(stream.is_muted());
    let controls = preview
        .widget
        .first_child()
        .and_then(|child| child.next_sibling())
        .expect("Controls exist");
    let row = controls.last_child().expect("Button row exists");
    let expand = row
        .last_child()
        .and_then(|child| child.downcast::<gtk::Button>().ok())
        .expect("Fullscreen action exists");
    assert_eq!(
        expand.tooltip_text().as_deref(),
        Some("Fullscreen video (F)")
    );
    assert!(preview.open_fullscreen());
    pump(Duration::from_millis(300));
    assert!(preview.fullscreen.borrow().is_some());
    let fullscreen = preview
        .fullscreen
        .borrow()
        .as_ref()
        .expect("Fullscreen exists")
        .clone();
    let controllers = fullscreen.observe_controllers();
    for position in 0..controllers.n_items() {
        if let Some(keys) = controllers
            .item(position)
            .and_then(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        {
            keys.emit_by_name::<bool>(
                "key-pressed",
                &[&gtk::gdk::Key::f, &0_u32, &gtk::gdk::ModifierType::empty()],
            );
            break;
        }
    }
    pump(Duration::from_millis(100));
    assert!(preview.fullscreen.borrow().is_none(), "F exits fullscreen");
    assert!(preview.open_fullscreen());
    pump(Duration::from_millis(100));
    preview.stop();
    assert!(preview.fullscreen.borrow().is_none());
    assert!(!stream.is_playing());
    window.close();
    pump(Duration::from_millis(100));
    std::fs::remove_file(path).expect("Temporary test clip must be removed");
}
