use super::*;
use crate::services::search::UnavailableTextExtraction;

struct Fixture(PathBuf);

impl Fixture {
    fn new(count: usize) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("generated fixture must provide the expected value")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "hermes-search-pages-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).expect("generated fixture must provide the expected value");
        for number in 0..count {
            std::fs::write(root.join(format!("match-{number:04}.txt")), b"Synthetic")
                .expect("generated fixture must provide the expected value");
        }
        Self(root)
    }

    fn search(&self) -> (SearchHandle, SearchReceiver) {
        index_tree(self.0.clone(), Arc::new(UnavailableTextExtraction))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = std::fs::remove_dir_all(&self.0);
    }
}

fn page(
    receiver: &SearchReceiver,
    generation: u64,
    shown: usize,
    total: usize,
) -> (Vec<SearchItem>, SearchPage) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if let Ok(SearchEvent::Results {
            generation: current,
            items,
            page: Some(page),
            indexing: false,
            ..
        }) = receiver.recv_timeout(TICK)
            && current == generation
            && page.shown == shown
            && page.total == total
        {
            assert_eq!(items.len(), shown);
            return (items, page);
        }
    }
    panic!("Expected page {shown}/{total} for generation {generation}");
}

#[test]
fn shared_results_page_all_matches_without_duplicate_advancement() {
    let fixture = Fixture::new(245);
    let (handle, receiver) = fixture.search();
    let generation = handle.query("match");
    let (first, cursor) = page(&receiver, generation, 100, 245);
    assert!(cursor.has_more());
    handle.more(cursor);
    let (second, next) = page(&receiver, generation, 200, 245);
    assert_eq!(&second[..100], first.as_slice());
    assert_eq!(cursor.revision, next.revision);
    handle.more(cursor);
    let (_, repeated) = page(&receiver, generation, 200, 245);
    assert_eq!(repeated, next);
    handle.more(next);
    let (last, end) = page(&receiver, generation, 245, 245);
    assert_eq!(&last[..200], second.as_slice());
    assert!(!end.has_more());
    assert!(last.windows(2).all(|items| items[0].path < items[1].path));
}

#[test]
fn result_tabs_request_every_match_and_filters_preserve_global_ranking() {
    let fixture = Fixture::new(245);
    std::fs::write(fixture.0.join("match"), b"Exact name")
        .expect("generated fixture must provide the expected value");
    let (handle, receiver) = fixture.search();
    let generation = handle.query_all("match");
    let (items, end) = page(&receiver, generation, 246, 246);
    assert_eq!(items[0].name, "match");
    assert!(!end.has_more());
    let generation = handle.query_all("match ext:txt type:file");
    let (items, _) = page(&receiver, generation, 245, 245);
    assert!(items.iter().all(|item| {
        item.path
            .extension()
            .expect("generated fixture must provide the expected value")
            == "txt"
    }));
}

#[test]
fn old_page_does_not_replace_new_query_and_new_index_invalidates_cursor() {
    let fixture = Fixture::new(205);
    let (handle, receiver) = fixture.search();
    let generation = handle.query("match");
    let (_, old) = page(&receiver, generation, 100, 205);
    handle.more(old);
    page(&receiver, generation, 200, 205);
    std::fs::write(fixture.0.join("match-9999.txt"), b"New file")
        .expect("generated fixture must provide the expected value");
    let (_, fresh) = page(&receiver, generation, 100, 206);
    assert_ne!(fresh.revision, old.revision);
    handle.more(old);
    let (_, unchanged) = page(&receiver, generation, 100, 206);
    assert_eq!(fresh, unchanged);
    let generation = handle.query("match-9999.txt");
    handle.more(fresh);
    let (items, end) = page(&receiver, generation, 1, 1);
    assert_eq!(items[0].name, "match-9999.txt");
    assert!(!end.has_more());
}

#[test]
fn exact_page_size_and_empty_queries_do_not_advertise_more_results() {
    let fixture = Fixture::new(100);
    let (handle, receiver) = fixture.search();
    let generation = handle.query("match");
    let (_, end) = page(&receiver, generation, 100, 100);
    assert!(!end.has_more());
    let generation = handle.query("ext:pdf");
    let (items, end) = page(&receiver, generation, 0, 0);
    assert!(items.is_empty());
    assert!(!end.has_more());
    let generation = handle.query("");
    let (_, end) = page(&receiver, generation, 0, 0);
    assert!(!end.has_more());
}

#[test]
fn oversized_candidate_index_falls_back_without_losing_tag_matches() {
    let mut worker = synthetic_worker(vec![
        tagged_item("large.flac", &"a".repeat(20_000)),
        tagged_item("small.flac", "Small title"),
    ]);
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results { items: first, .. } = worker
        .events
        .take()
        .expect("generated fixture must provide the expected value");
    assert_eq!(first.len(), 2);
    worker.prepare_candidates();
    assert!(worker.candidate_attempted);
    assert!(worker.candidates.is_none());
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results { items: second, .. } = worker
        .events
        .take()
        .expect("generated fixture must provide the expected value");
    assert_eq!(first, second);
    assert!(second.iter().any(|item| item.name == "large.flac"));
    assert!(second.iter().any(|item| item.name == "small.flac"));
}

#[test]
fn paged_candidates_preserve_complete_ranking_above_the_old_item_cutoff() {
    let items: Vec<_> = (0..40_257)
        .map(|number| tagged_item(&format!("match-{number:06}.flac"), "Title"))
        .collect();
    let mut worker = synthetic_worker(items);
    worker.query.limit = 100;
    worker.rescore_shared();
    let expected = ranked_rows(&worker);
    assert_eq!(expected.len(), 40_257);
    worker.prepare_candidates();
    assert!(
        worker.candidates.is_some(),
        "large eligible scopes must use indexed candidates"
    );
    worker.rescore_shared();
    assert_eq!(ranked_rows(&worker), expected);
    worker.publish(false);
    let SearchEvent::Results { items, page, .. } = worker.events.take().expect("first ranked page");
    assert_eq!(items.len(), 100);
    assert_eq!(page.expect("revision-bound page").total, 40_257);
}

#[test]
fn candidate_failure_replays_document_matches_without_spending_the_budget_twice() {
    use std::sync::atomic::AtomicUsize;
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
            Ok("needle".into())
        }
    }
    let fixture = Fixture::new(0);
    let mut items = Vec::new();
    for number in 0..300 {
        let extension = if (132..164).contains(&number) {
            "pdf"
        } else {
            "txt"
        };
        let path = fixture.0.join(format!("entry-{number:03}.{extension}"));
        std::fs::write(
            &path,
            if number < 132 {
                b"needle".as_slice()
            } else {
                b"empty".as_slice()
            },
        )
        .expect("write bounded content fixture");
        items.push(SearchItem::from_path(path, &fixture.0, false, None));
    }
    for identified in [false, true] {
        let mut items = items.clone();
        if identified {
            for item in &mut items {
                item.source_identity =
                    Some(crate::services::music_library::FileIdentity::from_metadata(
                        &std::fs::metadata(&item.path).expect("generated source metadata"),
                    ));
            }
        }
        let baseline_calls = Arc::new(AtomicUsize::new(0));
        let mut baseline = synthetic_worker(items.clone());
        baseline.query.text = "type:file content:needle".into();
        baseline.extraction = Arc::new(Extraction(baseline_calls.clone()));
        baseline.rescore_shared();
        assert_eq!(ranked_rows(&baseline).len(), 164);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut worker = synthetic_worker(items);
        worker.query.text = baseline.query.text.clone();
        worker.extraction = Arc::new(Extraction(calls.clone()));
        worker.prepare_candidates();
        worker
            .candidates
            .as_ref()
            .expect("prepared candidate index")
            .fail_after_page_for_test(1);
        worker.rescore_shared();
        assert_eq!(
            ranked_rows(&worker),
            ranked_rows(&baseline),
            "identified={identified}"
        );
        assert_eq!(worker.query_notice, baseline.query_notice);
        assert_eq!(
            calls.load(Ordering::Relaxed),
            MAX_DOCUMENT_EXTRACTIONS_PER_QUERY
        );
        assert_eq!(
            baseline_calls.load(Ordering::Relaxed),
            MAX_DOCUMENT_EXTRACTIONS_PER_QUERY
        );
        assert!(worker.content_scores.len() <= RESULT_LIMIT + MAX_DOCUMENT_EXTRACTIONS_PER_QUERY);
    }
}

fn tagged_item(name: &str, title: &str) -> SearchItem {
    let root = Path::new("/synthetic");
    let mut item = SearchItem::from_path(root.join(name), root, false, None);
    item.music_tags = Some(MusicTags {
        artist: Some("Fixture artist".into()),
        title: Some(title.into()),
        ..MusicTags::default()
    });
    item
}

pub(super) fn synthetic_worker(items: Vec<SearchItem>) -> Worker {
    Worker {
        commands: Arc::new(Mailbox {
            value: Mutex::new(None),
            ready: Condvar::new(),
        }),
        events: Arc::new(Mailbox {
            value: Mutex::new(None),
            ready: Condvar::new(),
        }),
        cancellation: Cancellation::default(),
        extraction: Arc::new(UnavailableTextExtraction),
        query: Query {
            generation: 1,
            text: "artist:\"Fixture artist\"".into(),
            cancelled: Arc::new(AtomicBool::new(false)),
            limit: MAX_INDEX_ENTRIES,
            grouping: None,
        },
        matches: Vec::new(),
        source: Arc::new(super::super::source::Source::from_items(items)),
        candidates: None,
        candidate_attempted: false,
        shared_indexing: false,
        root_invalidated: false,
        ranked: None,
        rank_notice: None,
        groups: None,
        revision: 0,
        shown_limit: RESULT_LIMIT,
        scan_notice: None,
        query_notice: None,
        document_budget: MAX_DOCUMENT_EXTRACTIONS_PER_QUERY,
        content_scores: HashMap::new(),
        last_publish: Instant::now(),
        limits: Limits::default(),
        index_cache: Arc::new(Mutex::new(VecDeque::new())),
    }
}

#[test]
fn prepared_sql_candidates_preserve_the_full_scorer_ranking() {
    let items: Vec<_> = (0..245)
        .map(|number| tagged_item(&format!("match-{number:04}.flac"), "Title"))
        .collect();
    let mut worker = synthetic_worker(items);
    worker.prepare_candidates();
    assert!(worker.candidates.is_some());
    for query in [
        "artist:\"Fixture artist\"",
        "type:audio ext:flac match",
        "title:missing",
        "type:folder",
        "in:match modified:today",
        "match-0244.flac",
    ] {
        worker.query.text = query.into();
        worker.rescore_shared();
        let indexed = ranked_rows(&worker);
        let candidates = worker.candidates.take();
        worker.rescore_shared();
        assert_eq!(indexed, ranked_rows(&worker), "{query}");
        worker.candidates = candidates;
    }
}

#[test]
fn more_cannot_overwrite_a_pending_query_with_the_same_generation() {
    let commands = Arc::new(Mailbox {
        value: Mutex::new(None),
        ready: Condvar::new(),
    });
    let handle = SearchHandle {
        next_generation: AtomicU64::new(1),
        commands: commands.clone(),
        events: Arc::new(Mailbox {
            value: Mutex::new(None),
            ready: Condvar::new(),
        }),
        startup_error: None,
        live: true,
        scan_cancelled: Cancellation::default(),
        query_cancelled: Mutex::new(Arc::new(AtomicBool::new(false))),
    };
    let generation = handle.query("new query");
    handle.more(SearchPage {
        generation,
        revision: 1,
        shown: 100,
        total: 200,
    });
    let Some(Command::Query(query)) = commands.take() else {
        panic!("Pending query was overwritten")
    };
    assert_eq!(query.generation, generation);
    assert_eq!(query.text, "new query");
    assert!(!query.cancelled.load(Ordering::Acquire));
}

// Collecting all ranks is restricted to regression comparisons.
pub(super) fn ranked_rows(worker: &Worker) -> Vec<(i64, usize)> {
    worker.ranked.as_ref().map_or_else(Vec::new, |ranked| {
        ranked
            .first(usize::MAX, &|| false)
            .expect("read verified test ranks")
    })
}
