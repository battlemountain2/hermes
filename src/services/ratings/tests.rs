// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn temp_path(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "hermes-ratings-test-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

fn wait_snapshot(store: &Store, predicate: impl Fn(&Snapshot) -> bool) -> Arc<Snapshot> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = store.snapshot();
        if predicate(&snapshot) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "Ratings worker timed out: {:?}",
            snapshot.error
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn ratings_data_validates_1_to_5_stars_and_supported_formats() {
    let mut data = RatingsData::default();
    let photo = PathBuf::from("/photos/landscape.jpg");
    let video = PathBuf::from("/videos/vacation.mp4");
    let doc = PathBuf::from("/docs/report.pdf");
    let music = PathBuf::from("/music/song.flac");
    let folder = PathBuf::from("/photos/album");

    // Valid ratings
    assert!(data.apply(Command::Set { path: photo.clone(), rating: Some(5) }).unwrap());
    assert!(data.apply(Command::Set { path: video.clone(), rating: Some(3) }).unwrap());
    assert!(data.apply(Command::Set { path: doc.clone(), rating: Some(1) }).unwrap());

    assert_eq!(data.ratings.get(&photo), Some(&5));
    assert_eq!(data.ratings.get(&video), Some(&3));
    assert_eq!(data.ratings.get(&doc), Some(&1));

    // Reject 0 or > 5 stars
    assert!(data.apply(Command::Set { path: photo.clone(), rating: Some(0) }).is_err());
    assert!(data.apply(Command::Set { path: photo.clone(), rating: Some(6) }).is_err());

    // Reject music (must use favorites instead)
    let music_err = data.apply(Command::Set { path: music, rating: Some(4) }).unwrap_err();
    assert!(music_err.contains("Music cannot be rated"));

    // Reject folders
    let folder_err = data.apply(Command::Set { path: folder, rating: Some(4) }).unwrap_err();
    assert!(folder_err.contains("Only photos, videos, and documents"));

    // Clear rating
    assert!(data.apply(Command::Set { path: photo.clone(), rating: None }).unwrap());
    assert_eq!(data.ratings.get(&photo), None);
}

#[test]
fn ratings_data_batch_sets_and_clears_multiple_files() {
    let mut data = RatingsData::default();
    let p1 = PathBuf::from("/photos/p1.png");
    let p2 = PathBuf::from("/photos/p2.png");
    let p3 = PathBuf::from("/photos/p3.png");

    let changed = data.apply(Command::SetMany {
        paths: vec![p1.clone(), p2.clone(), p3.clone()],
        rating: Some(4),
    }).unwrap();
    assert!(changed);

    assert_eq!(data.ratings.get(&p1), Some(&4));
    assert_eq!(data.ratings.get(&p2), Some(&4));
    assert_eq!(data.ratings.get(&p3), Some(&4));

    // Clear all
    let cleared = data.apply(Command::SetMany {
        paths: vec![p1.clone(), p2.clone()],
        rating: None,
    }).unwrap();
    assert!(cleared);
    assert_eq!(data.ratings.get(&p1), None);
    assert_eq!(data.ratings.get(&p2), None);
    assert_eq!(data.ratings.get(&p3), Some(&4));
}

#[test]
fn ratings_relocate_tracks_single_file_and_directory_moves() {
    let mut data = RatingsData::default();
    let img1 = PathBuf::from("/home/user/Pictures/2026/beach.jpg");
    let img2 = PathBuf::from("/home/user/Pictures/2026/sunset.png");
    let doc = PathBuf::from("/home/user/Documents/notes.txt");

    data.apply(Command::Set { path: img1.clone(), rating: Some(5) }).unwrap();
    data.apply(Command::Set { path: img2.clone(), rating: Some(4) }).unwrap();
    data.apply(Command::Set { path: doc.clone(), rating: Some(3) }).unwrap();

    // 1. Single file rename
    let img1_new = PathBuf::from("/home/user/Pictures/2026/beach_trip.jpg");
    let changed = data.apply(Command::Relocate {
        from: img1.clone(),
        to: img1_new.clone(),
    }).unwrap();
    assert!(changed);
    assert_eq!(data.ratings.get(&img1), None);
    assert_eq!(data.ratings.get(&img1_new), Some(&5));

    // 2. Directory move
    let from_dir = PathBuf::from("/home/user/Pictures/2026");
    let to_dir = PathBuf::from("/home/user/Pictures/Archive/2026");
    let changed = data.apply(Command::Relocate {
        from: from_dir,
        to: to_dir,
    }).unwrap();
    assert!(changed);

    let expected_img1 = PathBuf::from("/home/user/Pictures/Archive/2026/beach_trip.jpg");
    let expected_img2 = PathBuf::from("/home/user/Pictures/Archive/2026/sunset.png");

    assert_eq!(data.ratings.get(&img1_new), None);
    assert_eq!(data.ratings.get(&img2), None);
    assert_eq!(data.ratings.get(&expected_img1), Some(&5));
    assert_eq!(data.ratings.get(&expected_img2), Some(&4));
    assert_eq!(data.ratings.get(&doc), Some(&3)); // Unrelated file untouched
}

#[test]
fn ratings_store_persists_to_disk_and_reloads_fresh_instance() {
    let dir = temp_path("store");
    let file = dir.join("ratings.json");
    let store = Store::start(Some(file.clone()));
    wait_snapshot(&store, |s| s.ready);

    let img = PathBuf::from("/photos/pic.jpg");
    store.set(img.clone(), Some(5)).unwrap();
    store.flush(Duration::from_secs(2)).unwrap();
    drop(store);

    // Verify raw file on disk
    assert!(file.exists());
    let loaded = load(&file).unwrap();
    assert_eq!(loaded.ratings.get(&img), Some(&5));

    // Reload with fresh store
    let second_store = Store::start(Some(file));
    let snapshot = wait_snapshot(&second_store, |s| s.ready && s.data.ratings.contains_key(&img));
    assert_eq!(snapshot.data.ratings.get(&img), Some(&5));
    drop(second_store);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ratings_extension_rename_roundtrip_preserves_other_ratings_and_subsequent_edits() {
    for target in [
        "/generated/renamed.png",
        "/generated/renamed",
        "/generated/renamed.bin",
        "/generated/renamed.flac",
    ] {
        let root = temp_path("extension-rename");
        let file = root.join("ratings.json");
        let source = PathBuf::from("/generated/source.jpg");
        let unrelated = PathBuf::from("/generated/other.pdf");
        let target = PathBuf::from(target);
        let store = Store::start(Some(file.clone()));
        wait_snapshot(&store, |snapshot| snapshot.ready);
        store.set(source.clone(), Some(5)).unwrap();
        store.set(unrelated.clone(), Some(3)).unwrap();
        store.flush(Duration::from_secs(2)).unwrap();
        store.relocate(source.clone(), target.clone()).unwrap();
        store.flush(Duration::from_secs(2)).unwrap();
        let snapshot = store.snapshot();
        let expected_rating = is_ratable_format(&target).then_some(5);
        assert!(snapshot.error.is_none());
        assert_eq!(
            snapshot.data.ratings.len(),
            1 + usize::from(expected_rating.is_some())
        );
        assert_eq!(snapshot.data.ratings.get(&unrelated), Some(&3));
        assert!(!snapshot.data.ratings.contains_key(&source));
        assert_eq!(snapshot.data.ratings.get(&target).copied(), expected_rating);
        assert_eq!(load(&file).unwrap(), snapshot.data);
        drop(store);

        let restored = Store::start(Some(file.clone()));
        let snapshot = wait_snapshot(&restored, |snapshot| snapshot.ready);
        assert!(snapshot.error.is_none());
        assert_eq!(snapshot.data.ratings.get(&unrelated), Some(&3));
        restored.set(unrelated.clone(), Some(4)).unwrap();
        restored.set(source.clone(), Some(2)).unwrap();
        restored.flush(Duration::from_secs(2)).unwrap();
        let reloaded = load(&file).unwrap();
        assert_eq!(reloaded.ratings.get(&unrelated), Some(&4));
        assert_eq!(reloaded.ratings.get(&source), Some(&2));
        assert_eq!(reloaded.ratings.get(&target).copied(), expected_rating);
        drop(restored);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn ratings_load_recovers_valid_entries_and_remains_editable_after_save_and_reload() {
    let root = temp_path("individual-invalid");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("ratings.json");
    let mut ratings = serde_json::json!({
        "/generated/good.jpg": 5,
        "/generated/other.pdf": 2,
        "/generated/unsupported.bin": 4,
        "/generated/audio.flac": 3,
        "relative.jpg": 1,
        "/generated/../escape.jpg": 1,
        "/generated/zero.jpg": 0,
        "/generated/six.jpg": 6,
        "/generated/overflow.jpg": 256,
        "/generated/negative.jpg": -1,
        "/generated/fraction.jpg": 2.5,
        "/generated/string.jpg": "5",
        "/generated/null.jpg": null,
        "/generated/object.jpg": {},
        "/generated/array.jpg": [],
        "/generated/bool.jpg": true
    });
    ratings.as_object_mut().unwrap().insert(
        format!("/generated/{}.jpg", "x".repeat(4096)),
        serde_json::json!(4),
    );
    let original = serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "ratings": ratings
    }))
    .unwrap();
    fs::write(&file, &original).unwrap();
    let store = Store::start(Some(file.clone()));
    let snapshot = wait_snapshot(&store, |snapshot| snapshot.ready);
    assert!(snapshot.error.is_none());
    assert_eq!(snapshot.data.ratings.len(), 2);
    assert_eq!(store.get(Path::new("/generated/good.jpg")), Some(5));
    assert_eq!(store.get(Path::new("/generated/other.pdf")), Some(2));
    assert_eq!(fs::read(&file).unwrap(), original);
    store.set("/generated/new.png".into(), Some(3)).unwrap();
    store.flush(Duration::from_secs(2)).unwrap();
    assert_eq!(load(&file).unwrap().ratings.len(), 3);
    drop(store);

    let restored = Store::start(Some(file.clone()));
    let snapshot = wait_snapshot(&restored, |snapshot| snapshot.ready);
    assert!(snapshot.error.is_none());
    assert_eq!(snapshot.data.ratings.len(), 3);
    restored.set("/generated/good.jpg".into(), Some(4)).unwrap();
    restored.set("/generated/other.pdf".into(), None).unwrap();
    restored.flush(Duration::from_secs(2)).unwrap();
    let reloaded = load(&file).unwrap();
    assert_eq!(reloaded.ratings.len(), 2);
    assert_eq!(
        reloaded.ratings.get(Path::new("/generated/good.jpg")),
        Some(&4)
    );
    assert_eq!(
        reloaded.ratings.get(Path::new("/generated/new.png")),
        Some(&3)
    );
    drop(restored);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ratings_corrupt_or_unsupported_stores_remain_read_only_and_preserve_bytes() {
    let root = temp_path("invalid-store");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("ratings.json");
    for bytes in [
        b"not json".as_slice(),
        br#"{"version":99,"ratings":{"/generated/good.jpg":5}}"#,
        br#"{"version":1,"ratings":[]}"#,
        br#"{"ratings":{"/generated/good.jpg":5}}"#,
    ] {
        fs::write(&file, bytes).unwrap();
        let store = Store::start(Some(file.clone()));
        let initial = wait_snapshot(&store, |snapshot| snapshot.ready);
        assert!(initial.error.is_some());
        store.set("/generated/new.jpg".into(), Some(3)).unwrap();
        wait_snapshot(&store, |snapshot| snapshot.revision > initial.revision);
        assert!(store.flush(Duration::from_secs(2)).is_err());
        assert!(store.snapshot().data.ratings.is_empty());
        assert_eq!(fs::read(&file).unwrap(), bytes);
        drop(store);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ratings_save_rejects_invalid_data_without_replacing_existing_bytes() {
    let root = temp_path("save-validation");
    let file = root.join("ratings.json");
    let mut data = RatingsData::default();
    data.ratings.insert("/generated/good.jpg".into(), 4);
    save(&file, &data).unwrap();
    let original = fs::read(&file).unwrap();
    for (path, star) in [("/generated/bad.bin", 5), ("/generated/bad.jpg", 6)] {
        let mut invalid = data.clone();
        invalid.ratings.insert(path.into(), star);
        assert!(save(&file, &invalid).is_err());
        assert_eq!(fs::read(&file).unwrap(), original);
    }
    data.version = 99;
    assert!(save(&file, &data).is_err());
    assert_eq!(fs::read(&file).unwrap(), original);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ratings_worker_rejects_unserializable_paths_before_publishing_or_saving() {
    use std::os::unix::ffi::OsStringExt;

    let root = temp_path("publish-validation");
    let file = root.join("ratings.json");
    let store = Store::start(Some(file.clone()));
    wait_snapshot(&store, |snapshot| snapshot.ready);
    store.set("/generated/good.jpg".into(), Some(4)).unwrap();
    store.flush(Duration::from_secs(2)).unwrap();
    let original = fs::read(&file).unwrap();
    let invalid = PathBuf::from(std::ffi::OsString::from_vec(
        b"/generated/bad-\xff.jpg".to_vec(),
    ));
    let revision = store.snapshot().revision;
    store.set(invalid.clone(), Some(5)).unwrap();
    let rejected = wait_snapshot(&store, |snapshot| {
        snapshot.revision > revision && snapshot.error.is_some()
    });
    assert!(!rejected.data.ratings.contains_key(&invalid));
    assert_eq!(rejected.data.ratings.len(), 1);
    assert_eq!(fs::read(&file).unwrap(), original);
    store.set("/generated/good.jpg".into(), Some(2)).unwrap();
    store.flush(Duration::from_secs(2)).unwrap();
    assert!(store.snapshot().error.is_none());
    assert_eq!(
        load(&file)
            .unwrap()
            .ratings
            .get(Path::new("/generated/good.jpg")),
        Some(&2)
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ratings_relocate_collision_keeps_source_rating_through_save_and_reload() {
    let root = temp_path("collision");
    let file = root.join("ratings.json");
    let mut data = RatingsData::default();
    data.ratings.insert("/generated/source/photo.jpg".into(), 5);
    data.ratings
        .insert("/generated/destination/photo.jpg".into(), 2);
    data.ratings
        .insert("/generated/source-other/photo.jpg".into(), 3);
    assert!(
        data.apply(Command::Relocate {
            from: "/generated/source".into(),
            to: "/generated/destination".into(),
        })
        .unwrap()
    );
    save(&file, &data).unwrap();
    let mut restored = load(&file).unwrap();
    assert_eq!(restored.ratings.len(), 2);
    assert_eq!(
        restored
            .ratings
            .get(Path::new("/generated/destination/photo.jpg")),
        Some(&5)
    );
    assert_eq!(
        restored
            .ratings
            .get(Path::new("/generated/source-other/photo.jpg")),
        Some(&3)
    );
    restored
        .apply(Command::Set {
            path: "/generated/destination/photo.jpg".into(),
            rating: Some(4),
        })
        .unwrap();
    save(&file, &restored).unwrap();
    assert_eq!(load(&file).unwrap(), restored);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ratings_batch_capacity_counts_unique_new_paths() {
    let mut data = RatingsData::default();
    for index in 0..MAX_RATINGS - 1 {
        data.ratings
            .insert(format!("/generated/{index}.jpg").into(), 3);
    }
    let last = PathBuf::from("/generated/last.jpg");
    assert!(
        data.apply(Command::SetMany {
            paths: vec![last.clone(), last.clone()],
            rating: Some(5),
        })
        .unwrap()
    );
    assert_eq!(data.ratings.len(), MAX_RATINGS);
    assert_eq!(data.ratings.get(&last), Some(&5));
    validate(&data).unwrap();
    let before = data.clone();
    assert!(
        data.apply(Command::SetMany {
            paths: vec![last, "/generated/overflow.jpg".into()],
            rating: Some(1),
        })
        .is_err()
    );
    assert_eq!(data, before);
}
