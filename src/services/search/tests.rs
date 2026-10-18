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

#[test]
fn category_filters_distinguish_formats_without_a_filename_term() {
    let root = Path::new("/generated-search");
    for (name, category) in [("song.FLAC", "audio"), ("clip.MOV", "video"), ("photo.HEIC", "image"), ("paper.pdf", "document"), ("game.AppImage", "application"), ("backup.zip", "archive")] {
        let candidate = SearchItem::from_path(root.join(name), root, false, None);
        assert!(fuzzy_score(&candidate, &format!("type:{category}"), root).is_some(), "{name}");
        let (_, values) = search_filter_values(&format!("artist type:{category}"));
        assert_eq!(values.category.as_deref(), Some(category));
        assert!(fuzzy_score(&candidate, "type:folder", root).is_none());
    }
    let song = SearchItem::from_path(root.join("song.flac"), root, false, None);
    assert!(fuzzy_score(&song, "type:image", root).is_none());
    assert!(fuzzy_score(&song, "type:audio ext:mp3", root).is_none());
    let folder = SearchItem::from_path(root.join("album.flac"), root, true, None);
    assert!(fuzzy_score(&folder, "type:audio", root).is_none());
}

struct FixtureTextExtraction;

struct FixtureOfficeTextExtraction;

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

impl TextExtractionProvider for FixtureOfficeTextExtraction {
    fn extract_text(
        &self,
        _path: &Path,
        extractor: TextExtractor,
        byte_limit: usize,
        _cancelled: Arc<AtomicBool>,
    ) -> Result<String, String> {
        assert_eq!(extractor, TextExtractor::Office);
        Ok("Office text extracted safely"[..byte_limit.min(28)].to_owned())
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
        music_tags: None,
        search_metadata: String::new(),
        source_identity: None,
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
fn artist_and_title_terms_match_in_any_order_across_tag_fields() {
    let mut candidate = item("/home/me/Music/Album/01.flac");
    candidate.search_metadata = "nikes frank ocean blonde".into();
    candidate.music_tags = Some(crate::model::MusicTags {
        title: Some("Nikes".into()), artist: Some("Frank Ocean".into()), album: Some("Blonde".into()),
        ..crate::model::MusicTags::default()
    });
    for query in ["Frank Ocean Nikes", "Nikes Ocean Frank", "Blonde Nikes Frank", "Frank Ocean 01.flac"] {
        assert!(fuzzy_score(&candidate, query, Path::new("/home/me")).is_some(), "{query}");
    }
    assert!(fuzzy_score(&candidate, "Frank Ocean absent", Path::new("/home/me")).is_none());
}

fn tagged_item(path: &str) -> SearchItem {
    let mut candidate = item(path);
    candidate.music_tags = Some(crate::model::MusicTags {
        artist: Some("Frank Ocean".into()),
        album: Some("Blonde".into()),
        title: Some("Nikes".into()),
        ..crate::model::MusicTags::default()
    });
    candidate.search_metadata = "nikes frank ocean blonde".into();
    candidate
}

#[test]
fn metadata_filter_values_parse_quotes_case_and_existing_filters() {
    let (terms, filters) = search_filter_values(
        "01 ARTIST:\"Frank Ocean\" album:'Blonde' TITLE:Nikes type:audio ext:.FLAC in:Music modified:today content:\"Hidden Phrase\"",
    );
    assert_eq!(terms, "01");
    assert_eq!(filters.artist.as_deref(), Some("frank ocean"));
    assert_eq!(filters.album.as_deref(), Some("blonde"));
    assert_eq!(filters.title.as_deref(), Some("nikes"));
    assert_eq!(filters.directory, Some(false));
    assert_eq!(filters.category.as_deref(), Some("audio"));
    assert_eq!(filters.extension.as_deref(), Some("flac"));
    assert_eq!(filters.within.as_deref(), Some("music"));
    assert_eq!(filters.modified.as_deref(), Some("today"));
    assert_eq!(filters.content.as_deref(), Some("hidden phrase"));
}

#[test]
fn metadata_filters_preserve_empty_unknown_and_repeated_token_behavior() {
    let (terms, filters) = search_filter_values(
        "artist: album:'' title:\"\" composer:bach artist:old artist:\"Frank Ocean\" album:old album:Blonde title:old title:Nikes",
    );
    assert_eq!(terms, "artist: album: title: composer:bach");
    assert_eq!(filters.artist.as_deref(), Some("frank ocean"));
    assert_eq!(filters.album.as_deref(), Some("blonde"));
    assert_eq!(filters.title.as_deref(), Some("nikes"));
    assert!(filters.content.is_none());
    assert!(filters.directory.is_none());
}

#[test]
fn metadata_filters_match_without_free_text() {
    let root = Path::new("/home/me");
    let candidate = tagged_item("/home/me/Music/01.flac");
    for query in [
        "artist:\"FRANK OCEAN\"",
        "album:BLONDE",
        "title:NIKES",
        "artist:'Frank Ocean' album:Blonde title:Nikes",
    ] {
        assert_eq!(fuzzy_score(&candidate, query, root), Some(0), "{query}");
    }
    assert!(fuzzy_score(&candidate, "", root).is_none());
}

#[test]
fn metadata_filters_require_the_exact_requested_tag() {
    let root = Path::new("/home/me");
    let candidate = tagged_item("/home/me/Music/Other Artist/Other Album/Other Title.flac");
    for query in [
        "artist:Frank",
        "artist:\"Frank Ocean Live\"",
        "album:Blond",
        "title:Nks",
        "artist:Nikes",
        "album:Nikes",
        "title:Blonde",
        "artist:\"Other Artist\"",
        "album:\"Other Album\"",
        "title:\"Other Title\"",
        "artist:'Frank Ocean' album:Other title:Nikes",
    ] {
        assert!(fuzzy_score(&candidate, query, root).is_none(), "{query}");
    }
    let mut missing = item("/home/me/Music/Frank Ocean/Blonde/Nikes.flac");
    missing.search_metadata = candidate.search_metadata.clone();
    for tags in [
        None,
        Some(crate::model::MusicTags {
            album_artist: Some("Frank Ocean".into()),
            ..crate::model::MusicTags::default()
        }),
    ] {
        missing.music_tags = tags;
        for query in ["artist:'Frank Ocean'", "album:Blonde", "title:Nikes"] {
            assert!(fuzzy_score(&missing, query, root).is_none(), "{query}");
        }
    }
}

#[test]
fn metadata_filters_compose_with_existing_filters_and_free_text_ranking() {
    let root = Path::new("/home/me");
    let candidate = tagged_item("/home/me/Music/01.flac");
    for terms in ["01.flac", "01", "0fc", "Ocean Nikes"] {
        assert_eq!(
            fuzzy_score(
                &candidate,
                &format!("artist:'Frank Ocean' album:Blonde title:Nikes type:audio ext:flac in:Music {terms}"),
                root,
            ),
            fuzzy_score(&candidate, terms, root),
            "{terms}",
        );
    }
    for query in [
        "artist:'Frank Ocean' type:video",
        "album:Blonde ext:mp3",
        "title:Nikes in:Pictures",
        "artist:'Frank Ocean' type:folder",
        "artist:'Frank Ocean' definitely-missing",
    ] {
        assert!(fuzzy_score(&candidate, query, root).is_none(), "{query}");
    }
}

#[test]
fn metadata_filters_apply_before_recent_date_shortcut() {
    let root = Path::new("/home/me");
    let mut candidate = tagged_item("/home/me/Music/01.flac");
    let (today_start, today_end) = recent_bounds("modified:today").expect("today should parse");
    candidate.modified_unix_seconds = Some(today_start);
    assert_eq!(
        fuzzy_score(&candidate, "artist:'Frank Ocean' modified:today", root),
        Some(today_start),
    );
    for query in [
        "artist:Other modified:today",
        "album:Other modified:today",
        "title:Other modified:today",
    ] {
        assert!(fuzzy_score(&candidate, query, root).is_none(), "{query}");
    }
    candidate.modified_unix_seconds = Some(today_end);
    assert!(fuzzy_score(&candidate, "title:Nikes modified:today", root).is_none());
}

#[test]
fn metadata_filters_support_unicode_punctuation_and_escaped_quotes() {
    let root = Path::new("/home/me");
    let mut candidate = item("/home/me/Music/01.flac");
    candidate.music_tags = Some(crate::model::MusicTags {
        artist: Some("Björk".into()),
        album: Some("It's Oh So Quiet".into()),
        title: Some("Live: \"Jóga\"".into()),
        ..crate::model::MusicTags::default()
    });
    let query = r#"artist:BJÖRK album:"It's Oh So Quiet" title:"Live: \"JÓGA\"""#;
    let (terms, filters) = search_filter_values(query);
    assert!(terms.is_empty());
    assert_eq!(filters.artist.as_deref(), Some("björk"));
    assert_eq!(filters.album.as_deref(), Some("it's oh so quiet"));
    assert_eq!(filters.title.as_deref(), Some("live: \"jóga\""));
    assert!(fuzzy_score(&candidate, query, root).is_some());
    assert!(fuzzy_score(&candidate, "artist:Bjork", root).is_none());
}

#[test]
fn metadata_filters_compose_with_opt_in_content_and_preserve_extraction_budget() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("hermes-metadata-content-{unique}.pdf"));
    fs::write(&path, b"not parsed outside the provider")
        .expect("the PDF fixture should be written");
    let candidate = tagged_item(&path.to_string_lossy());
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut remaining = 1;
    let mut notice = None;
    let mut context = super::ContentSearchContext {
        text_extraction: &FixtureTextExtraction,
        cancelled: &cancelled,
        document_extractions_remaining: &mut remaining,
        notice: &mut notice,
    };
    for query in [
        "artist:other content:'extracted safely'",
        "album:other content:'extracted safely'",
        "title:other content:'extracted safely'",
    ] {
        assert!(
            super::score_item(
                &candidate,
                &super::ParsedSearchQuery::parse(query),
                &mut context,
            )
            .is_none()
        );
        assert_eq!(*context.document_extractions_remaining, 1);
    }
    let query = super::ParsedSearchQuery::parse(
        "artist:'frank ocean' album:blonde title:nikes type:document ext:pdf content:'extracted safely'",
    );
    assert!(super::score_item(&candidate, &query, &mut context).is_some());
    assert_eq!(*context.document_extractions_remaining, 0);
    assert!(context.notice.is_none());
    assert!(super::score_item(&candidate, &query, &mut context).is_none());
    assert!(
        context
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("limited"))
    );
    *context.document_extractions_remaining = 1;
    let missing_content = super::ParsedSearchQuery::parse("title:nikes content:'missing phrase'");
    assert!(super::score_item(&candidate, &missing_content, &mut context).is_none());
    fs::remove_file(path).expect("the PDF fixture should be removed");
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

#[test]
fn background_index_searches_office_text_through_the_provider() {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("the system clock should be after the Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("hermes-office-index-{unique}"));
    fs::create_dir_all(&root).expect("the office search fixture should be created");
    fs::write(root.join("letter.docx"), b"not parsed outside the provider")
        .expect("the office fixture should be written");

    let (search, events) = index_tree(root.clone(), Arc::new(FixtureOfficeTextExtraction));
    search.query("content:\"office text\"");
    let found = (0..30).any(|_| {
        events.recv_timeout(Duration::from_millis(100)).is_ok_and(
            |SearchEvent::Results { query, items, .. }| {
                query == "content:\"office text\""
                    && items.iter().any(|item| item.name == "letter.docx")
            },
        )
    });

    drop(search);
    fs::remove_dir_all(root).expect("the office search fixture should be removed");
    assert!(
        found,
        "the worker should search office text through its provider"
    );
}

#[test]
fn accented_diacritics_fold_in_free_text_matching() {
    let root = Path::new("/home/me");
    let mut bjork = item("/home/me/Music/Björk/Homogenic/Jóga.flac");
    bjork.name = "Jóga.flac".into();
    bjork.search_name = "jóga.flac".into();
    bjork.search_metadata = "jóga björk homogenic".into();
    bjork.music_tags = Some(crate::model::MusicTags {
        artist: Some("Björk".into()),
        album: Some("Homogenic".into()),
        title: Some("Jóga".into()),
        ..crate::model::MusicTags::default()
    });

    assert!(fuzzy_score(&bjork, "bjork", root).is_some());
    assert!(fuzzy_score(&bjork, "joga", root).is_some());
    assert!(fuzzy_score(&bjork, "bjork joga", root).is_some());
    assert!(fuzzy_score(&bjork, "Bjork", root).is_some());

    let mut cafe = item("/home/me/Documents/Café.pdf");
    cafe.name = "Café.pdf".into();
    cafe.search_name = "café.pdf".into();
    assert!(fuzzy_score(&cafe, "cafe", root).is_some());

    let mut beyonce = item("/home/me/Music/Beyoncé/Renaissance/Break.mp3");
    beyonce.name = "Break.mp3".into();
    beyonce.search_name = "break.mp3".into();
    beyonce.search_metadata = "break beyoncé renaissance".into();
    assert!(fuzzy_score(&beyonce, "beyonce", root).is_some());

    let mut heritage = item("/home/me/Notes/L'Héritage.txt");
    heritage.name = "L'Héritage.txt".into();
    heritage.search_name = "l'héritage.txt".into();
    assert!(fuzzy_score(&heritage, "heritage", root).is_some());
}

#[test]
fn bounded_typo_tolerance_matches_free_words_at_least_four_chars() {
    let root = Path::new("/home/me");
    let mut candidate = item("/home/me/Music/Cameron Winter/Heavy Metal/01.flac");
    candidate.name = "01.flac".into();
    candidate.search_name = "01.flac".into();
    candidate.search_metadata = "cameron winter heavy metal".into();
    candidate.music_tags = Some(crate::model::MusicTags {
        artist: Some("Cameron Winter".into()),
        album: Some("Heavy Metal".into()),
        title: Some("01".into()),
        ..crate::model::MusicTags::default()
    });

    assert!(fuzzy_score(&candidate, "Camron Winter", root).is_some());
    assert!(fuzzy_score(&candidate, "Cameron Wintr", root).is_some());

    let exact_score = fuzzy_score(&candidate, "Cameron Winter", root).unwrap();
    let typo_score = fuzzy_score(&candidate, "Camron Winter", root).unwrap();
    assert!(exact_score > typo_score);

    let mut beach_house = item("/home/me/Music/Beach House/Bloom/Myth.flac");
    beach_house.name = "Myth.flac".into();
    beach_house.search_name = "myth.flac".into();
    beach_house.search_metadata = "myth beach house bloom".into();
    assert!(fuzzy_score(&beach_house, "Beach Hose", root).is_some());

    let short = item("/home/me/cat.txt");
    assert!(fuzzy_score(&short, "car", root).is_none());

    assert!(fuzzy_score(&candidate, "Cmrn Wntr", root).is_none());
}

#[test]
fn reordered_metadata_matches_beat_incidental_path_matches() {
    let root = Path::new("/home/me");
    let mut metadata_match = item("/home/me/Music/Library/track01.flac");
    metadata_match.name = "track01.flac".into();
    metadata_match.search_name = "track01.flac".into();
    metadata_match.search_metadata = "nikes frank ocean blonde".into();
    metadata_match.music_tags = Some(crate::model::MusicTags {
        artist: Some("Frank Ocean".into()),
        album: Some("Blonde".into()),
        title: Some("Nikes".into()),
        ..crate::model::MusicTags::default()
    });

    let mut incidental_path = item("/home/me/frank/ocean/archive/document.txt");
    incidental_path.name = "document.txt".into();
    incidental_path.search_name = "document.txt".into();

    let meta_score = fuzzy_score(&metadata_match, "Ocean Frank", root).unwrap();
    let path_score = fuzzy_score(&incidental_path, "Ocean Frank", root).unwrap();
    assert!(meta_score > path_score, "Metadata tag match should rank above incidental path hit");
}

#[test]
fn phrase_interpretation_extracts_bounded_filters_and_leaves_unquoted_terms() {
    let (terms, filters) = search_filter_values("Beach House songs");
    assert_eq!(terms, "Beach House");
    assert_eq!(filters.category.as_deref(), Some("audio"));
    assert_eq!(filters.inferred.len(), 1);
    assert_eq!(filters.inferred[0].label, "Audio");

    let (terms, filters) = search_filter_values("PDFs in Downloads");
    assert_eq!(terms, "");
    assert_eq!(filters.category.as_deref(), Some("document"));
    assert_eq!(filters.extension.as_deref(), Some("pdf"));
    assert_eq!(filters.within.as_deref(), Some("downloads"));
    assert_eq!(filters.inferred.len(), 2);

    let (terms, filters) = search_filter_values("videos from last week");
    assert_eq!(terms, "");
    assert_eq!(filters.category.as_deref(), Some("video"));
    assert_eq!(filters.modified.as_deref(), Some("last-week"));
    assert_eq!(filters.inferred.len(), 2);

    let (terms, filters) = search_filter_values("FLAC songs by Cameron Winter");
    assert_eq!(terms, "");
    assert_eq!(filters.extension.as_deref(), Some("flac"));
    assert_eq!(filters.category.as_deref(), Some("audio"));
    assert_eq!(filters.artist.as_deref(), Some("cameron winter"));
    assert_eq!(filters.inferred.len(), 3);
}

#[test]
fn protected_titles_and_quoted_literals_do_not_over_interpret() {
    let (terms, filters) = search_filter_values("\"Yesterday\" songs");
    assert_eq!(terms, "Yesterday");
    assert_eq!(filters.category.as_deref(), Some("audio"));
    assert!(filters.modified.is_none());

    let (terms, filters) = search_filter_values("Yesterday");
    assert_eq!(terms, "Yesterday");
    assert!(filters.modified.is_none());

    let (terms, filters) = search_filter_values("Songs for the Deaf");
    assert_eq!(terms, "Songs for the Deaf");
    assert!(filters.category.is_none());
}

#[test]
fn calendar_week_bounds_vs_rolling_seven_days() {
    let dt = gtk::glib::DateTime::from_local(2026, 10, 15, 14, 30, 0.0).expect("datetime valid");
    let (last_week_start, last_week_end) = super::recent_bounds_at("modified:last-week", &dt)
        .expect("last-week bounds should calculate");
    let (past_7_start, past_7_end) = super::recent_bounds_at("modified:past-7-days", &dt)
        .expect("past-7-days bounds should calculate");

    assert_eq!(last_week_end - last_week_start, 7 * 86400);
    assert_eq!(past_7_end - past_7_start, 7 * 86400);
    assert_eq!(past_7_end, dt.to_unix());
    assert!(last_week_start < past_7_start);
    assert!(past_7_start < last_week_end);
    assert!(last_week_end < past_7_end);
}

#[test]
fn inferred_types_preserve_literal_names_and_rank_them_first() {
    let root = Path::new("/synthetic");
    let literal = item("/synthetic/Video Games.flac");
    let video = item("/synthetic/Games.mp4");
    assert!(fuzzy_score(&literal, "Video Games", root).unwrap()
        > fuzzy_score(&video, "Video Games", root).unwrap());
    for name in ["Photos of You.flac", "Documents of Love.flac", "Yesterday songs.txt"] {
        let candidate = item(&format!("/synthetic/{name}"));
        let title = Path::new(name).file_stem().unwrap().to_str().unwrap();
        assert!(fuzzy_score(&candidate, title, root).is_some(), "{title}");
    }
}

#[test]
fn inferred_filters_preserve_literal_metadata_without_matching_folder_noise() {
    let root = Path::new("/synthetic");
    let mut song = item("/synthetic/01.flac");
    song.search_metadata = "video games lana del rey lost in the dream".into();
    assert!(fuzzy_score(&song, "Video Games", root).is_some());
    assert!(fuzzy_score(&song, "Lost in the Dream", root).is_some());
    let (terms, filters) = search_filter_values("Lost in the Dream");
    assert_eq!(terms, "Lost in the Dream");
    assert_eq!(filters.within, None);
    let noise = item("/synthetic/Video Games/other.flac");
    assert!(fuzzy_score(&noise, "Video Games", root).is_none());
}

#[test]
fn explicit_constraints_stay_strict_for_literal_and_inferred_matches() {
    let root = Path::new("/synthetic");
    let literal = item("/synthetic/Video Games.flac");
    for query in ["Video Games type:video", "Video Games ext:mp3", "Video Games artist:Other"] {
        assert!(fuzzy_score(&literal, query, root).is_none(), "{query}");
    }
    let mut directory = item("/synthetic/videos");
    directory.is_directory = true;
    let (terms, filters) = search_filter_values("type:folder videos");
    assert_eq!(terms, "videos");
    assert_eq!(filters.category, None);
    assert!(fuzzy_score(&directory, "type:folder videos", root).is_some());
    let (terms, filters) = search_filter_values("type:audio Video Games");
    assert_eq!(terms, "Video Games");
    assert_eq!(filters.category.as_deref(), Some("audio"));
    assert!(filters.inferred.is_empty());
}

#[test]
fn artist_phrase_matches_songs_without_a_literal_song_word() {
    let root = Path::new("/synthetic");
    let mut song = item("/synthetic/01.flac");
    song.music_tags = Some(crate::model::MusicTags {
        artist: Some("Cameron Winter".into()),
        ..Default::default()
    });
    song.search_metadata = "cameron winter heavy metal".into();
    for query in ["songs by Cameron Winter", "ext:flac songs by Cameron Winter"] {
        let (terms, filters) = search_filter_values(query);
        assert_eq!(terms, "", "{query}");
        assert_eq!(filters.artist.as_deref(), Some("cameron winter"));
        assert_eq!(filters.category.as_deref(), Some("audio"));
        assert!(fuzzy_score(&song, query, root).is_some(), "{query}");
    }
}

#[test]
fn inferred_locations_include_the_current_scope_root() {
    let root = Path::new("/synthetic/Downloads");
    let pdf = SearchItem::from_path(root.join("report.pdf"), root, false, None);
    assert!(fuzzy_score(&pdf, "PDFs in Downloads", root).is_some());
    assert!(fuzzy_score(&pdf, "PDFs in Documents", root).is_none());
}

#[test]
fn literal_titles_rank_above_inferred_date_only_results() {
    let root = Path::new("/synthetic");
    let literal = item("/synthetic/videos from last week.flac");
    let mut video = item("/synthetic/clip.mp4");
    let now = gio::glib::DateTime::now_local().unwrap();
    let (start, _) = super::recent_bounds_at("modified:last-week", &now).unwrap();
    video.modified_unix_seconds = Some(start + 1);
    let query = "videos from last week";
    assert!(fuzzy_score(&literal, query, root).unwrap()
        > fuzzy_score(&video, query, root).unwrap());
    assert!(fuzzy_score(&literal, "videos from last week modified:last-week", root).is_none());
}

#[test]
fn quoted_terms_stay_literal_through_parsing_and_filter_reconstruction() {
    let root = Path::new("/synthetic");
    let candidate = item("/synthetic/Cameron Winter.flac");
    assert!(fuzzy_score(&candidate, "Camerom", root).is_some());
    assert!(fuzzy_score(&candidate, "\"Camerom\"", root).is_none());
    assert!(fuzzy_score(&candidate, "\"Winter Cameron\"", root).is_none());
    let (terms, filters) = search_filter_values("\"ext:flac\"");
    assert_eq!(terms, "ext:flac");
    assert_eq!(filters.extension, None);
    let query = "\"Video Games\" type:audio \"ext:flac\"";
    assert_eq!(super::search_filter_terms(query), "\"Video Games\" \"ext:flac\"");
    let reconstructed = format!("{} type:audio", super::search_filter_terms(query));
    assert!(search_filter_values(&reconstructed).1.inferred.is_empty());
}

#[test]
fn inferred_chip_dismissal_uses_original_unicode_token_positions() {
    for (query, expected) in [
        ("İ songs", "İ \"songs\""),
        ("  İ   songs  ", "  İ   \"songs\"  "),
        ("\"songs\" songs", "\"songs\" \"songs\""),
        ("songsong songs", "songsong \"songs\""),
        ("İ from   last week", "İ \"from last week\""),
    ] {
        let (_, filters) = search_filter_values(query);
        let chip = filters.inferred.first().unwrap();
        assert_eq!(chip.literal_query(query).as_deref(), Some(expected), "{query}");
        assert!(search_filter_values(expected).1.inferred.is_empty(), "{expected}");
        assert_eq!(chip.literal_query("a different query"), None);
    }
}

#[test]
fn inferred_chip_dismissal_preserves_interleaved_explicit_filters_and_escapes() {
    let query = "songs by Cameron ext:flac Winter";
    let (_, filters) = search_filter_values(query);
    let artist = filters.inferred.iter().find(|chip| chip.label.starts_with("Artist:")).unwrap();
    let literal = artist.literal_query(query).unwrap();
    assert_eq!(literal, "songs \"by Cameron\" ext:flac \"Winter\"");
    assert_eq!(search_filter_values(&literal).1.extension.as_deref(), Some("flac"));
    let escaped = "songs by Artist\\\"Name";
    let (_, filters) = search_filter_values(escaped);
    let artist = filters.inferred.iter().find(|chip| chip.label.starts_with("Artist:")).unwrap();
    let literal = artist.literal_query(escaped).unwrap();
    assert!(literal.contains("Artist\\\"Name"));
    assert!(search_filter_values(&literal).1.artist.is_none());
}

