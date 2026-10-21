use super::*;

fn identity() -> Identity {
    Identity {
        device: 1,
        inode: 1,
        size: 1,
        modified: (1, 1),
        changed: (1, 1),
    }
}

#[test]
fn budgets_include_metadata_capacity_and_evict_least_recent() {
    let mut cache = Cache::new(12);
    let first = Path::new("first");
    let second = Path::new("second");
    let third = Path::new("third");
    cache.insert(
        first,
        identity(),
        Operation::Image,
        vec![0; 4],
        Some(vec![0; 2]),
    );
    cache.insert(second, identity(), Operation::Image, vec![0; 6], None);
    assert_eq!(cache.bytes, 12);
    assert!(cache.get(first, identity(), Operation::Image).is_some());
    cache.insert(third, identity(), Operation::Image, vec![0; 6], None);
    assert!(cache.get(second, identity(), Operation::Image).is_none());
    assert!(cache.get(first, identity(), Operation::Image).is_some());
    cache.insert(
        third,
        identity(),
        Operation::Image,
        Vec::with_capacity(13),
        None,
    );
    assert!(cache.get(third, identity(), Operation::Image).is_none());
    assert_eq!(cache.bytes, 6);
}

#[test]
fn replacements_expiry_and_identity_changes_remove_old_payloads() {
    let mut cache = Cache::new(100);
    let path = Path::new("image");
    cache.insert(path, identity(), Operation::Image, vec![0; 5], None);
    cache.insert(path, identity(), Operation::Image, vec![0; 2], None);
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(cache.bytes, 2);
    let mut edited = identity();
    edited.modified.1 += 1;
    assert!(cache.get(path, edited, Operation::Image).is_none());
    assert_eq!(cache.bytes, 0);
    cache.insert(path, edited, Operation::Image, vec![0; 3], None);
    cache.entries.front_mut().unwrap().timestamp = Instant::now() - CACHE_TTL;
    assert!(cache.get(path, edited, Operation::Image).is_none());
    assert_eq!(cache.bytes, 0);
}

#[test]
fn entry_count_is_bounded_even_for_empty_payloads() {
    let mut cache = Cache::new(100);
    for index in 0..100 {
        cache.insert(
            Path::new(&index.to_string()),
            identity(),
            Operation::Image,
            Vec::new(),
            None,
        );
    }
    assert_eq!(cache.entries.len(), CACHE_ENTRIES);
}

#[test]
fn unchanged_previews_survive_a_normal_revisit_without_increasing_cache_budgets() {
    let mut cache = Cache::new(8);
    let path = Path::new("photo");
    cache.insert(path, identity(), Operation::PreviewImage, vec![1; 8], None);
    cache.entries.front_mut().unwrap().timestamp =
        Instant::now() - std::time::Duration::from_secs(90);
    assert_eq!(
        cache
            .get(path, identity(), Operation::PreviewImage)
            .unwrap()
            .0,
        vec![1; 8]
    );
    assert_eq!(cache.bytes, 8);
    cache.insert(
        Path::new("next"),
        identity(),
        Operation::PreviewImage,
        vec![2; 8],
        None,
    );
    assert!(
        cache
            .get(path, identity(), Operation::PreviewImage)
            .is_none()
    );
    assert_eq!(cache.bytes, 8);
}

#[test]
fn same_size_same_second_edits_and_atomic_replacements_have_distinct_identity() {
    use std::fs::{self, File, FileTimes};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let folder = std::env::temp_dir().join(format!(
        "hermes-cache-identity-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&folder).unwrap();
    let path = folder.join("image");
    let time = std::time::UNIX_EPOCH + std::time::Duration::new(1000, 100);
    fs::write(&path, b"first").unwrap();
    File::open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(time))
        .unwrap();
    let before = Identity::read(&path).unwrap();
    fs::write(&path, b"other").unwrap();
    File::open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(time + std::time::Duration::from_nanos(100)))
        .unwrap();
    let edited = Identity::read(&path).unwrap();
    assert_eq!(before.size, edited.size);
    assert_eq!(before.modified.0, edited.modified.0);
    assert_ne!(before, edited);
    let replacement = folder.join("replacement");
    fs::write(&replacement, b"third").unwrap();
    File::open(&replacement)
        .unwrap()
        .set_times(FileTimes::new().set_modified(time))
        .unwrap();
    fs::rename(replacement, &path).unwrap();
    let replaced = Identity::read(&path).unwrap();
    assert_ne!(before.inode, replaced.inode);
    assert_ne!(before, replaced);
    fs::remove_dir_all(folder).unwrap();
}
