use super::*;

#[test]
#[ignore = "Requires isolated GTK display, audio backend and FFmpeg; generated silent audio only"]
fn desktop_main_listening_seek_skip_queue_continuation_pause_and_preview_exclusion() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let root = std::env::temp_dir().join(format!("hermes-listen-native-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut tracks = Vec::new();
    for name in ["one", "two"] {
        let path = root.join(format!("{name}.flac"));
        assert!(
            std::process::Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "anullsrc=r=48000:cl=stereo",
                    "-t",
                    "3",
                    "-y"
                ])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        tracks.push(Track {
            path,
            tags: Default::default(),
        });
    }
    let controller = Controller::with_session(None);
    let store = controller.collection_store.borrow().clone();
    let pump = |predicate: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(12);
        while !predicate() {
            assert!(
                Instant::now() < deadline,
                "native listening timed out: {}",
                controller.notice.borrow()
            );
            for _ in 0..32 {
                if !glib::MainContext::default().iteration(false) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    controller.replace(tracks.clone(), 0);
    pump(&|| {
        controller
            .stream
            .borrow()
            .as_ref()
            .is_some_and(|stream| stream.is_prepared() && stream.is_playing())
    });
    controller.seek(2_800_000);
    pump(&|| controller.queue.borrow().current == Some(1));
    assert!(
        !store
            .snapshot()
            .data
            .play_counts()
            .contains_key(&tracks[0].path)
    );
    pump(&|| store.snapshot().data.play_counts().get(&tracks[1].path) == Some(&1));
    controller.pause();
    let counts = store.snapshot().data.play_counts();
    let wait = Instant::now() + Duration::from_millis(400);
    while Instant::now() < wait {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(store.snapshot().data.play_counts(), counts);
    controller.replace(vec![tracks[0].clone()], 0);
    pump(&|| store.snapshot().data.play_counts().get(&tracks[0].path) == Some(&1));
    controller.pause();
    let preview = PlaybackStream::new_audio(&tracks[1].path);
    preview.set_volume(0.0);
    pump(&|| preview.is_prepared());
    preview.play();
    let wait = Instant::now() + Duration::from_secs(2);
    while Instant::now() < wait {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
    preview.stop();
    assert_eq!(
        store.snapshot().data.play_counts().get(&tracks[1].path),
        Some(&1)
    );
    controller.shutdown();
    std::fs::remove_dir_all(root).unwrap();
}
