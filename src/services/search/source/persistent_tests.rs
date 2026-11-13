use super::*;
use crate::services::search::{SearchEvent, UnavailableTextExtraction};
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hermes-persistent-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("fixture root");
        Self { root }
    }
    fn entry(&self, name: &str) -> Entry {
        Entry {
            path: self.root.join(name),
            is_directory: false,
            modified_unix_seconds: None,
            identity: None,
            music_tags: None,
        }
    }
    fn cache(&self) -> PathBuf {
        self.root.join("cache/source.sqlite")
    }
    fn persistent(&self, entries: Arc<Vec<Entry>>) -> Source {
        Source::persistent_at(
            self.root.clone(),
            entries,
            usize::MAX,
            usize::MAX,
            &Cancellation::default(),
            &self.cache(),
        )
        .expect("persistent query source")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn sqlite_source_releases_input_entries_and_preserves_scoped_lookup_and_pages() {
    let fixture = Fixture::new();
    let entries: Arc<Vec<Entry>> = Arc::new(
        (0..513)
            .map(|number| {
                if number % 2 == 0 {
                    fixture.entry(&format!("FILE-{number:04}.TXT"))
                } else {
                    Entry {
                        path: fixture.root.with_extension("outside").join("excluded"),
                        ..fixture.entry("unused")
                    }
                }
            })
            .collect(),
    );
    let memory = Source::new(
        fixture.root.clone(),
        entries.clone(),
        usize::MAX,
        usize::MAX,
        &|| false,
    )
    .expect("memory reference");
    let source = fixture.persistent(entries.clone());
    assert!(matches!(source.backing, Backing::Sqlite(_)));
    let weak = Arc::downgrade(&entries);
    assert_eq!(Arc::strong_count(&entries), 2);
    let mut observed = Vec::new();
    let mut sizes = Vec::new();
    source
        .for_each_page(&|| false, |page| {
            sizes.push(page.len());
            for (position, item) in page {
                assert_eq!(Some(item.clone()), memory.get(*position)?);
                // Reentrant lookup must not retain the page reader's mutex.
                assert_eq!(Some(item.clone()), source.get(*position)?);
                observed.push(*position);
            }
            Ok(())
        })
        .expect("SQLite pages");
    assert_eq!(sizes, [256, 1]);
    assert_eq!(observed, (0..513).step_by(2).collect::<Vec<_>>());
    assert!(source.get(1).expect("scope hole").is_none());
    drop(memory);
    drop(entries);
    assert!(weak.upgrade().is_none());
    assert!(source.get(512).expect("independent pinned data").is_some());
}

#[test]
fn sqlite_page_visitor_error_and_cancellation_allow_a_fresh_query() {
    let fixture = Fixture::new();
    let source = fixture.persistent(Arc::new(
        (0..600)
            .map(|number| fixture.entry(&format!("item-{number}.txt")))
            .collect(),
    ));
    let cancelled = std::cell::Cell::new(false);
    let mut visits = 0;
    let result = source.for_each_page(&|| cancelled.get(), |_| {
        visits += 1;
        cancelled.set(true);
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(visits, 1);
    assert!(
        source
            .for_each_page(&|| false, |_| Err("generated visitor failure".into()))
            .is_err()
    );
    let mut total = 0;
    source
        .for_each_page(&|| false, |page| {
            total += page.len();
            Ok(())
        })
        .expect("new query reuses valid source");
    assert_eq!(total, 600);
}

#[test]
fn projected_scoring_preserves_full_scores_and_restores_complete_result_metadata() {
    let fixture = Fixture::new();
    let entries = ["Café.flac", "Track-2024.flac", "Other.txt"]
        .into_iter()
        .map(|name| Entry {
            modified_unix_seconds: Some(123),
            music_tags: Some(crate::model::MusicTags {
                title: Some("Björk Song".into()),
                artist: Some("Artist".into()),
                album: Some("Album".into()),
                disc: Some(2),
                track: Some(1),
                ..crate::model::MusicTags::default()
            }),
            ..fixture.entry(name)
        })
        .collect();
    let source = fixture.persistent(Arc::new(entries));
    for text in [
        "cafe",
        "Björk",
        "Track-2024",
        "type:audio song",
        "artist:\"Artist\"",
        "songs by Artist",
        "\"Björk Song\"",
    ] {
        let query = super::super::ParsedSearchQuery::parse(&text.to_lowercase());
        let mut expected = Vec::new();
        source
            .for_each_page(&|| false, |page| {
                for (position, item) in page {
                    if let Some(score) = super::super::fuzzy_score(item, text, &fixture.root) {
                        expected.push((*position, score));
                    }
                }
                Ok(())
            })
            .expect("full score reference");
        let mut observed = Vec::new();
        source
            .for_scoring_page(&query, false, &|| false, |page| {
                for (position, item) in page {
                    if query.can_project_score() {
                        assert!(item.music_tags.is_none());
                    }
                    if let Some(score) = super::super::fuzzy_score(item, text, &fixture.root) {
                        observed.push((*position, score));
                    }
                }
                Ok(())
            })
            .expect("projected score pages");
        assert_eq!(observed, expected, "{text}");
    }
    let query = super::super::ParsedSearchQuery::parse("song");
    source
        .for_scoring_page(&query, true, &|| false, |page| {
            assert!(page.iter().all(|(_, item)| item.music_tags.is_some()));
            Ok(())
        })
        .expect("entity filtering needs full metadata");
    let complete = source
        .get(0)
        .expect("materialized result")
        .expect("stored song");
    assert_eq!(complete.modified_unix_seconds, Some(123));
    assert_eq!(complete.music_tags.expect("complete tags").disc, Some(2));
}

#[test]
fn index_tree_publishes_results_using_the_persistent_source_cache() {
    let fixture = Fixture::new();
    for number in 0..40 {
        fs::write(
            fixture.root.join(format!("needle-{number:03}.txt")),
            b"generated source",
        )
        .expect("fixture file");
    }
    let (handle, receiver) = crate::services::search::index_tree(
        fixture.root.clone(),
        Arc::new(UnavailableTextExtraction),
    );
    let generation = handle.query_all("needle");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(SearchEvent::Results {
            generation: current,
            indexing: false,
            items,
            notice,
            ..
        }) = receiver.try_recv()
            && current == generation
        {
            assert!(notice.is_none(), "{notice:?}");
            assert_eq!(items.len(), 40);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "normal search did not publish a complete snapshot"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let cache = persistent_path(&fixture.root).expect("profile cache path");
    assert!(cache.is_file());
    let connection =
        rusqlite::Connection::open_with_flags(cache, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("inspect committed query cache");
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("query schema version");
    assert!(version > 0);
}
