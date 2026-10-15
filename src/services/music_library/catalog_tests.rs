use super::*;
use std::{
    collections::HashSet,
    io::Write,
    os::unix::{ffi::OsStringExt, fs::OpenOptionsExt},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

const DEADLINE: Duration = Duration::from_secs(4);

#[test]
#[ignore = "Generated 10k-file catalog timing; no personal data or desktop session"]
fn generated_catalog_restart_and_query_timings() {
    use crate::{adapters::LocalTextExtractionProvider, services::SearchEvent};
    let fixture = Fixture::new();
    for number in 0..10_000 {
        fixture.file(format!("document-{number:06}.txt"), b"Synthetic document");
    }
    for number in 0..80 {
        fixture.song(&format!("Album/track-{number:03}.flac"), "Synthetic song");
    }
    for restart in 0..3 {
        fixture.calls.store(0, Ordering::Relaxed);
        let started = Instant::now();
        let mut subscription = fixture.subscribe();
        let mut first = None;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(current) = subscription.poll() {
                if !current.entries.is_empty() {
                    first.get_or_insert(started.elapsed());
                }
                if !current.indexing && current.entries.len() == 10_081 {
                    assert!(current.notice.is_none(), "{:?}", current.notice);
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "Generated catalog did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let complete = started.elapsed();
        let (handle, receiver) = crate::services::search::index_tree(
            fixture.root.clone(),
            Arc::new(LocalTextExtractionProvider),
        );
        let mut samples = Vec::new();
        for number in 0..21 {
            let query = format!("document-{number:06}.txt");
            let queried = Instant::now();
            let generation = handle.query(&query);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Ok(SearchEvent::Results {
                    generation: current,
                    items,
                    ..
                }) = receiver.recv_timeout(TICK)
                    && current == generation
                {
                    assert_eq!(items[0].name, query);
                    samples.push(queried.elapsed());
                    break;
                }
                assert!(Instant::now() < deadline, "Generated query did not finish");
            }
        }
        samples.sort();
        eprintln!(
            "Generated catalog restart={restart}: first={:?}, complete={complete:?}, synthetic tag reads={}, query p50={:?}, p95={:?}, SQLite bytes={}",
            first.unwrap(),
            fixture.calls.load(Ordering::Relaxed),
            samples[10],
            samples[19],
            fs::metadata(&fixture.catalog_path).unwrap().len()
        );
        if restart > 0 {
            assert_eq!(fixture.calls.load(Ordering::Relaxed), 0);
        }
        drop(handle);
        stop(subscription);
    }
}

struct Fixture {
    root: PathBuf,
    parent: PathBuf,
    catalog_path: PathBuf,
    calls: Arc<AtomicUsize>,
    instances: Mutex<Vec<Weak<Library>>>,
}

impl Fixture {
    fn new() -> Self {
        Self::at(None)
    }

    fn at(root: Option<PathBuf>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let parent = std::env::temp_dir().join(format!(
            "hermes general catalog-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = root.unwrap_or_else(|| parent.join("Home fixture"));
        fs::create_dir_all(&root).unwrap();
        Self {
            root,
            catalog_path: parent.join(".cache/general.sqlite"),
            parent,
            calls: Arc::new(AtomicUsize::new(0)),
            instances: Mutex::new(Vec::new()),
        }
    }

    fn file(&self, name: impl AsRef<Path>, data: impl AsRef<[u8]>) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, data).unwrap();
        path
    }

    fn song(&self, name: &str, title: &str) -> PathBuf {
        self.file(
            name,
            serde_json::to_vec(&MusicTags {
                title: Some(title.into()),
                artist: Some("Synthetic artist".into()),
                album: Some("Synthetic album".into()),
                disc: Some(1),
                track: Some(2),
                ..MusicTags::default()
            })
            .unwrap(),
        )
    }

    fn configuration(&self) -> Configuration {
        let mut configuration = Configuration::new(self.root.clone());
        configuration.catalog_path = Some(self.catalog_path.clone());
        configuration.cache_path = self.parent.join(".cache/unused-tags.json");
        configuration.debounce = Duration::from_millis(20);
        configuration.monitor_limit = 64;
        let calls = self.calls.clone();
        configuration.tags = Arc::new(move |path, _| {
            calls.fetch_add(1, Ordering::Relaxed);
            serde_json::from_slice(&fs::read(path).ok()?).ok()
        });
        configuration
    }

    fn start(&self, configuration: Configuration) -> Subscription {
        assert_eq!(configuration.root, self.root);
        assert_eq!(
            configuration.catalog_path.as_ref(),
            Some(&self.catalog_path)
        );
        let subscription = subscribe_with(configuration);
        self.instances
            .lock()
            .unwrap()
            .push(Arc::downgrade(&subscription.library));
        subscription
    }

    fn subscribe(&self) -> Subscription {
        self.start(self.configuration())
    }

    fn saved(&self) -> (Vec<Entry>, Option<String>) {
        catalog_disk::read(
            &self.catalog_path,
            &self.root,
            MAX_ENTRIES,
            MAX_INDEX_BYTES,
            &Cancellation::default(),
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let instances = self.instances.lock().unwrap();
        for weak in instances.iter() {
            if let Some(library) = weak.upgrade() {
                library.cancellation.cancel();
                library.ready.notify_all();
            }
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while instances.iter().any(|weak| weak.strong_count() > 0) && Instant::now() < deadline {
            std::thread::sleep(TICK);
        }
        let _removed = fs::remove_dir_all(&self.parent);
    }
}

fn until(mut predicate: impl FnMut() -> bool, description: &str) {
    let deadline = Instant::now() + DEADLINE;
    while !predicate() {
        assert!(Instant::now() < deadline, "Timed out: {description}");
        std::thread::sleep(TICK);
    }
}

fn snapshot(
    subscription: &mut Subscription,
    predicate: impl Fn(&Snapshot) -> bool,
) -> Arc<Snapshot> {
    let deadline = Instant::now() + DEADLINE;
    let mut last = None;
    loop {
        if let Some(current) = subscription.poll() {
            if predicate(&current) {
                return current;
            }
            last = Some(current);
        }
        assert!(
            Instant::now() < deadline,
            "Catalog did not converge: {:?}",
            last.as_ref().map(|current| (
                current.indexing,
                &current.notice,
                current
                    .entries
                    .iter()
                    .map(|entry| &entry.path)
                    .collect::<Vec<_>>()
            ))
        );
        std::thread::sleep(TICK);
    }
}

fn ready(subscription: &mut Subscription, predicate: impl Fn(&Snapshot) -> bool) -> Arc<Snapshot> {
    snapshot(subscription, |current| {
        !current.indexing && predicate(current)
    })
}

fn stop(subscription: Subscription) {
    let weak = Arc::downgrade(&subscription.library);
    drop(subscription);
    until(|| weak.strong_count() == 0, "catalog worker shutdown");
}

fn title<'entry>(entry: &'entry Entry) -> Option<&'entry str> {
    entry.music_tags.as_ref()?.title.as_deref()
}

fn has_title(snapshot: &Snapshot, path: &Path, expected: &str) -> bool {
    snapshot
        .entries
        .iter()
        .any(|entry| entry.path == path && title(entry) == Some(expected))
}

fn paths(snapshot: &Snapshot) -> HashSet<PathBuf> {
    snapshot
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect()
}

#[test]
fn general_subscribers_share_names_types_and_live_add_move_remove_updates() {
    let fixture = Fixture::new();
    let text = fixture.file("Documents/read me.txt", b"Synthetic text");
    let image = fixture.file("Pictures/photo.png", b"Synthetic image");
    let song = fixture.song("Tracks/song.flac", "Shared song");
    let mut first = fixture.subscribe();
    let mut second = fixture.subscribe();
    assert!(Arc::ptr_eq(&first.library, &second.library));
    let initial = ready(&mut first, |current| {
        has_title(current, &song, "Shared song")
    });
    let peer = ready(&mut second, |current| {
        has_title(current, &song, "Shared song")
    });
    assert_eq!(initial.entries, peer.entries);
    assert_eq!(initial.entries.len(), 6);
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
    for (path, family) in [
        (&text, FormatFamily::PlainText),
        (&image, FormatFamily::Image),
    ] {
        let entry = initial
            .entries
            .iter()
            .find(|entry| &entry.path == path)
            .unwrap();
        assert!(!entry.is_directory && !entry.is_audio());
        assert_eq!(classify_by_name(entry.path.as_os_str()), family);
        assert!(entry.identity.is_some() && entry.music_tags.is_none());
    }
    assert!(
        initial
            .entries
            .iter()
            .filter(|entry| entry.is_directory)
            .count()
            == 3
    );
    let added = fixture.file("New folder/added.pdf", b"Synthetic PDF");
    let first_added = ready(&mut first, |current| paths(current).contains(&added));
    let second_added = ready(&mut second, |current| paths(current).contains(&added));
    assert_eq!(paths(&first_added), paths(&second_added));
    let moved_directory = fixture.root.join("Moved folder");
    fs::rename(fixture.root.join("New folder"), &moved_directory).unwrap();
    let moved = moved_directory.join("added.pdf");
    let moved_snapshot = ready(&mut second, |current| {
        paths(current).contains(&moved)
            && current
                .entries
                .iter()
                .all(|entry| !entry.path.starts_with(fixture.root.join("New folder")))
    });
    assert_eq!(moved_snapshot.entries.len(), 8);
    fs::remove_dir_all(&moved_directory).unwrap();
    ready(&mut first, |current| current.entries.len() == 6);
    ready(&mut second, |current| current.entries.len() == 6);
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
    drop(first);
    assert!(!second.library.cancellation.is_cancelled());
}

#[test]
fn sqlite_restart_reuses_unchanged_tags_and_native_filenames_without_decoding() {
    let fixture = Fixture::new();
    let song = fixture.song("song.flac", "Persisted title");
    let native = fixture.file(
        std::ffi::OsString::from_vec(b"notes-\xff.txt".to_vec()),
        b"Native filename",
    );
    let mut subscription = fixture.subscribe();
    let initial = ready(&mut subscription, |current| {
        current.entries.len() == 2 && has_title(current, &song, "Persisted title")
    });
    until(|| fixture.catalog_path.is_file(), "first SQLite snapshot");
    stop(subscription);
    assert!(
        fs::read(&fixture.catalog_path)
            .unwrap()
            .starts_with(b"SQLite format 3\0")
    );
    assert!(!fixture.parent.join(".cache/unused-tags.json").exists());
    let (saved, notice) = fixture.saved();
    assert!(notice.is_none());
    assert_eq!(saved.len(), 2);
    assert_eq!(&saved, initial.entries.as_ref());
    assert!(saved.iter().any(|entry| entry.path == native));
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
    let mut restarted = fixture.subscribe();
    let current = ready(&mut restarted, |current| {
        current.entries.len() == 2 && has_title(current, &song, "Persisted title")
    });
    assert!(paths(&current).contains(&native));
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
    restarted.refresh();
    ready(&mut restarted, |current| {
        has_title(current, &song, "Persisted title")
    });
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn fully_covered_folder_search_reuses_active_parent_catalog_and_filters_siblings() {
    use crate::{adapters::LocalTextExtractionProvider, services::SearchEvent};
    let fixture = Fixture::new();
    fixture.file("Chosen/needle.txt", b"Chosen result");
    fixture.file("Other/needle.txt", b"Sibling result");
    let mut parent = fixture.subscribe();
    ready(&mut parent, |current| current.entries.len() == 4);
    let child_root = fixture.root.join("Chosen");
    let child = subscribe_catalog(child_root.clone());
    assert!(Arc::ptr_eq(&child.library, &parent.library));
    let (handle, receiver) = crate::services::search::index_tree(
        child_root.clone(),
        Arc::new(LocalTextExtractionProvider),
    );
    let generation = handle.query("needle");
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Ok(SearchEvent::Results {
            generation: current,
            items,
            indexing: false,
            ..
        }) = receiver.recv_timeout(TICK)
            && current == generation
        {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].path, child_root.join("needle.txt"));
            break;
        }
        assert!(Instant::now() < deadline, "Scoped query did not settle");
    }
    drop(handle);
    drop(child);
    stop(parent);
}

#[test]
fn saved_identities_reject_same_size_edits_and_deletions_then_restart_and_refresh_repair_tags() {
    let fixture = Fixture::new();
    let edited = fixture.song("edited.flac", "Old title");
    let deleted = fixture.song("deleted.flac", "Deleted title");
    let unchanged = fixture.song("unchanged.flac", "Unchanged title");
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |current| {
        current.entries.len() == 3
            && current
                .entries
                .iter()
                .all(|entry| entry.music_tags.is_some())
    });
    until(|| fixture.catalog_path.is_file(), "saved tag identities");
    stop(subscription);
    let original = fs::metadata(&edited).unwrap().len();
    std::thread::sleep(Duration::from_millis(2));
    fixture.song("edited.flac", "New title");
    assert_eq!(fs::metadata(&edited).unwrap().len(), original);
    fs::remove_file(&deleted).unwrap();
    let (saved, notice) = fixture.saved();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].path, unchanged);
    assert_eq!(title(&saved[0]), Some("Unchanged title"));
    assert!(
        notice
            .as_deref()
            .is_some_and(|notice| notice.contains("changed"))
    );
    let mut restarted = fixture.subscribe();
    ready(&mut restarted, |current| {
        current.entries.len() == 2
            && has_title(current, &edited, "New title")
            && !paths(current).contains(&deleted)
    });
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 4);
    fixture.song("edited.flac", "Now title");
    restarted.refresh();
    ready(&mut restarted, |current| {
        has_title(current, &edited, "Now title")
    });
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 5);
}

#[test]
fn disk_read_carries_partial_coverage_notice_with_validated_entries() {
    let fixture = Fixture::new();
    let path = fixture.file("searchable.txt", b"Synthetic file");
    let identity = FileIdentity::read(&path).unwrap();
    let entries = vec![Entry {
        path,
        is_directory: false,
        modified_unix_seconds: Some(identity.modified.0),
        identity: Some(identity),
        music_tags: None,
    }];
    let notice = "Synthetic partial coverage: metadata limit reached";
    catalog_disk::write(
        &fixture.catalog_path,
        &fixture.root,
        &entries,
        Some(notice),
        &Cancellation::default(),
    )
    .unwrap();
    let (saved, saved_notice) = fixture.saved();
    assert_eq!(saved, entries);
    assert_eq!(saved_notice.as_deref(), Some(notice));
    assert!(
        catalog_disk::read(
            &fixture.catalog_path,
            &fixture.root,
            0,
            MAX_INDEX_BYTES,
            &Cancellation::default()
        )
        .is_err()
    );
    assert!(
        catalog_disk::read(
            &fixture.catalog_path,
            &fixture.root,
            MAX_ENTRIES,
            1,
            &Cancellation::default()
        )
        .is_err()
    );
}

#[test]
fn simultaneous_home_and_music_share_one_tag_reader_and_home_drop_preserves_music() {
    let mut music = tests::Fixture::new();
    music.cache_path = music.root.parent().unwrap().join(".cache/tags.json");
    music.song("song.flac", "Shared original");
    let song = music.root.join("song.flac");
    let fixture = Fixture::at(Some(music.root.parent().unwrap().to_path_buf()));
    let document = fixture.file("Documents/note.txt", b"Synthetic home document");
    let mut music_subscription = music.subscribe();
    let mut configuration = fixture.configuration();
    configuration.music_source = Some(music.root.clone());
    let mut home = fixture.start(configuration);
    let home_initial = ready(&mut home, |current| {
        has_title(current, &song, "Shared original") && paths(current).contains(&document)
    });
    let music_initial = ready(&mut music_subscription, |current| {
        has_title(current, &song, "Shared original")
    });
    assert_eq!(
        home_initial
            .entries
            .iter()
            .filter(|entry| entry.path == song)
            .count(),
        1
    );
    assert_eq!(
        home_initial.entries.iter().find(|entry| entry.path == song),
        music_initial
            .entries
            .iter()
            .find(|entry| entry.path == song)
    );
    assert_eq!(music.calls.load(Ordering::Relaxed), 1);
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        music_subscription
            .library
            .shared
            .lock()
            .unwrap()
            .subscribers,
        2
    );
    music.song("song.flac", "Shared updated title");
    let music_updated = ready(&mut music_subscription, |current| {
        has_title(current, &song, "Shared updated title")
    });
    let home_updated = ready(&mut home, |current| {
        has_title(current, &song, "Shared updated title")
    });
    assert_eq!(
        home_updated.entries.iter().find(|entry| entry.path == song),
        music_updated
            .entries
            .iter()
            .find(|entry| entry.path == song)
    );
    assert_eq!(music.calls.load(Ordering::Relaxed), 2);
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 0);
    fs::remove_file(&song).unwrap();
    ready(&mut music_subscription, |current| {
        !paths(current).contains(&song)
    });
    ready(&mut home, |current| !paths(current).contains(&song));
    stop(home);
    assert_eq!(
        music_subscription
            .library
            .shared
            .lock()
            .unwrap()
            .subscribers,
        1
    );
    assert!(!music_subscription.library.cancellation.is_cancelled());
    music.song("after-home.flac", "Music stays alive");
    ready(&mut music_subscription, |current| {
        has_title(
            current,
            &music.root.join("after-home.flac"),
            "Music stays alive",
        )
    });
    assert_eq!(music.calls.load(Ordering::Relaxed), 3);
}

#[test]
fn composed_home_prioritizes_music_and_honors_entry_and_byte_limits() {
    for byte_limited in [false, true] {
        let mut music = tests::Fixture::new();
        music.cache_path = music.root.parent().unwrap().join(".cache/tags.json");
        music.song("song.flac", "Priority song");
        let song = music.root.join("song.flac");
        let fixture = Fixture::at(Some(music.root.parent().unwrap().to_path_buf()));
        fixture.file("ordinary.txt", b"Synthetic ordinary file");
        let mut music_subscription = music.subscribe();
        ready(&mut music_subscription, |current| {
            has_title(current, &song, "Priority song")
        });
        let mut configuration = fixture.configuration();
        configuration.music_source = Some(music.root.clone());
        configuration.entries = 2;
        if byte_limited {
            configuration.bytes = 1;
        }
        let mut home = fixture.start(configuration);
        let current = ready(&mut home, |current| current.notice.is_some());
        assert!(current.notice.as_deref().unwrap().contains("limit"));
        if byte_limited {
            assert!(current.entries.is_empty());
        } else {
            assert_eq!(current.entries.len(), 2);
            assert_eq!(current.entries[0].path, music.root);
            assert!(current.entries[0].is_directory);
            assert_eq!(current.entries[1].path, song);
            assert_eq!(title(&current.entries[1]), Some("Priority song"));
        }
        assert_eq!(fixture.calls.load(Ordering::Relaxed), 0);
        assert_eq!(music.calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn unmonitored_general_catalog_reconciles_synthetic_changes_periodically() {
    let fixture = Fixture::new();
    let mut configuration = fixture.configuration();
    configuration.monitor_limit = 0;
    configuration.refresh_interval = Duration::from_millis(100);
    let mut subscription = fixture.start(configuration);
    let initial = ready(&mut subscription, |current| current.entries.is_empty());
    assert!(
        initial
            .notice
            .as_deref()
            .unwrap()
            .contains("cannot be watched")
    );
    let added = fixture.file("Added folder/file.txt", b"Periodic fixture");
    ready(&mut subscription, |current| paths(current).contains(&added));
    fs::remove_dir_all(fixture.root.join("Added folder")).unwrap();
    let removed = ready(&mut subscription, |current| current.entries.is_empty());
    assert!(
        removed
            .notice
            .as_deref()
            .unwrap()
            .contains("cannot be watched")
    );
}

#[test]
fn entry_byte_and_tag_limits_report_partial_coverage_without_losing_audio_names() {
    for limit in 0..3 {
        let fixture = Fixture::new();
        let songs = [
            fixture.song("first.flac", "First"),
            fixture.song("second.flac", "Second"),
            fixture.song("third.flac", "Third"),
        ];
        let mut configuration = fixture.configuration();
        match limit {
            0 => configuration.entries = 1,
            1 => configuration.bytes = 1,
            _ => configuration.tracks = 1,
        }
        let mut subscription = fixture.start(configuration);
        let current = ready(&mut subscription, |current| current.notice.is_some());
        assert!(current.notice.as_deref().unwrap().contains("limit"));
        match limit {
            0 => assert_eq!(current.entries.len(), 1),
            1 => {
                assert!(current.entries.is_empty());
                assert_eq!(fixture.calls.load(Ordering::Relaxed), 0);
            }
            _ => {
                assert_eq!(paths(&current), songs.into_iter().collect());
                assert_eq!(
                    current
                        .entries
                        .iter()
                        .filter(|entry| entry.music_tags.is_some())
                        .count(),
                    1
                );
                assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
                assert!(current.notice.as_deref().unwrap().contains("filename"));
            }
        }
    }
}

#[test]
fn metadata_byte_budget_keeps_all_filenames_and_reports_discarded_tags() {
    let fixture = Fixture::new();
    let songs = [
        fixture.song("first.flac", &"A".repeat(200)),
        fixture.song("second.flac", &"B".repeat(200)),
    ];
    let mut configuration = fixture.configuration();
    configuration.entries = songs.len();
    configuration.bytes = songs
        .iter()
        .map(|path| size_of::<Entry>() + path.capacity())
        .sum::<usize>()
        + 32;
    let mut subscription = fixture.start(configuration);
    let current = ready(&mut subscription, |current| {
        current.entries.len() == 2 && current.notice.is_some()
    });
    assert_eq!(paths(&current), songs.into_iter().collect());
    assert!(
        current
            .entries
            .iter()
            .all(|entry| entry.music_tags.is_none())
    );
    assert!(current.notice.as_deref().unwrap().contains("metadata"));
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 2);
}

struct ReleaseOnDrop(Arc<AtomicBool>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[test]
fn filenames_publish_before_blocked_metadata_and_complete_after_release() {
    let fixture = Fixture::new();
    let song = fixture.song("song.flac", "Eventually tagged");
    let document = fixture.file("document.txt", b"Names available immediately");
    let released = Arc::new(AtomicBool::new(false));
    let release = ReleaseOnDrop(released.clone());
    let entered = Arc::new(AtomicBool::new(false));
    let reader_entered = entered.clone();
    let calls = fixture.calls.clone();
    let mut configuration = fixture.configuration();
    configuration.tags = Arc::new(move |path, cancellation| {
        calls.fetch_add(1, Ordering::Relaxed);
        reader_entered.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !released.load(Ordering::Acquire)
            && !cancellation.is_cancelled()
            && Instant::now() < deadline
        {
            std::thread::sleep(TICK);
        }
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    });
    let mut subscription = fixture.start(configuration);
    until(
        || entered.load(Ordering::Acquire),
        "synthetic tag reader start",
    );
    let names = snapshot(&mut subscription, |current| {
        current.indexing && current.entries.len() == 2
    });
    assert_eq!(
        paths(&names),
        [song.clone(), document].into_iter().collect()
    );
    assert!(names.entries.iter().all(|entry| entry.music_tags.is_none()));
    assert!(!fixture.catalog_path.exists());
    drop(release);
    ready(&mut subscription, |current| {
        has_title(current, &song, "Eventually tagged")
    });
    until(
        || fixture.catalog_path.is_file(),
        "complete snapshot persisted",
    );
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn general_scan_and_saved_catalog_exclude_hidden_paths_and_symlink_targets() {
    let fixture = Fixture::new();
    fixture.song(".hidden/secret.flac", "Hidden song");
    fixture.file(".secret.txt", b"Hidden file");
    let visible = fixture.song("Visible/song.flac", "Visible song");
    let outside = fixture.parent.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.txt"), b"Outside fixture").unwrap();
    std::os::unix::fs::symlink(&outside, fixture.root.join("linked-directory")).unwrap();
    std::os::unix::fs::symlink(&visible, fixture.root.join("linked.flac")).unwrap();
    std::os::unix::fs::symlink(&fixture.root, fixture.root.join("loop")).unwrap();
    let mut subscription = fixture.subscribe();
    let current = ready(&mut subscription, |current| {
        has_title(current, &visible, "Visible song")
    });
    assert_eq!(
        paths(&current),
        [fixture.root.join("Visible"), visible]
            .into_iter()
            .collect()
    );
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
    until(
        || fixture.catalog_path.is_file(),
        "privacy-filtered snapshot",
    );
    stop(subscription);
    let (saved, _) = fixture.saved();
    assert_eq!(saved.len(), 2);
    fs::remove_dir_all(fixture.root.join("Visible")).unwrap();
    std::os::unix::fs::symlink(&outside, fixture.root.join("Visible")).unwrap();
    let (saved, notice) = fixture.saved();
    assert!(saved.is_empty());
    assert!(
        notice
            .as_deref()
            .is_some_and(|notice| notice.contains("changed"))
    );
}

#[test]
fn missing_or_corrupt_sqlite_catalog_falls_back_to_usable_names_with_honest_notices() {
    for corrupt in [false, true] {
        let fixture = Fixture::new();
        let name = fixture.file("usable.txt", b"Fallback fixture");
        let mut configuration = fixture.configuration();
        let corruption = b"This is deliberately not a SQLite database";
        if corrupt {
            fs::create_dir_all(fixture.catalog_path.parent().unwrap()).unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                fixture.catalog_path.parent().unwrap(),
                fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&fixture.catalog_path)
                .unwrap();
            file.write_all(corruption).unwrap();
        } else {
            assert!(!fixture.catalog_path.exists());
        }
        configuration.monitor_limit = 64;
        let mut subscription = fixture.start(configuration);
        let current = ready(&mut subscription, |current| {
            paths(current).contains(&name) && (!corrupt || current.notice.is_some())
        });
        assert_eq!(current.entries.len(), 1);
        assert_eq!(fixture.calls.load(Ordering::Relaxed), 0);
        if corrupt {
            let notice = current.notice.as_deref().unwrap().to_ascii_lowercase();
            assert!(notice.contains("index") || notice.contains("catalog"));
            assert!(
                notice.contains("unavailable")
                    || notice.contains("could not")
                    || notice.contains("corrupt")
            );
            stop(subscription);
            assert!(
                catalog_disk::read(
                    &fixture.catalog_path,
                    &fixture.root,
                    MAX_ENTRIES,
                    MAX_INDEX_BYTES,
                    &Cancellation::default()
                )
                .is_err()
            );
            assert_eq!(
                fs::read(&fixture.catalog_path).unwrap().as_slice(),
                corruption.as_slice()
            );
        } else {
            assert!(current.notice.is_none(), "{:?}", current.notice);
            until(
                || fixture.catalog_path.is_file(),
                "missing catalog initialized",
            );
            stop(subscription);
            assert_eq!(fixture.saved().0.len(), 1);
        }
    }
}

#[test]
fn missing_general_root_reports_unavailability_and_recovers_without_manual_refresh() {
    let fixture = Fixture::new();
    fs::remove_dir(&fixture.root).unwrap();
    let mut subscription = fixture.subscribe();
    let missing = ready(&mut subscription, |current| {
        current.entries.is_empty()
            && current
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("unavailable"))
    });
    assert!(
        missing
            .notice
            .as_deref()
            .unwrap()
            .contains(fixture.root.to_str().unwrap())
    );
    let recovered = fixture.file("Recovered/file.txt", b"Recovered synthetic root");
    ready(&mut subscription, |current| {
        paths(current).contains(&recovered) && current.notice.is_none()
    });
}
