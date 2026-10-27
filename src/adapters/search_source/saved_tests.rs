use super::*;

#[test]
#[ignore = "Private XDG paths and GLib loop; saved query reopens and follows live file changes"]
fn saved_query_reopens_as_live_results_without_copying_files() {
    use crate::services::{
        SearchGroup, SearchScope,
        saved_searches::{Command, Store},
    };
    let root = std::env::temp_dir().join(format!("hermes-saved-query-live-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let photos = root.join("photos");
    std::fs::create_dir(&photos).unwrap();
    std::fs::write(photos.join("first.jpg"), b"Generated names-only fixture").unwrap();
    std::fs::write(photos.join("other.mp4"), b"Generated names-only fixture").unwrap();
    let path = root.join("private/queries.json");
    let store = Store::start(path);
    assert!(store.flush(Duration::from_secs(3)));
    let search = ScopedSearch::new(
        SearchScope::ThisFolder,
        &root,
        "type:image ext:jpg in:photos",
    )
    .with_group(SearchGroup::Files, None);
    store
        .send(Command::Create {
            name: "Trip photos".into(),
            uri: search.to_uri(),
        })
        .unwrap();
    assert!(store.flush(Duration::from_secs(3)));
    let saved = store.snapshot().entries.into_iter().next().unwrap();
    let reopened = store;
    reopened.send(Command::Reload).unwrap();
    assert!(reopened.flush(Duration::from_secs(3)));
    assert_eq!(reopened.snapshot().entries, [saved.clone()]);
    let expected = Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = expected.clone();
    let failures = Rc::new(std::cell::RefCell::new(Vec::new()));
    let errors = failures.clone();
    let source = SearchFileSource::new();
    let location = Location::uri(&saved.uri);
    source.validate_location(&location).unwrap();
    let handle = source.enumerate(
        DirectoryRequest {
            id: crate::services::RequestId(912),
            location,
            batch_size: 100,
            include_hidden: false,
        },
        Rc::new(move |event| match event {
            DirectoryEvent::Snapshot { entries, .. } => {
                captured.replace(entries);
            }
            DirectoryEvent::Failed { message, .. } => {
                errors.borrow_mut().push(message);
            }
            _ => {}
        }),
    );
    let wait_for = |name: &str| {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            assert!(failures.borrow().is_empty(), "{:?}", failures.borrow());
            if expected.borrow().len() == 1 && expected.borrow()[0].display_name == name {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "Missing {name}");
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    wait_for("first.jpg");
    std::fs::rename(photos.join("first.jpg"), photos.join("renamed.jpg")).unwrap();
    wait_for("renamed.jpg");
    assert_eq!(std::fs::read_dir(&photos).unwrap().count(), 2);
    assert!(photos.join("other.mp4").exists());
    drop(handle);
    drop(reopened);
    std::fs::remove_dir_all(root).unwrap();
}
