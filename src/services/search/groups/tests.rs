use std::{cell::Cell, collections::HashSet, path::Path};

use super::{
    ENTITY_OVERHEAD_BYTES, GroupIndex, MAX_ENTITIES, MAX_METADATA_BYTES, SearchCounts,
    SearchEntity, SearchEntityKey, SearchGroup, SearchGroups, SearchResult, SearchResultKey,
    key_bytes, matches_entity,
};
use crate::{
    model::MusicTags,
    services::search::{ParsedSearchQuery, SearchItem},
};

fn item(name: &str, directory: bool) -> SearchItem {
    let root = Path::new("/synthetic-groups");
    SearchItem::from_path(root.join(name), root, directory, None)
}

fn song(
    name: &str,
    artist: Option<&str>,
    album: Option<&str>,
    album_artist: Option<&str>,
) -> SearchItem {
    let mut candidate = item(name, false);
    candidate.music_tags = Some(MusicTags {
        title: Some("Unrelated track".into()),
        artist: artist.map(str::to_owned),
        album: album.map(str::to_owned),
        album_artist: album_artist.map(str::to_owned),
        ..MusicTags::default()
    });
    candidate.search_metadata = [artist, album, album_artist, Some("Unrelated track")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    candidate
}

fn query(text: &str) -> ParsedSearchQuery {
    ParsedSearchQuery::parse(&text.trim().to_lowercase())
}

fn ranking(items: &[SearchItem]) -> Vec<(i64, usize)> {
    (0..items.len()).map(|position| (0, position)).collect()
}

fn index(items: &[SearchItem]) -> GroupIndex {
    GroupIndex::build(items, &ranking(items), &query("type:file"), &|| false).unwrap()
}

fn entity(result: &SearchResult) -> &SearchEntity {
    match result {
        SearchResult::Album(entity) | SearchResult::Artist(entity) => entity,
        SearchResult::File(_) => panic!("Expected an entity"),
    }
}

fn keys(results: &[SearchResult]) -> Vec<SearchResultKey> {
    results.iter().map(SearchResult::key).collect()
}

fn group_keys(results: &[SearchResult], group: SearchGroup) -> Vec<SearchResultKey> {
    results
        .iter()
        .filter(|result| result.group() == group)
        .map(SearchResult::key)
        .collect()
}

fn mixed_items() -> Vec<SearchItem> {
    let mut items = vec![item("folder.flac", true), item("notes.txt", false)];
    for position in 0..9 {
        items.push(song(
            &format!("{position}.flac"),
            Some(&format!("Artist {position}")),
            Some(&format!("Album {position}")),
            None,
        ));
    }
    items
}

#[test]
fn dto_contract_and_result_identity_keep_virtual_entities_separate_from_files() {
    fn copy_eq_debug<Value: Copy + Eq + std::fmt::Debug>() {}
    fn clone_eq_debug<Value: Clone + Eq + std::fmt::Debug>() {}
    fn hash_ord<Value: std::hash::Hash + Ord>() {}
    copy_eq_debug::<SearchGroup>();
    copy_eq_debug::<SearchCounts>();
    clone_eq_debug::<SearchEntity>();
    clone_eq_debug::<SearchResult>();
    clone_eq_debug::<SearchGroups>();
    hash_ord::<SearchEntityKey>();
    assert_eq!(SearchGroup::default(), SearchGroup::All);
    assert_eq!(SearchCounts::default().total(SearchGroup::All), 0);
    let items = vec![song("track.FLAC", Some("Artist"), Some("Album"), None)];
    let grouped = index(&items);
    let results = grouped.page(&items, SearchGroup::All, usize::MAX);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].group(), SearchGroup::Songs);
    assert_eq!(results[0].file(), Some(&items[0]));
    assert_eq!(
        results[0].key(),
        SearchResultKey::File(items[0].path.clone())
    );
    assert_eq!(results[1].group(), SearchGroup::Albums);
    assert_eq!(results[2].group(), SearchGroup::Artists);
    assert!(results[1].file().is_none());
    assert!(results[2].file().is_none());
    assert_ne!(results[1].key(), results[2].key());
    assert_eq!(keys(&results).into_iter().collect::<HashSet<_>>().len(), 3);
    let dto = SearchGroups {
        group: SearchGroup::All,
        results,
        counts: grouped.counts(),
    };
    assert_eq!(dto, dto.clone());
    assert_eq!(dto.counts.total(dto.group), 3);
}

#[test]
fn every_audio_extension_alias_and_case_is_a_song_but_folders_are_files() {
    let mut items = Vec::new();
    for extension in "flac mp3 ogg opus m4a aac wav wma".split_whitespace() {
        for spelling in [extension.to_owned(), extension.to_uppercase()] {
            items.push(item(&format!("track.{spelling}"), false));
            items.push(item(&format!("folder.{spelling}"), true));
        }
    }
    let grouped = index(&items);
    assert_eq!(
        grouped.counts(),
        SearchCounts {
            files: 16,
            songs: 16,
            albums: 0,
            artists: 0
        }
    );
    for group in [SearchGroup::Files, SearchGroup::Songs] {
        let page = grouped.page(&items, group, usize::MAX);
        assert_eq!(page.len(), 16);
        assert!(page.iter().all(|result| result.group() == group));
    }
}

#[test]
fn tagged_non_audio_and_audio_named_directories_never_create_virtual_entities() {
    let mut items = Vec::new();
    for name in [
        "video.mp4",
        "picture.JPG",
        "notes.rs",
        "book.epub",
        "unknown.blob",
        "no-extension",
    ] {
        items.push(song(name, Some("Artist"), Some("Album"), None));
    }
    let mut directory = song("folder.flac", Some("Artist"), Some("Album"), None);
    directory.is_directory = true;
    items.push(directory);
    let grouped = index(&items);
    assert_eq!(
        grouped.counts(),
        SearchCounts {
            files: 7,
            songs: 0,
            albums: 0,
            artists: 0
        }
    );
    assert!(
        grouped
            .page(&items, SearchGroup::All, 100)
            .iter()
            .all(|result| result.group() == SearchGroup::Files)
    );
    let key = SearchEntityKey::Artist("artist".into());
    assert!(items.iter().all(|item| !matches_entity(item, &key)));
}

#[test]
fn unicode_lowercase_and_trimmed_identity_merge_without_ascii_transliteration() {
    let items = vec![
        song("z.flac", Some(" BJÖRK "), Some(" DÉBUT "), None),
        song("a.flac", Some("Björk"), Some("Début"), None),
        song("other.flac", Some("Bjork"), Some("Debut"), None),
        song("unicode.flac", Some("東京"), Some("音楽"), None),
        song("expansion.flac", Some("İ"), Some("İ"), None),
        song("expanded.flac", Some("i\u{307}"), Some("i\u{307}"), None),
    ];
    let grouped = index(&items);
    assert_eq!(grouped.counts().albums, 4);
    assert_eq!(grouped.counts().artists, 4);
    let albums = grouped.page(&items, SearchGroup::Albums, 20);
    let debut = albums
        .iter()
        .map(entity)
        .find(|entity| {
            entity.key
                == SearchEntityKey::Album {
                    artist: "björk".into(),
                    album: "début".into(),
                }
        })
        .unwrap();
    assert_eq!(debut.songs, 2);
    assert_eq!(debut.title, "Début");
    assert_eq!(debut.artist.as_deref(), Some("Björk"));
    assert_eq!(debut.representative, items[1]);
    assert!(matches_entity(&items[0], &debut.key));
    assert!(!matches_entity(&items[2], &debut.key));
    let expanded = albums
        .iter()
        .map(entity)
        .find(|entity| {
            entity.key
                == SearchEntityKey::Album {
                    artist: "i\u{307}".into(),
                    album: "i\u{307}".into(),
                }
        })
        .unwrap();
    assert_eq!(expanded.songs, 2);
}

#[test]
fn compilation_and_disc_folders_merge_by_album_artist_not_path_or_track_artist() {
    let mut first = song(
        "Disc 2/z.flac",
        Some("First performer"),
        Some("Compilation"),
        Some("Various Artists"),
    );
    first.music_tags.as_mut().unwrap().disc = Some(2);
    let mut second = song(
        "elsewhere/Disc 1/a.flac",
        Some("Second performer"),
        Some("COMPILATION"),
        Some("VARIOUS ARTISTS"),
    );
    second.music_tags.as_mut().unwrap().disc = Some(1);
    let third = song(
        "another-folder/x.flac",
        Some("First performer"),
        Some("Compilation"),
        Some("Various artists"),
    );
    let items = vec![first, second, third];
    let grouped = index(&items);
    assert_eq!(
        grouped.counts(),
        SearchCounts {
            files: 0,
            songs: 3,
            albums: 1,
            artists: 2
        }
    );
    let albums = grouped.page(&items, SearchGroup::Albums, 100);
    assert_eq!(entity(&albums[0]).songs, 3);
    assert!(
        items
            .iter()
            .all(|item| matches_entity(item, &entity(&albums[0]).key))
    );
    let artists = grouped.page(&items, SearchGroup::Artists, 100);
    assert_eq!(entity(&artists[0]).title, "First performer");
    assert_eq!(entity(&artists[0]).songs, 2);
    assert!(entity(&artists[0]).artist.is_none());
    assert!(!matches_entity(&items[1], &entity(&artists[0]).key));
    assert!(!matches_entity(
        &items[0],
        &SearchEntityKey::Artist("various artists".into())
    ));
}

#[test]
fn same_album_title_with_different_artists_remains_separate() {
    let items = vec![
        song(
            "same-folder/one.flac",
            Some("One"),
            Some("Greatest Hits"),
            None,
        ),
        song(
            "same-folder/two.flac",
            Some("Two"),
            Some("Greatest Hits"),
            None,
        ),
    ];
    let grouped = index(&items);
    assert_eq!(grouped.counts().albums, 2);
    let albums = grouped.page(&items, SearchGroup::Albums, 10);
    assert_ne!(albums[0].key(), albums[1].key());
    assert_eq!(entity(&albums[0]).title, entity(&albums[1]).title);
    assert_eq!(entity(&albums[0]).songs, 1);
    assert_eq!(entity(&albums[1]).songs, 1);
}

#[test]
fn artist_without_album_and_album_artist_fallback_use_the_same_drilldown_logic() {
    let items = vec![
        song("one.flac", Some("Solo"), None, None),
        song("two.flac", None, Some("Record"), Some("Fallback")),
        song("three.flac", Some("   "), None, Some("Fallback")),
        song(
            "four.flac",
            Some("Actual"),
            Some("Record"),
            Some("Fallback"),
        ),
        song("five.flac", Some("Actual"), Some("Record"), Some(" ")),
    ];
    let grouped = index(&items);
    assert_eq!(
        grouped.counts(),
        SearchCounts {
            files: 0,
            songs: 5,
            albums: 2,
            artists: 3
        }
    );
    for group in [SearchGroup::Albums, SearchGroup::Artists] {
        for result in grouped.page(&items, group, 100) {
            let entity = entity(&result);
            assert_eq!(
                items
                    .iter()
                    .filter(|item| matches_entity(item, &entity.key))
                    .count(),
                entity.songs
            );
        }
    }
    let artists = grouped.page(&items, SearchGroup::Artists, 100);
    let fallback = artists
        .iter()
        .map(entity)
        .find(|entity| entity.title == "Fallback")
        .unwrap();
    assert_eq!(fallback.songs, 2);
    assert!(!matches_entity(&items[3], &fallback.key));
}

#[test]
fn empty_missing_and_invalid_tags_keep_physical_songs_without_empty_virtual_labels() {
    let mut items = vec![item("untagged.flac", false)];
    for (artist, album, album_artist) in [
        (None, None, None),
        (Some(""), Some(""), Some("")),
        (Some("  "), Some(" \u{2003} "), None),
        (None, Some("Unknown artist record"), None),
        (Some("Invalid\nArtist"), Some("Record"), None),
        (Some("Artist"), Some("Invalid\0Album"), None),
    ] {
        items.push(song(
            &format!("{}.flac", items.len()),
            artist,
            album,
            album_artist,
        ));
    }
    let too_long = "界".repeat(257);
    items.push(song(
        "oversized.flac",
        Some(&too_long),
        Some("Record"),
        None,
    ));
    let mut invalid_title = song("invalid-title.flac", Some("Artist"), Some("Record"), None);
    invalid_title.music_tags.as_mut().unwrap().title = Some("Bad\rtitle".into());
    items.push(invalid_title);
    let grouped = index(&items);
    assert_eq!(
        grouped.counts(),
        SearchCounts {
            files: 0,
            songs: items.len(),
            albums: 0,
            artists: 0
        }
    );
    assert_eq!(
        grouped.page(&items, SearchGroup::Songs, usize::MAX).len(),
        items.len()
    );
    assert!(grouped.notice().is_none());
    for candidate in &items {
        assert!(!matches_entity(
            candidate,
            &SearchEntityKey::Artist("artist".into())
        ));
    }
}

#[test]
fn valid_256_character_unicode_tags_are_not_rejected_by_byte_length() {
    let label = "界".repeat(256);
    let items = vec![song("track.flac", Some(&label), Some(&label), None)];
    let grouped = index(&items);
    assert_eq!(grouped.counts().albums, 1);
    assert_eq!(grouped.counts().artists, 1);
    assert_eq!(
        entity(&grouped.page(&items, SearchGroup::Albums, 1)[0]).title,
        label
    );
}

#[test]
fn song_title_or_path_matches_do_not_automatically_match_album_or_artist_labels() {
    let items = vec![song(
        "zzzzzz-track.flac",
        Some("Performer"),
        Some("Record"),
        None,
    )];
    let parsed = query("zzzzzz");
    assert!(
        crate::services::search::fuzzy_score(&items[0], "zzzzzz", Path::new("/synthetic-groups"))
            .is_some()
    );
    let grouped = GroupIndex::build(&items, &[(30_000, 0)], &parsed, &|| false).unwrap();
    assert_eq!(
        grouped.counts(),
        SearchCounts {
            files: 0,
            songs: 1,
            albums: 0,
            artists: 0
        }
    );
    assert_eq!(grouped.page(&items, SearchGroup::All, 100).len(), 1);
}

#[test]
fn entity_labels_support_substrings_unordered_tokens_and_fuzzy_terms() {
    let items = vec![song(
        "track.flac",
        Some("Frank Ocean"),
        Some("Blonde"),
        None,
    )];
    for (text, albums, artists) in [
        ("ocean", 1, 1),
        ("cean", 1, 1),
        ("ocean frank", 1, 1),
        ("ocn frnk", 1, 1),
        ("blonde ocean", 1, 0),
        ("ocean blonde", 1, 0),
        ("blnd", 1, 0),
        ("blonde impossible", 0, 0),
    ] {
        let grouped = GroupIndex::build(&items, &ranking(&items), &query(text), &|| false).unwrap();
        assert_eq!(grouped.counts().albums, albums, "{text}");
        assert_eq!(grouped.counts().artists, artists, "{text}");
        assert_eq!(grouped.counts().songs, 1);
    }
}

#[test]
fn compilation_album_labels_use_album_artist_not_hidden_track_performer() {
    let items = vec![song(
        "track.flac",
        Some("Zzzzz"),
        Some("Record"),
        Some("Various"),
    )];
    let grouped = GroupIndex::build(&items, &ranking(&items), &query("zzzzz"), &|| false).unwrap();
    assert_eq!(grouped.counts().albums, 0);
    assert_eq!(grouped.counts().artists, 1);
}

#[test]
fn exact_album_labels_rank_before_substring_token_and_fuzzy_labels() {
    let items = vec![
        song(
            "fuzzy.flac",
            Some("Performer"),
            Some("Balloons Dream Episode"),
            None,
        ),
        song(
            "substring.flac",
            Some("Performer"),
            Some("Blonde Deluxe"),
            None,
        ),
        song("exact.flac", Some("Performer"), Some("Blonde"), None),
    ];
    let grouped = GroupIndex::build(&items, &ranking(&items), &query("blonde"), &|| false).unwrap();
    let albums = grouped.page(&items, SearchGroup::Albums, 10);
    assert_eq!(
        albums
            .iter()
            .map(|result| entity(result).title.as_str())
            .collect::<Vec<_>>(),
        ["Blonde", "Blonde Deluxe", "Balloons Dream Episode"]
    );
    assert_eq!(grouped.counts().songs, 3);
    assert_eq!(grouped.counts().artists, 0);
}

#[test]
fn structured_and_content_filters_are_already_applied_to_source_pairs() {
    let items = vec![
        song(
            "one.flac",
            Some("One"),
            Some("Compilation"),
            Some("Various"),
        ),
        song(
            "two.flac",
            Some("Two"),
            Some("Compilation"),
            Some("Various"),
        ),
        song(
            "three.flac",
            Some("Two"),
            Some("Compilation"),
            Some("Various"),
        ),
    ];
    let parsed = query(
        "artist:two album:compilation title:'Unrelated track' content:needle ext:flac in:three",
    );
    let grouped = GroupIndex::build(&items, &[(8_000, 2)], &parsed, &|| false).unwrap();
    assert_eq!(
        grouped.counts(),
        SearchCounts {
            files: 0,
            songs: 1,
            albums: 1,
            artists: 1
        }
    );
    let albums = grouped.page(&items, SearchGroup::Albums, 100);
    assert_eq!(entity(&albums[0]).songs, 1);
    assert_eq!(entity(&albums[0]).representative, items[2]);
    assert_eq!(
        entity(&grouped.page(&items, SearchGroup::Artists, 100)[0]).title,
        "Two"
    );
}

#[test]
fn counts_and_matching_song_totals_cover_all_eligible_pairs_not_the_first_hundred() {
    let mut items = Vec::new();
    for position in 0..305 {
        items.push(song(
            &format!("{position:03}.flac"),
            Some("Artist"),
            Some("Album"),
            None,
        ));
    }
    items.push(song("excluded.flac", Some("Artist"), Some("Album"), None));
    let ranked = ranking(&items[..305]);
    let grouped = GroupIndex::build(&items, &ranked, &query("type:audio"), &|| false).unwrap();
    assert_eq!(
        grouped.counts(),
        SearchCounts {
            files: 0,
            songs: 305,
            albums: 1,
            artists: 1
        }
    );
    assert_eq!(grouped.page(&items, SearchGroup::Songs, 100).len(), 100);
    assert_eq!(grouped.page(&items, SearchGroup::Songs, 200).len(), 200);
    for group in [SearchGroup::Albums, SearchGroup::Artists] {
        assert_eq!(entity(&grouped.page(&items, group, 1)[0]).songs, 305);
    }
    assert_eq!(grouped.counts().total(SearchGroup::All), 307);
}

#[test]
fn files_and_songs_preserve_original_ranked_order_without_reranking() {
    let items = vec![
        item("a.txt", false),
        item("a.flac", false),
        item("z.txt", false),
        item("z.flac", false),
        item("folder.mp3", true),
    ];
    let ranked = [(1, 3), (2, 4), (3, 2), (4, 1), (5, 0)];
    let grouped = GroupIndex::build(&items, &ranked, &query("type:file"), &|| false).unwrap();
    let files = grouped.page(&items, SearchGroup::Files, 100);
    let songs = grouped.page(&items, SearchGroup::Songs, 100);
    assert_eq!(
        keys(&files),
        [4, 2, 0].map(|position| SearchResultKey::File(items[position].path.clone()))
    );
    assert_eq!(
        keys(&songs),
        [3, 1].map(|position| SearchResultKey::File(items[position].path.clone()))
    );
}

#[test]
fn all_paging_distributes_round_robin_prefix_quotas_then_emits_section_order() {
    let items = mixed_items();
    let grouped = index(&items);
    let groups = [
        SearchGroup::Files,
        SearchGroup::Songs,
        SearchGroup::Albums,
        SearchGroup::Artists,
    ];
    for (limit, expected) in [
        (0, [0, 0, 0, 0]),
        (1, [1, 0, 0, 0]),
        (4, [1, 1, 1, 1]),
        (8, [2, 2, 2, 2]),
        (12, [2, 4, 3, 3]),
        (usize::MAX, [2, 9, 9, 9]),
    ] {
        let page = grouped.page(&items, SearchGroup::All, limit);
        assert_eq!(page.len(), expected.iter().sum::<usize>());
        let mut section_order = Vec::new();
        for (group, expected_count) in groups.into_iter().zip(expected) {
            let prefix = group_keys(&page, group);
            assert_eq!(prefix.len(), expected_count);
            assert_eq!(prefix, keys(&grouped.page(&items, group, expected_count)));
            section_order.extend(std::iter::repeat_n(group, expected_count));
        }
        assert_eq!(
            page.iter().map(SearchResult::group).collect::<Vec<_>>(),
            section_order
        );
    }
}

#[test]
fn more_keeps_nested_per_group_prefixes_and_counts_are_independent_of_page() {
    let items = mixed_items();
    let grouped = index(&items);
    let counts = grouped.counts();
    let mut previous = Vec::new();
    for limit in 0..=counts.total(SearchGroup::All) + 3 {
        let page = grouped.page(&items, SearchGroup::All, limit);
        for group in [
            SearchGroup::Files,
            SearchGroup::Songs,
            SearchGroup::Albums,
            SearchGroup::Artists,
        ] {
            let before = group_keys(&previous, group);
            let after = group_keys(&page, group);
            assert!(after.starts_with(&before));
            let explicit = keys(&grouped.page(&items, group, limit));
            assert!(keys(&grouped.page(&items, group, usize::MAX)).starts_with(&explicit));
            if limit >= 4 {
                assert!(!after.is_empty());
            }
        }
        assert_eq!(grouped.counts(), counts);
        previous = page;
    }
}

#[test]
fn all_paging_skips_empty_groups_and_fills_the_available_prefixes() {
    let items = vec![
        item("a.txt", false),
        item("b.txt", false),
        item("c.txt", false),
    ];
    let grouped = index(&items);
    assert_eq!(grouped.page(&items, SearchGroup::All, 2).len(), 2);
    assert_eq!(grouped.page(&items, SearchGroup::All, 10).len(), 3);
    let items = vec![
        song("one.flac", Some("One"), Some("Record"), None),
        song("two.flac", Some("Two"), Some("Record"), None),
    ];
    let grouped = index(&items);
    let page = grouped.page(&items, SearchGroup::All, 4);
    assert_eq!(group_keys(&page, SearchGroup::Songs).len(), 2);
    assert_eq!(group_keys(&page, SearchGroup::Albums).len(), 1);
    assert_eq!(group_keys(&page, SearchGroup::Artists).len(), 1);
    let empty = index(&[]);
    assert!(empty.page(&[], SearchGroup::All, usize::MAX).is_empty());
    assert_eq!(empty.counts(), SearchCounts::default());
}

#[test]
fn entity_ranking_is_label_based_and_deterministic_under_source_and_rank_permutations() {
    let items = vec![
        song("z.flac", Some("FRANK OCEAN"), Some("BLONDE"), None),
        song("a.flac", Some("Frank Ocean"), Some("Blonde"), None),
        song(
            "other.flac",
            Some("Frank Ocean Live"),
            Some("Blonde Live"),
            None,
        ),
        song("fuzzy.flac", Some("Freaking Oceans"), Some("Beyond"), None),
    ];
    let ranked = [(500, 0), (400, 3), (300, 2), (200, 1)];
    let grouped = GroupIndex::build(&items, &ranked, &query("frank ocean"), &|| false).unwrap();
    let mut reversed = items.clone();
    reversed.reverse();
    let other = GroupIndex::build(
        &reversed,
        &ranking(&reversed),
        &query("frank ocean"),
        &|| false,
    )
    .unwrap();
    for group in [SearchGroup::Albums, SearchGroup::Artists] {
        assert_eq!(
            grouped.page(&items, group, 100),
            other.page(&reversed, group, 100)
        );
    }
    let artists = grouped.page(&items, SearchGroup::Artists, 100);
    assert_eq!(entity(&artists[0]).title, "Frank Ocean");
    assert_eq!(entity(&artists[0]).representative, items[1]);
    assert_eq!(entity(&artists[0]).songs, 2);
    assert_eq!(entity(&artists[1]).title, "Frank Ocean Live");
}

#[cfg(unix)]
#[test]
fn representatives_use_native_path_order_without_lossy_path_identity() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let root = Path::new("/synthetic-groups");
    let mut high = song("placeholder.flac", Some("ARTIST"), Some("ALBUM"), None);
    high.path = root.join(OsString::from_vec(b"\xff.flac".to_vec()));
    let mut low = song("placeholder.flac", Some("Artist"), Some("Album"), None);
    low.path = root.join(OsString::from_vec(b"\xfe.flac".to_vec()));
    let items = vec![high, low];
    assert_eq!(
        items[0].path.to_string_lossy(),
        items[1].path.to_string_lossy()
    );
    let grouped = index(&items);
    for group in [SearchGroup::Albums, SearchGroup::Artists] {
        let page = grouped.page(&items, group, 1);
        assert_eq!(entity(&page[0]).representative, items[1]);
    }
    assert_ne!(
        SearchResult::File(items[0].clone()).key(),
        SearchResult::File(items[1].clone()).key()
    );
}

#[test]
fn cancellation_is_checked_before_work_per_physical_record_and_before_publication() {
    let items = vec![
        item("one.txt", false),
        item("two.txt", false),
        item("three.txt", false),
    ];
    let ranked = ranking(&items);
    let calls = Cell::new(0);
    let result = GroupIndex::build(&items, &ranked, &query("type:file"), &|| {
        let call = calls.get() + 1;
        calls.set(call);
        call == 3
    });
    assert!(result.err().unwrap().contains("cancelled"));
    assert_eq!(calls.get(), 3);
    assert!(GroupIndex::build(&[], &[], &query(""), &|| true).is_err());
    let calls = Cell::new(0);
    let result = GroupIndex::build(&items, &ranked, &query("type:file"), &|| {
        calls.set(calls.get() + 1);
        calls.get() == 5
    });
    assert!(result.is_err());
    let calls = Cell::new(0);
    let result = GroupIndex::build_with_limits(
        &items,
        &ranked,
        &query(""),
        &|| {
            calls.set(calls.get() + 1);
            calls.get() == 4
        },
        0,
        0,
    );
    assert!(result.is_err());
}

#[test]
fn invalid_rank_positions_fail_without_panicking() {
    let items = vec![item("one.txt", false)];
    assert!(
        GroupIndex::build(&items, &[(0, 1)], &query(""), &|| false)
            .err()
            .unwrap()
            .contains("position")
    );
}

#[test]
fn entity_budget_preserves_every_physical_match_and_updates_retained_summaries() {
    let items = vec![
        song("z.flac", Some("Artist"), Some("Album"), None),
        song("other.flac", Some("Other"), Some("Other"), None),
        item("notes.txt", false),
        song("a.flac", Some("ARTIST"), Some("ALBUM"), None),
    ];
    let grouped = GroupIndex::build_with_limits(
        &items,
        &ranking(&items),
        &query("type:file"),
        &|| false,
        MAX_METADATA_BYTES,
        1,
    )
    .unwrap();
    assert_eq!(
        grouped.counts(),
        SearchCounts {
            files: 1,
            songs: 3,
            albums: 1,
            artists: 0
        }
    );
    assert!(grouped.notice().unwrap().contains("partial"));
    assert!(
        grouped
            .notice()
            .unwrap()
            .contains("all matching files and songs")
    );
    let albums = grouped.page(&items, SearchGroup::Albums, 10);
    assert_eq!(entity(&albums[0]).songs, 2);
    assert_eq!(entity(&albums[0]).representative, items[3]);
    assert_eq!(grouped.page(&items, SearchGroup::Songs, 100).len(), 3);
    assert_eq!(grouped.page(&items, SearchGroup::Files, 100).len(), 1);
}

#[test]
fn metadata_budget_accounts_normalized_keys_and_never_discards_physical_matches() {
    let items = vec![
        song("one.flac", Some("İ"), Some("界"), None),
        item("one.txt", false),
    ];
    let key = SearchEntityKey::Album {
        artist: "İ".to_lowercase(),
        album: "界".into(),
    };
    let bytes = std::mem::size_of::<GroupIndex>() + ENTITY_OVERHEAD_BYTES + key_bytes(&key);
    let fitting = GroupIndex::build_with_limits(
        &items,
        &ranking(&items),
        &query(""),
        &|| false,
        bytes,
        MAX_ENTITIES,
    )
    .unwrap();
    assert_eq!(
        fitting.counts(),
        SearchCounts {
            files: 1,
            songs: 1,
            albums: 1,
            artists: 0
        }
    );
    assert!(fitting.notice().is_some());
    let not_fitting = GroupIndex::build_with_limits(
        &items,
        &ranking(&items),
        &query(""),
        &|| false,
        bytes - 1,
        MAX_ENTITIES,
    )
    .unwrap();
    assert_eq!(
        not_fitting.counts(),
        SearchCounts {
            files: 1,
            songs: 1,
            albums: 0,
            artists: 1
        }
    );
    assert!(not_fitting.notice().is_some());
    let zero = GroupIndex::build_with_limits(&items, &ranking(&items), &query(""), &|| false, 0, 0)
        .unwrap();
    assert_eq!(
        zero.counts(),
        SearchCounts {
            files: 1,
            songs: 1,
            albums: 0,
            artists: 0
        }
    );
    assert_eq!(zero.page(&items, SearchGroup::All, 100).len(), 2);
}

#[test]
fn hitting_exactly_the_entity_limit_is_not_partial_until_a_new_entity_is_rejected() {
    let items = vec![song("one.flac", Some("Artist"), Some("Album"), None)];
    let grouped = GroupIndex::build_with_limits(
        &items,
        &ranking(&items),
        &query(""),
        &|| false,
        MAX_METADATA_BYTES,
        2,
    )
    .unwrap();
    assert!(grouped.notice().is_none());
    assert_eq!(grouped.counts().albums + grouped.counts().artists, 2);
}

#[test]
fn production_metadata_limit_bounds_aggregation_and_all_total_can_exceed_source_limit() {
    let label_suffix = "界".repeat(240);
    let mut items = Vec::new();
    for position in 0..20_001 {
        let label = format!("{position:05}{label_suffix}");
        items.push(song(
            &format!("{position:05}.flac"),
            Some(&label),
            Some(&label),
            None,
        ));
    }
    let grouped = index(&items);
    assert_eq!(grouped.counts().songs, items.len());
    assert!(grouped.counts().albums + grouped.counts().artists < MAX_ENTITIES);
    assert!(grouped.notice().is_some());
    let metadata_bytes = std::mem::size_of::<GroupIndex>()
        + grouped
            .albums
            .iter()
            .chain(&grouped.artists)
            .map(|position| ENTITY_OVERHEAD_BYTES + key_bytes(&position.key))
            .sum::<usize>();
    assert!(metadata_bytes <= MAX_METADATA_BYTES);
    assert_eq!(
        grouped.page(&items, SearchGroup::Songs, usize::MAX).len(),
        items.len()
    );
    let simple: Vec<_> = (0..40_001)
        .map(|position| item(&format!("{position:05}.txt"), false))
        .collect();
    let grouped = index(&simple);
    assert_eq!(grouped.counts().total(SearchGroup::All), 40_001);
    assert_eq!(
        grouped.page(&simple, SearchGroup::All, usize::MAX).len(),
        40_001
    );
    assert!(grouped.notice().is_none());
}

#[test]
fn forty_thousand_derived_entity_limit_is_shared_between_albums_and_artists() {
    let items: Vec<_> = (0..20_001)
        .map(|position| {
            let label = position.to_string();
            song(
                &format!("{position}.flac"),
                Some(&label),
                Some(&label),
                None,
            )
        })
        .collect();
    let production = index(&items);
    assert!(production.counts().total(SearchGroup::All) > 40_000);
    assert_eq!(production.counts().songs, items.len());
    let grouped = GroupIndex::build_with_limits(
        &items,
        &ranking(&items),
        &query(""),
        &|| false,
        usize::MAX,
        MAX_ENTITIES,
    )
    .unwrap();
    assert_eq!(
        grouped.counts().albums + grouped.counts().artists,
        MAX_ENTITIES
    );
    assert_eq!(grouped.counts().songs, items.len());
    assert_eq!(grouped.counts().total(SearchGroup::All), 60_001);
    assert!(grouped.notice().is_some());
}
