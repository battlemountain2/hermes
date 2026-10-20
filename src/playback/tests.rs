use super::*;

#[test]
fn repeated_pause_resume_retains_negotiated_audio_without_resetting_output() {
    gst::init().unwrap();
    let pipeline = gst::parse::launch(
        "appsrc name=pcm is-live=true format=time do-timestamp=true caps=audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved ! fakesink name=capture sync=false async=false signal-handoffs=true",
    ).unwrap().downcast::<gst::Pipeline>().unwrap();
    let source = pipeline.by_name("pcm").unwrap().downcast::<app::AppSrc>().unwrap();
    let sink = pipeline.by_name("capture").unwrap();
    let (sender, receiver) = mpsc::channel();
    sink.connect("handoff", false, move |values| {
        let buffer = values[1].get::<gst::Buffer>().unwrap();
        sender.send(buffer.map_readable().unwrap().as_slice().to_vec()).unwrap();
        None
    });
    let bus = pipeline.bus().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pipeline.set_state(output_state(&Command::Play)).unwrap();
        for cycle in 0..12 {
            let before_pause = (cycle as f32 / 32.0).to_le_bytes().repeat(480 * 2);
            source.push_buffer(pcm_buffer(before_pause.clone())).unwrap();
            assert_eq!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(), before_pause);
            pipeline.set_state(output_state(&Command::Pause)).unwrap();
            let (_, state, _) = pipeline.state(gst::ClockTime::from_seconds(2));
            assert_eq!(state, gst::State::Paused);
            assert!(source.static_pad("src").unwrap().current_caps().is_some());
            assert!(receiver.recv_timeout(Duration::from_millis(5)).is_err());
            pipeline.set_state(output_state(&Command::Play)).unwrap();
            let resumed = ((cycle + 16) as f32 / 32.0).to_le_bytes().repeat(480 * 2);
            source.push_buffer(pcm_buffer(resumed.clone())).unwrap();
            assert_eq!(receiver.recv_timeout(Duration::from_secs(2)).unwrap(), resumed);
        }
        for message in bus.iter() {
            if let gst::MessageView::StateChanged(change) = message.view() {
                assert_ne!((change.old(), change.current()), (gst::State::Paused, gst::State::Ready));
            }
        }
        assert_eq!(output_state(&Command::Seek { epoch: 1, timestamp: 0 }), gst::State::Ready);
        assert_eq!(output_state(&Command::Stop), gst::State::Null);
    }));
    pipeline.set_state(gst::State::Null).unwrap();
    result.unwrap();
}

#[test]
fn pcm_packets_have_sample_accurate_durations_including_split_tails() {
    gst::init().unwrap();
    for frames in [1, 480, 8192, 37] {
        let buffer = pcm_buffer(0.5_f32.to_le_bytes().repeat(frames * 2));
        assert_eq!(buffer.size(), frames * 8);
        assert_eq!(
            buffer.duration(),
            Some(gst::ClockTime::from_nseconds(
                frames as u64 * 1_000_000_000 / 48_000
            ))
        );
        assert_eq!(buffer.pts(), None);
        assert!(!buffer.flags().contains(gst::BufferFlags::GAP));
    }
}

#[test]
#[ignore = "Explicit short video fixture and built sandbox worker; paused/muted output"]
fn isolated_short_video_plays_to_end_and_replays_with_audio() {
    let path = std::env::var_os("HERMES_STREAM_TEST_FILE").expect("Explicit short video path");
    let session = Session::start(Path::new(&path)).unwrap();
    session.set_volume(true, 0.0);
    let start = Instant::now();
    let mut duration = 0;
    let mut frame_seen = false;
    while duration == 0 || !frame_seen {
        let updates = session.take_updates();
        assert!(updates.error.is_none(), "{:?}", updates.error);
        if let Some((length, audio, video, seekable)) = updates.prepared {
            assert!(audio && video && seekable);
            assert!(length > 0 && length < 15_000_000, "Use a short fixture");
            duration = length;
        }
        frame_seen |= updates.frame.is_some();
        assert!(start.elapsed() < Duration::from_secs(12));
        std::thread::sleep(Duration::from_millis(5));
    }
    for replay in [false, true] {
        if replay {
            assert!(session.seek(0));
            let start = Instant::now();
            loop {
                let updates = session.take_updates();
                assert!(updates.error.is_none(), "{:?}", updates.error);
                if let Some(success) = updates.seeked {
                    assert!(success);
                    break;
                }
                assert!(start.elapsed() < Duration::from_secs(12));
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(session.play());
        let start = Instant::now();
        let mut advanced = false;
        loop {
            let updates = session.take_updates();
            assert!(updates.error.is_none(), "{:?}", updates.error);
            advanced |= updates
                .position
                .is_some_and(|position| position > duration / 2);
            if updates.ended {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_micros(duration as u64) + Duration::from_secs(8)
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(advanced);
    }
    session.stop();
}

#[test]
#[ignore = "Private GStreamer registry/plugins; generated 4K color bars only"]
fn generated_early_video_scaling_preserves_preview_colors() {
    let root = std::path::PathBuf::from(std::env::var_os("HERMES_PREVIEW_PROBE").unwrap());
    assert!(
        std::path::PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap()).starts_with(root)
    );
    gst::init().unwrap();
    let sample = |transforms: &str| {
        let description = format!(
            "videotestsrc num-buffers=1 pattern=smpte ! video/x-raw,format=I420,width=3840,height=2160,framerate=60/1,colorimetry=bt709 ! {transforms} ! video/x-raw,format=RGBA,width=1280,height=720,framerate=30/1,pixel-aspect-ratio=1/1 ! appsink name=frames"
        );
        let pipeline = gst::parse::launch(&description)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let sink = pipeline
            .by_name("frames")
            .unwrap()
            .downcast::<app::AppSink>()
            .unwrap();
        pipeline.set_state(gst::State::Paused).unwrap();
        let sample = sink
            .try_pull_preroll(gst::ClockTime::from_seconds(5))
            .unwrap();
        let info = gstreamer_video::VideoInfo::from_caps(sample.caps().unwrap()).unwrap();
        assert_eq!((info.width(), info.height()), (1280, 720));
        let bytes = sample
            .buffer()
            .unwrap()
            .map_readable()
            .unwrap()
            .as_slice()
            .to_vec();
        pipeline.set_state(gst::State::Null).unwrap();
        bytes
    };
    let before = sample(
        "videoflip video-direction=auto ! videoconvert ! videoscale ! videorate drop-only=true max-rate=30",
    );
    let after = sample(
        "videorate drop-only=true max-rate=30 ! videoflip video-direction=auto ! videoscale ! videoconvert",
    );
    assert_eq!(before.len(), after.len());
    let difference = before
        .iter()
        .zip(&after)
        .map(|(before, after)| u64::from(before.abs_diff(*after)))
        .sum::<u64>() as f64
        / before.len() as f64;
    eprintln!("4K scaling representative RGBA mean difference: {difference}");
    assert!(difference < 4.0);
    assert!(after.chunks_exact(4).all(|pixel| pixel[3] == 255));
}

#[test]
#[ignore = "Explicit private generated heavy-video corpus, codecs and built helper"]
fn generated_heavy_video_preroll_rotation_vfr_and_seek_remain_bounded() {
    let root = std::path::PathBuf::from(std::env::var_os("HERMES_PREVIEW_PROBE").unwrap());
    assert!(root.starts_with("/tmp"));
    assert!(
        std::path::PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap()).starts_with(&root)
    );
    if let Some(warmup) = warmup() {
        warmup.join().unwrap().unwrap();
    }
    for name in ["h264-4k60.mov", "h264-portrait.mov", "hevc.mov", "vfr.mp4"] {
        let started = Instant::now();
        let session = Session::start(&root.join("fixtures").join(name)).unwrap();
        session.set_volume(true, 0.0);
        let mut duration = 0;
        let mut frame = None;
        while duration == 0 || frame.is_none() {
            let updates = session.take_updates();
            assert!(updates.error.is_none(), "{name}: {:?}", updates.error);
            if let Some((length, _, video, seekable)) = updates.prepared {
                assert!(video && seekable);
                duration = length;
            }
            if let Some((_, width, height, pixels)) = updates.frame {
                assert!(width > 0 && width <= 1280 && height > 0 && height <= 720);
                assert_eq!(pixels.len(), width as usize * height as usize * 4);
                assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
                if name == "h264-portrait.mov" {
                    assert!(height > width);
                } else {
                    assert!(width > height);
                }
                frame = Some((width, height));
            }
            assert!(started.elapsed() < Duration::from_secs(12));
            std::thread::sleep(Duration::from_millis(2));
        }
        eprintln!(
            "Heavy video {name} first real frame {:?} in {:?}",
            frame.unwrap(),
            started.elapsed()
        );
        let target = duration / 2;
        assert!(session.seek(target));
        let started = Instant::now();
        let mut seeked = false;
        while !seeked {
            let updates = session.take_updates();
            assert!(updates.error.is_none(), "{name}: {:?}", updates.error);
            if let Some(success) = updates.seeked {
                assert!(success);
                seeked = true;
            }
            assert!(started.elapsed() < Duration::from_secs(12));
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(session.play());
        let started = Instant::now();
        let mut ended = false;
        while !ended {
            let updates = session.take_updates();
            assert!(updates.error.is_none(), "{name}: {:?}", updates.error);
            ended = updates.ended;
            assert!(started.elapsed() < Duration::from_secs(12));
            std::thread::sleep(Duration::from_millis(2));
        }
        session.stop();
    }
}

#[test]
#[ignore = "Requires an explicit private generated preview audit and a built helper"]
fn generated_idle_warmup_leaves_playback_paused_and_reuses_the_registry() {
    let root = std::path::PathBuf::from(std::env::var_os("HERMES_PREVIEW_PROBE").unwrap());
    assert!(root.starts_with("/tmp"));
    assert!(
        root.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hermes-preview-audit.")
    );
    assert!(
        std::path::PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap()).starts_with(&root)
    );
    let start = Instant::now();
    warmup().unwrap().join().unwrap().unwrap();
    let warm_time = start.elapsed();
    assert!(warmup().is_none());
    let start = Instant::now();
    let session = Session::start(&root.join("fixtures/hevc.mov")).unwrap();
    session.set_volume(true, 0.0);
    let mut prepared = false;
    let mut frame = false;
    while !prepared || !frame {
        let updates = session.take_updates();
        assert!(updates.error.is_none(), "{:?}", updates.error);
        prepared |= updates.prepared.is_some();
        frame |= updates.frame.is_some();
        assert!(start.elapsed() < Duration::from_secs(12));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!session.playing.load(Ordering::Acquire));
    eprintln!(
        "Idle registry warmup={warm_time:?}, first HEVC frame after warmup={:?}",
        start.elapsed()
    );
    session.stop();
}

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
