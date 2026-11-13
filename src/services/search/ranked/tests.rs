use std::{
    cell::Cell,
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

use rusqlite::{config::DbConfig, limits::Limit};

use super::{
    MAX_DATABASE_BYTES, MAX_KEY_BYTES, MAX_PATH_BYTES, MAX_ROW_BYTES, PAGE_SIZE, RANK_PAGE_SIZE,
    Ranked, album_numbers, sort_key,
};
use crate::{model::MusicTags, services::search::SearchItem};

fn item(path: impl Into<PathBuf>) -> SearchItem {
    SearchItem::from_path(path.into(), Path::new("/"), false, None)
}

fn raw_item(path: &[u8]) -> SearchItem {
    item(PathBuf::from(OsString::from_vec(path.to_vec())))
}

fn build(count: usize) -> Ranked {
    let mut ranked = Ranked::new(false, &|| false).expect("create ranking");
    for position in (0..count).rev() {
        ranked
            .push(0, position, &item("/same/path"), &|| false)
            .expect("insert rank");
    }
    ranked.finish(&|| false).expect("finish ranking");
    ranked
}

fn collect(ranked: &Ranked) -> Vec<(i64, usize)> {
    let mut rows = Vec::new();
    ranked
        .for_each_page(&|| false, |page| {
            assert!(page.len() <= RANK_PAGE_SIZE);
            rows.extend_from_slice(page);
            Ok(())
        })
        .expect("read ranking");
    rows
}

fn path_cases() -> Vec<SearchItem> {
    [
        b"".as_slice(),
        b"/",
        b"//",
        b"///",
        b".",
        b"./",
        b"./.",
        b"..",
        b"../",
        b"../a",
        b"./..",
        b"./a",
        b"./a/.",
        b"a",
        b"a/",
        b"a/.",
        b"a//./",
        b"a/a",
        b"a/../a",
        b"a/..",
        b"a/../",
        b"a/../..",
        b"a/../b",
        b"a-",
        b"a.",
        b"a..",
        b"a0",
        b"aa",
        b"/a",
        b"//a",
        b"/./a",
        b"/a/.",
        b"/a/./b/",
        b"/a//b",
        b"/a/../b",
        b"/..",
        b"/../a",
        b"/b",
        b"a\x00",
        b"a\x00\x01",
        b"a\x00\xff",
        b"a\x01",
        b"a\xff",
        b"a\xfe/b",
        b"a/\xff",
        b"\x80",
        b"\xfe",
        b"\xff",
    ]
    .into_iter()
    .map(raw_item)
    .collect()
}

#[test]
fn encoded_component_order_matches_rust_paths_pairwise() {
    let mut items = path_cases();
    for byte in [0, 1, 2, b'.', b'-', b'0', b'a', 127, 128, 254, 255] {
        for prefix in [b"".as_slice(), b"/", b"./", b"../", b"a/"] {
            let mut path = prefix.to_vec();
            path.extend_from_slice(&[byte, b'x']);
            items.push(raw_item(&path));
            path.extend_from_slice(b"/../y");
            items.push(raw_item(&path));
        }
    }
    let keys: Vec<_> = items
        .iter()
        .map(|item| sort_key(false, 0, 0, item, &|| false).expect("encode path"))
        .collect();
    for (left, left_item) in items.iter().enumerate() {
        for (right, right_item) in items.iter().enumerate() {
            assert_eq!(
                keys[left].cmp(&keys[right]),
                left_item.path.cmp(&right_item.path),
                "{:?} versus {:?}",
                left_item.path,
                right_item.path
            );
        }
    }
}

#[test]
fn sqlite_order_matches_score_path_comparator_at_signed_extremes() {
    let paths = path_cases();
    let scores = [i64::MIN, i64::MAX, 0, -1, 1, i64::MIN + 1, i64::MAX - 1];
    let mut ranked = Ranked::new(false, &|| false).expect("create ranking");
    let count = paths.len() * scores.len();
    let mut expected = Vec::new();
    for position in (0..count).rev() {
        let score = scores[position / paths.len()];
        ranked
            .push(score, position, &paths[position % paths.len()], &|| false)
            .expect("insert score and path");
        expected.push((score, position));
    }
    expected.sort_unstable_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| {
                paths[left.1 % paths.len()]
                    .path
                    .cmp(&paths[right.1 % paths.len()].path)
            })
            .then_with(|| left.1.cmp(&right.1))
    });
    ranked.finish(&|| false).expect("finish ranking");
    assert_eq!(ranked.len(), count);
    assert_eq!(collect(&ranked), expected);
}

#[test]
fn album_numbers_and_ranking_preserve_worker_defaults_and_fallbacks() {
    let cases = [
        ("song.flac", None, None, (1, u32::MAX)),
        ("song.flac", Some(0), Some(0), (1, u32::MAX)),
        (
            "song.flac",
            Some(u32::MAX),
            Some(u32::MAX),
            (u32::MAX, u32::MAX),
        ),
        ("02. song.flac", None, None, (1, 2)),
        ("  003-song.flac", None, Some(0), (1, 3)),
        ("04_song.flac", Some(2), None, (2, 4)),
        ("05 song.flac", None, None, (1, 5)),
        ("06song.flac", None, None, (1, u32::MAX)),
        ("00 song.flac", None, None, (1, u32::MAX)),
        ("1000000 song.flac", None, None, (1, 1_000_000)),
        ("1000001 song.flac", None, None, (1, u32::MAX)),
        ("4294967296 song.flac", None, None, (1, u32::MAX)),
        ("１２ song.flac", None, None, (1, u32::MAX)),
        ("01 song.flac", Some(3), Some(8), (3, 8)),
        ("01 song.flac", Some(1), Some(1_000_001), (1, 1_000_001)),
        ("01", None, None, (1, u32::MAX)),
        (".hidden", None, None, (1, u32::MAX)),
    ];
    let mut items = Vec::new();
    for (name, disc, track, expected) in cases {
        let mut candidate = item(Path::new("/album").join(name));
        candidate.music_tags = Some(MusicTags {
            disc,
            track,
            ..MusicTags::default()
        });
        assert_eq!(album_numbers(&candidate), expected, "{name}");
        items.push(candidate);
    }
    // Equal normalized paths/numbers, conflicting scores and reverse insertion
    // distinguish the source-position tie from score and insertion order.
    items.push(items[1].clone());
    items.push(item("/album//song.flac"));
    let mut ranked = Ranked::new(true, &|| false).expect("create album ranking");
    let mut expected = Vec::new();
    for position in (0..items.len()).rev() {
        let score = position as i64;
        ranked
            .push(score, position, &items[position], &|| false)
            .expect("insert album rank");
        expected.push((score, position));
    }
    expected.sort_unstable_by(|left, right| {
        album_numbers(&items[left.1])
            .cmp(&album_numbers(&items[right.1]))
            .then_with(|| items[left.1].path.cmp(&items[right.1].path))
            .then_with(|| left.1.cmp(&right.1))
    });
    ranked.finish(&|| false).expect("finish album ranking");
    assert_eq!(collect(&ranked), expected);
}

#[test]
fn keyset_reads_cover_more_than_256_pages_in_source_tie_order() {
    let count = RANK_PAGE_SIZE * 257 + 3;
    let ranked = build(count);
    let mut visited = 0;
    let mut pages = 0;
    ranked
        .for_each_page(&|| false, |page| {
            assert_eq!(page.len(), (count - visited).min(RANK_PAGE_SIZE));
            for &(score, position) in page {
                assert_eq!((score, position), (0, visited));
                visited += 1;
            }
            pages += 1;
            Ok(())
        })
        .expect("page the full ranking");
    assert_eq!(pages, 258);
    assert_eq!(visited, count);
}

#[test]
fn zero_one_and_page_boundaries_return_only_requested_prefix() {
    for count in [
        0,
        1,
        RANK_PAGE_SIZE - 1,
        RANK_PAGE_SIZE,
        RANK_PAGE_SIZE + 1,
        513,
    ] {
        let ranked = build(count);
        for limit in [0, 1, 255, 256, 257, 512, 513, usize::MAX] {
            let expected: Vec<_> = (0..count.min(limit))
                .map(|position| (0, position))
                .collect();
            assert_eq!(
                ranked.first(limit, &|| false).expect("read prefix"),
                expected
            );
        }
        assert_eq!(collect(&ranked).len(), count);
    }
}

#[test]
fn first_stops_before_following_rows_and_pages() {
    let ranked = build(RANK_PAGE_SIZE * 3);
    ranked.fail_after_page_for_test(2);
    let polls = Cell::new(0);
    assert_eq!(
        ranked
            .first(1, &|| {
                polls.set(polls.get() + 1);
                polls.get() > 20
            })
            .expect("one displayed row needs only bounded cancellation polls"),
        vec![(0, 0)]
    );
    assert_eq!(ranked.first(256, &|| false).expect("one page").len(), 256);
    assert!(ranked.first(257, &|| false).is_err());
    assert!(ranked.first(1, &|| false).is_err());
    assert_eq!(ranked.len(), 0);
}

#[test]
fn unfinished_and_failed_revisions_never_return_a_prefix() {
    let mut ranked = Ranked::new(false, &|| false).expect("create ranking");
    ranked.push(1, 0, &item("/one"), &|| false).expect("insert");
    assert_eq!(ranked.len(), 0);
    assert!(ranked.first(1, &|| false).is_err());
    assert!(ranked.finish(&|| false).is_err());
    assert!(ranked.connection.borrow().is_none());
    let mut visits = 0;
    assert!(
        ranked
            .for_each_page(&|| false, |_| {
                visits += 1;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(visits, 0);
}

#[test]
fn duplicate_position_latches_and_discards_all_earlier_rows() {
    let mut ranked = Ranked::new(false, &|| false).expect("create ranking");
    ranked.push(1, 7, &item("/one"), &|| false).expect("insert");
    let failure = ranked
        .push(2, 7, &item("/two"), &|| false)
        .expect_err("duplicate position");
    assert_eq!(
        ranked.finish(&|| false).expect_err("latched failure"),
        failure
    );
    assert_eq!(
        ranked
            .push(3, 8, &item("/three"), &|| false)
            .expect_err("latched failure"),
        failure
    );
    assert!(ranked.first(1, &|| false).is_err());
    assert!(ranked.connection.borrow().is_none());
    assert_eq!(ranked.len(), 0);
}

#[test]
fn cancellation_at_creation_push_and_commit_latches_failure() {
    assert!(Ranked::new(false, &|| true).is_err());
    for cancel_at in [1, 2, 3, 4, 5] {
        let mut ranked = Ranked::new(false, &|| false).expect("create ranking");
        let polls = Cell::new(0);
        assert!(
            ranked
                .push(0, 0, &item("/some/path"), &|| {
                    polls.set(polls.get() + 1);
                    polls.get() >= cancel_at
                })
                .is_err()
        );
        assert!(ranked.finish(&|| false).is_err());
        assert!(ranked.connection.borrow().is_none());
    }
    for cancel_at in [1, 2] {
        let mut ranked = Ranked::new(false, &|| false).expect("create ranking");
        ranked.push(0, 0, &item("/one"), &|| false).expect("insert");
        let polls = Cell::new(0);
        assert!(
            ranked
                .finish(&|| {
                    polls.set(polls.get() + 1);
                    polls.get() >= cancel_at
                })
                .is_err()
        );
        assert!(ranked.first(1, &|| false).is_err());
        assert!(ranked.connection.borrow().is_none());
    }
}

#[test]
fn cancellation_midpage_withholds_page_and_latches_failure() {
    let ranked = build(300);
    let polls = Cell::new(0);
    let mut visits = 0;
    assert!(
        ranked
            .for_each_page(
                &|| {
                    polls.set(polls.get() + 1);
                    polls.get() >= 25
                },
                |_| {
                    visits += 1;
                    Ok(())
                }
            )
            .is_err()
    );
    assert_eq!(visits, 0);
    assert!(ranked.first(1, &|| false).is_err());
    assert!(ranked.connection.borrow().is_none());
}

#[test]
fn cancellation_after_final_page_is_observed_for_partial_and_full_pages() {
    for count in [1, 256, 257, 512] {
        let ranked = build(count);
        let cancelled = Cell::new(false);
        let mut visited = 0;
        assert!(
            ranked
                .for_each_page(&|| cancelled.get(), |page| {
                    visited += page.len();
                    cancelled.set(visited == count);
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(visited, count);
        assert!(ranked.first(1, &|| false).is_err());
        assert_eq!(ranked.len(), 0);
    }
}

#[test]
fn prefix_cancellation_discards_the_collected_prefix() {
    let baseline = build(3);
    let polls = Cell::new(0);
    baseline
        .first(1, &|| {
            polls.set(polls.get() + 1);
            false
        })
        .expect("measure one-row read");
    let cancel_at = polls.get();
    let ranked = build(3);
    let polls = Cell::new(0);
    assert!(
        ranked
            .first(1, &|| {
                polls.set(polls.get() + 1);
                polls.get() == cancel_at
            })
            .is_err()
    );
    assert!(ranked.first(1, &|| false).is_err());
    assert_eq!(ranked.len(), 0);
}

#[test]
fn injected_late_failure_reaches_visitor_then_latches_without_prefix() {
    let ranked = build(600);
    ranked.fail_after_page_for_test(2);
    let mut pages = 0;
    assert!(
        ranked
            .for_each_page(&|| false, |_| {
                pages += 1;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(pages, 2);
    assert!(ranked.connection.borrow().is_none());
    assert!(ranked.first(1, &|| false).is_err());
    assert_eq!(ranked.len(), 0);

    let ranked = build(600);
    ranked.fail_after_page_for_test(2);
    assert!(ranked.first(513, &|| false).is_err());
    assert!(ranked.first(1, &|| false).is_err());
}

#[test]
fn visitor_and_sqlite_read_errors_latch() {
    let ranked = build(2);
    assert_eq!(
        ranked.for_each_page(&|| false, |_| Err("visitor failure".into())),
        Err("visitor failure".into())
    );
    assert_eq!(ranked.first(1, &|| false), Err("visitor failure".into()));
    assert!(ranked.connection.borrow().is_none());

    let ranked = build(2);
    ranked
        .connection
        .borrow()
        .as_ref()
        .expect("connection")
        .execute_batch("PRAGMA query_only=OFF; DROP INDEX ranked_order;")
        .expect("inject missing index");
    assert!(ranked.first(1, &|| false).is_err());
    assert!(ranked.connection.borrow().is_none());
}

#[test]
fn full_database_failure_is_bounded_and_discards_unfinished_prefix() {
    let mut ranked = Ranked::new(false, &|| false).expect("create ranking");
    ranked
        .connection
        .borrow()
        .as_ref()
        .expect("connection")
        .execute_batch("PRAGMA max_page_count=8;")
        .expect("lower page ceiling");
    ranked
        .push(1, 0, &item("/small"), &|| false)
        .expect("insert earlier row");
    // Stay inside the supported path budget but force SQLite's page ceiling.
    let large = item(format!("/{}", "x".repeat(MAX_PATH_BYTES - 1)));
    assert!(ranked.push(1, 1, &large, &|| false).is_err());
    assert!(ranked.connection.borrow().is_none());
    assert!(ranked.finish(&|| false).is_err());
    assert!(ranked.first(1, &|| false).is_err());
    assert_eq!(ranked.len(), 0);
}

#[test]
fn raw_path_budget_allows_worst_case_key_expansion() {
    for path in [
        vec![0; 4096],
        vec![0; MAX_PATH_BYTES],
        b"a/".repeat(MAX_PATH_BYTES / 2),
        b"\x00/".repeat(MAX_PATH_BYTES / 2),
    ] {
        let mut ranked = Ranked::new(false, &|| false).expect("create ranking");
        let candidate = raw_item(&path);
        let key = sort_key(false, i64::MIN, u64::MAX, &candidate, &|| false)
            .expect("encode worst-case path");
        assert!(key.len() <= MAX_KEY_BYTES);
        ranked
            .push(i64::MIN, 0, &candidate, &|| false)
            .expect("insert expanded key");
        ranked.finish(&|| false).expect("finish expanded key");
        assert_eq!(
            ranked.first(1, &|| false).expect("read expanded key"),
            vec![(i64::MIN, 0)]
        );
    }
    let mut ranked = Ranked::new(false, &|| false).expect("create ranking");
    assert!(
        ranked
            .push(0, 0, &raw_item(&vec![b'x'; MAX_PATH_BYTES + 1]), &|| false)
            .is_err()
    );
    assert!(ranked.finish(&|| false).is_err());
}

#[test]
fn reads_use_only_the_chosen_order_index_without_a_temporary_sort() {
    for album_order in [false, true] {
        let mut ranked = Ranked::new(album_order, &|| false).expect("create ranking");
        ranked
            .push(0, 1000, &item("/one"), &|| false)
            .expect("insert sparse source position");
        ranked.finish(&|| false).expect("finish");
        let connection = ranked.connection.borrow();
        let connection = connection.as_ref().expect("connection");
        let indexes: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='index'",
                [],
                |row| row.get(0),
            )
            .expect("count indexes");
        assert_eq!(indexes, 1);
        let mut statement = connection.prepare(
            "EXPLAIN QUERY PLAN SELECT score, position, sort_key FROM ranked INDEXED BY ranked_order WHERE sort_key > ?1 ORDER BY sort_key LIMIT ?2"
        ).expect("prepare query plan");
        let plan = statement
            .query_map(rusqlite::params![Vec::<u8>::new(), 256], |row| {
                row.get::<_, String>(3)
            })
            .expect("query plan")
            .collect::<Result<Vec<_>, _>>()
            .expect("read plan")
            .join(" ");
        assert!(
            plan.contains("SEARCH ranked USING INDEX ranked_order"),
            "{plan}"
        );
        assert!(!plan.contains("TEMP B-TREE"), "{plan}");
        assert_eq!(
            ranked.first(1, &|| false).expect("sparse rank"),
            vec![(0, 1000)]
        );
    }
}

#[test]
fn sqlite_owns_tempfile_cleanup_and_enforces_memory_disk_and_defensive_limits() {
    let ranked = build(1000);
    {
        let connection = ranked.connection.borrow();
        let connection = connection.as_ref().expect("connection");
        for (pragma, expected) in [
            ("page_size", PAGE_SIZE as i64),
            ("max_page_count", (MAX_DATABASE_BYTES / PAGE_SIZE) as i64),
            ("cache_size", -2048),
            ("temp_store", 1),
            ("query_only", 1),
        ] {
            let actual: i64 = connection
                .query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))
                .expect("read pragma");
            assert_eq!(actual, expected, "{pragma}");
        }
        // SQLite reports no mmap row for an anonymous temporary database.
        let mut mmap = connection
            .prepare("PRAGMA mmap_size")
            .expect("prepare mmap pragma");
        let mut rows = mmap.query([]).expect("read mmap pragma");
        if let Some(row) = rows.next().expect("read optional mmap row") {
            assert_eq!(row.get::<_, i64>(0).expect("mmap bytes"), 0);
        }
        let filename: String = connection
            .query_row("PRAGMA database_list", [], |row| row.get(2))
            .expect("read anonymous filename");
        assert!(filename.is_empty());
        assert_eq!(
            connection
                .limit(Limit::SQLITE_LIMIT_LENGTH)
                .expect("length limit"),
            MAX_ROW_BYTES as i32
        );
        assert_eq!(
            connection
                .limit(Limit::SQLITE_LIMIT_ATTACHED)
                .expect("attachment limit"),
            0
        );
        assert!(
            connection
                .db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE)
                .expect("defensive config")
        );
        assert!(
            !connection
                .db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA)
                .expect("trusted schema config")
        );
    }
    // Explicit close exercises the same SQLite-owned cleanup as Ranked's Drop.
    // No application-named rank file or manual unlink path exists.
    ranked
        .connection
        .borrow_mut()
        .take()
        .expect("owned connection")
        .close()
        .expect("close anonymous ranking database");
    drop(ranked);
}
