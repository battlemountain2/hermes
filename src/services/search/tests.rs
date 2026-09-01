// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use super::{SearchEvent, SearchItem, fuzzy_score, index_tree, recent_bounds};

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
fn background_index_returns_results_for_queries_received_while_walking() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("strata-search-{unique}"));
    fs::create_dir_all(root.join("nested")).expect("the search fixture should be created");
    fs::write(root.join("nested/needle.txt"), b"result")
        .expect("the search fixture file should be written");

    let (search, events) = index_tree(root.clone());
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
