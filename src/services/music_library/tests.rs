use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub(crate) struct Fixture {
    pub root: PathBuf,
    pub cache_path: PathBuf,
    pub calls: Arc<AtomicUsize>,
    parent: PathBuf,
    instances: Mutex<Vec<Weak<Library>>>,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let parent = std::env::temp_dir().join(format!(
            "hermes-shared-library-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = parent.join("Music");
        fs::create_dir_all(&root).unwrap();
        Self {
            cache_path: parent.join("cache/tags.json"),
            root,
            parent,
            calls: Arc::new(AtomicUsize::new(0)),
            instances: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn song(&self, name: &str, title: &str) {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let tags = MusicTags {
            title: Some(title.into()),
            artist: Some("Hermes fixture artist".into()),
            album: Some("Fixture album".into()),
            track: Some(1),
            ..MusicTags::default()
        };
        fs::write(path, serde_json::to_vec(&tags).unwrap()).unwrap();
    }

    fn configuration(&self) -> Configuration {
        let mut configuration = Configuration::new(self.root.clone());
        configuration.cache_path = self.cache_path.clone();
        configuration.debounce = Duration::from_millis(20);
        let calls = self.calls.clone();
        configuration.tags = Arc::new(move |path, _| {
            calls.fetch_add(1, Ordering::Relaxed);
            serde_json::from_slice(&fs::read(path).ok()?).ok()
        });
        configuration
    }

    pub(crate) fn subscribe(&self) -> Subscription {
        self.start(self.configuration())
    }

    fn start(&self, configuration: Configuration) -> Subscription {
        let subscription = subscribe_with(configuration);
        self.instances
            .lock()
            .unwrap()
            .push(Arc::downgrade(&subscription.library));
        subscription
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

pub(crate) fn ready(
    subscription: &mut Subscription,
    predicate: impl Fn(&Snapshot) -> bool,
) -> Arc<Snapshot> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = None;
    loop {
        if let Some(snapshot) = subscription.poll() {
            if !snapshot.indexing && predicate(&snapshot) {
                return snapshot;
            }
            last = Some(snapshot);
        }
        assert!(
            Instant::now() < deadline,
            "Library did not settle: {:?}",
            last.as_ref().map(|snapshot| (
                &snapshot.notice,
                snapshot.indexing,
                snapshot.entries.len()
            ))
        );
        std::thread::sleep(TICK);
    }
}

fn stop(subscription: Subscription) {
    let weak = Arc::downgrade(&subscription.library);
    drop(subscription);
    let deadline = Instant::now() + Duration::from_secs(2);
    while weak.strong_count() > 0 {
        assert!(Instant::now() < deadline, "Library worker did not stop");
        std::thread::sleep(TICK);
    }
}

#[test]
fn subscribers_share_decoding_and_do_not_steal_each_others_updates() {
    let fixture = Fixture::new();
    fixture.song("Album/01.flac", "Shared sunrise");
    let mut first = fixture.subscribe();
    let mut second = subscribe(fixture.root.clone());
    assert!(Arc::ptr_eq(&first.library, &second.library));
    let snapshot = ready(&mut first, |snapshot| {
        snapshot
            .entries
            .iter()
            .any(|entry| entry.music_tags.is_some())
    });
    let other = ready(&mut second, |snapshot| {
        snapshot
            .entries
            .iter()
            .any(|entry| entry.music_tags.is_some())
    });
    assert_eq!(snapshot.entries, other.entries);
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
    drop(first);
    fixture.song("Album/02.flac", "Second sunrise");
    ready(&mut second, |snapshot| {
        snapshot
            .entries
            .iter()
            .filter(|entry| entry.music_tags.is_some())
            .count()
            == 2
    });
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 2);
}

#[test]
fn persistent_tags_survive_restart_but_same_size_edits_are_revalidated() {
    let fixture = Fixture::new();
    fixture.song("01.flac", "Old song");
    fixture.song("removed.flac", "Removed song");
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .filter(|entry| entry.music_tags.is_some())
            .count()
            == 2
    });
    assert!(fixture.cache_path.is_file());
    stop(subscription);
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 2);
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .filter(|entry| entry.music_tags.is_some())
            .count()
            == 2
    });
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 2);
    stop(subscription);
    std::thread::sleep(Duration::from_millis(2));
    fixture.song("01.flac", "New song");
    fs::remove_file(fixture.root.join("removed.flac")).unwrap();
    let mut subscription = fixture.subscribe();
    let snapshot = ready(&mut subscription, |snapshot| {
        snapshot.entries.len() == 1
            && snapshot.entries[0]
                .music_tags
                .as_ref()
                .and_then(|tags| tags.title.as_deref())
                == Some("New song")
    });
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 3);
    assert_eq!(snapshot.entries[0].path, fixture.root.join("01.flac"));
}

#[test]
fn monitors_follow_new_folders_renames_atomic_tag_replacements_and_deletions() {
    let fixture = Fixture::new();
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| snapshot.entries.is_empty());
    fixture.song("First/01.flac", "First song");
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .any(|entry| entry.music_tags.is_some())
    });
    fs::rename(fixture.root.join("First"), fixture.root.join("Renamed")).unwrap();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .any(|entry| entry.path == fixture.root.join("Renamed/01.flac"))
            && snapshot
                .entries
                .iter()
                .all(|entry| !entry.path.starts_with(fixture.root.join("First")))
    });
    fixture.song("replacement.flac", "Replaced song");
    fs::rename(
        fixture.root.join("replacement.flac"),
        fixture.root.join("Renamed/01.flac"),
    )
    .unwrap();
    ready(&mut subscription, |snapshot| {
        snapshot.entries.iter().any(|entry| {
            entry
                .music_tags
                .as_ref()
                .and_then(|tags| tags.title.as_deref())
                == Some("Replaced song")
        })
    });
    fs::remove_dir_all(fixture.root.join("Renamed")).unwrap();
    ready(&mut subscription, |snapshot| snapshot.entries.is_empty());
}

#[test]
fn missing_root_recovers_after_deletion_and_recreation_without_manual_refresh() {
    let fixture = Fixture::new();
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| snapshot.entries.is_empty());
    fs::remove_dir(&fixture.root).unwrap();
    ready(&mut subscription, |snapshot| {
        snapshot
            .notice
            .as_ref()
            .is_some_and(|notice| notice.contains("unavailable"))
    });
    fixture.song("Recovered/01.flac", "Recovered song");
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .any(|entry| entry.music_tags.is_some())
            && snapshot.notice.is_none()
    });
}

#[test]
fn hidden_files_symlinks_and_invalid_cached_paths_are_excluded() {
    let fixture = Fixture::new();
    fixture.song(".hidden/01.flac", "Private fixture");
    fixture.song("Visible/01.flac", "Visible fixture");
    std::os::unix::fs::symlink(&fixture.root, fixture.root.join("loop")).unwrap();
    std::os::unix::fs::symlink(
        fixture.root.join("Visible/01.flac"),
        fixture.root.join("linked.flac"),
    )
    .unwrap();
    let mut subscription = fixture.subscribe();
    let snapshot = ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .any(|entry| entry.music_tags.is_some())
    });
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
    stop(subscription);
    let mut cached: Vec<cache::CachedTags> =
        serde_json::from_slice(&fs::read(&fixture.cache_path).unwrap()).unwrap();
    cached[0].identity.path = crate::model::Location::local(fixture.root.join("../outside.flac"));
    fs::write(&fixture.cache_path, serde_json::to_vec(&cached).unwrap()).unwrap();
    assert!(cache::read(&fixture.cache_path, &fixture.root).is_empty());
    fs::write(&fixture.cache_path, "not JSON").unwrap();
    assert!(cache::read(&fixture.cache_path, &fixture.root).is_empty());
}

#[test]
fn valid_untagged_files_are_cached_instead_of_parsed_on_every_refresh() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("untagged.flac"),
        serde_json::to_vec(&MusicTags::default()).unwrap(),
    )
    .unwrap();
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .first()
            .is_some_and(|entry| entry.music_tags == Some(MusicTags::default()))
    });
    subscription.refresh();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .first()
            .is_some_and(|entry| entry.music_tags.is_some())
    });
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
    stop(subscription);
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .first()
            .is_some_and(|entry| entry.music_tags.is_some())
    });
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn failed_metadata_remains_searchable_and_is_retried_after_refresh() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("broken.flac"), "invalid metadata fixture").unwrap();
    let mut subscription = fixture.subscribe();
    let snapshot = ready(&mut subscription, |snapshot| {
        snapshot.entries.len() == 1 && snapshot.notice.is_some()
    });
    assert!(snapshot.entries[0].music_tags.is_none());
    fixture.song("broken.flac", "Recovered tags");
    subscription.refresh();
    ready(&mut subscription, |snapshot| {
        snapshot.entries[0].music_tags.is_some() && snapshot.notice.is_none()
    });
}

#[test]
fn limited_monitors_fall_back_to_periodic_verification() {
    let fixture = Fixture::new();
    let mut configuration = fixture.configuration();
    configuration.monitor_limit = 0;
    configuration.refresh_interval = Duration::from_millis(100);
    let mut subscription = fixture.start(configuration);
    let snapshot = ready(&mut subscription, |snapshot| snapshot.entries.is_empty());
    assert!(
        snapshot
            .notice
            .as_ref()
            .unwrap()
            .contains("cannot be watched")
    );
    fixture.song("Added/01.flac", "Fallback discovery");
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .any(|entry| entry.music_tags.is_some())
    });
}

#[test]
fn index_entry_track_and_byte_limits_report_partial_results() {
    for limit in 0..3 {
        let fixture = Fixture::new();
        fixture.song("01.flac", "First");
        fixture.song("02.flac", "Second");
        let mut configuration = fixture.configuration();
        match limit {
            0 => configuration.entries = 1,
            1 => configuration.tracks = 1,
            _ => configuration.bytes = 1,
        }
        let mut subscription = fixture.start(configuration);
        let snapshot = ready(&mut subscription, |snapshot| snapshot.notice.is_some());
        if limit == 1 {
            assert_eq!(snapshot.entries.len(), 2);
            assert!(snapshot.entries.iter().filter(|entry| entry.music_tags.is_some()).count() <= 1);
        } else {
            assert!(snapshot.entries.len() <= 1);
        }
        assert!(snapshot.notice.as_ref().unwrap().contains("limit"));
    }
}

#[test]
fn only_the_last_subscriber_cancels_in_flight_metadata() {
    let fixture = Fixture::new();
    fixture.song("01.flac", "Blocked song");
    let entered = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    let read_entered = entered.clone();
    let read_cancelled = cancelled.clone();
    let mut configuration = fixture.configuration();
    configuration.tags = Arc::new(move |_, token| {
        read_entered.store(true, Ordering::Release);
        while !token.is_cancelled() {
            std::thread::sleep(TICK);
        }
        read_cancelled.store(true, Ordering::Release);
        None
    });
    let first = fixture.start(configuration);
    let second = subscribe(fixture.root.clone());
    let deadline = Instant::now() + Duration::from_secs(2);
    while !entered.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(TICK);
    }
    drop(first);
    assert!(!second.library.cancellation.is_cancelled());
    stop(second);
    assert!(cancelled.load(Ordering::Acquire));
}

#[test]
fn source_change_during_parse_is_retried_without_persisting_stale_tags() {
    let fixture = Fixture::new();
    fixture.song("01.flac", "Original song");
    let calls = fixture.calls.clone();
    let mut configuration = fixture.configuration();
    configuration.tags = Arc::new(move |path, _| {
        if calls.fetch_add(1, Ordering::Relaxed) == 0 {
            let tags = MusicTags {
                title: Some("Updated song".into()),
                ..MusicTags::default()
            };
            fs::write(path, serde_json::to_vec(&tags).unwrap()).unwrap();
            return Some(MusicTags {
                title: Some("Stale song".into()),
                ..MusicTags::default()
            });
        }
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    });
    let mut subscription = fixture.start(configuration);
    let snapshot = ready(&mut subscription, |snapshot| {
        snapshot.entries.first().is_some_and(|entry| {
            entry
                .music_tags
                .as_ref()
                .and_then(|tags| tags.title.as_deref())
                == Some("Updated song")
        })
    });
    assert_eq!(
        snapshot.entries[0]
            .music_tags
            .as_ref()
            .unwrap()
            .title
            .as_deref(),
        Some("Updated song")
    );
    let cache = cache::read(&fixture.cache_path, &fixture.root);
    assert_eq!(
        cache[&fixture.root.join("01.flac")].tags.title.as_deref(),
        Some("Updated song")
    );
}

#[test]
fn file_activation_reuses_shared_tags_but_rejects_changed_sources() {
    let fixture = Fixture::new();
    fixture.song("01.flac", "Shared playback title");
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .first()
            .is_some_and(|entry| entry.music_tags.is_some())
    });
    let tags = crate::adapters::music_metadata::read_tags(
        &fixture.root.join("01.flac"),
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(tags.title.as_deref(), Some("Shared playback title"));
    fixture.song("01.flac", "Changed playback title");
    assert!(cached_tags(&fixture.root.join("01.flac")).is_none());
}

#[test]
fn cache_preserves_native_non_utf8_paths_and_legacy_identity_shape() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let path = fixture
        .root
        .join(std::ffi::OsString::from_vec(b"song-\xff.flac".to_vec()));
    fs::write(&path, serde_json::to_vec(&MusicTags::default()).unwrap()).unwrap();
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .first()
            .is_some_and(|entry| entry.music_tags.is_some())
    });
    stop(subscription);
    let data = fs::read(&fixture.cache_path).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&data).unwrap();
    assert!(json[0]["identity"]["device"].is_number());
    assert!(json[0]["identity"]["path"]["value"].is_array());
    let cached = cache::read(&fixture.cache_path, &fixture.root);
    assert_eq!(cached[&path].tags, MusicTags::default());
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .first()
            .is_some_and(|entry| entry.path == path && entry.music_tags.is_some())
    });
    assert_eq!(fixture.calls.load(Ordering::Relaxed), 1);
}

#[test]
#[ignore = "Read-only real Music library; requires the built production sandbox helper"]
fn real_shared_library_cold_search_music_and_persistent_warm_restart() {
    let fixture = Fixture::new();
    let root = crate::services::SearchScope::MusicLibrary.root(&gio::glib::home_dir());
    let samples = [
        ("Nikes Frank Ocean", "Frank Ocean - Blonde/1. Nikes.flac"),
        (
            "Beach House Space Song",
            "Beach House - Depression Cherry (2015)/3. Beach House - Space Song.flac",
        ),
        ("Clairo Juna", "Clairo - Charm/7. Juna.flac"),
    ];
    for (_, path) in samples {
        assert!(root.join(path).is_file());
    }
    let executable = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/strata");
    assert!(executable.is_file());
    let calls = fixture.calls.clone();
    let tags: Arc<TagReader> = Arc::new(move |path, cancellation| {
        calls.fetch_add(1, Ordering::Relaxed);
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
    let mut configuration = fixture.configuration();
    configuration.root = root.clone();
    configuration.tags = tags.clone();
    let started = Instant::now();
    let mut subscription = fixture.start(configuration);
    let mut music = crate::music::library::Scan::start(root.clone());
    let (search, events) = crate::services::index_tree(
        root.clone(),
        Arc::new(crate::adapters::LocalTextExtractionProvider),
    );
    assert!(search.is_live());
    for (query, path) in samples {
        let queried = Instant::now();
        search.query(query);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(crate::services::SearchEvent::Results {
                query: current,
                items,
                ..
            }) = events.recv_timeout(TICK)
                && current == query
                && items.iter().any(|item| item.path == root.join(path))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Missing live shared match {query}"
            );
        }
        eprintln!("Shared live search {query}: {:?}", queried.elapsed());
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    let snapshot = loop {
        if let Some(snapshot) = subscription.poll()
            && !snapshot.indexing
        {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "Cold shared indexing did not complete"
        );
        std::thread::sleep(TICK);
    };
    for (_, path) in samples {
        assert!(
            snapshot
                .entries
                .iter()
                .any(|entry| entry.path == root.join(path) && entry.music_tags.is_some())
        );
    }
    let audio = snapshot
        .entries
        .iter()
        .filter(|entry| entry.is_audio())
        .count();
    assert_eq!(
        fixture.calls.load(Ordering::Relaxed),
        audio,
        "Search and Music must decode each tag set just once"
    );
    let tracks = music.poll().unwrap();
    assert_eq!(tracks.tracks.len(), audio);
    eprintln!(
        "Shared cold metadata: {:?}, {audio} tag reads",
        started.elapsed()
    );
    drop(music);
    drop(search);
    stop(subscription);
    fixture.calls.store(0, Ordering::Relaxed);
    let mut configuration = fixture.configuration();
    configuration.root = root;
    configuration.tags = tags;
    let started = Instant::now();
    let mut subscription = fixture.start(configuration);
    let snapshot = ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .filter(|entry| entry.music_tags.is_some())
            .count()
            == audio
    });
    assert!(snapshot.notice.is_none(), "{:?}", snapshot.notice);
    assert_eq!(
        fixture.calls.load(Ordering::Relaxed),
        0,
        "Restart should use identity-validated persistent tags"
    );
    eprintln!(
        "Persistent warm shared restart: {:?}, zero tag reads",
        started.elapsed()
    );
}
