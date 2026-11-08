use super::pagination_tests::{ranked_rows, synthetic_worker};
use super::*;

fn items(count: usize) -> Vec<SearchItem> {
    let root = Path::new("/synthetic");
    (0..count)
        .rev()
        .map(|number| {
            SearchItem::from_path(
                root.join(format!("match-{number:04}.txt")),
                root,
                false,
                None,
            )
        })
        .collect()
}

#[test]
fn disk_rank_order_matches_the_full_scorer_across_pages() {
    let mut worker = synthetic_worker(items(777));
    worker.query.text = "match".into();
    worker.query.limit = 100;
    let parsed = ParsedSearchQuery::parse(&worker.query.text);
    let source = worker.shared_items.clone();
    let mut expected: Vec<_> = source
        .iter()
        .enumerate()
        .filter_map(|(position, item)| worker.score(item, &parsed).map(|score| (score, position)))
        .collect();
    expected.sort_unstable_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| source[left.1].path.cmp(&source[right.1].path))
    });
    worker.rescore_shared();
    assert_eq!(ranked_rows(&worker), expected);
    worker.publish(false);
    let SearchEvent::Results {
        items,
        page: Some(page),
        notice,
        ..
    } = worker.events.take().expect("published first disk page")
    else {
        panic!("Expected a revision-bound page")
    };
    assert_eq!(items.len(), 100);
    assert_eq!(page.total, 777);
    assert!(notice.is_none());
    for (item, &(_, position)) in items.iter().zip(&expected) {
        assert_eq!(item, &source[position]);
    }
}

#[test]
fn late_disk_read_failure_withdraws_the_revision_and_rebuild_recovers() {
    let mut worker = synthetic_worker(items(600));
    worker.query.text = "match".into();
    worker.query.limit = 100;
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results {
        page: Some(old_page),
        ..
    } = worker.events.take().expect("published healthy page")
    else {
        panic!("Expected a revision-bound page")
    };
    worker
        .ranked
        .as_ref()
        .expect("finished disk ranks")
        .fail_after_page_for_test(1);
    worker.shown_limit = 300;
    worker.publish(false);
    let SearchEvent::Results {
        items,
        page: Some(page),
        groups,
        notice,
        ..
    } = worker.events.take().expect("published failure")
    else {
        panic!("Expected a withdrawn revision")
    };
    assert!(items.is_empty());
    assert!(groups.is_none());
    assert_eq!((page.shown, page.total), (0, 0));
    assert!(page.revision > old_page.revision);
    assert!(
        notice
            .expect("visible storage failure")
            .contains("Search results unavailable")
    );
    assert!(worker.ranked.is_none());
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results {
        items,
        page: Some(recovered),
        notice,
        ..
    } = worker.events.take().expect("published recovered page")
    else {
        panic!("Expected a recovered revision")
    };
    assert_eq!((items.len(), recovered.total), (100, 600));
    assert_ne!(recovered.revision, old_page.revision);
    assert!(notice.is_none());
    worker.commands.put(Command::More(old_page));
    worker.pending_shared(false);
    let SearchEvent::Results {
        page: Some(unchanged),
        ..
    } = worker.events.take().expect("published unchanged cursor")
    else {
        panic!("Expected the current revision")
    };
    assert_eq!(unchanged, recovered);
}

#[test]
fn failed_rank_stream_does_not_return_partial_groups() {
    let source = items(600);
    let mut ranked = super::super::ranked::Ranked::new(false, &|| false).expect("create ranks");
    for (position, item) in source.iter().enumerate() {
        ranked
            .push(0, position, item, &|| false)
            .expect("insert rank");
    }
    ranked.finish(&|| false).expect("finish ranks");
    ranked.fail_after_page_for_test(1);
    let result = super::super::groups::GroupIndex::build_paged(
        &source,
        &ranked,
        &ParsedSearchQuery::parse("match"),
        &|| false,
    );
    assert!(result.is_err());
}

#[test]
fn cancelled_disk_revision_cannot_publish_and_next_query_recovers() {
    let mut worker = synthetic_worker(items(300));
    worker.query.text = "match".into();
    worker.rescore_shared();
    worker.query.cancelled.store(true, Ordering::Release);
    worker.publish(false);
    assert!(worker.events.take().is_none());
    worker.query.cancelled = Arc::new(AtomicBool::new(false));
    worker.query.generation += 1;
    worker.query.text = "match-0010".into();
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results {
        items,
        generation,
        notice,
        ..
    } = worker.events.take().expect("published replacement query");
    assert_eq!(generation, 2);
    assert_eq!(items.first().expect("best match").name, "match-0010.txt");
    assert!(notice.is_none());
}
