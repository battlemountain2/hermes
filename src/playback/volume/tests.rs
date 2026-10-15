use super::*;

#[test]
fn volume_requests_are_bounded_and_keep_the_latest_pair() {
    let (control, receiver) = Control::new();
    for index in 0..100_000 {
        control.set(index % 2 == 0, f64::from(index % 100) / 100.0);
    }
    control.set(false, 0.37);
    assert_eq!(receiver.try_recv(), Ok(()));
    assert_eq!(receiver.try_recv(), Err(mpsc::TryRecvError::Empty));
    let requested = control.latest.load(Ordering::Acquire);
    assert_eq!(requested, encode(false, 0.37));
}

#[test]
fn blocked_audio_writer_does_not_block_volume_motion_or_lose_unmute() {
    let (control, receiver) = Control::new();
    let latest = control.latest.clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = stopped.clone();
    let (applied, received) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        consume(receiver, &latest, &worker_stopped, |muted, volume| {
            applied.send((muted, volume)).unwrap();
            resumed.recv().unwrap();
        });
    });
    control.set(true, 0.9);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (true, 0.9)
    );
    for index in 0..100_000 {
        control.set(true, f64::from(index % 100) / 100.0);
    }
    control.set(false, 0.23);
    resume.send(()).unwrap();
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (false, 0.23)
    );
    stopped.store(true, Ordering::Release);
    control.set(true, 0.0);
    resume.send(()).unwrap();
    worker.join().unwrap();
    assert!(received.try_recv().is_err());
}

#[test]
fn volume_safety_clamps_nonfinite_out_of_range_and_negative_zero() {
    for muted in [false, true] {
        for (input, expected) in [
            (f64::NAN, 0.0),
            (f64::INFINITY, 0.0),
            (f64::NEG_INFINITY, 0.0),
            (-3.0, 0.0),
            (-0.0, 0.0),
            (9.0, 1.0),
            (0.25, 0.25),
        ] {
            let (control, receiver) = Control::new();
            control.set(muted, input);
            let latest = control.latest.clone();
            drop(control);
            let mut applied = Vec::new();
            consume(
                receiver,
                &latest,
                &AtomicBool::new(false),
                |mute, volume| {
                    applied.push((mute, volume));
                },
            );
            assert_eq!(applied, [(muted, expected)]);
            assert!(!applied[0].1.is_sign_negative());
        }
    }
}

#[test]
fn stopped_volume_worker_skips_pending_audio_changes() {
    let (control, receiver) = Control::new();
    control.set(false, 1.0);
    consume(receiver, &control.latest, &AtomicBool::new(true), |_, _| {
        panic!("Stopped streams must not apply pending volume")
    });
}

#[test]
fn native_audio_unmutes_only_after_applying_the_requested_gain() {
    gst::init().unwrap();
    let element = gst::ElementFactory::make("volume").build().unwrap();
    element.set_property("mute", true);
    element.set_property("volume", 0.0_f64);
    let (sent, received) = mpsc::channel();
    element.connect_notify(Some("mute"), move |element, _| {
        sent.send((
            element.property::<bool>("mute"),
            element.property::<f64>("volume"),
        ))
        .unwrap();
    });
    let stopped = Arc::new(AtomicBool::new(false));
    let control = Control::start(element.clone(), stopped.clone());
    let expect_audio = |expected_mute: bool, expected_volume: f64| {
        let (muted, volume) = received.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(muted, expected_mute);
        assert!((volume - expected_volume).abs() < 0.000001);
    };
    control.set(false, 0.12);
    expect_audio(false, 0.12);
    control.set(true, 0.9);
    expect_audio(true, 0.12);
    control.set(false, 0.04);
    expect_audio(false, 0.04);
    stopped.store(true, Ordering::Release);
    control.wake();
}

#[test]
fn native_zero_gain_and_mute_keep_continuous_pcm_and_restore_exact_gain() {
    gst::init().unwrap();
    let pipeline = gst::parse::launch(
        "appsrc name=pcm is-live=true format=time caps=audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved ! volume name=level ! appsink name=capture sync=false async=false",
    )
    .unwrap()
    .downcast::<gst::Pipeline>()
    .unwrap();
    let source = pipeline
        .by_name("pcm")
        .unwrap()
        .downcast::<app::AppSrc>()
        .unwrap();
    let capture = pipeline
        .by_name("capture")
        .unwrap()
        .downcast::<app::AppSink>()
        .unwrap();
    let volume = pipeline.by_name("level").unwrap();
    preserve_pcm_silence(&volume).unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    for packet in 0..240_u64 {
        let (muted, gain) = match packet % 120 {
            0..20 => (false, 0.5),
            20..60 => (false, 0.0),
            60..80 => (false, 0.25),
            80..100 => (true, 0.75),
            _ => (false, 1.0),
        };
        volume.set_property("mute", muted);
        volume.set_property("volume", gain);
        let data = 0.5_f32.to_le_bytes().repeat(960);
        let mut buffer = pcm_buffer(data);
        let buffer_ref = buffer.get_mut().unwrap();
        let timestamp = gst::ClockTime::from_mseconds(packet * 10);
        buffer_ref.set_pts(timestamp);
        buffer_ref.set_offset(packet * 480);
        buffer_ref.set_flags(gst::BufferFlags::DISCONT);
        source.push_buffer(buffer).unwrap();
        let sample = capture
            .try_pull_sample(gst::ClockTime::from_seconds(2))
            .expect("Continuous audio packet, including while silent");
        let output = sample.buffer().unwrap();
        assert!(!output.flags().contains(gst::BufferFlags::GAP));
        assert!(output.flags().contains(gst::BufferFlags::DISCONT));
        assert_eq!(output.pts(), Some(timestamp));
        assert_eq!(output.offset(), packet * 480);
        assert_eq!(output.duration(), Some(gst::ClockTime::from_mseconds(10)));
        let expected = if muted { 0.0 } else { (gain * 0.5) as f32 };
        let mapped = output.map_readable().unwrap();
        assert_eq!(mapped.size(), 3840);
        for value in mapped.as_slice().chunks_exact(4) {
            assert_eq!(f32::from_le_bytes(value.try_into().unwrap()), expected);
        }
    }
    pipeline.set_state(gst::State::Null).unwrap();
}

#[test]
fn silent_pcm_probe_keeps_empty_gap_markers_and_shared_buffer_metadata() {
    gst::init().unwrap();
    let volume = gst::ElementFactory::make("volume").build().unwrap();
    preserve_pcm_silence(&volume).unwrap();
    let output_pad = volume.static_pad("src").unwrap();
    let sink_pad = gst::Pad::builder(gst::PadDirection::Sink)
        .chain_function(|_, _, buffer| {
            if buffer.size() == 0 {
                assert!(buffer.flags().contains(gst::BufferFlags::GAP));
            } else {
                assert!(!buffer.flags().contains(gst::BufferFlags::GAP));
            }
            assert_eq!(buffer.pts(), Some(gst::ClockTime::from_seconds(3)));
            assert!(buffer.flags().contains(gst::BufferFlags::DISCONT));
            Ok(gst::FlowSuccess::Ok)
        })
        .build();
    output_pad.set_active(true).unwrap();
    sink_pad.set_active(true).unwrap();
    output_pad.link(&sink_pad).unwrap();
    assert!(output_pad.push_event(gst::event::StreamStart::new("private-pcm-test")));
    let caps = "audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved"
        .parse::<gst::Caps>()
        .unwrap();
    assert!(output_pad.push_event(gst::event::Caps::new(&caps)));
    assert!(
        output_pad.push_event(gst::event::Segment::new(&gst::FormattedSegment::<
            gst::ClockTime,
        >::new()))
    );
    for size in [0, 3840] {
        let mut buffer = gst::Buffer::from_mut_slice(vec![0_u8; size]);
        buffer
            .get_mut()
            .unwrap()
            .set_pts(gst::ClockTime::from_seconds(3));
        buffer
            .get_mut()
            .unwrap()
            .set_flags(gst::BufferFlags::GAP | gst::BufferFlags::DISCONT);
        let retained = buffer.clone();
        output_pad.push(buffer).unwrap();
        assert!(retained.flags().contains(gst::BufferFlags::GAP));
    }
    output_pad.unlink(&sink_pad).unwrap();
    output_pad.set_active(false).unwrap();
    sink_pad.set_active(false).unwrap();
}
