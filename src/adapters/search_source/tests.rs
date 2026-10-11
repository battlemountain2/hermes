use super::*;
use crate::services::SearchScope;
use std::path::Path;

fn results(names: &[&str], indexing: bool) -> SearchEvent {
    SearchEvent::Results {
        query: "song".into(),
        indexing,
        notice: None,
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
fn provisional_search_tab_results_replace_instead_of_accumulating() {
    let events = std::cell::RefCell::new(Vec::new());
    let emit = |event| events.borrow_mut().push(event);
    let active = Cell::new(true);
    let mut previous = None;
    let request_id = crate::services::RequestId(12);
    assert_eq!(
        deliver_results(
            results(&["first.flac"], true),
            "song",
            request_id,
            &mut previous,
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
        request_id,
        &mut previous,
        &active,
        &emit,
    );
    assert_eq!(events.borrow().len(), 1);
    deliver_results(
        results(&["second.flac"], true),
        "song",
        request_id,
        &mut previous,
        &active,
        &emit,
    );
    assert_eq!(previous.as_ref().unwrap()[0].display_name, "second.flac");
    assert_eq!(
        deliver_results(
            results(&["second.flac"], false),
            "song",
            request_id,
            &mut previous,
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
    let request_id = crate::services::RequestId(12);
    deliver_results(
        results(&["song.flac"], false),
        "different",
        request_id,
        &mut previous,
        &active,
        &emit,
    );
    assert!(events.borrow().is_empty());
    assert_eq!(
        deliver_results(
            results(&["song.flac"], false),
            "song",
            request_id,
            &mut previous,
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
        request_id,
        &mut previous,
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
    let mut event = results(&[], false);
    let SearchEvent::Results { notice, .. } = &mut event;
    *notice = Some("Cannot search this scope".into());
    deliver_results(
        event,
        "song",
        crate::services::RequestId(12),
        &mut previous,
        &Cell::new(true),
        &emit,
    );
    assert!(
        matches!(events.borrow().as_slice(), [DirectoryEvent::Snapshot { entries, .. }, DirectoryEvent::Failed { .. }] if entries.is_empty())
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
