use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt, symlink},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::services::{SearchEntityKey, SearchGroup, SearchScope};

use super::*;

const TIMEOUT: Duration = Duration::from_secs(5);

struct Fixture {
    root: PathBuf,
    workers: Mutex<Vec<Weak<Mutex<Snapshot>>>>,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self {
            root: std::env::temp_dir().join(format!(
                "strata-saved-searches-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
            workers: Mutex::default(),
        }
    }

    fn path(&self) -> PathBuf {
        self.root.join("queries-v1.json")
    }

    fn store(&self) -> Store {
        let store = Store::start(self.path());
        self.workers
            .lock()
            .unwrap()
            .push(Arc::downgrade(&store.0.snapshot));
        store
    }

    fn write(&self, bytes: &[u8]) {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.root)
            .unwrap();
        fs::write(self.path(), bytes).unwrap();
        fs::set_permissions(self.path(), fs::Permissions::from_mode(0o600)).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        while self
            .workers
            .lock()
            .unwrap()
            .iter()
            .any(|worker| worker.strong_count() != 0)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        let _removed = fs::remove_dir_all(&self.root);
    }
}

fn retire(store: Store) {
    let worker = Arc::downgrade(&store.0.snapshot);
    drop(store);
    let deadline = Instant::now() + TIMEOUT;
    while worker.strong_count() != 0 {
        assert!(
            Instant::now() < deadline,
            "saved search worker did not drain"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn scoped() -> ScopedSearch {
    ScopedSearch::new(
        SearchScope::ThisFolder,
        Path::new("/synthetic/offline/music"),
        "artist:\"A & B\" type:audio",
    )
}

#[test]
fn validation_errors_can_be_corrected_without_reloading_or_losing_saved_queries() {
    let fixture = Fixture::new();
    let store = fixture.store();
    assert!(store.flush(TIMEOUT));
    let first = send_ok(&store, create("Photos"));
    let error = send_error(&store, create("PHOTOS"));
    assert!(error.writable);
    assert_eq!(error.entries, first.entries);
    let corrected = send_ok(&store, create("Videos"));
    assert!(corrected.writable);
    assert_eq!(corrected.entries.len(), 2);
    assert_eq!(corrected.entries[0], first.entries[0]);
}

fn create(name: &str) -> Command {
    Command::Create {
        name: name.into(),
        uri: scoped().to_uri(),
    }
}

fn send_ok(store: &Store, command: Command) -> Snapshot {
    store.send(command).unwrap();
    assert!(store.flush(TIMEOUT), "{:?}", store.snapshot().error);
    let snapshot = store.snapshot();
    assert!(snapshot.ready);
    assert!(snapshot.error.is_none());
    snapshot
}

fn send_error(store: &Store, command: Command) -> Snapshot {
    store.send(command).unwrap();
    assert!(!store.flush(TIMEOUT));
    let snapshot = store.snapshot();
    assert!(snapshot.ready);
    assert!(snapshot.error.is_some());
    snapshot
}

#[test]
fn round_trip_canonicalizes_physical_scopes_and_preserves_song_membership() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut searches = Vec::new();
    for scope in [
        SearchScope::ThisFolder,
        SearchScope::Home,
        SearchScope::MusicLibrary,
    ] {
        for group in [SearchGroup::All, SearchGroup::Files, SearchGroup::Songs] {
            searches.push(
                ScopedSearch::new(scope, &scoped().root, "  artist:\"A & B\"  ")
                    .with_group(group, None),
            );
        }
    }
    for entity in [
        SearchEntityKey::Artist("björk & friends".into()),
        SearchEntityKey::Album {
            artist: "various artists".into(),
            album: "echoes = 1?".into(),
        },
        SearchEntityKey::Album {
            artist: String::new(),
            album: "unknown performer".into(),
        },
    ] {
        searches.push(scoped().with_group(SearchGroup::Songs, Some(entity)));
    }
    for (position, search) in searches.iter().enumerate() {
        let uri = if search.group == SearchGroup::All {
            format!("{}&group=all", search.to_uri())
        } else {
            search.to_uri()
        };
        send_ok(
            &store,
            Command::Create {
                name: format!(" Query {position} "),
                uri,
            },
        );
    }
    let entries = store.snapshot().entries;
    for (position, (entry, search)) in entries.iter().zip(&searches).enumerate() {
        assert_eq!(entry.id, position as u64 + 1);
        assert_eq!(entry.name, format!("Query {position}"));
        assert_eq!(entry.uri, search.to_uri());
        assert_eq!(ScopedSearch::from_uri(&entry.uri).unwrap(), *search);
    }
    let document: Document = serde_json::from_slice(&fs::read(fixture.path()).unwrap()).unwrap();
    assert_eq!(document.entries, entries);
    retire(store);
    let restarted = fixture.store();
    assert!(restarted.flush(TIMEOUT));
    assert_eq!(restarted.snapshot().entries, entries);
    assert_eq!(
        fs::metadata(&fixture.root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(fixture.path()).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(fixture.path().with_extension("json.lock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn unicode_names_casefold_trim_and_reject_controls_and_duplicates() {
    let mut data = Document::default();
    data.apply(create("  Straße  ")).unwrap();
    assert_eq!(data.entries[0].name, "Straße");
    for duplicate in ["STRASSE", " strasse ", "straße"] {
        assert!(data.apply(create(duplicate)).is_err());
    }
    data.apply(create("Σ")).unwrap();
    assert!(data.apply(create("ς")).is_err());
    data.apply(create(&"音".repeat(80))).unwrap();
    for name in [
        "",
        "   ",
        "bad\nname",
        "\ntrimmed",
        "trailing\t",
        "bad\0name",
    ] {
        assert!(data.apply(create(name)).is_err());
    }
    assert!(data.apply(create(&"音".repeat(81))).is_err());
    assert!(data.apply(create(&" ".repeat(MAX_NAME_BYTES + 1))).is_err());
    let duplicate = data.entries[1].clone();
    assert!(
        data.apply(Command::Update {
            previous: duplicate,
            name: "Strasse".into(),
            uri: scoped().to_uri(),
        })
        .is_err()
    );
}

#[test]
fn edit_delete_use_exact_preconditions_and_never_reuse_identifiers() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let original = send_ok(&store, create("First")).entries.remove(0);
    let mut replacement = scoped().with_group(SearchGroup::Files, None);
    replacement.query = "ext:rs".into();
    let updated = send_ok(
        &store,
        Command::Update {
            previous: original.clone(),
            name: "Renamed".into(),
            uri: replacement.to_uri(),
        },
    )
    .entries
    .remove(0);
    assert_eq!(updated.id, original.id);
    for stale in [
        Command::Delete(original.clone()),
        Command::Update {
            previous: original,
            name: "Lost edit".into(),
            uri: scoped().to_uri(),
        },
    ] {
        assert_eq!(send_error(&store, stale).entries, vec![updated.clone()]);
    }
    let unchanged = send_ok(
        &store,
        Command::Update {
            previous: updated.clone(),
            name: updated.name.clone(),
            uri: updated.uri.clone(),
        },
    );
    let revision = unchanged.revision;
    send_ok(
        &store,
        Command::Update {
            previous: updated.clone(),
            name: updated.name.clone(),
            uri: updated.uri.clone(),
        },
    );
    assert_eq!(store.revision(), revision + 1);
    assert!(
        send_ok(&store, Command::Delete(updated.clone()))
            .entries
            .is_empty()
    );
    assert!(
        send_error(&store, Command::Delete(updated))
            .entries
            .is_empty()
    );
    let created = send_ok(&store, create("Again")).entries.remove(0);
    assert_eq!(created.id, 2);
    retire(store);
    let restarted = fixture.store();
    let entries = send_ok(&restarted, create("Third")).entries;
    assert_eq!(
        entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        vec![2, 3]
    );
}

#[test]
fn entry_id_and_location_limits_reject_without_unbounded_snapshots() {
    let mut data = Document::default();
    for position in 0..MAX_ENTRIES {
        data.apply(create(&format!("Query {position}"))).unwrap();
    }
    assert!(data.apply(create("Overflow")).is_err());
    assert!(data.validate().is_ok());
    data.next_id = u64::MAX;
    let previous = data.entries.remove(0);
    assert!(data.apply(create("Exhausted")).is_err());
    data.entries.push(previous.clone());
    data.entries[0].id = 0;
    assert!(data.validate().is_err());
    data.entries[0] = previous;
    assert!(data.validate().is_err());
    let base = scoped().to_uri();
    for uri in [
        "file:///synthetic/not-a-query".into(),
        format!("{base}&group=albums"),
        format!("{base}&group=artists"),
        format!("{base}&group=files&artist-group=artist"),
        format!("{base}&q=duplicate"),
        "search://?scope=folder&root=relative&q=hello".into(),
        "search://?scope=folder&root=file%3A%2F%2F%2Fsynthetic&q=%00".into(),
        format!("search://{}", "q".repeat(MAX_QUERY_BYTES + 1)),
        format!(
            "search://?scope=folder&root=/{}&q=valid",
            "r".repeat(MAX_ROOT_BYTES)
        ),
        "x".repeat(MAX_URI_BYTES + 1),
    ] {
        assert!(canonical_uri(&uri).is_err(), "{uri}");
    }
}

#[test]
fn corrupt_future_and_invalid_documents_preserve_bytes_until_explicit_reload() {
    let mut duplicate = Document::default();
    duplicate.apply(create("One")).unwrap();
    duplicate.entries.push(duplicate.entries[0].clone());
    for bytes in [
        b"{not-json".to_vec(),
        br#"{"version":2,"next_id":1,"entries":[]}"#.to_vec(),
        br#"{"version":1,"next_id":0,"entries":[]}"#.to_vec(),
        serde_json::to_vec(&duplicate).unwrap(),
        vec![b' '; MAX_FILE_BYTES as usize + 1],
    ] {
        let fixture = Fixture::new();
        fixture.write(&bytes);
        let store = fixture.store();
        assert!(!store.flush(TIMEOUT));
        assert!(store.snapshot().ready);
        let revision = store.revision();
        assert!(send_error(&store, create("Blocked")).entries.is_empty());
        assert_eq!(store.revision(), revision + 1);
        assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
        let repaired = serde_json::to_vec(&Document::default()).unwrap();
        fixture.write(&repaired);
        assert!(
            send_error(&store, create("Still blocked"))
                .entries
                .is_empty()
        );
        assert_eq!(fs::read(fixture.path()).unwrap(), repaired);
        send_ok(&store, Command::Reload);
        assert_eq!(send_ok(&store, create("Recovered")).entries.len(), 1);
    }
}

#[test]
fn external_edits_and_deletions_are_never_overwritten_and_reload_recovers() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let original = send_ok(&store, create("Original")).entries;
    let mut external = Document::default();
    external.apply(create("External")).unwrap();
    let bytes = serde_json::to_vec_pretty(&external).unwrap();
    fixture.write(&bytes);
    assert_eq!(
        send_error(&store, create("Do not overwrite")).entries,
        original
    );
    assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
    assert_eq!(
        send_error(&store, Command::Delete(original[0].clone())).entries,
        original
    );
    assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
    let restored = send_ok(&store, Command::Reload);
    assert_eq!(restored.entries, external.entries);
    send_ok(&store, create("After reload"));
    fs::remove_file(fixture.path()).unwrap();
    assert_eq!(
        send_error(&store, create("Do not recreate")).entries.len(),
        2
    );
    assert!(!fixture.path().exists());
    assert!(send_ok(&store, Command::Reload).entries.is_empty());
    send_ok(&store, create("Explicit recreation"));
    assert!(fixture.path().exists());
}

#[test]
fn external_creation_and_format_only_edits_require_reload_even_for_noop_updates() {
    let fixture = Fixture::new();
    let store = fixture.store();
    assert!(store.flush(TIMEOUT));
    assert!(!fixture.path().exists());
    let bytes = serde_json::to_vec(&Document::default()).unwrap();
    fixture.write(&bytes);
    assert!(
        send_error(&store, create("External creation"))
            .entries
            .is_empty()
    );
    assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
    send_ok(&store, Command::Reload);
    let previous = send_ok(&store, create("One")).entries.remove(0);
    let document: Document = serde_json::from_slice(&fs::read(fixture.path()).unwrap()).unwrap();
    let reformatted = serde_json::to_vec_pretty(&document).unwrap();
    fixture.write(&reformatted);
    let failed = send_error(
        &store,
        Command::Update {
            previous: previous.clone(),
            name: previous.name.clone(),
            uri: previous.uri.clone(),
        },
    );
    assert_eq!(failed.entries, vec![previous.clone()]);
    assert_eq!(fs::read(fixture.path()).unwrap(), reformatted);
    send_ok(&store, Command::Reload);
    send_ok(
        &store,
        Command::Update {
            previous: previous.clone(),
            name: previous.name.clone(),
            uri: previous.uri.clone(),
        },
    );
    assert_eq!(fs::read(fixture.path()).unwrap(), reformatted);
}

#[test]
fn privacy_permission_changes_never_publish_uncommitted_changes() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let entries = send_ok(&store, create("Committed")).entries;
    let bytes = fs::read(fixture.path()).unwrap();
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o500)).unwrap();
    assert_eq!(send_error(&store, create("Not committed")).entries, entries);
    assert_eq!(fs::read(fixture.path()).unwrap(), bytes);
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o700)).unwrap();
    send_error(&store, create("Still requires reload"));
    send_ok(&store, Command::Reload);
    assert_eq!(send_ok(&store, create("Recovered")).entries.len(), 2);
}

#[test]
fn independent_store_ownership_is_exclusive_and_reload_retries_after_release() {
    let fixture = Fixture::new();
    let owner = fixture.store();
    let original = send_ok(&owner, create("Owned")).entries;
    let follower = fixture.store();
    assert!(!follower.flush(TIMEOUT));
    assert!(follower.snapshot().error.unwrap().contains("owner"));
    assert!(
        send_error(&follower, create("Cannot write"))
            .entries
            .is_empty()
    );
    let clone = owner.clone();
    retire(follower);
    drop(owner);
    let follower = fixture.store();
    assert!(!follower.flush(TIMEOUT));
    retire(clone);
    assert_eq!(send_ok(&follower, Command::Reload).entries, original);
    send_ok(&follower, create("New owner"));
}

#[test]
fn last_store_drop_drains_pending_commands_without_a_worker_reference_cycle() {
    let fixture = Fixture::new();
    let store = fixture.store();
    assert!(store.flush(TIMEOUT));
    for position in 0..QUEUE_CAPACITY {
        store.send(create(&format!("Pending {position}"))).unwrap();
    }
    retire(store);
    let restarted = fixture.store();
    assert!(restarted.flush(TIMEOUT));
    assert_eq!(restarted.snapshot().entries.len(), QUEUE_CAPACITY);
    assert_eq!(
        send_ok(&restarted, create("After draining"))
            .entries
            .last()
            .unwrap()
            .id,
        9
    );
}

#[test]
fn revision_acknowledges_every_command_but_not_barriers_and_snapshots_are_independent() {
    let fixture = Fixture::new();
    let store = fixture.store();
    assert!(store.flush(TIMEOUT));
    let initial = store.snapshot();
    assert!(initial.revision > 0);
    assert_eq!(store.revision(), initial.revision);
    assert!(store.flush(TIMEOUT));
    assert_eq!(store.revision(), initial.revision);
    send_ok(&store, Command::Reload);
    assert_eq!(store.revision(), initial.revision + 1);
    let created = send_ok(&store, create("One"));
    assert_eq!(created.revision, initial.revision + 2);
    let failed = send_error(&store, create("ONE"));
    assert_eq!(failed.revision, created.revision + 1);
    let failed_revision = failed.revision;
    send_error(&store, create("ONE"));
    assert_eq!(store.revision(), failed_revision + 1);
    send_ok(&store, Command::Reload);
    assert_eq!(store.revision(), failed_revision + 2);
    let previous = store.snapshot().entries[0].clone();
    let noop = send_ok(
        &store,
        Command::Update {
            previous: previous.clone(),
            name: previous.name.clone(),
            uri: previous.uri.clone(),
        },
    );
    assert_eq!(noop.revision, failed_revision + 3);
    assert!(store.flush(TIMEOUT));
    assert_eq!(store.revision(), noop.revision);
    let mut detached = store.snapshot();
    detached.entries.clear();
    detached.error = Some("not shared".into());
    assert_eq!(store.snapshot().entries.len(), 1);
    assert!(store.snapshot().error.is_none());
    assert!(initial.entries.is_empty());
}

#[test]
fn bounded_queue_send_and_barrier_never_wait_indefinitely() {
    let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    let store = Store(Arc::new(Shared {
        sender,
        snapshot: Arc::new(Mutex::new(Snapshot::default())),
    }));
    assert!(
        store
            .send(Command::Create {
                name: "n".repeat(MAX_NAME_BYTES + 1),
                uri: scoped().to_uri()
            })
            .is_err()
    );
    assert!(receiver.try_recv().is_err());
    assert!(
        store
            .send(Command::Delete(SavedSearch {
                id: 1,
                name: "Small".into(),
                uri: "x".repeat(MAX_URI_BYTES + 1)
            }))
            .is_err()
    );
    for position in 0..QUEUE_CAPACITY {
        store.send(create(&format!("Queued {position}"))).unwrap();
    }
    assert!(store.send(Command::Reload).is_err());
    assert!(!store.flush(Duration::ZERO));
    assert!(!store.flush(Duration::from_millis(1)));
    drop(receiver);
    assert!(store.send(Command::Reload).is_err());
    assert!(!store.flush(TIMEOUT));
}

#[test]
fn file_byte_limit_fails_before_publishing_or_replacing_the_committed_document() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut search = scoped();
    search.query = "%".repeat(MAX_QUERY_BYTES);
    let uri = search.to_uri();
    let mut previous = Vec::new();
    let mut rejected = false;
    for position in 0..MAX_ENTRIES {
        store
            .send(Command::Create {
                name: format!("Large {position}"),
                uri: uri.clone(),
            })
            .unwrap();
        let flushed = store.flush(TIMEOUT);
        let snapshot = store.snapshot();
        let bytes = fs::read(fixture.path()).unwrap();
        assert!(bytes.len() as u64 <= MAX_FILE_BYTES);
        if !flushed {
            assert_eq!(bytes, previous);
            let committed: Document = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(snapshot.entries, committed.entries);
            rejected = true;
            break;
        }
        previous = bytes;
    }
    assert!(rejected);
    send_ok(&store, Command::Reload);
    let previous = store.snapshot().entries[0].clone();
    send_ok(&store, Command::Delete(previous));
}

#[test]
fn symlinks_hardlinks_insecure_paths_and_replaced_locks_are_not_followed_or_overwritten() {
    let fixture = Fixture::new();
    let target = Fixture::new();
    let bytes = serde_json::to_vec(&Document::default()).unwrap();
    target.write(&bytes);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&fixture.root)
        .unwrap();
    symlink(target.path(), fixture.path()).unwrap();
    let store = fixture.store();
    assert!(send_error(&store, create("Symlink")).entries.is_empty());
    assert_eq!(fs::read(target.path()).unwrap(), bytes);
    fs::remove_file(fixture.path()).unwrap();
    fs::hard_link(target.path(), fixture.path()).unwrap();
    send_error(&store, Command::Reload);
    assert_eq!(fs::read(target.path()).unwrap(), bytes);
    fs::remove_file(fixture.path()).unwrap();
    send_ok(&store, Command::Reload);
    send_ok(&store, create("Safe"));
    let lock_path = fixture.path().with_extension("json.lock");
    fs::remove_file(&lock_path).unwrap();
    symlink(target.path(), &lock_path).unwrap();
    let committed = fs::read(fixture.path()).unwrap();
    send_error(&store, create("Replaced lock"));
    assert_eq!(fs::read(fixture.path()).unwrap(), committed);
    assert_eq!(fs::read(target.path()).unwrap(), bytes);
    send_error(&store, Command::Reload);
    fs::remove_file(lock_path).unwrap();
    send_ok(&store, Command::Reload);
    send_ok(&store, create("Recovered lock"));
    let private_bytes = fs::read(fixture.path()).unwrap();
    retire(store);
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o755)).unwrap();
    let insecure = fixture.store();
    assert!(!insecure.flush(TIMEOUT));
    assert_eq!(
        fs::metadata(&fixture.root).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(fs::read(fixture.path()).unwrap(), private_bytes);
}
