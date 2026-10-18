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
