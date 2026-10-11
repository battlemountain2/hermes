use super::*;

#[test]
#[ignore = "Requires a GTK display; synthetic texture, no personal media"]
fn desktop_poster_fallback_yields_to_frames_cancellation_and_errors() {
    gtk::init().unwrap();
    let stream: PlaybackStream = glib::Object::new();
    assert!(stream.needs_poster());
    let texture = gtk::gdk::MemoryTexture::new(
        1,
        1,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_static(&[0, 0, 0, 255]),
        4,
    );
    stream.imp().texture.replace(Some(texture));
    assert!(!stream.needs_poster());
    stream.imp().texture.borrow_mut().take();
    stream.imp().poster_cancellation.cancel();
    assert!(!stream.needs_poster());
    let failed: PlaybackStream = glib::Object::new();
    failed.set_error(glib::Error::new(
        gtk::gio::IOErrorEnum::Failed,
        "Synthetic decoder failure",
    ));
    assert!(!failed.needs_poster());
    let stopped: PlaybackStream = glib::Object::new();
    stopped.stop();
    assert!(!stopped.needs_poster());
}

#[test]
#[ignore = "Requires private GTK display, FFmpeg and a built worker; generated MOV only"]
fn desktop_mov_starts_with_a_real_frame_and_seeks_without_normalization() {
    gtk::init().unwrap();
    let path = std::env::temp_dir().join(format!("hermes-mov-startup-{}.mov", std::process::id()));
    assert!(
        std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=24",
                "-t",
                "3",
                "-an",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-g",
                "72"
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    for _ in 0..3 {
        let start = std::time::Instant::now();
        let stream = PlaybackStream::new(&path);
        let deadline = start + Duration::from_secs(12);
        while (!stream.is_prepared() || stream.imp().texture.borrow().is_none())
            && stream.error().is_none()
            && std::time::Instant::now() < deadline
        {
            for _ in 0..64 {
                if !glib::MainContext::default().iteration(false) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(stream.error().is_none(), "{:?}", stream.error());
        assert!(stream.is_prepared() && stream.imp().texture.borrow().is_some());
        eprintln!(
            "Generated H264 720p MOV first decoded frame: {:?}",
            start.elapsed()
        );
        assert!(!stream.needs_poster());
        assert!(stream.duration() >= 2_900_000);
        assert!(!stream.is_playing());
        stream.seek(1_500_000);
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        while stream.is_seeking()
            && stream.error().is_none()
            && std::time::Instant::now() < deadline
        {
            for _ in 0..64 {
                if !glib::MainContext::default().iteration(false) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(stream.error().is_none() && !stream.is_seeking());
        assert!(stream.timestamp() >= 1_490_000);
        stream.stop();
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
#[ignore = "Requires desktop, FFmpeg and a built target/debug/strata sandbox worker"]
fn desktop_full_length_seek_fullscreen_and_cleanup() {
    use std::time::Instant;
    gtk::init().expect("Display available");
    crate::assets::prepare().expect("Icons load");
    let path = std::env::temp_dir().join(format!("hermes-stream-{}.mp4", std::process::id()));
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=15",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440",
            "-t",
            "45",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
        ])
        .arg(&path)
        .status()
        .expect("FFmpeg installed");
    assert!(status.success());
    let stream = PlaybackStream::new(&path);
    stream.set_muted(true);
    let preview = super::super::media::MediaPreview::new_full(&stream);
    let window = gtk::Window::builder()
        .default_width(500)
        .default_height(320)
        .child(&preview.widget)
        .build();
    window.present();
    let pump_until = |condition: &dyn Fn() -> bool, seconds: u64| {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while !condition() && stream.error().is_none() && Instant::now() < deadline {
            for _ in 0..128 {
                if condition() || stream.error().is_some() || Instant::now() >= deadline {
                    break;
                }
                if !glib::MainContext::default().iteration(false) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(stream.error().is_none(), "{:?}", stream.error());
        assert!(
            condition(),
            "Playback condition timed out at {}",
            stream.timestamp()
        );
    };
    let startup = Instant::now();
    pump_until(
        &|| stream.is_prepared() && stream.imp().texture.borrow().is_some(),
        12,
    );
    eprintln!("Streaming startup: {:?}", startup.elapsed());
    assert!(stream.duration() >= 44_000_000);
    assert!(stream.has_audio() && stream.has_video() && stream.is_seekable());
    assert!(!stream.is_playing());
    stream.seek(29_000_000);
    eprintln!("Seeking to 29 seconds");
    pump_until(&|| !stream.is_seeking(), 12);
    assert!(stream.timestamp() >= 28_900_000);
    stream.play();
    pump_until(&|| stream.timestamp() > 31_000_000, 8);
    assert!(preview.open_fullscreen());
    let before = stream.timestamp();
    pump_until(&|| stream.timestamp() > before + 300_000, 3);
    stream.pause();
    assert!(!stream.is_playing());
    stream.seek(43_500_000);
    eprintln!("Seeking near end");
    pump_until(&|| !stream.is_seeking(), 12);
    stream.play();
    pump_until(&|| stream.is_ended(), 5);
    eprintln!(
        "Reached end after {:?}; position={}",
        startup.elapsed(),
        stream.timestamp()
    );
    stream.seek(0);
    eprintln!("Replaying after end");
    pump_until(&|| !stream.is_seeking(), 12);
    eprintln!(
        "Replay seek finished after {:?}; position={}; playing={}; ended={}",
        startup.elapsed(),
        stream.timestamp(),
        stream.is_playing(),
        stream.is_ended()
    );
    stream.play();
    pump_until(
        &|| stream.timestamp() > 200_000 && stream.timestamp() < 2_000_000,
        3,
    );
    assert!(!stream.is_ended());
    stream.seek(20_000_000);
    stream.seek(5_000_000);
    stream.seek(10_000_000);
    pump_until(
        &|| {
            !stream.is_seeking()
                && stream.timestamp() > 10_100_000
                && stream.timestamp() < 12_000_000
        },
        12,
    );
    assert!(stream.is_playing());
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
    let status = preview
        .widget
        .last_child()
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    for _ in 0..2 {
        stream.pause();
        stream.seek(43_500_000);
        assert_eq!(status.text(), "Seeking…");
        pump_until(&|| !stream.is_seeking(), 12);
        stream.play();
        pump_until(&|| stream.is_ended(), 5);
        assert_eq!(
            play.tooltip_text().as_deref(),
            Some("Replay from beginning")
        );
        assert!(status.text().contains("Video finished"));
        play.emit_clicked();
        pump_until(
            &|| {
                stream.timestamp() > 200_000
                    && stream.timestamp() < 2_000_000
                    && !stream.is_seeking()
            },
            12,
        );
        assert!(!stream.is_ended());
        assert!(stream.is_playing());
        assert_eq!(play.tooltip_text().as_deref(), Some("Pause"));
        assert_eq!(status.text(), "Full-length playback");
    }
    preview.stop();
    assert!(stream.imp().session.borrow().is_none());
    window.close();
    let abandoned = PlaybackStream::new(&path);
    abandoned.stop();
    let deadline = Instant::now() + Duration::from_millis(700);
    while Instant::now() < deadline {
        for _ in 0..128 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(abandoned.imp().session.borrow().is_none());
    assert!(!abandoned.is_prepared());
    assert!(abandoned.error().is_none());
    std::fs::remove_file(path).unwrap();
}
