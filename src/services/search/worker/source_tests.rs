use super::pagination_tests::{ranked_rows, synthetic_worker};
use super::*;
use crate::services::music_library::Entry;

#[test]
fn root_replacement_withdraws_stale_results_before_any_memory_fallback() {
    use crate::services::music_library::tests::{Fixture as LibraryFixture, ready, stop};
    let fixture = LibraryFixture::new();
    fixture.song("old.flac", "Old");
    let mut library = fixture.subscribe();
    let snapshot = ready(&mut library, |snapshot| !snapshot.entries.is_empty());
    let mut worker = synthetic_worker(vec![SearchItem::from_path(
        fixture.root.join("old.flac"),
        &fixture.root,
        false,
        None,
    )]);
    worker.query.text = "old".into();
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results {
        page: Some(old), ..
    } = worker.events.take().expect("old results")
    else {
        panic!("expected revision")
    };
    std::fs::rename(&fixture.root, fixture.root.with_extension("previous")).expect("replace root");
    std::fs::create_dir(&fixture.root).expect("replacement root");
    assert!(worker.reject_changed_root(&mut library, &snapshot, &fixture.root));
    let SearchEvent::Results {
        items,
        page: Some(page),
        indexing,
        notice,
        ..
    } = worker.events.take().expect("withdrawal")
    else {
        panic!("expected withdrawn revision")
    };
    assert!(items.is_empty());
    assert_eq!(page.total, 0);
    assert!(page.revision > old.revision);
    assert!(indexing);
    assert!(notice.expect("root status").contains("verified index"));
    assert!(worker.source.get(0).expect("empty source").is_none());
    stop(library);
}

#[test]
fn more_withdraws_an_installed_memory_source_after_replacement_without_a_snapshot_event() {
    let fixture = Fixture::new();
    let root = fixture.0.join("Selected");
    std::fs::create_dir(&root).expect("selected root");
    let entries = Arc::new(
        (0..260)
            .map(|number| Entry {
                path: root.join(format!("match-{number:03}.txt")),
                is_directory: false,
                modified_unix_seconds: None,
                identity: None,
                music_tags: None,
            })
            .collect(),
    );
    let mut worker = synthetic_worker(Vec::new());
    worker.source = Arc::new(
        super::super::source::Source::new(
            root.clone(),
            entries,
            MAX_INDEX_ENTRIES,
            MAX_INDEX_BYTES,
            &|| false,
        )
        .expect("installed fallback source"),
    );
    worker.query.text = "match".into();
    worker.query.limit = 100;
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results {
        page: Some(old), ..
    } = worker.events.take().expect("healthy page")
    else {
        panic!("expected revision")
    };
    std::fs::rename(&root, fixture.0.join("Previous")).expect("replace selected folder");
    std::fs::create_dir(&root).expect("replacement root");
    worker.commands.put(Command::More(old));
    worker.pending_shared(false);
    let SearchEvent::Results {
        items,
        page: Some(page),
        notice,
        ..
    } = worker.events.take().expect("withdrawn page")
    else {
        panic!("expected withdrawn revision")
    };
    assert!(items.is_empty());
    assert_eq!(page.total, 0);
    assert!(page.revision > old.revision);
    assert!(notice.expect("root error").contains("root changed"));
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("fixture clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hermes-worker-source-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("private fixture root");
        Self(path)
    }

    fn entries(&self, count: usize) -> Arc<Vec<Entry>> {
        let outside = self.0.with_extension("outside");
        let mut entries = vec![Entry {
            path: self.0.clone(),
            is_directory: true,
            modified_unix_seconds: None,
            identity: None,
            music_tags: None,
        }];
        for number in 0..count {
            entries.push(Entry {
                path: outside.join(format!("excluded-{number}.txt")),
                is_directory: false,
                modified_unix_seconds: None,
                identity: None,
                music_tags: None,
            });
            entries.push(Entry {
                path: self.0.join(format!("{number:04}.flac")),
                is_directory: false,
                modified_unix_seconds: Some(123),
                identity: None,
                music_tags: Some(MusicTags {
                    title: Some(format!("Song {number}")),
                    artist: Some("Fixture artist".into()),
                    album: Some("Album".into()),
                    ..MusicTags::default()
                }),
            });
        }
        Arc::new(entries)
    }

    fn source(&self, entries: Arc<Vec<Entry>>) -> Arc<super::super::source::Source> {
        Arc::new(
            super::super::source::Source::new(
                self.0.clone(),
                entries,
                MAX_INDEX_ENTRIES,
                MAX_INDEX_BYTES,
                &|| false,
            )
            .expect("scope a shared library snapshot"),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn flat_materialization_observes_cancellation_between_source_lookups() {
    let fixture = Fixture::new();
    let source = fixture.source(fixture.entries(260));
    let polls = std::cell::Cell::new(0);
    let rows = (0..260).map(|number| (0, number * 2 + 2)).collect();
    let result = materialize_items(&source, rows, &|| {
        polls.set(polls.get() + 1);
        polls.get() >= 4
    });
    assert!(
        result
            .expect_err("cancel after three resolved rows")
            .contains("cancelled")
    );
    assert_eq!(polls.get(), 4);
}

#[test]
fn shared_source_holes_preserve_candidate_rank_and_group_positions() {
    let fixture = Fixture::new();
    let mut worker = synthetic_worker(Vec::new());
    worker.source = fixture.source(fixture.entries(260));
    worker.query.limit = 100;
    worker.rescore_shared();
    let baseline = ranked_rows(&worker);
    assert_eq!(baseline.len(), 260);
    assert!(
        baseline
            .iter()
            .all(|&(_, position)| position > 0 && position % 2 == 0)
    );
    worker.prepare_candidates();
    assert!(worker.candidates.is_some());
    worker.rescore_shared();
    assert_eq!(ranked_rows(&worker), baseline);
    worker.publish(false);
    let SearchEvent::Results {
        items,
        page: Some(page),
        notice,
        ..
    } = worker.events.take().expect("published scoped source")
    else {
        panic!("Expected source page")
    };
    assert_eq!((items.len(), page.total), (100, 260));
    assert!(items.iter().all(|item| item.path.starts_with(&fixture.0)));
    assert!(notice.is_none());

    worker.query.grouping = Some(Grouping {
        group: SearchGroup::All,
        entity: None,
        virtual_results: true,
    });
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results {
        groups: Some(groups),
        notice,
        ..
    } = worker.events.take().expect("published grouped source")
    else {
        panic!("Expected source groups")
    };
    assert_eq!(
        (
            groups.counts.songs,
            groups.counts.albums,
            groups.counts.artists
        ),
        (260, 1, 1)
    );
    for result in groups.results {
        match result {
            super::super::SearchResult::File(item) => assert!(item.path.starts_with(&fixture.0)),
            super::super::SearchResult::Album(entity)
            | super::super::SearchResult::Artist(entity) => {
                assert_eq!(entity.representative.name, "0000.flac");
                assert_eq!(entity.songs, 260);
            }
        }
    }
    assert!(notice.is_none());
}

#[test]
fn replacing_source_rebuilds_positions_and_rejects_the_previous_page() {
    let fixture = Fixture::new();
    let original = fixture.entries(260);
    let mut worker = synthetic_worker(Vec::new());
    worker.source = fixture.source(original.clone());
    worker.query.limit = 100;
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results {
        page: Some(old), ..
    } = worker.events.take().expect("old source page")
    else {
        panic!("Expected source page")
    };
    let pinned = worker.source.clone();
    worker.source = fixture.source(fixture.entries(1));
    worker.candidates = None;
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results {
        items,
        page: Some(current),
        ..
    } = worker.events.take().expect("new source page")
    else {
        panic!("Expected new source page")
    };
    assert_eq!((items.len(), current.total), (1, 1));
    assert!(current.revision > old.revision);
    assert!(pinned.get(520).expect("pinned source read").is_some());
    assert!(
        worker
            .source
            .get(520)
            .expect("replacement source read")
            .is_none()
    );
    worker.commands.put(Command::More(old));
    worker.pending_shared(false);
    let SearchEvent::Results { page, .. } = worker.events.take().expect("stale page rejected");
    assert_eq!(page, Some(current));
}

#[test]
fn missing_grouped_source_position_withdraws_results_under_a_new_revision() {
    let fixture = Fixture::new();
    let mut worker = synthetic_worker(Vec::new());
    worker.source = fixture.source(fixture.entries(260));
    worker.query.grouping = Some(Grouping {
        group: SearchGroup::All,
        entity: None,
        virtual_results: true,
    });
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results {
        page: Some(old),
        groups: Some(_),
        ..
    } = worker.events.take().expect("healthy grouped page")
    else {
        panic!("Expected groups")
    };
    // Simulate a future fallible backing store losing a required position.
    worker.source = Arc::new(super::super::source::Source::empty());
    worker.publish(false);
    let SearchEvent::Results {
        items,
        page: Some(page),
        groups,
        notice,
        ..
    } = worker.events.take().expect("withdrawn grouped page")
    else {
        panic!("Expected withdrawal")
    };
    assert!(items.is_empty());
    assert!(groups.is_none());
    assert_eq!((page.shown, page.total), (0, 0));
    assert!(page.revision > old.revision);
    assert!(
        notice
            .expect("visible source failure")
            .contains("invalid source position")
    );
}
