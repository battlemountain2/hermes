use std::{cell::Cell, path::Path, time::Instant};

use super::{CANDIDATE_PAGE_SIZE, Candidates, MAX_DATABASE_BYTES, MAX_ROW_BYTES, PAGE_SIZE};
use crate::{
    model::MusicTags,
    services::search::{ParsedSearchQuery, SearchItem, matches_category},
};

fn item(name: &str, directory: bool) -> SearchItem {
    let root = Path::new("/synthetic-candidates");
    SearchItem::from_path(root.join(name), root, directory, None)
}

fn song(name: &str, artist: &str, album: &str, title: &str) -> SearchItem {
    let mut candidate = item(name, false);
    candidate.music_tags = Some(MusicTags {
        artist: Some(artist.into()),
        album: Some(album.into()),
        title: Some(title.into()),
        ..MusicTags::default()
    });
    candidate
}

fn query(text: &str) -> ParsedSearchQuery {
    ParsedSearchQuery::parse(&text.trim().to_lowercase())
}

fn matching(candidates: &Candidates, text: &str) -> Vec<usize> {
    candidates
        .matching(&query(text), &|| false)
        .expect("expected test value from candidates.matching(&query(text), &|| false)")
}

fn scored_indices(items: &[SearchItem], text: &str) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            crate::services::search::fuzzy_score(
                candidate,
                text,
                Path::new("/synthetic-candidates"),
            )
            .is_some()
        })
        .map(|(sequence, _)| sequence)
        .collect()
}

#[test]
fn every_category_alias_has_exact_candidate_parity_with_the_scorer() {
    let groups = [
        ("audio", "flac mp3 ogg opus m4a aac wav wma"),
        ("video", "mp4 mkv webm mov avi m4v mpeg mpg ogv"),
        (
            "image",
            "svg png jpg jpeg jpe jfif webp gif bmp avif tif tiff heic heif 3fr arw cr2 cr3 dcr dng erf kdc mef mos mrw nef nrw orf pef raf raw rw2 rwl sr2 srf srw x3f",
        ),
        (
            "document",
            "pdf docx odt xlsx xls ods bash c cc conf cpp cs css csv fish go h hpp htm html ini java js json jsonl jsx kt kts less log lua md markdown py rb rs rst scss service sh sql toml ts tsv tsx txt xml yaml yml zsh",
        ),
        ("application", "desktop exe appimage"),
        (
            "archive",
            "epub cbz cbr zip tar gz bz2 xz 7z rar zst tgz tbz2 txz",
        ),
    ];
    let mut items = Vec::new();
    for (category, extensions) in groups {
        for extension in extensions.split_ascii_whitespace() {
            for name in [
                format!("alias.{extension}"),
                format!("ALIAS.{}", extension.to_uppercase()),
            ] {
                let candidate = item(&name, false);
                assert!(
                    crate::services::search::fuzzy_score(
                        &candidate,
                        &format!("type:{category}"),
                        Path::new("/synthetic-candidates"),
                    )
                    .is_some(),
                    "{name}: {category}"
                );
                items.push(candidate);
                items.push(item(&name, true));
            }
        }
    }
    for name in [
        "Dockerfile",
        "GEMFILE",
        "Makefile",
        "meson.build",
        "unknown.blob",
        "extensionless",
        "shape.stl",
        "shape.3mf",
    ] {
        items.push(item(name, false));
        items.push(item(name, true));
    }
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    for category in super::CATEGORIES {
        let text = format!("type:{category}");
        assert_eq!(
            matching(&candidates, &text),
            scored_indices(&items, &text),
            "{category}"
        );
    }
}

#[test]
fn inferred_filters_do_not_prune_literal_matches_from_candidate_queries() {
    let items = vec![
        item("Video Games.flac", false),
        item("Games.mp4", false),
        item("Video Games.mp3", false),
        item("videos", true),
        item("other.flac", false),
    ];
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    for text in [
        "Video Games",
        "Video Games ext:flac",
        "type:folder videos",
        "Video Games type:video",
    ] {
        let retrieved = matching(&candidates, text);
        let scored = scored_indices(&items, text);
        assert!(!scored.is_empty(), "{text}");
        assert!(
            scored.iter().all(|position| retrieved.contains(position)),
            "{text}"
        );
    }
    assert_eq!(matching(&candidates, "Video Games ext:flac"), vec![0, 4]);
}

#[test]
#[ignore = "Generated 10k temporary-SQLite timing fixture; run explicitly with output; no end-to-end speed claim"]
fn generated_10k_exact_filter_retrieval_timings() {
    let items: Vec<_> = (0..10_000)
        .map(|sequence| match sequence % 10 {
            0..=5 => song(
                &format!(
                    "track-{sequence}.{}",
                    if sequence % 2 == 0 { "FLAC" } else { "mp3" }
                ),
                &format!("Artist {}", sequence % 100),
                &format!("Album {}", (sequence / 100) % 20),
                &format!("Title {sequence}"),
            ),
            6 => item(&format!("photo-{sequence}.HEIC"), false),
            7 => item(&format!("paper-{sequence}.pdf"), false),
            8 => item(&format!("archive-{sequence}.zip"), false),
            _ => item(&format!("folder-{sequence}.flac"), true),
        })
        .collect();
    let started = Instant::now();
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    let build_elapsed = started.elapsed();
    let page_count: i64 = candidates
        .connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .expect("expected test value from candidates .connection .query_row(\"PRAGMA page_count\", [], |row| row.get(0))");
    eprintln!(
        "Generated 10k candidate fixture: profile={}, items={}, build_ms={:.3}, database_bytes={}; warm retrieval only, no end-to-end speedup claim; cancellation is checked at row boundaries, not within SQLite VM steps",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        items.len(),
        build_elapsed.as_secs_f64() * 1000.0,
        page_count * PAGE_SIZE as i64,
    );
    for text in [
        "type:file",
        "type:folder",
        "type:audio",
        "ext:flac",
        "artist:\"Artist 0\"",
        "album:\"Album 0\"",
        "title:\"Title 0\"",
        "type:file type:audio ext:flac artist:\"Artist 0\" album:\"Album 0\" title:\"Title 0\"",
        "artist:missing",
    ] {
        let parsed = query(text);
        let expected = scored_indices(&items, text);
        assert_eq!(
            candidates
                .matching(&parsed, &|| false)
                .expect("expected test value from candidates.matching(&parsed, &|| false)"),
            expected
        );
        let mut samples = Vec::with_capacity(21);
        let mut candidate_counts = Vec::with_capacity(21);
        for _sample in 0..21 {
            let started = Instant::now();
            let retrieved = candidates
                .matching(&parsed, &|| false)
                .expect("expected test value from candidates.matching(&parsed, &|| false)");
            let elapsed = started.elapsed();
            candidate_counts.push(retrieved.len());
            samples.push(elapsed);
            assert_eq!(retrieved, expected, "{text}");
        }
        let samples_us: Vec<_> = samples.iter().map(|elapsed| elapsed.as_micros()).collect();
        samples.sort_unstable();
        eprintln!(
            "exact_filter={text:?} samples_us={samples_us:?} candidate_counts={candidate_counts:?} p50_ms={:.3} p95_ms={:.3}",
            samples[10].as_secs_f64() * 1000.0,
            samples[19].as_secs_f64() * 1000.0,
        );
    }
}

#[test]
fn exact_fields_use_rust_unicode_lowercasing_not_sqlite_nocase() {
    let items = [
        song("track.Ü", "İSTANBUL", "ΑΘΗΝΑ", "ПЕСНЯ"),
        song("other.ü", "Istanbul", "ΑΘΗΝΑ", "ПЕСНЯ"),
        song("third.flac", "İSTANBUL Live", "ΑΘΗΝΑ", "ПЕСНЯ"),
    ];
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    assert_eq!(
        matching(&candidates, "artist:İSTANBUL album:ΑΘΗΝΑ title:ПЕСНЯ ext:Ü"),
        vec![0]
    );
    assert_eq!(matching(&candidates, "artist:istanbul"), vec![1]);
    assert_eq!(matching(&candidates, "artist:İSTANBUL"), vec![0]);
    assert!(matching(&candidates, "title:ПЕС").is_empty());
}

#[test]
fn every_exact_constraint_intersects_and_preserves_input_order() {
    let mut folder = song("folder.flac", "Artist", "Album", "Title");
    folder.is_directory = true;
    let items = [
        song("z.FLAC", "Artist", "Album", "Title"),
        song("b.flac", "Other", "Album", "Title"),
        song("c.flac", "Artist", "Other", "Title"),
        song("d.flac", "Artist", "Album", "Other"),
        song("e.mp3", "Artist", "Album", "Title"),
        folder,
        song("a.flac", "Artist", "Album", "Title"),
    ];
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    assert_eq!(
        matching(
            &candidates,
            "type:file type:audio ext:.FLAC artist:Artist album:Album title:Title"
        ),
        vec![0, 6]
    );
    assert_eq!(matching(&candidates, "type:folder artist:Artist"), vec![5]);
    assert!(matching(&candidates, "type:folder type:audio").is_empty());
    assert!(matching(&candidates, "type:folder ext:flac").is_empty());
}

#[test]
fn category_aliases_match_the_existing_category_predicate() {
    let names = [
        "song.FLAC",
        "song.opus",
        "clip.MOV",
        "clip.ogv",
        "photo.jpg",
        "vector.svg",
        "photo.HEIC",
        "photo.heif",
        "camera.NEF",
        "map.tiff",
        "paper.pdf",
        "letter.odt",
        "sheet.xlsx",
        "sheet.ods",
        "source.rs",
        "README",
        "LICENSE",
        "launcher.desktop",
        "game.EXE",
        "game.AppImage",
        "backup.zip",
        "backup.tgz",
        "book.epub",
        "comic.cbz",
        "comic.cbr",
        "unknown.blob",
        "extensionless",
    ];
    let mut items: Vec<_> = names.iter().map(|name| item(name, false)).collect();
    items.extend(names.iter().map(|name| item(name, true)));
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    for category in super::CATEGORIES {
        let expected: Vec<_> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| !item.is_directory && matches_category(&item.path, category))
            .map(|(sequence, _)| sequence)
            .collect();
        assert!(!expected.is_empty(), "{category}");
        assert_eq!(
            matching(&candidates, &format!("type:{category}")),
            expected,
            "{category}"
        );
    }
    let unknown = ParsedSearchQuery {
        category: Some("unknown".into()),
        ..ParsedSearchQuery::default()
    };
    assert!(
        candidates
            .matching(&unknown, &|| false)
            .expect("expected test value from candidates.matching(&unknown, &|| false)")
            .is_empty()
    );
}

#[test]
fn missing_tags_are_not_empty_tags_or_album_artist_fallbacks() {
    let mut fallback = item("fallback.flac", false);
    fallback.music_tags = Some(MusicTags {
        album_artist: Some("Artist".into()),
        ..MusicTags::default()
    });
    let items = [
        item("untagged.flac", false),
        fallback,
        song("empty.flac", "", "", ""),
        song("tagged.flac", "Artist", "Album", "Title"),
    ];
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    assert_eq!(matching(&candidates, "artist:Artist"), vec![3]);
    let empty = ParsedSearchQuery {
        artist: Some(String::new()),
        ..ParsedSearchQuery::default()
    };
    assert_eq!(
        candidates
            .matching(&empty, &|| false)
            .expect("expected test value from candidates.matching(&empty, &|| false)"),
        vec![2]
    );
}

#[test]
fn exact_values_are_bound_as_literals() {
    let literal = "x' OR 1=1 --";
    let items = [
        song("first.flac", literal, "Album", "Title"),
        song("second.flac", "Other", "Album", "Title"),
    ];
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    let parsed = ParsedSearchQuery {
        artist: Some(literal.to_lowercase()),
        ..ParsedSearchQuery::default()
    };
    assert_eq!(
        candidates
            .matching(&parsed, &|| false)
            .expect("expected test value from candidates.matching(&parsed, &|| false)"),
        vec![0]
    );
    assert!(matching(&candidates, "artist:missing").is_empty());
}

#[test]
fn no_exact_fields_returns_the_full_set_without_scoring_other_fields() {
    let items: Vec<_> = (0..1200)
        .map(|sequence| item(&format!("{sequence}.flac"), sequence % 2 == 0))
        .collect();
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    let expected: Vec<_> = (0..items.len()).collect();
    assert_eq!(matching(&candidates, ""), expected);
    assert_eq!(
        matching(&candidates, "absent fuzzy terms in:missing"),
        expected
    );
    let unscored = ParsedSearchQuery {
        terms: "absent".into(),
        within: Some("missing".into()),
        modified: Some((0, 1)),
        content: Some("missing content".into()),
        ..ParsedSearchQuery::default()
    };
    assert_eq!(
        candidates
            .matching(&unscored, &|| false)
            .expect("expected test value from candidates.matching(&unscored, &|| false)"),
        expected
    );
    assert_eq!(
        matching(&candidates, "content:missing"),
        (0..items.len())
            .filter(|sequence| sequence % 2 != 0)
            .collect::<Vec<_>>()
    );
}

#[test]
fn sql_matches_have_no_result_limit_and_keep_the_last_eligible_item() {
    let items: Vec<_> = (0..1200)
        .map(|sequence| {
            song(
                &format!("{}.flac", 1200 - sequence),
                "Artist",
                "Album",
                "Title",
            )
        })
        .collect();
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    assert_eq!(
        matching(
            &candidates,
            "type:file type:audio ext:flac artist:Artist album:Album title:Title"
        ),
        (0..items.len()).collect::<Vec<_>>()
    );
}

#[test]
fn empty_input_and_no_matches_are_successful_empty_sets() {
    let candidates = Candidates::build(&[], &|| false)
        .expect("expected test value from Candidates::build(&[], &|| false)");
    assert!(matching(&candidates, "").is_empty());
    assert!(matching(&candidates, "type:audio artist:Artist").is_empty());
}

#[test]
fn cancellation_rejects_builds_and_queries_instead_of_returning_partial_sets() {
    let items: Vec<_> = (0..200)
        .map(|sequence| item(&format!("{sequence}.flac"), false))
        .collect();
    assert!(Candidates::build(&items, &|| true).is_err());
    for threshold in [10, items.len() + 30] {
        let checks = Cell::new(0);
        assert!(
            Candidates::build(&items, &|| {
                checks.set(checks.get() + 1);
                checks.get() >= threshold
            })
            .is_err()
        );
        assert!(checks.get() >= threshold);
    }
    let candidates = Candidates::build(&items, &|| false)
        .expect("expected test value from Candidates::build(&items, &|| false)");
    for parsed in [query(""), query("type:audio ext:flac")] {
        assert!(candidates.matching(&parsed, &|| true).is_err());
        let checks = Cell::new(0);
        assert!(
            candidates
                .matching(&parsed, &|| {
                    checks.set(checks.get() + 1);
                    checks.get() >= 20
                })
                .is_err()
        );
        assert_eq!(
            matching(&candidates, "type:audio"),
            (0..items.len()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn oversized_input_and_normalized_rows_are_rejected_without_truncation() {
    let oversized = song("large.flac", &"a".repeat(MAX_ROW_BYTES), "Album", "Title");
    assert!(Candidates::build(&[oversized], &|| false).is_err());
    let expanding = song(
        "expanding.flac",
        &"Ⱥ".repeat(4200),
        &"a".repeat(4200),
        "Title",
    );
    assert!(
        super::item_bytes(&expanding)
            .expect("expected test value from super::item_bytes(&expanding)")
            < MAX_ROW_BYTES
    );
    assert!(Candidates::build(&[expanding], &|| false).is_err());
}

#[test]
fn aggregate_input_budget_is_rejected_before_building() {
    let candidate = song("small.flac", &"a".repeat(8000), "Album", "Title");
    let row_bytes = super::item_bytes(&candidate)
        .expect("expected test value from super::item_bytes(&candidate)");
    assert!(row_bytes < MAX_ROW_BYTES);
    let items = vec![candidate; MAX_DATABASE_BYTES / row_bytes + 100];
    assert!(
        items
            .iter()
            .map(|item| super::item_bytes(item)
                .expect("expected test value from super::item_bytes(item)"))
            .sum::<usize>()
            > MAX_DATABASE_BYTES
    );
    assert!(Candidates::build(&items, &|| false).is_err());
}

#[test]
fn database_budget_failure_does_not_publish_a_partial_index() {
    let candidate = song("large.flac", &"a".repeat(4096), &"b".repeat(4096), "Title");
    let items = vec![candidate; 3000];
    assert!(items.iter().all(|item| {
        super::item_bytes(item).expect("expected test value from super::item_bytes(item)")
            < MAX_ROW_BYTES
    }));
    assert!(
        items
            .iter()
            .map(|item| super::item_bytes(item)
                .expect("expected test value from super::item_bytes(item)"))
            .sum::<usize>()
            < MAX_DATABASE_BYTES
    );
    assert!(Candidates::build(&items, &|| false).is_err());
}

#[test]
fn sqlite_is_ephemeral_and_has_bounded_pages_cache_and_values() {
    let candidates = Candidates::build(&[item("small.flac", false)], &|| false).expect(
        "expected test value from Candidates::build(&[item(\"small.flac\", false)], &|| false)",
    );
    let connection = &candidates.connection;
    for (pragma, expected) in [
        ("page_size", PAGE_SIZE as i64),
        ("max_page_count", (MAX_DATABASE_BYTES / PAGE_SIZE) as i64),
        ("cache_size", -2048),
        ("temp_store", 1),
        ("query_only", 1),
    ] {
        let actual: i64 = connection
            .query_row(&format!("PRAGMA {pragma}"), [], |row| row.get(0))
            .expect("expected test value from connection .query_row(&format!(\"PRAGMA {pragma}\"), [], |row| row.get(0))");
        assert_eq!(actual, expected, "{pragma}");
    }
    let filename: String = connection
        .query_row("PRAGMA database_list", [], |row| row.get(2))
        .expect("expected test value from connection .query_row(\"PRAGMA database_list\", [], |row| row.get(2))");
    assert!(filename.is_empty());
    assert_eq!(
        connection
            .limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH)
            .expect("expected test value from connection .limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH)"),
        MAX_ROW_BYTES as i32
    );
    let oversized = ParsedSearchQuery {
        title: Some("a".repeat(MAX_ROW_BYTES + 1)),
        ..ParsedSearchQuery::default()
    };
    assert!(candidates.matching(&oversized, &|| false).is_err());
}

#[test]
fn sqlite_failure_is_an_error_not_an_empty_candidate_set() {
    let candidates = Candidates::build(&[item("small.flac", false)], &|| false).expect(
        "expected test value from Candidates::build(&[item(\"small.flac\", false)], &|| false)",
    );
    candidates
        .connection
        .execute_batch("PRAGMA query_only=OFF; DROP TABLE categories;")
        .expect("expected test value from candidates .connection .execute_batch(\"PRAGMA query_only=OFF; DROP TABLE categories;\")");
    assert!(
        candidates
            .matching(&query("type:audio"), &|| false)
            .is_err()
    );
}

#[test]
fn one_hundred_thousand_candidates_are_visited_in_bounded_ordered_pages() {
    let items: Vec<_> = (0..100_000)
        .map(|number| {
            item(
                &format!(
                    "entry-{number:06}.{}",
                    if number % 100 == 0 { "txt" } else { "flac" }
                ),
                false,
            )
        })
        .collect();
    let candidates = Candidates::build(&items, &|| false)
        .expect("build 100k generated candidate rows within the auxiliary byte budget");
    let mut count = 0;
    let mut largest_page = 0;
    candidates
        .for_each_page(&query(""), &|| false, |page| {
            assert_eq!(page.first(), Some(&count));
            assert!(page.windows(2).all(|pair| pair[0] + 1 == pair[1]));
            count += page.len();
            largest_page = largest_page.max(page.len());
            Ok(())
        })
        .expect("visit every generated row");
    assert_eq!(count, 100_000);
    assert_eq!(largest_page, CANDIDATE_PAGE_SIZE);
    let mut filtered = 0;
    candidates
        .for_each_page(&query("type:document ext:txt"), &|| false, |page| {
            assert!(page.iter().all(|position| position % 100 == 0));
            filtered += page.len();
            Ok(())
        })
        .expect("page exact indexed filters above the previous item cap");
    assert_eq!(filtered, 1000);
}

#[test]
fn all_six_filter_bindings_and_page_bounds_preserve_scorer_parity() {
    let items: Vec<_> = (0..777)
        .map(|number| song(&format!("entry-{number}.flac"), "Artist", "Album", "Title"))
        .collect();
    let candidates = Candidates::build(&items, &|| false).expect("generated tagged candidates");
    let parsed = ParsedSearchQuery {
        directory: Some(false),
        category: Some("audio".into()),
        extension: Some("flac".into()),
        artist: Some("artist".into()),
        album: Some("album".into()),
        title: Some("title".into()),
        ..ParsedSearchQuery::default()
    };
    assert_eq!(
        candidates
            .matching(&parsed, &|| false)
            .expect("six filters fit the bounded parameter policy"),
        (0..items.len()).collect::<Vec<_>>()
    );
}

#[test]
fn visitor_failure_and_cancellation_after_a_page_stop_further_visits() {
    let items = vec![item("entry.flac", false); CANDIDATE_PAGE_SIZE * 3];
    let candidates = Candidates::build(&items, &|| false).expect("generated paged candidates");
    let visits = Cell::new(0);
    let result = candidates.for_each_page(&query("type:audio"), &|| false, |_| {
        visits.set(visits.get() + 1);
        Err("generated ranking failure".into())
    });
    assert_eq!(result, Err("generated ranking failure".into()));
    assert_eq!(visits.get(), 1);
    let cancelled = Cell::new(false);
    let result = candidates.for_each_page(&query("type:audio"), &|| cancelled.get(), |_| {
        cancelled.set(true);
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(
        matching(&candidates, "type:audio").len(),
        items.len(),
        "next query starts from its own cursor"
    );
}

#[test]
fn sqlite_failure_after_an_earlier_page_is_not_a_successful_partial_set() {
    let items = vec![item("entry.flac", false); CANDIDATE_PAGE_SIZE * 3];
    let candidates = Candidates::build(&items, &|| false).expect("generated paged candidates");
    let mut count = 0;
    let result = candidates.for_each_page(&query("type:audio"), &|| false, |page| {
        count += page.len();
        candidates
            .connection
            .execute_batch("PRAGMA query_only=OFF; DROP TABLE categories;")
            .expect("inject failure between pages");
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(count, CANDIDATE_PAGE_SIZE);
}

#[test]
fn every_filter_subset_preserves_the_full_scorer_candidates() {
    let items: Vec<_> = (0..777)
        .map(|number| {
            let mut row = song(
                &format!(
                    "entry-{number}.{}",
                    if number % 2 == 0 { "flac" } else { "txt" }
                ),
                if number % 3 == 0 { "Artist" } else { "Other" },
                if number % 5 == 0 { "Album" } else { "Other" },
                if number % 7 == 0 { "Title" } else { "Other" },
            );
            row.is_directory = number % 11 == 0;
            row
        })
        .collect();
    let candidates = Candidates::build(&items, &|| false).expect("mixed generated candidates");
    for mask in 0..64 {
        let parsed = ParsedSearchQuery {
            directory: (mask & 1 != 0).then_some(false),
            category: (mask & 2 != 0).then(|| "audio".into()),
            extension: (mask & 4 != 0).then(|| "flac".into()),
            artist: (mask & 8 != 0).then(|| "artist".into()),
            album: (mask & 16 != 0).then(|| "album".into()),
            title: (mask & 32 != 0).then(|| "title".into()),
            ..ParsedSearchQuery::default()
        };
        let mut budget = 32;
        let mut notice = None;
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let expected: Vec<_> = items
            .iter()
            .enumerate()
            .filter_map(|(position, item)| {
                if parsed.is_empty() {
                    return Some(position);
                }
                crate::services::search::score_item(
                    item,
                    &parsed,
                    &mut crate::services::search::ContentSearchContext {
                        text_extraction: &crate::services::search::UnavailableTextExtraction,
                        cancelled: &cancelled,
                        document_extractions_remaining: &mut budget,
                        notice: &mut notice,
                    },
                )
                .map(|_| position)
            })
            .collect();
        assert_eq!(
            candidates
                .matching(&parsed, &|| false)
                .expect("bounded filter traversal"),
            expected,
            "filter mask {mask}"
        );
    }
}

#[test]
fn exact_page_boundaries_and_unindexed_large_fields_are_preserved() {
    for count in [0, 255, 256, 257, 512] {
        let items = vec![item("entry.flac", false); count];
        let candidates = Candidates::build(&items, &|| false).expect("page boundary fixture");
        assert_eq!(
            matching(&candidates, "type:audio"),
            (0..count).collect::<Vec<_>>()
        );
    }
    let mut row = song("entry.flac", "Artist", "Album", "Title");
    row.search_metadata = "unindexed ".repeat(100_000);
    row.music_tags.as_mut().expect("fixture tags").album_artist =
        Some("unindexed ".repeat(100_000));
    let candidates = Candidates::build(&[row], &|| false)
        .expect("auxiliary budget excludes fields it does not store");
    assert_eq!(matching(&candidates, "artist:artist"), vec![0]);
}
