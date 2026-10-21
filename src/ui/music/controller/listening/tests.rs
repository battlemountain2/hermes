use super::*;

#[test]
fn actual_playing_time_counts_once_per_instance_and_not_by_position() {
    let mut listening = Listening::default();
    let path = PathBuf::from("/generated/track.flac");
    let start = Instant::now();
    listening.reset(Some(path.clone()));
    assert!(
        listening
            .sample(start, 60_000_000, 240_000_000, true)
            .is_none()
    );
    for tick in 1..300 {
        assert!(
            listening
                .sample(
                    start + Duration::from_millis(tick * 100),
                    60_000_000 + tick as i64 * 100_000,
                    240_000_000,
                    true
                )
                .is_none()
        );
    }
    let record = listening
        .sample(
            start + Duration::from_secs(30),
            90_000_000,
            240_000_000,
            true,
        )
        .unwrap();
    assert_eq!(record.0, path);
    assert_eq!(record.1, 30_000_000);
    listening.counted = true;
    assert!(
        listening
            .sample(
                start + Duration::from_millis(30_100),
                90_100_000,
                240_000_000,
                true
            )
            .is_none()
    );
    listening.reset(Some(path));
    assert!(!listening.counted);
    assert_eq!(listening.elapsed, 0);
}

#[test]
fn pause_seek_stall_and_long_ui_delay_do_not_add_fictitious_listening() {
    let mut listening = Listening::default();
    listening.reset(Some("/generated/track.flac".into()));
    let start = Instant::now();
    listening.sample(start, 0, 0, true);
    listening.sample(start + Duration::from_millis(100), 100_000, 0, true);
    assert_eq!(listening.elapsed, 100_000);
    listening.sample(start + Duration::from_secs(2), 100_000, 0, false);
    listening.sample(start + Duration::from_secs(10), 100_000, 0, true);
    listening.sample(start + Duration::from_millis(10_100), 200_000, 0, true);
    assert_eq!(listening.elapsed, 200_000);
    listening.sample(start + Duration::from_millis(10_200), 80_000_000, 0, true);
    assert_eq!(listening.elapsed, 200_000);
    listening.discontinue();
    listening.sample(start + Duration::from_millis(10_300), 10_000_000, 0, true);
    listening.sample(start + Duration::from_millis(10_400), 10_000_000, 0, true);
    assert_eq!(listening.elapsed, 200_000);
    listening.sample(start + Duration::from_secs(60), 60_000_000, 0, true);
    assert_eq!(listening.elapsed, 200_000);
}

#[test]
fn short_track_half_duration_and_each_queue_continuation_get_separate_instances() {
    let mut listening = Listening::default();
    let start = Instant::now();
    for (index, path) in ["/generated/one.flac", "/generated/two.flac"]
        .into_iter()
        .enumerate()
    {
        let start = start + Duration::from_secs(index as u64 * 20);
        listening.reset(Some(path.into()));
        for tick in 0..100 {
            assert!(
                listening
                    .sample(
                        start + Duration::from_millis(tick * 100),
                        tick as i64 * 100_000,
                        20_000_000,
                        true
                    )
                    .is_none()
            );
        }
        let record = listening
            .sample(
                start + Duration::from_secs(10),
                10_000_000,
                20_000_000,
                true,
            )
            .unwrap();
        assert_eq!(record.0, PathBuf::from(path));
        listening.counted = true;
    }
}

#[test]
fn unstarted_or_preview_streams_have_no_main_player_path_to_record() {
    let mut listening = Listening::default();
    listening.elapsed = 40_000_000;
    assert!(
        listening
            .sample(Instant::now(), 40_000_000, 120_000_000, true)
            .is_none()
    );
}
