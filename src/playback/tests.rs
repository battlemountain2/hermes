use super::*;

#[test]
#[ignore = "Requires HERMES_AUDIO_TEST_FILE and a built sandbox worker"]
fn isolated_audio_survives_startup_thread_exit() {
    let path = std::env::var_os("HERMES_AUDIO_TEST_FILE").expect("Audio fixture path");
    let session = std::thread::spawn(move || {
        let session = Session::start(Path::new(&path)).expect("Session starts");
        session.set_volume(true, 0.0);
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let updates = session.take_updates();
            assert!(updates.error.is_none(), "{:?}", updates.error);
            if updates.prepared.is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "Decoder did not prepare");
            std::thread::sleep(Duration::from_millis(10));
        }
        session
    })
    .join()
    .unwrap();
    assert!(session.play());
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut advanced = false;
    while !advanced && Instant::now() < deadline {
        let updates = session.take_updates();
        assert!(updates.error.is_none(), "{:?}", updates.error);
        advanced = updates.position.is_some_and(|position| position > 500_000);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(advanced);
    session.stop();
}

#[test]
#[ignore = "Requires HERMES_AUDIO_TEST_FILE and a built target/debug/strata sandbox worker"]
fn isolated_audio_start_middle_seek_and_end() {
    let path = std::env::var_os("HERMES_AUDIO_TEST_FILE").expect("Audio fixture path");
    let session = Session::start(Path::new(&path)).expect("Session starts");
    session.set_volume(true, 0.0);
    let mut duration = 0;
    let deadline = Instant::now() + Duration::from_secs(12);
    while duration == 0 && Instant::now() < deadline {
        let updates = session.take_updates();
        assert!(updates.error.is_none(), "{:?}", updates.error);
        if let Some((length, audio, _, seekable)) = updates.prepared {
            assert!(audio && seekable);
            duration = length;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(duration > 4_000_000);
    assert!(session.play());
    for target in [0, duration / 2] {
        if target > 0 {
            assert!(session.seek(target));
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut advanced = false;
        while !advanced && Instant::now() < deadline {
            let updates = session.take_updates();
            assert!(updates.error.is_none(), "{:?}", updates.error);
            advanced = updates.position.is_some_and(|position| {
                position > target + if target == 0 { 20_000_000 } else { 500_000 }
            });
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(advanced, "Audio failed to advance after {target}");
    }
    assert!(session.seek(duration - 1_500_000));
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut ended = false;
    while !ended && Instant::now() < deadline {
        let updates = session.take_updates();
        assert!(updates.error.is_none(), "{:?}", updates.error);
        ended = updates.ended;
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ended, "Audio failed to finish");
    session.stop();
}

#[test]
fn raw_audio_is_finite_and_clipped() {
    let mut data: Vec<u8> = [f32::NAN, f32::INFINITY, -3.0, 0.4, 2.0]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    sanitize_pcm(&mut data);
    let values: Vec<f32> = data
        .chunks_exact(4)
        .map(|sample| f32::from_le_bytes(sample.try_into().unwrap()))
        .collect();
    assert_eq!(values, [0.0, 0.0, -1.0, 0.4, 1.0]);
}

#[test]
fn unexpected_worker_exit_reports_status_without_hiding_decoder_reason() {
    for reason in [None, Some("Invalid audio stream".to_owned())] {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "exit 7"])
            .spawn()
            .unwrap();
        let updates = Mutex::new(Updates {
            error: reason.clone(),
            ..Updates::default()
        });
        monitor(
            &mut child,
            &AtomicBool::new(false),
            &AtomicBool::new(false),
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            &AtomicU64::new(0),
            Instant::now(),
            &updates,
        );
        let error = updates.lock().unwrap().error.clone().unwrap();
        if let Some(reason) = reason {
            assert_eq!(error, reason);
        } else {
            assert!(error.contains("exit status: 7"), "{error}");
        }
    }
}

#[test]
fn streaming_sandbox_keeps_original_private_without_duration_cpu_cutoff() {
    let command = sandbox_command(Path::new("/fixture.mp4")).unwrap();
    let arguments: Vec<_> = command
        .get_args()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    assert!(arguments.iter().any(|value| value == "--unshare-all"));
    assert!(arguments.iter().any(|value| value == "--as=1342177280"));
    assert!(arguments.iter().any(|value| value == "/input"));
    assert!(arguments.iter().any(|value| value == "--playback-worker"));
    assert!(!arguments.iter().any(|value| value.starts_with("--cpu=")));
    assert!(!arguments.iter().any(|value| value == "/home"));
}

#[test]
fn stalled_seek_is_killed_even_with_other_heartbeats() {
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    let started = Instant::now() - Duration::from_secs(13);
    let updates = Mutex::new(Updates::default());
    monitor(
        &mut child,
        &AtomicBool::new(false),
        &AtomicBool::new(false),
        &AtomicBool::new(true),
        &AtomicU64::new(0),
        &AtomicU64::new(13_000),
        started,
        &updates,
    );
    assert!(updates.lock().unwrap().error.is_some());
    assert!(child.try_wait().unwrap().is_some());
}

#[test]
fn cancellation_reaps_the_decoder() {
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    let updates = Mutex::new(Updates::default());
    monitor(
        &mut child,
        &AtomicBool::new(true),
        &AtomicBool::new(false),
        &AtomicBool::new(false),
        &AtomicU64::new(0),
        &AtomicU64::new(0),
        Instant::now(),
        &updates,
    );
    assert!(updates.lock().unwrap().error.is_none());
    assert!(child.try_wait().unwrap().is_some());
}

#[test]
fn paused_session_does_not_use_the_active_playback_deadline() {
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    let stopped = Arc::new(AtomicBool::new(false));
    let cancel = stopped.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        cancel.store(true, Ordering::Release);
    });
    let updates = Mutex::new(Updates::default());
    monitor(
        &mut child,
        &stopped,
        &AtomicBool::new(false),
        &AtomicBool::new(false),
        &AtomicU64::new(0),
        &AtomicU64::new(0),
        Instant::now() - Duration::from_secs(30),
        &updates,
    );
    thread.join().unwrap();
    assert!(updates.lock().unwrap().error.is_none());
    assert!(child.try_wait().unwrap().is_some());
}

#[test]
#[ignore = "Requires codecs and a built target/debug/strata sandbox worker"]
fn malformed_media_fails_closed_with_decoder_reason() {
    let path = std::env::temp_dir().join(format!("hermes-bad-media-{}.mp4", std::process::id()));
    std::fs::write(&path, b"not a media container").unwrap();
    let session = Session::start(&path).expect("Session process starts");
    let started = Instant::now();
    let mut reason = None;
    while started.elapsed() < Duration::from_secs(12) && reason.is_none() {
        let updates = session.take_updates();
        assert!(updates.prepared.is_none());
        reason = updates.error;
        std::thread::sleep(Duration::from_millis(10));
    }
    session.stop();
    std::fs::remove_file(path).unwrap();
    let reason = reason.expect("Bad input returns a bounded decoder error");
    assert!(reason.len() <= 2048);
    assert!(!reason.contains("disconnected"), "{reason}");
}

#[test]
#[ignore = "Requires HERMES_STREAM_TEST_FILE, codecs and a built target/debug/strata"]
fn isolated_local_file_start_seek_and_stop() {
    let path = std::env::var_os("HERMES_STREAM_TEST_FILE").expect("Test video path provided");
    let started = Instant::now();
    let session = Session::start(Path::new(&path)).expect("Session starts");
    let mut duration = 0;
    let mut frame_seen = false;
    while started.elapsed() < Duration::from_secs(12) && (duration == 0 || !frame_seen) {
        let updates = session.take_updates();
        assert!(updates.error.is_none(), "{:?}", updates.error);
        if let Some((length, audio, video, seekable)) = updates.prepared {
            assert!(video && seekable);
            eprintln!(
                "Prepared in {:?}; duration={length}; audio={audio}",
                started.elapsed()
            );
            duration = length;
        }
        if let Some((_, width, height, _)) = updates.frame {
            eprintln!("Frame {width}x{height} at {:?}", started.elapsed());
            frame_seen = true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(duration > 0 && frame_seen);
    let target = duration * 4 / 5;
    assert!(session.seek(target));
    let started = Instant::now();
    let mut seeked = false;
    while started.elapsed() < Duration::from_secs(12) && !seeked {
        let updates = session.take_updates();
        assert!(updates.error.is_none(), "{:?}", updates.error);
        if let Some(success) = updates.seeked {
            assert!(success);
            seeked = true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(seeked);
    eprintln!("Seek completed in {:?}", started.elapsed());
    assert!(session.play());
    let started = Instant::now();
    let mut advanced = false;
    while started.elapsed() < Duration::from_secs(4) && !advanced {
        let updates = session.take_updates();
        assert!(updates.error.is_none(), "{:?}", updates.error);
        advanced = updates
            .position
            .is_some_and(|position| position > target + 100_000);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(advanced);
    session.stop();
}
