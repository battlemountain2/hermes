use super::*;

#[test]
fn cancelled_audio_preview_jobs_do_not_open_a_file_or_spawn_a_decoder() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        read_audio_artwork(
            std::path::Path::new("/nonexistent/track.flac"),
            &cancellation
        )
        .is_none()
    );
    assert_eq!(
        read_audio_metadata(
            std::path::Path::new("/nonexistent/track.flac"),
            &cancellation
        )
        .err()
        .unwrap(),
        "Preview cancelled"
    );
}

fn identity(index: u64) -> Identity {
    Identity {
        path: PathBuf::from(format!("/album/{index}.flac")),
        device: 1,
        inode: index,
        size: 100,
        modified: (1, 1),
        changed: (1, 1),
    }
}

#[test]
fn cache_is_bounded_and_refreshes_recent_entries() {
    let mut cache = Cache::default();
    for index in 0..512 {
        cache.put(identity(index), MusicTags::default());
    }
    assert!(cache.get(&identity(0)).is_some());
    cache.put(identity(512), MusicTags::default());
    assert_eq!(cache.0.len(), 512);
    assert!(cache.get(&identity(0)).is_some());
    assert!(cache.get(&identity(1)).is_none());
}

#[test]
fn cache_invalidates_same_size_same_second_edits_and_replacements() {
    let mut cache = Cache::default();
    let original = identity(1);
    cache.put(original.clone(), MusicTags::default());
    let mut changed = original.clone();
    changed.modified.1 += 1;
    assert!(cache.get(&changed).is_none());
    changed = original.clone();
    changed.inode += 1;
    assert!(cache.get(&changed).is_none());
    changed = original;
    changed.changed.1 += 1;
    assert!(cache.get(&changed).is_none());
    cache.put(changed, MusicTags::default());
    assert_eq!(cache.0.len(), 1);
}
