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

#[test]
fn relocate_updates_favorites_playlists_and_history_for_single_file_and_directory() {
    let mut data = Collections::default();
    let track1 = PathBuf::from("/Music/Artist/Album/track1.flac");
    let track2 = PathBuf::from("/Music/Artist/Album/track2.flac");
    let other = PathBuf::from("/Music/Other/single.mp3");

    // Populate favorites, playlist, indexed and played
    data.apply(Command::Favorite(vec![track1.clone(), other.clone()], true)).unwrap();
    data.apply(Command::Create("Favorites Mix".into())).unwrap();
    data.apply(Command::Add(1, vec![track1.clone(), track2.clone()])).unwrap();
    data.apply(Command::Observe(vec![track1.clone()], 1000)).unwrap();
    data.apply(Command::Played(track1.clone())).unwrap();

    // 1. Single file rename: track1 -> track1_remastered
    let track1_new = PathBuf::from("/Music/Artist/Album/track1_remastered.flac");
    let changed = data.apply(Command::Relocate {
        from: track1.clone(),
        to: track1_new.clone(),
    }).unwrap();
    assert!(changed);

    assert_eq!(
        data.favorite_paths(),
        [track1_new.clone(), other.clone()].into_iter().collect()
    );
    let playlist = data.playlist(1).unwrap();
    assert_eq!(
        playlist.tracks.iter().filter_map(|l| l.native_path()).collect::<Vec<_>>(),
        vec![track1_new.as_path(), track2.as_path()]
    );
    assert_eq!(data.added_dates().get(&track1_new), Some(&1000));
    assert_eq!(data.added_dates().get(&track1), None);
    assert_eq!(data.play_counts().get(&track1_new), Some(&1));
    assert_eq!(data.play_counts().get(&track1), None);

    // 2. Directory move/rename: entire "/Music/Artist/Album" -> "/Archive/Music/Album"
    let old_album = PathBuf::from("/Music/Artist/Album");
    let new_album = PathBuf::from("/Archive/Music/Album");
    let changed = data.apply(Command::Relocate {
        from: old_album,
        to: new_album,
    }).unwrap();
    assert!(changed);

    let expected_t1 = PathBuf::from("/Archive/Music/Album/track1_remastered.flac");
    let expected_t2 = PathBuf::from("/Archive/Music/Album/track2.flac");

    assert_eq!(
        data.favorite_paths(),
        [expected_t1.clone(), other.clone()].into_iter().collect()
    );
    let playlist = data.playlist(1).unwrap();
    assert_eq!(
        playlist.tracks.iter().filter_map(|l| l.native_path()).collect::<Vec<_>>(),
        vec![expected_t1.as_path(), expected_t2.as_path()]
    );
    assert_eq!(data.added_dates().get(&expected_t1), Some(&1000));
    assert_eq!(data.play_counts().get(&expected_t1), Some(&1));
}

#[test]
fn relocate_deduplicates_when_target_already_in_favorites() {
    let mut data = Collections::default();
    let track1 = PathBuf::from("/Music/track1.flac");
    let track2 = PathBuf::from("/Music/track2.flac");
    data.apply(Command::Favorite(vec![track1.clone(), track2.clone()], true)).unwrap();
    assert_eq!(data.favorite_paths().len(), 2);

    // Relocate track1 to track2 -> should deduplicate cleanly
    let changed = data.apply(Command::Relocate {
        from: track1,
        to: track2.clone(),
    }).unwrap();
    assert!(changed);
    assert_eq!(data.favorite_paths(), [track2].into_iter().collect());
}

fn close_persistent_store(store: Store, path: &Path) {
    drop(store);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(storage) = disk::Storage::open(path) {
            drop(storage);
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Collection worker did not release storage"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn replacing_member_moves_deduplicate_stably_and_allow_edits_after_save_and_reload() {
    for directory_move in [false, true] {
        let root = temporary("relocate-collision");
        let file = root.join("collections.json");
        let source = PathBuf::from("/generated/source/song.flac");
        let destination = PathBuf::from("/generated/destination/song.flac");
        let middle = PathBuf::from("/generated/middle.mp3");
        let sibling = PathBuf::from("/generated/source-other/song.flac");
        let nested_source = PathBuf::from("/generated/source/nested/other.flac");
        let nested_destination = PathBuf::from("/generated/destination/nested/other.flac");
        let mut data = Collections::default();
        data.apply(Command::Create("Source first".into())).unwrap();
        data.apply(Command::Create("Destination first".into()))
            .unwrap();
        data.apply(Command::Add(
            1,
            vec![
                source.clone(),
                middle.clone(),
                destination.clone(),
                sibling.clone(),
            ],
        ))
        .unwrap();
        data.apply(Command::Add(
            2,
            vec![
                destination.clone(),
                middle.clone(),
                source.clone(),
                sibling.clone(),
            ],
        ))
        .unwrap();
        data.apply(Command::Favorite(
            vec![source.clone(), destination.clone(), sibling.clone()],
            true,
        ))
        .unwrap();
        data.apply(Command::Observe(vec![destination.clone()], 200))
            .unwrap();
        data.apply(Command::Observe(vec![source.clone()], 100))
            .unwrap();
        data.apply(Command::Observe(vec![sibling.clone()], 300))
            .unwrap();
        data.played = vec![
            PlayCount {
                path: Location::local(source.clone()),
                count: 7,
            },
            PlayCount {
                path: Location::local(destination.clone()),
                count: 9,
            },
        ];
        if directory_move {
            for id in [1, 2] {
                data.apply(Command::Add(
                    id,
                    vec![nested_destination.clone(), nested_source.clone()],
                ))
                .unwrap();
            }
            data.apply(Command::Favorite(
                vec![nested_source.clone(), nested_destination.clone()],
                true,
            ))
            .unwrap();
            data.apply(Command::Observe(vec![nested_source.clone()], 50))
                .unwrap();
            data.apply(Command::Observe(vec![nested_destination.clone()], 75))
                .unwrap();
            data.played.extend([
                PlayCount {
                    path: Location::local(nested_source.clone()),
                    count: u64::MAX,
                },
                PlayCount {
                    path: Location::local(nested_destination.clone()),
                    count: 1,
                },
            ]);
        }
        let mut storage = disk::Storage::open(&file).unwrap();
        storage.save(&data).unwrap();
        drop(storage);

        let store = Store::start(Some(file.clone()));
        let initial = wait(&store, |snapshot| snapshot.ready);
        assert!(initial.error.is_none());
        let (from, to) = if directory_move {
            (
                PathBuf::from("/generated/source"),
                PathBuf::from("/generated/destination"),
            )
        } else {
            (source.clone(), destination.clone())
        };
        store.relocate(from, to).unwrap();
        store.flush(Duration::from_secs(2)).unwrap();
        let moved = store.snapshot();
        assert!(moved.error.is_none());
        validate(&moved.data).unwrap();
        let mut expected = vec![destination.clone(), middle.clone(), sibling.clone()];
        if directory_move {
            expected.push(nested_destination.clone());
        }
        for id in [1, 2] {
            assert_eq!(
                moved.data.playlist(id).unwrap().tracks,
                expected
                    .iter()
                    .cloned()
                    .map(Location::local)
                    .collect::<Vec<_>>()
            );
        }
        assert!(!moved.data.favorite_paths().contains(&source));
        assert!(moved.data.favorite_paths().contains(&destination));
        assert!(moved.data.favorite_paths().contains(&sibling));
        assert_eq!(
            moved.data.favorite_paths().len(),
            if directory_move { 3 } else { 2 }
        );
        assert_eq!(moved.data.added_dates()[&destination], 100);
        assert_eq!(moved.data.added_dates()[&sibling], 300);
        assert_eq!(moved.data.play_counts()[&destination], 16);
        assert!(!moved.data.added_dates().contains_key(&source));
        assert!(!moved.data.play_counts().contains_key(&source));
        if directory_move {
            assert_eq!(moved.data.added_dates()[&nested_destination], 50);
            assert_eq!(moved.data.play_counts()[&nested_destination], u64::MAX);
        }
        assert_eq!(
            load(&file).unwrap().playlist(1).unwrap().tracks,
            moved.data.playlist(1).unwrap().tracks
        );
        close_persistent_store(store, &file);

        let restored = Store::start(Some(file.clone()));
        let initial = wait(&restored, |snapshot| snapshot.ready);
        assert!(initial.error.is_none());
        assert_eq!(
            initial.data.playlist(1).unwrap().tracks.len(),
            expected.len()
        );
        restored
            .send(Command::Move(1, destination.clone(), 1))
            .unwrap();
        restored
            .send(Command::Remove(2, vec![destination.clone()]))
            .unwrap();
        restored
            .send(Command::Favorite(vec![destination.clone()], false))
            .unwrap();
        restored.send(Command::Played(destination.clone())).unwrap();
        restored.flush(Duration::from_secs(2)).unwrap();
        let edited = load(&file).unwrap();
        assert_eq!(
            edited.playlist(1).unwrap().tracks[0].native_path(),
            Some(middle.as_path())
        );
        assert_eq!(
            edited.playlist(1).unwrap().tracks[1].native_path(),
            Some(destination.as_path())
        );
        assert_eq!(edited.playlist(2).unwrap().tracks.len(), expected.len() - 1);
        assert!(!edited.favorite_paths().contains(&destination));
        assert_eq!(edited.play_counts()[&destination], 17);
        assert_eq!(edited.added_dates()[&destination], 100);
        assert!(restored.snapshot().error.is_none());
        close_persistent_store(restored, &file);

        let final_store = Store::start(Some(file.clone()));
        let final_snapshot = wait(&final_store, |snapshot| snapshot.ready);
        assert!(final_snapshot.error.is_none());
        assert_eq!(
            final_snapshot.data.playlist(1).unwrap().tracks,
            edited.playlist(1).unwrap().tracks
        );
        assert_eq!(final_snapshot.data.play_counts()[&destination], 17);
        close_persistent_store(final_store, &file);
        fs::remove_dir_all(root).unwrap();
    }
}
