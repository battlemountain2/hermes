use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hermes-queue-session-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("queue.json")
    }
    fn track(&self, name: &str) -> Track {
        let path = self.0.join(name);
        fs::write(&path, b"queue fixture; not decoded").unwrap();
        Track {
            path,
            tags: MusicTags {
                title: Some(name.into()),
                ..MusicTags::default()
            },
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn restart_preserves_explicit_order_history_duplicates_and_transport_settings() {
    let fixture = Fixture::new();
    let first = fixture.track("one.flac");
    let second = fixture.track("two.mp3");
    let mut queue = Queue::default();
    queue.replace(vec![first.clone(), second.clone(), second], 0);
    queue.set_shuffle(true, 42);
    queue.move_before(2, 1);
    queue.advance(false);
    queue.repeat = Repeat::All;
    write(&fixture.path(), &Session::capture(&queue, 42_000_000, 0.35)).unwrap();
    let restored = load(&fixture.path()).unwrap().unwrap();
    assert_eq!(restored.position, 42_000_000);
    assert_eq!(restored.volume, 0.35);
    let restored = restored.into_queue();
    assert_eq!(restored.tracks, queue.tracks);
    assert_eq!(restored.order, queue.order);
    assert_eq!(restored.history, queue.history);
    assert_eq!(restored.current, queue.current);
    assert_eq!(restored.repeat, Repeat::All);
    assert!(restored.shuffle);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(fixture.path()).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn missing_songs_are_pruned_and_missing_current_advances_without_old_position() {
    let fixture = Fixture::new();
    let tracks = vec![
        fixture.track("one.flac"),
        fixture.track("two.flac"),
        fixture.track("three.flac"),
        fixture.track("four.flac"),
    ];
    let mut queue = Queue::default();
    queue.replace(tracks.clone(), 1);
    write(&fixture.path(), &Session::capture(&queue, 99_000_000, 0.75)).unwrap();
    fs::remove_file(&tracks[1].path).unwrap();
    fs::remove_file(&tracks[2].path).unwrap();
    let restored = load(&fixture.path()).unwrap().unwrap();
    assert_eq!(restored.position, 0);
    let queue = restored.into_queue();
    assert_eq!(queue.tracks, vec![tracks[0].clone(), tracks[3].clone()]);
    assert_eq!(queue.current, Some(1));
    assert_eq!(queue.playing(), Some(&tracks[3]));
}

#[test]
fn completed_and_empty_queues_do_not_restart_the_first_song() {
    let fixture = Fixture::new();
    let mut queue = Queue::default();
    queue.replace(vec![fixture.track("one.flac")], 0);
    queue.advance(true);
    write(&fixture.path(), &Session::capture(&queue, 0, 0.75)).unwrap();
    assert!(
        load(&fixture.path())
            .unwrap()
            .unwrap()
            .into_queue()
            .current
            .is_none()
    );
    queue.replace(Vec::new(), 0);
    write(&fixture.path(), &Session::capture(&queue, 0, 0.75)).unwrap();
    assert!(
        load(&fixture.path())
            .unwrap()
            .unwrap()
            .into_queue()
            .tracks
            .is_empty()
    );
}

#[test]
fn corrupt_oversized_and_invalid_sessions_are_rejected() {
    let fixture = Fixture::new();
    assert!(load(&fixture.path()).unwrap().is_none());
    fs::write(fixture.path(), b"not json").unwrap();
    assert!(load(&fixture.path()).is_err());
    let file = fs::File::create(fixture.path()).unwrap();
    file.set_len(MAX_BYTES + 1).unwrap();
    assert!(load(&fixture.path()).is_err());
    let mut queue = Queue::default();
    queue.replace(vec![fixture.track("one.flac")], 0);
    let good = Session::capture(&queue, 0, 0.75);
    for change in 0..9 {
        let mut session = good.clone();
        match change {
            0 => session.version = 99,
            1 => session.order = vec![7],
            2 => session.order = vec![0, 0],
            3 => session.current = Some(99),
            4 => session.history = vec![99],
            5 => session.position = -1,
            6 => session.volume = 7.0,
            7 => session.tracks[0].path = Location::local("/tmp/../song.flac"),
            8 => session.tracks[0].tags.title = Some("bad\nmetadata".into()),
            _ => unreachable!(),
        }
        fs::write(fixture.path(), serde_json::to_vec(&session).unwrap()).unwrap();
        assert!(
            load(&fixture.path()).is_err(),
            "Invalid session variant {change}"
        );
    }
}

#[test]
fn native_non_utf8_song_paths_round_trip() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let path = fixture
        .0
        .join(std::ffi::OsString::from_vec(b"song-\xff.flac".to_vec()));
    fs::write(&path, b"fixture").unwrap();
    let mut queue = Queue::default();
    queue.replace(
        vec![Track {
            path: path.clone(),
            tags: MusicTags::default(),
        }],
        0,
    );
    write(&fixture.path(), &Session::capture(&queue, 0, 0.75)).unwrap();
    assert_eq!(
        load(&fixture.path())
            .unwrap()
            .unwrap()
            .into_queue()
            .playing()
            .unwrap()
            .path,
        path
    );
}

#[test]
fn background_saves_coalesce_and_shutdown_flushes_the_last_session() {
    let fixture = Fixture::new();
    let mut queue = Queue::default();
    queue.replace(vec![fixture.track("one.flac")], 0);
    let mut store = Store::new(fixture.path());
    for position in 0..100 {
        store.save(Session::capture(&queue, position, 0.75));
    }
    store.finish();
    assert!(store.take_error().is_none());
    assert_eq!(load(&fixture.path()).unwrap().unwrap().position, 99);
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 2);
}

#[test]
fn failed_saves_report_errors_without_destroying_the_existing_queue() {
    let fixture = Fixture::new();
    let mut queue = Queue::default();
    queue.replace(vec![fixture.track("one.flac")], 0);
    let good = Session::capture(&queue, 10, 0.75);
    write(&fixture.path(), &good).unwrap();
    let mut store = Store::new(fixture.path());
    let mut invalid = good;
    invalid.order.clear();
    store.save(invalid);
    store.finish();
    assert!(store.take_error().is_some());
    assert_eq!(load(&fixture.path()).unwrap().unwrap().position, 10);
}
