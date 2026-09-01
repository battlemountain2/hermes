// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, SystemTime},
};

use super::{
    SearchEvent, SearchItem, TextExtractionProvider, UnavailableTextExtraction, fuzzy_score,
    index_tree, recent_bounds, search_filter_values,
};
use crate::services::TextExtractor;

struct FixtureTextExtraction;

impl TextExtractionProvider for FixtureTextExtraction {
    fn extract_text(
        &self,
        _path: &Path,
        extractor: TextExtractor,
        byte_limit: usize,
        _cancelled: Arc<AtomicBool>,
    ) -> Result<String, String> {
        assert_eq!(extractor, TextExtractor::Pdf);
        Ok("A phrase extracted safely from a PDF"[..byte_limit.min(36)].to_owned())
    }
}

fn item(path: &str) -> SearchItem {
    let name = Path::new(path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    SearchItem {
        path: PathBuf::from(path),
        search_name: name.to_lowercase(),
        search_path: path.to_lowercase(),
        name,
        is_directory: false,
        modified_unix_seconds: None,
    }
}

#[test]
fn exact_names_rank_above_substrings_and_fuzzy_matches() {
    let root = Path::new("/home/me");
    let exact =
        fuzzy_score(&item("/home/me/notes"), "notes", root).expect("an exact name should match");
    let substring = fuzzy_score(&item("/home/me/my-notes.txt"), "notes", root)
        .expect("a name substring should match");
    let fuzzy = fuzzy_score(&item("/home/me/nested-object-types.rs"), "notes", root)
        .expect("an ordered fuzzy subsequence should match");
    assert!(exact > substring);
    assert!(substring > fuzzy);
}

#[test]
fn searches_relative_path_fragments_and_rejects_non_matches() {
    let candidate = item("/home/me/themes/azure/colors.toml");
    assert!(fuzzy_score(&candidate, "themes/azure", Path::new("/home/me")).is_some());
    assert!(fuzzy_score(&candidate, "definitely-missing", Path::new("/home/me")).is_none());
}

#[test]
fn recent_queries_use_non_overlapping_local_day_ranges() {
    let (today_start, today_end) = recent_bounds("modified:today").expect("today should parse");
    let (yesterday_start, yesterday_end) =
        recent_bounds("modified:yesterday").expect("yesterday should parse");
    assert_eq!(yesterday_end, today_start);
    assert!(yesterday_start < yesterday_end);
    assert!(today_start < today_end);
    assert!(recent_bounds("today").is_none());
}

#[test]
fn structured_filters_can_match_without_a_name_term() {
    let mut candidate = item("/home/me/src/search.rs");
    candidate.modified_unix_seconds = Some(
        recent_bounds("modified:today")
            .expect("today should parse")
            .0,
    );
    assert!(fuzzy_score(&candidate, "type:file ext:rs in:src", Path::new("/home/me")).is_some());
    assert!(fuzzy_score(&candidate, "type:folder", Path::new("/home/me")).is_none());
    assert!(fuzzy_score(&candidate, "ext:png", Path::new("/home/me")).is_none());
}

#[test]
fn structured_filters_combine_with_fuzzy_name_terms() {
    let candidate = item("/home/me/themes/azure/colors.toml");
    assert!(fuzzy_score(&candidate, "colors ext:toml", Path::new("/home/me")).is_some());
    assert!(fuzzy_score(&candidate, "missing ext:toml", Path::new("/home/me")).is_none());
}

#[test]
fn content_filter_matches_case_insensitive_quoted_phrases() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("hermes-content-{unique}.md"));
    fs::write(&path, "A Hidden Phrase lives inside this document.")
        .expect("the content fixture should be written");
    let candidate = item(&path.to_string_lossy());

    assert!(
        fuzzy_score(
            &candidate,
            "content:\"hidden phrase\"",
            path.parent().expect("the fixture should have a parent")
        )
        .is_some()
    );
    assert!(
        fuzzy_score(
            &candidate,
            "content:\"missing phrase\"",
            path.parent().expect("the fixture should have a parent")
        )
        .is_none()
    );
    fs::remove_file(path).expect("the content fixture should be removed");
}

#[test]
fn content_filter_skips_binary_looking_files() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("hermes-binary-{unique}.txt"));
    fs::write(&path, b"needle\0binary").expect("the binary fixture should be written");
    let candidate = item(&path.to_string_lossy());

    assert!(fuzzy_score(&candidate, "content:needle", Path::new("/tmp")).is_none());
    fs::remove_file(path).expect("the binary fixture should be removed");
}

#[test]
fn filter_values_round_trip_quoted_content_and_locations() {
    let (terms, filters) = search_filter_values(
        "report type:file ext:md in:\"Work Notes\" modified:yesterday content:\"action item\"",
    );
    assert_eq!(terms, "report");
    assert_eq!(filters.directory, Some(false));
    assert_eq!(filters.extension.as_deref(), Some("md"));
    assert_eq!(filters.within.as_deref(), Some("work notes"));
    assert_eq!(filters.modified.as_deref(), Some("yesterday"));
    assert_eq!(filters.content.as_deref(), Some("action item"));
}

#[test]
fn background_index_can_search_inside_text_files() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("hermes-content-index-{unique}"));
    fs::create_dir_all(&root).expect("the content index fixture should be created");
    fs::write(root.join("ordinary.md"), "the uniquely searchable phrase")
        .expect("the content fixture should be written");

    let (search, events) = index_tree(root.clone(), Arc::new(UnavailableTextExtraction));
    search.query("content:\"uniquely searchable\"");
    let found = (0..30).any(|_| {
        events.recv_timeout(Duration::from_millis(100)).is_ok_and(
            |SearchEvent::Results { query, items, .. }| {
                query == "content:\"uniquely searchable\""
                    && items.iter().any(|item| item.name == "ordinary.md")
            },
        )
    });

    drop(search);
    fs::remove_dir_all(root).expect("the content index fixture should be removed");
    assert!(
        found,
        "the worker should publish a file whose content matches"
    );
}

#[test]
fn background_index_returns_results_for_queries_received_while_walking() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-search-{unique}"));
    fs::create_dir_all(root.join("nested")).expect("the search fixture should be created");
    fs::write(root.join("nested/needle.txt"), b"result")
        .expect("the search fixture file should be written");

    let (search, events) = index_tree(root.clone(), Arc::new(UnavailableTextExtraction));
    search.query("needle");
    let found = (0..20).any(|_| {
        events.recv_timeout(Duration::from_millis(100)).is_ok_and(
            |SearchEvent::Results { query, items, .. }| {
                query == "needle" && items.iter().any(|item| item.name == "needle.txt")
            },
        )
    });

    drop(search);
    fs::remove_dir_all(root).expect("the search fixture should be removed");
    assert!(found, "the worker should publish the matching indexed file");
}

#[test]
fn background_index_searches_pdf_text_through_the_provider() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("hermes-pdf-index-{unique}"));
    fs::create_dir_all(&root).expect("the PDF search fixture should be created");
    fs::write(root.join("manual.pdf"), b"not parsed outside the provider")
        .expect("the PDF fixture should be written");

    let (search, events) = index_tree(root.clone(), Arc::new(FixtureTextExtraction));
    search.query("content:\"extracted safely\"");
    let found = (0..30).any(|_| {
        events.recv_timeout(Duration::from_millis(100)).is_ok_and(
            |SearchEvent::Results { query, items, .. }| {
                query == "content:\"extracted safely\""
                    && items.iter().any(|item| item.name == "manual.pdf")
            },
        )
    });

    drop(search);
    fs::remove_dir_all(root).expect("the PDF search fixture should be removed");
    assert!(
        found,
        "the worker should search PDF text through its provider"
    );
}
