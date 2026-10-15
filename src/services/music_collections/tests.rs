use super::*;

fn paths() -> Vec<PathBuf> {
    [
        "/generated/album/one.flac",
        "/generated/album/two.flac",
        "/generated/other/three.mp3",
    ]
    .map(PathBuf::from)
    .to_vec()
}

pub(super) fn wait(store: &Store, predicate: impl Fn(&Snapshot) -> bool) -> Arc<Snapshot> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = store.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "collection worker timed out: {:?}",
            snapshot.error
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn temporary(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "hermes-collections-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

#[test]
fn favorites_album_paths_are_idempotent_and_removed_only_explicitly() {
    let mut data = Collections::default();
    let tracks = paths();
    assert!(data.apply(Command::Favorite(tracks.clone(), true)).unwrap());
    assert!(!data.apply(Command::Favorite(tracks.clone(), true)).unwrap());
    assert!(
        data.apply(Command::Favorite(vec![tracks[1].clone()], false))
            .unwrap()
    );
    assert_eq!(
        data.favorite_paths(),
        [tracks[0].clone(), tracks[2].clone()].into_iter().collect()
    );
    assert!(data.apply(Command::Observe(vec![], 100)).is_ok());
    assert_eq!(data.favorite_paths().len(), 2);
}

#[test]
fn manual_playlists_create_rename_deduplicate_reorder_remove_and_delete() {
    let mut data = Collections::default();
    let tracks = paths();
    data.apply(Command::Create("  Evening  ".into())).unwrap();
    data.apply(Command::Add(1, tracks.clone())).unwrap();
    assert!(!data.apply(Command::Add(1, tracks.clone())).unwrap());
    data.apply(Command::Rename(1, "Favorites mix".into()))
        .unwrap();
    data.apply(Command::Move(1, tracks[2].clone(), -1)).unwrap();
    assert_eq!(
        data.playlist(1).unwrap().tracks[1].native_path(),
        Some(tracks[2].as_path())
    );
    data.apply(Command::MoveBefore(
        1,
        tracks[0].clone(),
        tracks[1].clone(),
        true,
    ))
    .unwrap();
    assert_eq!(
        data.playlist(1).unwrap().tracks[2].native_path(),
        Some(tracks[0].as_path())
    );
    data.apply(Command::Remove(1, vec![tracks[1].clone()]))
        .unwrap();
    assert_eq!(data.playlist(1).unwrap().name, "Favorites mix");
    assert_eq!(data.playlist(1).unwrap().tracks.len(), 2);
    assert!(!data.apply(Command::Move(1, tracks[2].clone(), -1)).unwrap());
    assert!(
        !data
            .apply(Command::MoveBefore(
                1,
                tracks[0].clone(),
                PathBuf::from("/absent.flac"),
                false
            ))
            .unwrap()
    );
    data.apply(Command::Delete(1)).unwrap();
    data.apply(Command::Create("New".into())).unwrap();
    assert_eq!(data.playlists[0].id, 2);
}

#[test]
fn invalid_names_paths_ids_and_limits_are_rejected() {
    let mut data = Collections::default();
    for name in ["", "   ", "bad\nname"] {
        assert!(data.apply(Command::Create(name.into())).is_err());
    }
    assert!(data.apply(Command::Create("z".repeat(81))).is_err());
    for path in [
        "relative.flac",
        "/generated/../track.flac",
        "/generated/text.txt",
    ] {
        assert!(
            data.apply(Command::Favorite(vec![path.into()], true))
                .is_err()
        );
    }
    assert!(data.apply(Command::Rename(99, "Absent".into())).is_err());
    for _ in 0..MAX_PLAYLISTS {
        data.apply(Command::Create("Mix".into())).unwrap();
    }
    assert!(data.apply(Command::Create("Overflow".into())).is_err());
    data.playlists[0].id = 0;
    assert!(validate(&data).is_err());
    let store = Store::start(None);
    assert!(
        store
            .send(Command::Add(1, vec![paths()[0].clone(); MAX_TRACKS + 1]))
            .is_err()
    );
}

#[test]
fn first_index_date_is_stable_across_reindex_removal_and_return() {
    let mut data = Collections::default();
    let tracks = paths();
    data.apply(Command::Observe(vec![tracks[0].clone()], 100))
        .unwrap();
    data.apply(Command::Observe(
        vec![tracks[0].clone(), tracks[1].clone()],
        200,
    ))
    .unwrap();
    data.apply(Command::Observe(vec![], 300)).unwrap();
    assert!(
        !data
            .apply(Command::Observe(vec![tracks[0].clone()], 400))
            .unwrap()
    );
    assert_eq!(data.added_dates()[&tracks[0]], 100);
    assert_eq!(data.added_dates()[&tracks[1]], 200);
}

#[test]
fn meaningful_listening_thresholds_exclude_short_skips_and_support_short_tracks() {
    let store = Store::start(None);
    let path = paths()[0].clone();
    assert!(
        !store
            .record_listen(path.clone(), 29_999_999, 240_000_000)
            .unwrap()
    );
    assert!(
        !store
            .record_listen(path.clone(), 9_999_999, 20_000_000)
            .unwrap()
    );
    assert!(!store.record_listen(path.clone(), 29_999_999, 0).unwrap());
    assert!(
        store
            .record_listen(path.clone(), 10_000_000, 20_000_000)
            .unwrap()
    );
    wait(&store, |snapshot| {
        snapshot.data.play_counts().get(&path) == Some(&1)
    });
    assert!(store.record_listen(path.clone(), 30_000_000, 0).unwrap());
    wait(&store, |snapshot| {
        snapshot.data.play_counts().get(&path) == Some(&2)
    });
}

#[test]
fn byte_safe_atomic_roundtrip_preserves_missing_paths_and_never_writes_audio() {
    use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
    let root = temporary("roundtrip");
    fs::create_dir_all(&root).unwrap();
    let audio = root.join("existing.flac");
    fs::write(&audio, b"unchanged generated fixture").unwrap();
    let missing = root.join(std::ffi::OsString::from_vec(b"missing-\xff.flac".to_vec()));
    let mut data = Collections::default();
    data.apply(Command::Create("Mix".into())).unwrap();
    data.apply(Command::Add(1, vec![audio.clone(), missing.clone()]))
        .unwrap();
    data.apply(Command::Favorite(vec![missing.clone()], true))
        .unwrap();
    data.apply(Command::Observe(vec![audio.clone()], 100))
        .unwrap();
    data.apply(Command::Played(audio.clone())).unwrap();
    let path = root.join("collections.json");
    write(&path, &data).unwrap();
    let restored = load(&path).unwrap();
    assert_eq!(
        restored.playlist(1).unwrap().tracks[1].native_path(),
        Some(missing.as_path())
    );
    assert!(restored.favorite_paths().contains(&missing));
    assert_eq!(restored.play_counts()[&audio], 1);
    assert_eq!(fs::read(&audio).unwrap(), b"unchanged generated fixture");
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn worker_preserves_corrupt_or_future_version_data_instead_of_overwriting_it() {
    let root = temporary("corrupt");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("collections.json");
    for bytes in [
        b"not json".to_vec(),
        serde_json::to_vec(&Collections {
            version: 99,
            ..Default::default()
        })
        .unwrap(),
    ] {
        fs::write(&path, &bytes).unwrap();
        let store = Store::start(Some(path.clone()));
        wait(&store, |snapshot| {
            snapshot.ready && snapshot.error.is_some()
        });
        let revision = store.snapshot().revision;
        store
            .send(Command::Create("Don't overwrite".into()))
            .unwrap();
        wait(&store, |snapshot| snapshot.revision > revision);
        drop(store);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_save_retries_after_storage_recovers_without_another_mutation() {
    let root = temporary("retry");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("blocked/collections.json");
    let store = Store::start(Some(path.clone()));
    wait(&store, |snapshot| snapshot.ready);
    fs::create_dir(&path).unwrap();
    store.send(Command::Create("Unsaved".into())).unwrap();
    let failed = wait(&store, |snapshot| snapshot.error.is_some());
    assert_eq!(failed.data.playlists[0].name, "Unsaved");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(store.snapshot().revision, failed.revision);
    fs::remove_dir(&path).unwrap();
    wait(&store, |snapshot| {
        snapshot.revision > failed.revision && snapshot.error.is_none()
    });
    assert_eq!(load(&path).unwrap().playlists[0].name, "Unsaved");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn disconnect_flushes_accepted_mutations_and_restart_restores_without_audio_reads() {
    let root = temporary("flush");
    let path = root.join("collections.json");
    let store = Store::start(Some(path.clone()));
    wait(&store, |snapshot| snapshot.ready);
    store.send(Command::Create("Saved".into())).unwrap();
    store.send(Command::Add(1, paths())).unwrap();
    wait(&store, |snapshot| {
        snapshot
            .data
            .playlist(1)
            .is_some_and(|playlist| playlist.tracks.len() == 3)
    });
    drop(store);
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let restored = Store::start(Some(path));
    wait(&restored, |snapshot| {
        snapshot.ready
            && snapshot
                .data
                .playlist(1)
                .is_some_and(|playlist| playlist.tracks.len() == 3)
    });
    drop(restored);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_shutdown_flush_acknowledges_background_save_without_waiting_for_debounce() {
    let root = temporary("shutdown");
    let path = root.join("collections.json");
    let store = Store::start(Some(path.clone()));
    wait(&store, |snapshot| snapshot.ready);
    store.send(Command::Favorite(paths(), true)).unwrap();
    store.flush(Duration::from_secs(2)).unwrap();
    assert_eq!(load(&path).unwrap().favorite_paths().len(), 3);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
