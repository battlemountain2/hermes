use super::*;
use crate::services::SearchScope;
use std::path::Path;

fn results(names: &[&str], indexing: bool) -> SearchEvent {
    SearchEvent::Results {
        generation: 1,
        query: "song".into(),
        indexing,
        notice: None,
        page: None,
        groups: None,
        items: names
            .iter()
            .map(|name| {
                crate::services::SearchItem::from_path(
                    Path::new("/Music").join(name),
                    Path::new("/Music"),
                    false,
                    None,
                )
            })
            .collect(),
    }
}

#[test]
fn grouped_payloads_cannot_be_mistaken_for_file_system_entries() {
    let mut event = results(&["real-song.flac"], false);
    let SearchEvent::Results { groups, .. } = &mut event;
    *groups = Some(crate::services::SearchGroups {
        group: crate::services::SearchGroup::Albums,
        results: Vec::new(),
        counts: crate::services::SearchCounts::default(),
    });
    let mut previous = None;
    let mut notice = None;
    let delivered = Cell::new(false);
    assert_eq!(
        deliver_results(
            event,
            "song",
            1,
            crate::services::RequestId(1),
            &mut previous,
            &mut notice,
            &Cell::new(true),
            &|_| delivered.set(true)
        ),
        glib::ControlFlow::Continue
    );
    assert!(!delivered.get());
    assert!(previous.is_none());
}

#[test]
fn search_tab_preserves_all_matches_and_ignores_page_metadata() {
    let events = std::cell::RefCell::new(Vec::new());
    let emit = |event| events.borrow_mut().push(event);
    let mut previous = None;
    let mut previous_notice = None;
    let names: Vec<_> = (0..205)
        .map(|number| format!("song-{number:03}.flac"))
        .collect();
    let borrowed: Vec<_> = names.iter().map(String::as_str).collect();
    let mut event = results(&borrowed, false);
    let SearchEvent::Results { page, .. } = &mut event;
    *page = Some(crate::services::SearchPage {
        generation: 1,
        revision: 8,
        shown: 205,
        total: 205,
    });
    assert_eq!(
        deliver_results(
            event,
            "song",
            1,
            crate::services::RequestId(12),
            &mut previous,
            &mut previous_notice,
            &Cell::new(true),
            &emit,
        ),
        glib::ControlFlow::Break
    );
    let events = events.borrow();
    let [
        DirectoryEvent::Snapshot { entries, .. },
        DirectoryEvent::Finished { .. },
    ] = events.as_slice()
    else {
        panic!("Expected the complete ranked snapshot followed by completion");
    };
    assert_eq!(entries.len(), 205);
    assert_eq!(
        entries
            .iter()
            .map(|entry| &entry.display_name)
            .collect::<Vec<_>>(),
        names.iter().collect::<Vec<_>>()
    );
}

#[test]
fn provisional_search_tab_results_replace_instead_of_accumulating() {
    let events = std::cell::RefCell::new(Vec::new());
    let emit = |event| events.borrow_mut().push(event);
    let active = Cell::new(true);
    let mut previous = None;
    let mut previous_notice = None;
    let request_id = crate::services::RequestId(12);
    assert_eq!(
        deliver_results(
            results(&["first.flac"], true),
            "song",
            1,
            request_id,
            &mut previous,
            &mut previous_notice,
            &active,
            &emit
        ),
        glib::ControlFlow::Continue
    );
    assert!(
        matches!(events.borrow().as_slice(), [DirectoryEvent::Snapshot { entries, .. }] if entries.len() == 1)
    );
    deliver_results(
        results(&["first.flac"], true),
        "song",
        1,
        request_id,
        &mut previous,
        &mut previous_notice,
        &active,
        &emit,
    );
    assert_eq!(events.borrow().len(), 1);
    deliver_results(
        results(&["second.flac"], true),
        "song",
        1,
        request_id,
        &mut previous,
        &mut previous_notice,
        &active,
        &emit,
    );
    assert_eq!(previous.as_ref().unwrap()[0].display_name, "second.flac");
    assert_eq!(
        deliver_results(
            results(&["second.flac"], false),
            "song",
            1,
            request_id,
            &mut previous,
            &mut previous_notice,
            &active,
            &emit
        ),
        glib::ControlFlow::Break
    );
    assert!(matches!(
        events.borrow().last(),
        Some(DirectoryEvent::Finished { .. })
    ));
    assert_eq!(events.borrow().len(), 3);
}

#[test]
fn stale_and_cancelled_search_updates_do_not_emit_finish_after_navigation() {
    let events = std::cell::RefCell::new(Vec::new());
    let active = Cell::new(true);
    let emit = |event| {
        events.borrow_mut().push(event);
        active.set(false);
    };
    let mut previous = None;
    let mut previous_notice = None;
    let request_id = crate::services::RequestId(12);
    deliver_results(
        results(&["song.flac"], false),
        "different",
        1,
        request_id,
        &mut previous,
        &mut previous_notice,
        &active,
        &emit,
    );
    assert!(events.borrow().is_empty());
    assert_eq!(
        deliver_results(
            results(&["song.flac"], false),
            "song",
            1,
            request_id,
            &mut previous,
            &mut previous_notice,
            &active,
            &emit
        ),
        glib::ControlFlow::Break
    );
    assert!(matches!(
        events.borrow().as_slice(),
        [DirectoryEvent::Snapshot { .. }]
    ));
    deliver_results(
        results(&[], false),
        "song",
        1,
        request_id,
        &mut previous,
        &mut previous_notice,
        &active,
        &emit,
    );
    assert_eq!(events.borrow().len(), 1);
}

#[test]
fn final_empty_partial_search_reports_failure_not_false_success() {
    let events = std::cell::RefCell::new(Vec::new());
    let emit = |event| events.borrow_mut().push(event);
    let mut previous = None;
    let mut previous_notice = None;
    let mut event = results(&[], false);
    let SearchEvent::Results { notice, .. } = &mut event;
    *notice = Some("Cannot search this scope".into());
    deliver_results(
        event,
        "song",
        1,
        crate::services::RequestId(12),
        &mut previous,
        &mut previous_notice,
        &Cell::new(true),
        &emit,
    );
    assert!(
        matches!(events.borrow().as_slice(), [DirectoryEvent::Snapshot { entries, .. }, DirectoryEvent::Notice { .. }, DirectoryEvent::Failed { .. }] if entries.is_empty())
    );
}

#[test]
fn legacy_search_uses_home_including_music() {
    let search = ScopedSearch::from_uri("search://modified:today").expect("Legacy search parses");
    assert_eq!(search.scope, SearchScope::Home);
    assert_eq!(search.root, glib::home_dir());
    assert_eq!(search.query, "modified:today");
}

#[test]
fn repeated_query_text_still_rejects_an_older_generation() {
    let events = std::cell::RefCell::new(Vec::new());
    let emit = |event| events.borrow_mut().push(event);
    let mut previous = None;
    let mut previous_notice = None;
    deliver_results(
        results(&["old.flac"], false),
        "song",
        3,
        crate::services::RequestId(12),
        &mut previous,
        &mut previous_notice,
        &Cell::new(true),
        &emit,
    );
    assert!(events.borrow().is_empty());
    assert!(previous.is_none());
}

#[test]
fn partial_notices_update_and_clear_without_hiding_matching_files() {
    let events = std::cell::RefCell::new(Vec::new());
    let emit = |event| events.borrow_mut().push(event);
    let mut previous = None;
    let mut previous_notice = None;
    let mut partial = results(&["song.flac"], true);
    let SearchEvent::Results { notice, .. } = &mut partial;
    *notice = Some("Some folders could not be indexed".into());
    let active = Cell::new(true);
    let request_id = crate::services::RequestId(12);
    let mut repeated = results(&["song.flac"], true);
    let SearchEvent::Results { notice, .. } = &mut repeated;
    *notice = Some("Some folders could not be indexed".into());
    for event in [partial, repeated, results(&["song.flac"], false)] {
        deliver_results(
            event,
            "song",
            1,
            request_id,
            &mut previous,
            &mut previous_notice,
            &active,
            &emit,
        );
    }
    assert!(matches!(events.borrow().as_slice(), [
        DirectoryEvent::Snapshot { entries, .. },
        DirectoryEvent::Notice { message: Some(_), .. },
        DirectoryEvent::Notice { message: None, .. },
        DirectoryEvent::Finished { .. },
    ] if entries.len() == 1));
}

#[test]
fn scoped_queries_round_trip_without_losing_filters_or_punctuation() {
    for scope in [
        SearchScope::MusicLibrary,
        SearchScope::Home,
        SearchScope::ThisFolder,
    ] {
        let search = ScopedSearch::new(
            scope,
            Path::new("/home/me/Album & More"),
            "artist name content:\"A & B? % #é\"",
        );
        assert_eq!(
            ScopedSearch::from_uri(&search.to_uri()).expect("Scoped URI parses"),
            search
        );
    }
}

#[test]
fn scoped_folder_uris_preserve_non_utf8_native_paths() {
    use std::os::unix::ffi::OsStringExt;
    let root = std::path::PathBuf::from(std::ffi::OsString::from_vec(
        b"/home/me/album-\xff".to_vec(),
    ));
    let search = ScopedSearch::new(SearchScope::ThisFolder, &root, "song");
    assert_eq!(ScopedSearch::from_uri(&search.to_uri()).unwrap(), search);
}

#[test]
fn malformed_or_ambiguous_scopes_are_rejected() {
    let source = SearchFileSource::new();
    for uri in [
        "file:///tmp",
        "search://?scope=no&q=x&root=/tmp",
        "search://?scope=folder&root=relative&q=x",
        "search://?scope=folder&root=/tmp&q=%GG",
        "search://?scope=folder&root=/tmp&q=x&q=y",
        "search://?scope=music&root=/tmp&q=x",
    ] {
        assert!(
            source.validate_location(&Location::uri(uri)).is_err(),
            "{uri}"
        );
    }
}

#[test]
fn invalid_uri_emits_a_failure_without_starting_a_worker() {
    let failures = Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = failures.clone();
    let _handle = SearchFileSource::new().enumerate(
        DirectoryRequest {
            id: crate::services::RequestId(7),
            location: Location::uri("search://?scope=invalid"),
            batch_size: 10,
            include_hidden: false,
        },
        Rc::new(move |event| captured.borrow_mut().push(event)),
    );
    assert!(matches!(
        failures.borrow().as_slice(),
        [DirectoryEvent::Failed {
            request_id: crate::services::RequestId(7),
            ..
        }]
    ));
}
