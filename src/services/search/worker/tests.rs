use super::*;
use crate::services::search::UnavailableTextExtraction;
use std::os::unix::fs::MetadataExt;
use std::{fs, sync::atomic::AtomicUsize, time::SystemTime};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("hermes-song-search-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn file(&self, name: &str) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"metadata fixture").unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

fn finish(receiver: &SearchReceiver, expected: &str) -> (Vec<SearchItem>, Option<String>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(SearchEvent::Results {
            query,
            items,
            indexing: false,
            notice,
        }) = receiver.recv_timeout(TICK)
        {
            if query == expected {
                return (items, notice);
            }
        }
    }
    panic!("Search did not finish: {expected}");
}

fn reader() -> Arc<TagReader> {
    Arc::new(|path, cancellation| {
        if cancellation.is_cancelled() {
            return None;
        }
        Some(MusicTags {
            title: Some(
                if path.ends_with("01.flac") {
                    "A Secret Sunrise"
                } else {
                    "Another Journey"
                }
                .into(),
            ),
            artist: Some("Cross Album Artist".into()),
            album_artist: Some("Compilation Collective".into()),
            album: Some("Invisible Album Tag".into()),
            ..MusicTags::default()
        })
    })
}

#[test]
fn shared_scope_routes_only_visible_music_subtrees() {
    let music = Path::new("/home/me/Music");
    assert!(shared_scope(music, music));
    assert!(shared_scope(Path::new("/home/me/Music/Album"), music));
    assert!(!shared_scope(
        Path::new("/home/me/Music/../Documents"),
        music
    ));
    assert!(!shared_scope(Path::new("/home/me/Music/.private"), music));
    assert!(!shared_scope(Path::new("/home/me/MusicBackup"), music));
}

#[test]
fn music_view_and_active_search_share_live_library_metadata() {
    use crate::services::music_library::tests::{Fixture as LibraryFixture, ready};
    let fixture = LibraryFixture::new();
    fixture.song("Album/01.flac", "Shared lantern");
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .any(|entry| entry.music_tags.is_some())
    });
    let mut music = crate::music::library::Scan::start(fixture.root.clone());
    let (handle, events) = start_shared(
        fixture.root.clone(),
        Arc::new(UnavailableTextExtraction),
        crate::services::music_library::subscribe(fixture.root.clone()),
    );
    assert!(handle.is_live());
    handle.query("lantern");
    assert_eq!(finish(&events, "lantern").0.len(), 1);
    fixture.song("Album/02.flac", "Second lantern");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut searched = false;
    let mut browsed = false;
    while Instant::now() < deadline && (!searched || !browsed) {
        if let Ok(SearchEvent::Results {
            query,
            items,
            indexing: false,
            ..
        }) = events.recv_timeout(TICK)
            && query == "lantern"
            && items.len() == 2
        {
            searched = true;
        }
        if let Some(snapshot) = music.poll()
            && !snapshot.indexing
            && snapshot.tracks.len() == 2
        {
            browsed = true;
        }
    }
    assert!(
        searched && browsed,
        "Both consumers should see automatic additions"
    );
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 2);
    drop(handle);
    drop(subscription);
    fixture.song("Album/03.flac", "Third lantern");
    loop {
        if let Some(snapshot) = music.poll()
            && !snapshot.indexing
            && snapshot.tracks.len() == 3
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Closing search must not stop Music's library"
        );
        std::thread::sleep(TICK);
    }
}

#[test]
fn shared_folder_scope_keeps_other_albums_out_and_removes_deleted_hits() {
    use crate::services::music_library::tests::{Fixture as LibraryFixture, ready};
    let fixture = LibraryFixture::new();
    fixture.song("First/01.flac", "Morning lantern");
    fixture.song("Second/01.flac", "Evening lantern");
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .filter(|entry| entry.music_tags.is_some())
            .count()
            == 2
    });
    let root = fixture.root.join("First");
    let (handle, events) = start_shared(
        root.clone(),
        Arc::new(UnavailableTextExtraction),
        crate::services::music_library::subscribe(fixture.root.clone()),
    );
    handle.query("lantern");
    let items = finish(&events, "lantern").0;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].path, root.join("01.flac"));
    fs::remove_file(root.join("01.flac")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(SearchEvent::Results {
            query,
            items,
            indexing: false,
            ..
        }) = events.recv_timeout(TICK)
            && query == "lantern"
            && items.is_empty()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Deleted hits should disappear from active search"
        );
    }
}

#[test]
fn shared_updates_preserve_content_results_without_repeating_bounded_extractions() {
    struct Extraction(Arc<AtomicUsize>);
    impl TextExtractionProvider for Extraction {
        fn extract_text(
            &self,
            _: &Path,
            _: crate::services::TextExtractor,
            _: usize,
            _: Arc<AtomicBool>,
        ) -> Result<String, String> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok("A cached library document phrase".into())
        }
    }
    use crate::services::music_library::tests::{Fixture as LibraryFixture, ready};
    let fixture = LibraryFixture::new();
    for number in 0..35 {
        fs::write(
            fixture.root.join(format!("{number:02}.pdf")),
            b"Generated document fixture",
        )
        .unwrap();
    }
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| snapshot.entries.len() == 35);
    let calls = Arc::new(AtomicUsize::new(0));
    let (handle, events) = start_shared(
        fixture.root.clone(),
        Arc::new(Extraction(calls.clone())),
        crate::services::music_library::subscribe(fixture.root.clone()),
    );
    handle.query("content:\"library document\" ext:pdf");
    let (items, notice) = finish(&events, "content:\"library document\" ext:pdf");
    assert_eq!(items.len(), MAX_DOCUMENT_EXTRACTIONS_PER_QUERY);
    assert!(notice.is_some());
    fixture.song("Album/01.flac", "Unrelated music update");
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .any(|entry| entry.music_tags.is_some())
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(SearchEvent::Results {
            items,
            indexing: false,
            ..
        }) = events.recv_timeout(TICK)
        {
            assert_eq!(items.len(), MAX_DOCUMENT_EXTRACTIONS_PER_QUERY);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Live content results disappeared during a music update"
        );
    }
    assert_eq!(
        calls.load(Ordering::Relaxed),
        MAX_DOCUMENT_EXTRACTIONS_PER_QUERY
    );
}

fn await_cache(index_cache: &Mutex<VecDeque<CachedIndex>>, root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !index_cache
        .lock()
        .unwrap()
        .iter()
        .any(|cached| cached.root == root)
    {
        assert!(Instant::now() < deadline, "Index was not cached");
        std::thread::sleep(TICK);
    }
}

#[test]
#[ignore = "Reads the desktop user's real Music library through the metadata sandbox; run explicitly"]
fn real_library_search_cold_and_warm_refresh() {
    let root = super::super::SearchScope::MusicLibrary.root(&gio::glib::home_dir());
    let samples = [
        ("Nikes Frank Ocean", "Frank Ocean - Blonde/1. Nikes.flac"),
        (
            "Beach House Space Song",
            "Beach House - Depression Cherry (2015)/3. Beach House - Space Song.flac",
        ),
        ("Clairo Juna", "Clairo - Charm/7. Juna.flac"),
    ];
    for (_, path) in &samples {
        assert!(root.join(path).is_file(), "Missing real fixture {path}");
    }
    let index_cache = Arc::new(Mutex::new(VecDeque::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let executable = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/strata");
    assert!(
        executable.is_file(),
        "Build the production binary before this fixture"
    );
    let tags: Arc<TagReader> = Arc::new(move |path, cancellation| {
        counted.fetch_add(1, Ordering::Relaxed);
        let output = crate::sandbox::parse_one_shot_with_executable(
            &path.canonicalize().ok()?,
            crate::sandbox::ParseOperation::MusicTags,
            0,
            cancellation,
            &executable,
        )
        .ok()?;
        serde_json::from_slice(&output.data).ok()
    });
    let started = Instant::now();
    let (handle, events) = start_with_cache(
        root.clone(),
        Arc::new(UnavailableTextExtraction),
        tags.clone(),
        Limits::default(),
        index_cache.clone(),
    );
    for (query, path) in samples {
        let queried = Instant::now();
        handle.query(query);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(SearchEvent::Results {
                query: current,
                items,
                ..
            }) = events.recv_timeout(TICK)
                && current == query
                && items.iter().any(|item| item.path == root.join(path))
            {
                break;
            }
            assert!(Instant::now() < deadline, "Missing live match {query}");
        }
        eprintln!("Live library match {query}: {:?}", queried.elapsed());
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    while !index_cache
        .lock()
        .unwrap()
        .iter()
        .any(|cached| cached.root == root)
    {
        assert!(
            Instant::now() < deadline,
            "Real metadata scan did not finish"
        );
        std::thread::sleep(TICK);
    }
    eprintln!(
        "Cold library metadata scan: {:?}, {} tag reads",
        started.elapsed(),
        calls.load(Ordering::Relaxed)
    );
    for (query, path) in samples {
        handle.query(query);
        let items = finish(&events, query).0;
        assert!(
            items
                .iter()
                .any(|item| item.path == root.join(path) && item.music_tags.is_some()),
            "Missing tagged result {query}"
        );
    }
    drop(handle);
    calls.store(0, Ordering::Relaxed);
    let started = Instant::now();
    let (handle, events) = start_with_cache(
        root.clone(),
        Arc::new(UnavailableTextExtraction),
        tags,
        Limits::default(),
        index_cache,
    );
    handle.query("Nikes Frank Ocean");
    assert!(
        finish(&events, "Nikes Frank Ocean")
            .0
            .iter()
            .any(|item| item.path == root.join(samples[0].1))
    );
    assert_eq!(
        calls.load(Ordering::Relaxed),
        0,
        "Warm refresh should reuse validated tags"
    );
    eprintln!("Warm library refresh: {:?}", started.elapsed());
}

#[test]
fn reopened_cache_rescans_additions_removals_and_changed_tags() {
    let fixture = Fixture::new();
    fixture.file("unchanged.flac");
    fixture.file("removed.flac");
    fs::write(fixture.0.join("changed.flac"), "old title").unwrap();
    let index_cache = Arc::new(Mutex::new(VecDeque::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = calls.clone();
    let tags: Arc<TagReader> = Arc::new(move |path, _| {
        captured.lock().unwrap().push(path.to_owned());
        Some(MusicTags {
            title: Some(fs::read_to_string(path).unwrap()),
            ..MusicTags::default()
        })
    });
    let (handle, events) = start_with_cache(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        tags.clone(),
        Limits::default(),
        index_cache.clone(),
    );
    handle.query("old title");
    assert_eq!(finish(&events, "old title").0.len(), 1);
    await_cache(&index_cache, &fixture.0);
    drop(handle);
    calls.lock().unwrap().clear();
    fs::remove_file(fixture.0.join("removed.flac")).unwrap();
    fs::rename(
        fixture.0.join("changed.flac"),
        fixture.0.join("changed-old.tmp"),
    )
    .unwrap();
    fs::write(fixture.0.join("changed.flac"), "new title").unwrap();
    fixture.file("added.flac");
    let (handle, events) = start_with_cache(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        tags,
        Limits::default(),
        index_cache,
    );
    handle.query("new title");
    let (items, notice) = finish(&events, "new title");
    assert_eq!(items.len(), 1);
    assert!(notice.is_none(), "{notice:?}");
    assert_eq!(items[0].path, fixture.0.join("changed.flac"));
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert!(
        !calls
            .lock()
            .unwrap()
            .contains(&fixture.0.join("unchanged.flac"))
    );
    handle.query("removed.flac");
    assert!(finish(&events, "removed.flac").0.is_empty());
    handle.query("added.flac");
    assert_eq!(finish(&events, "added.flac").0.len(), 1);
}

#[test]
fn failed_metadata_is_retried_when_reopening_cached_scope() {
    let fixture = Fixture::new();
    fixture.file("01.flac");
    let index_cache = Arc::new(Mutex::new(VecDeque::new()));
    let (handle, events) = start_with_cache(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        Arc::new(|_, _| None),
        Limits::default(),
        index_cache.clone(),
    );
    handle.query("secret sunrise");
    assert!(finish(&events, "secret sunrise").1.is_some());
    await_cache(&index_cache, &fixture.0);
    drop(handle);
    let (handle, events) = start_with_cache(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        reader(),
        Limits::default(),
        index_cache,
    );
    handle.query("secret sunrise");
    let (items, notice) = finish(&events, "secret sunrise");
    assert_eq!(items.len(), 1);
    assert!(notice.is_none());
}

#[test]
fn same_size_in_place_tag_edit_invalidates_warm_metadata() {
    let fixture = Fixture::new();
    let path = fixture.0.join("needle.flac");
    fs::write(&path, "old title").unwrap();
    let index_cache = Arc::new(Mutex::new(VecDeque::new()));
    let tags: Arc<TagReader> = Arc::new(|path, _| {
        Some(MusicTags {
            title: Some(fs::read_to_string(path).unwrap()),
            ..MusicTags::default()
        })
    });
    let (handle, events) = start_with_cache(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        tags.clone(),
        Limits::default(),
        index_cache.clone(),
    );
    handle.query("old title");
    assert_eq!(finish(&events, "old title").0.len(), 1);
    await_cache(&index_cache, &fixture.0);
    drop(handle);
    let before = fs::metadata(&path).unwrap();
    std::thread::sleep(Duration::from_millis(2));
    fs::write(&path, "new title").unwrap();
    let after = fs::metadata(&path).unwrap();
    assert_eq!(before.ino(), after.ino());
    assert_eq!(before.len(), after.len());
    assert_ne!(
        FileIdentity::from_metadata(&before),
        FileIdentity::from_metadata(&after)
    );
    let (handle, events) = start_with_cache(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        tags,
        Limits::default(),
        index_cache,
    );
    handle.query("new title");
    assert_eq!(finish(&events, "new title").0.len(), 1);
    handle.query("old title");
    assert!(finish(&events, "old title").0.is_empty());
}

#[test]
fn source_changed_during_tag_read_does_not_publish_stale_metadata() {
    let fixture = Fixture::new();
    fixture.file("needle.flac");
    let (handle, events) = start(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        Arc::new(|path, _| {
            fs::write(path, "changed while parsing").unwrap();
            Some(MusicTags {
                title: Some("stale secret".into()),
                ..MusicTags::default()
            })
        }),
    );
    handle.query("stale secret");
    let (items, notice) = finish(&events, "stale secret");
    assert!(items.is_empty());
    assert!(notice.unwrap().contains("changed during indexing"));
    handle.query("needle");
    let items = finish(&events, "needle").0;
    assert_eq!(items.len(), 1);
    assert!(items[0].music_tags.is_none());
}

#[test]
fn reverse_artist_title_terms_match_paths_without_waiting_for_tags() {
    let fixture = Fixture::new();
    fixture.file("Frank Ocean - Blonde/1. Nikes.flac");
    let tags: Arc<TagReader> = Arc::new(|_, cancellation| {
        while !cancellation.is_cancelled() {
            std::thread::sleep(TICK);
        }
        None
    });
    let (handle, events) = start(fixture.0.clone(), Arc::new(UnavailableTextExtraction), tags);
    handle.query("Nikes Frank Ocean");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(SearchEvent::Results {
            query,
            items,
            indexing,
            ..
        }) = events.recv_timeout(TICK)
            && query == "Nikes Frank Ocean"
            && items.iter().any(|item| item.name == "1. Nikes.flac")
        {
            assert!(indexing);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Path matches should not await metadata"
        );
    }
}

#[test]
fn song_tags_match_across_albums_and_home_includes_music() {
    let fixture = Fixture::new();
    fixture.file("Music/First/01.flac");
    fixture.file("Music/Second/02.mp3");
    let (handle, events) = start(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        reader(),
    );
    handle.query("cross album artist");
    let (items, notice) = finish(&events, "cross album artist");
    assert_eq!(items.len(), 2);
    assert!(notice.is_none(), "{notice:?}");
    assert!(
        items
            .iter()
            .all(|item| item.path.starts_with(fixture.0.join("Music")))
    );
    for (query, count) in [
        ("secret sunrise", 1),
        ("compilation collective", 2),
        ("invisible album tag", 2),
        ("sunrise cross album", 1),
    ] {
        handle.query(query);
        assert_eq!(finish(&events, query).0.len(), count, "{query}");
    }
}

#[test]
fn inaccessible_root_reports_error_instead_of_silent_empty_results() {
    let fixture = Fixture::new();
    let missing = fixture.0.join("missing-library");
    let (handle, events) = start(
        missing.clone(),
        Arc::new(UnavailableTextExtraction),
        reader(),
    );
    handle.query("song");
    let (items, notice) = finish(&events, "song");
    assert!(items.is_empty());
    assert!(
        notice
            .unwrap()
            .contains(&missing.to_string_lossy().to_string())
    );
}

#[test]
fn metadata_failure_keeps_filename_matches_and_reports_partial_search() {
    let fixture = Fixture::new();
    fixture.file("broken-song.flac");
    let (handle, events) = start(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        Arc::new(|_, _| None),
    );
    handle.query("broken-song");
    let (items, notice) = finish(&events, "broken-song");
    assert_eq!(items.len(), 1);
    assert!(notice.unwrap().contains("tags could not be read"));
}

#[test]
fn updates_continue_while_metadata_is_blocked_and_drop_cancels_scan() {
    let fixture = Fixture::new();
    fixture.file("First/needle.flac");
    fixture.file("Second/later.mp3");
    let entered = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let read_entered = entered.clone();
    let read_stopped = stopped.clone();
    let read_calls = calls.clone();
    let tags: Arc<TagReader> = Arc::new(move |_, cancellation| {
        read_calls.fetch_add(1, Ordering::Relaxed);
        read_entered.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !cancellation.is_cancelled() && Instant::now() < deadline {
            std::thread::sleep(TICK);
        }
        read_stopped.store(cancellation.is_cancelled(), Ordering::Release);
        None
    });
    let (handle, events) = start(fixture.0.clone(), Arc::new(UnavailableTextExtraction), tags);
    handle.query("needle");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut found = false;
    while Instant::now() < deadline {
        if let Ok(SearchEvent::Results {
            query,
            items,
            indexing,
            ..
        }) = events.recv_timeout(TICK)
        {
            if query == "needle" && indexing && items.iter().any(|item| item.name == "needle.flac")
            {
                found = true;
                break;
            }
        }
    }
    assert!(found, "Filename results must not wait for song tags");
    handle.query("later");
    let mut updated = false;
    while Instant::now() < deadline {
        if let Ok(SearchEvent::Results {
            query,
            items,
            indexing,
            ..
        }) = events.recv_timeout(TICK)
        {
            if query == "later" && indexing && items.iter().any(|item| item.name == "later.mp3") {
                updated = true;
                break;
            }
        }
    }
    assert!(updated);
    while !entered.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(TICK);
    }
    let token = handle.scan_cancelled.clone();
    drop(handle);
    assert!(token.is_cancelled());
    while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(TICK);
    }
    assert!(stopped.load(Ordering::Acquire));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn mailboxes_keep_only_latest_and_cancel_superseded_query_tokens() {
    let commands = Arc::new(Mailbox {
        value: Mutex::new(None),
        ready: Condvar::new(),
    });
    let events = Arc::new(Mailbox {
        value: Mutex::new(None),
        ready: Condvar::new(),
    });
    let handle = SearchHandle {
        commands: commands.clone(),
        events: events.clone(),
        startup_error: None,
        live: false,
        scan_cancelled: Cancellation::default(),
        query_cancelled: Mutex::new(Arc::new(AtomicBool::new(false))),
    };
    handle.query("obsolete");
    let old = commands.take().unwrap();
    for position in 0..10_000 {
        handle.query(&format!("query-{position}"));
        events.put(SearchEvent::Results {
            query: position.to_string(),
            items: Vec::new(),
            indexing: true,
            notice: None,
        });
    }
    assert!(old.cancelled.load(Ordering::Acquire));
    assert_eq!(commands.take().unwrap().text, "query-9999");
    assert!(commands.take().is_none());
    let SearchEvent::Results { query, .. } = events.take().unwrap();
    assert_eq!(query, "9999");
    assert!(events.take().is_none());
}

struct CancellableExtraction(Arc<AtomicBool>);

impl TextExtractionProvider for CancellableExtraction {
    fn extract_text(
        &self,
        _: &Path,
        _: crate::services::TextExtractor,
        _: usize,
        cancelled: Arc<AtomicBool>,
    ) -> Result<String, String> {
        self.0.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(TICK);
        }
        assert!(
            cancelled.load(Ordering::Acquire),
            "Superseding the query must cancel extraction"
        );
        Err("Cancelled".into())
    }
}

#[test]
fn new_query_cancels_stale_document_work_and_never_republishes_it() {
    let fixture = Fixture::new();
    fixture.file("slow.pdf");
    fixture.file("needle.txt");
    let entered = Arc::new(AtomicBool::new(false));
    let (handle, events) = start(
        fixture.0.clone(),
        Arc::new(CancellableExtraction(entered.clone())),
        reader(),
    );
    handle.query("content:obsolete");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !entered.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(TICK);
    }
    assert!(entered.load(Ordering::Acquire));
    handle.query("needle");
    let (items, notice) = finish(&events, "needle");
    assert_eq!(items.len(), 1);
    assert!(notice.is_none());
    std::thread::sleep(Duration::from_millis(80));
    if let Some(SearchEvent::Results { query, .. }) = events.try_recv() {
        assert_eq!(query, "needle");
    }
}

#[test]
fn returned_results_are_bounded_and_ties_have_stable_path_order() {
    let fixture = Fixture::new();
    for number in 0..150 {
        fixture.file(&format!("{number:03}/needle.txt"));
    }
    let (handle, events) = start(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        reader(),
    );
    handle.query("needle");
    let (items, notice) = finish(&events, "needle");
    assert_eq!(items.len(), RESULT_LIMIT);
    assert!(items.windows(2).all(|pair| pair[0].path < pair[1].path));
    assert!(notice.unwrap().contains("best 100"));
}

#[test]
fn entry_byte_and_metadata_caps_report_partial_indexes() {
    for limits in [
        Limits {
            entries: 2,
            ..Limits::default()
        },
        Limits {
            bytes: 100,
            ..Limits::default()
        },
        Limits {
            metadata: 1,
            ..Limits::default()
        },
    ] {
        let fixture = Fixture::new();
        for number in 0..4 {
            fixture.file(&format!("song-{number}.flac"));
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let tags = reader();
        let (handle, events) = start_with_limits(
            fixture.0.clone(),
            Arc::new(UnavailableTextExtraction),
            Arc::new(move |path, cancellation| {
                counted.fetch_add(1, Ordering::Relaxed);
                tags(path, cancellation)
            }),
            limits,
        );
        handle.query("song");
        let (items, notice) = finish(&events, "song");
        if limits.entries == 2 {
            assert_eq!(items.len(), 2);
        }
        if limits.bytes == 100 {
            assert!(items.is_empty());
        }
        if limits.metadata == 1 {
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        }
        assert!(notice.unwrap().contains("limited to"));
    }
}

#[test]
fn home_indexes_music_first_even_when_other_folders_exhaust_the_cap() {
    let fixture = Fixture::new();
    for number in 0..30 {
        fixture.file(&format!("Documents/noise-{number}.txt"));
    }
    fixture.file("Music/First/01.flac");
    fixture.file("Music/Second/02.mp3");
    let (handle, events) = start_with_limits(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        reader(),
        Limits {
            entries: 6,
            ..Limits::default()
        },
    );
    handle.query("cross album artist");
    let (items, notice) = finish(&events, "cross album artist");
    assert_eq!(items.len(), 2);
    assert!(notice.unwrap().contains("limited to"));
}

#[test]
fn hidden_folders_and_symlink_cycles_are_not_traversed_but_ignores_do_not_hide_songs() {
    let fixture = Fixture::new();
    fixture.file(".private/secret-song.flac");
    fixture.file("Visible/needle.flac");
    fs::write(fixture.0.join(".ignore"), "Visible/\n").unwrap();
    std::os::unix::fs::symlink(&fixture.0, fixture.0.join("loop")).unwrap();
    let (handle, events) = start(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        reader(),
    );
    handle.query("needle");
    assert_eq!(finish(&events, "needle").0.len(), 1);
    handle.query("secret-song");
    assert!(finish(&events, "secret-song").0.is_empty());
}

#[test]
fn cached_index_reopens_without_reading_song_tags_again_and_checks_missing_roots() {
    let fixture = Fixture::new();
    fixture.file("needle.flac");
    let index_cache = Arc::new(Mutex::new(VecDeque::new()));
    let (handle, events) = start_with_cache(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        reader(),
        Limits::default(),
        index_cache.clone(),
    );
    handle.query("sunrise");
    finish(&events, "sunrise");
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if index_cache
            .lock()
            .unwrap()
            .iter()
            .any(|cached| cached.root == fixture.0)
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(TICK);
    }
    drop(handle);
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let (handle, events) = start_with_cache(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        Arc::new(move |_, _| {
            counted.fetch_add(1, Ordering::Relaxed);
            None
        }),
        Limits::default(),
        index_cache.clone(),
    );
    handle.query("cross album artist");
    assert_eq!(finish(&events, "cross album artist").0.len(), 1);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    drop(handle);
    fs::remove_dir_all(&fixture.0).unwrap();
    let (handle, events) = start_with_cache(
        fixture.0.clone(),
        Arc::new(UnavailableTextExtraction),
        reader(),
        Limits::default(),
        index_cache,
    );
    handle.query("needle");
    let (items, notice) = finish(&events, "needle");
    assert!(items.is_empty());
    assert!(notice.unwrap().contains("Cannot search"));
}
