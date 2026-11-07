// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Fixture {
    path: PathBuf,
    roots: Roots,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hermes-import-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("expected test value from std::time::SystemTime::now() .duration_since(std::time::UNIX_EPOCH)")
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("expected test value from fs::DirBuilder::new().mode(0o700).create(&path)");
        let roots = Roots {
            source: [
                path.join("old-config"),
                path.join("old-data"),
                path.join("old-state"),
            ],
            target: [path.join("config"), path.join("data"), path.join("state")],
            development: false,
        };
        Self { path, roots }
    }

    fn source(&self, root: usize, name: &str, bytes: &[u8]) {
        let path = self.roots.source[root].join(name);
        fs::create_dir_all(
            path.parent()
                .expect("expected test value from path.parent()"),
        )
        .expect("create generated fixture directory");
        fs::write(path, bytes).expect("write generated fixture");
    }

    fn target(&self, root: usize, name: &str) -> PathBuf {
        self.roots.target[root].join(name)
    }

    fn transaction(&self) -> Directory {
        Directory::open(&self.roots.target[2], true)
            .expect("expected test value from Directory::open(&self.roots.target[2], true)")
            .child(Path::new(TRANSACTION), true)
            .expect("expected test value from Directory::open(&self.roots.target[2], true) .unwrap() .child(Path::new(TRANSACTION), true)")
    }

    fn prepared(&self) -> (Directory, Journal) {
        let directory = self.transaction();
        let journal = Journal {
            version: 2,
            completed: false,
            keep_hermes: false,
            entries: self
                .roots
                .plan()
                .expect("expected test value from self.roots.plan()"),
        };
        save(&directory, 0, &journal)
            .expect("expected test value from save(&directory, 0, &journal)");
        (directory, journal)
    }

    fn old_journal(&self, completed: bool) -> Vec<u8> {
        fs::create_dir_all(&self.roots.target[2]).expect("create generated fixture directory");
        // Untrusted old paths must never be used for recovery.
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": 1, "completed": completed,
            "entries": [{"source": "/untrusted/source", "destination": "/untrusted/destination", "bytes": 0, "success": false}]
        })).expect("serialize generated JSON fixture");
        fs::write(self.target(2, "migration-v1.json"), &bytes).expect("write generated fixture");
        bytes
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn inspection_requires_consent_without_creating_destinations() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"legacy");
    assert_eq!(
        fixture
            .roots
            .inspect()
            .expect("expected test value from fixture.roots.inspect()"),
        Status::Consent { recovering: false }
    );
    assert!(fixture.roots.target.iter().all(|path| !path.exists()));
}

#[test]
fn development_does_not_even_inspect_legacy_and_cannot_import() {
    let mut fixture = Fixture::new();
    symlink("/missing/legacy", &fixture.roots.source[0])
        .expect("expected test value from symlink(\"/missing/legacy\", &fixture.roots.source[0])");
    fixture.roots.development = true;
    assert_eq!(
        fixture
            .roots
            .inspect()
            .expect("expected test value from fixture.roots.inspect()"),
        Status::Ready
    );
    assert!(matches!(
        fixture.roots.decide(Choice::Import),
        Err(Error::Development)
    ));
    assert!(fixture.roots.target.iter().all(|path| !path.exists()));
}

#[test]
fn import_has_private_verified_backups_and_excludes_caches_and_integration() {
    let fixture = Fixture::new();
    for (root, name, bytes) in [
        (0, "settings.toml", b"settings".as_slice()),
        (0, "themes/custom.toml", b"theme".as_slice()),
        (1, "ratings-v1.json", b"ratings".as_slice()),
        (1, "saved-searches/queries-v1.json", b"queries".as_slice()),
        (2, "music-queue-v1.json", b"queue".as_slice()),
    ] {
        fixture.source(root, name, bytes);
    }
    fixture.source(0, "folder-integration.json", b"enabled");
    fixture.source(0, "picker-integration.json", b"enabled");
    fixture.source(1, "music-library/cache.json", b"cache");
    fixture
        .roots
        .decide(Choice::Import)
        .expect("expected test value from fixture.roots.decide(Choice::Import)");
    let directory = fixture.transaction();
    let (_, journal) = load(&directory)
        .expect("expected test value from load(&directory)")
        .expect("expected test value from load(&directory).unwrap()");
    assert!(journal.completed);
    assert_eq!(journal.entries.len(), 5);
    assert_eq!(
        fs::metadata(directory.path())
            .expect("expected test value from fs::metadata(directory.path())")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    for (index, entry) in journal.entries.iter().enumerate() {
        let original = fs::read(fixture.roots.source[entry.root].join(&entry.relative))
            .expect("read generated fixture");
        assert_eq!(
            fs::read(fixture.roots.target[entry.root].join(&entry.relative))
                .expect("read generated fixture"),
            original
        );
        let backup = directory.path().join(format!("backups/{index:03}.backup"));
        assert_eq!(fs::read(&backup).expect("read generated fixture"), original);
        assert_eq!(
            fs::metadata(backup)
                .expect("expected test value from fs::metadata(backup)")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(!fixture.target(0, "folder-integration.json").exists());
    assert!(!fixture.target(0, "picker-integration.json").exists());
    assert!(!fixture.target(1, "music-library").exists());
    assert_eq!(
        fixture
            .roots
            .inspect()
            .expect("expected test value from fixture.roots.inspect()"),
        Status::Ready
    );
    fixture
        .roots
        .decide(Choice::Import)
        .expect("expected test value from fixture.roots.decide(Choice::Import)");
}

#[test]
fn partial_failure_resumes_and_preserves_newer_completed_destination() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"legacy settings");
    fixture.source(1, "ratings-v1.json", b"legacy ratings");
    fs::write(&fixture.roots.target[1], b"blocked parent").expect("write generated fixture");
    assert!(fixture.roots.decide(Choice::Import).is_err());
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).expect("read generated fixture"),
        b"legacy settings"
    );
    assert_eq!(
        fixture
            .roots
            .inspect()
            .expect("expected test value from fixture.roots.inspect()"),
        Status::Consent { recovering: true }
    );
    let (_, journal) = load(&fixture.transaction())
        .expect("expected test value from load(&fixture.transaction())")
        .expect("expected test value from load(&fixture.transaction()).unwrap()");
    assert!(!journal.completed);
    assert_eq!(journal.entries[0].state, EntryState::Published);
    fs::write(fixture.target(0, "settings.toml"), b"newer Hermes settings")
        .expect("write generated fixture");
    fs::remove_file(&fixture.roots.target[1]).expect("remove generated fixture");
    fixture
        .roots
        .decide(Choice::Import)
        .expect("expected test value from fixture.roots.decide(Choice::Import)");
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).expect("read generated fixture"),
        b"newer Hermes settings"
    );
    assert_eq!(
        fs::read(fixture.target(1, "ratings-v1.json")).expect("read generated fixture"),
        b"legacy ratings"
    );
    assert_eq!(
        fs::read(fixture.roots.source[0].join("settings.toml")).expect("read generated fixture"),
        b"legacy settings"
    );
    assert_eq!(
        fixture
            .roots
            .inspect()
            .expect("expected test value from fixture.roots.inspect()"),
        Status::Ready
    );
}

#[test]
fn old_predictable_staging_symlink_cannot_change_sentinel() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"incoming");
    fs::create_dir_all(&fixture.roots.target[0]).expect("create generated fixture directory");
    let sentinel = fixture.path.join("sentinel");
    fs::write(&sentinel, b"untouched").expect("write generated fixture");
    let trap = fixture
        .target(0, "settings.toml")
        .with_extension("hermes-migrating-tmp");
    symlink(&sentinel, &trap).expect("expected test value from symlink(&sentinel, &trap)");
    fixture
        .roots
        .decide(Choice::Import)
        .expect("expected test value from fixture.roots.decide(Choice::Import)");
    assert_eq!(
        fs::read(&sentinel).expect("read generated fixture"),
        b"untouched"
    );
    assert!(
        fs::symlink_metadata(trap)
            .expect("expected test value from fs::symlink_metadata(trap)")
            .file_type()
            .is_symlink()
    );
    assert!(
        !fs::symlink_metadata(fixture.target(0, "settings.toml"))
            .expect("expected test value from fs::symlink_metadata(fixture.target(0, \"settings.toml\"))")
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).expect("read generated fixture"),
        b"incoming"
    );
}

#[test]
fn existing_and_raced_in_destinations_are_never_replaced() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"old");
    fs::create_dir_all(&fixture.roots.target[0]).expect("create generated fixture directory");
    fs::write(fixture.target(0, "settings.toml"), b"keep existing")
        .expect("write generated fixture");
    fixture
        .roots
        .decide(Choice::Import)
        .expect("expected test value from fixture.roots.decide(Choice::Import)");
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).expect("read generated fixture"),
        b"keep existing"
    );
    let directory = Directory::open(&fixture.roots.target[0], false)
        .expect("expected test value from Directory::open(&fixture.roots.target[0], false)");
    assert!(matches!(
        publish(&directory, Path::new("settings.toml"), b"replace"),
        Err(Error::Io(io::ErrorKind::AlreadyExists))
    ));
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).expect("read generated fixture"),
        b"keep existing"
    );
    let sentinel = fixture.path.join("sentinel");
    fs::write(&sentinel, b"sentinel").expect("write generated fixture");
    symlink(&sentinel, fixture.target(0, "trails.toml"))
        .expect("expected test value from symlink(&sentinel, fixture.target(0, \"trails.toml\"))");
    assert!(matches!(
        publish(&directory, Path::new("trails.toml"), b"replace"),
        Err(Error::Io(io::ErrorKind::AlreadyExists))
    ));
    assert_eq!(
        fs::read(sentinel).expect("read generated fixture"),
        b"sentinel"
    );
}

#[test]
fn source_leaf_and_ancestor_symlinks_are_rejected() {
    for ancestor in [false, true] {
        let fixture = Fixture::new();
        let outside = fixture.path.join("outside");
        fs::create_dir_all(&outside).expect("create generated fixture directory");
        fs::write(outside.join("settings.toml"), b"sentinel").expect("write generated fixture");
        if ancestor {
            symlink(&outside, &fixture.roots.source[0])
                .expect("expected test value from symlink(&outside, &fixture.roots.source[0])");
        } else {
            fs::create_dir_all(&fixture.roots.source[0])
                .expect("create generated fixture directory");
            symlink(
                outside.join("settings.toml"),
                fixture.roots.source[0].join("settings.toml"),
            )
            .expect("expected test value from symlink( outside.join(\"settings.toml\"), fixture.roots.source[0].join(\"settings.toml\"), )");
        }
        assert!(fixture.roots.inspect().is_err());
        assert!(fixture.roots.decide(Choice::Import).is_err());
        assert!(!fixture.target(0, "settings.toml").exists());
        assert_eq!(
            fs::read(outside.join("settings.toml")).expect("read generated fixture"),
            b"sentinel"
        );
    }
}

#[test]
fn destination_leaf_and_ancestor_symlinks_are_rejected() {
    for ancestor in [false, true] {
        let fixture = Fixture::new();
        fixture.source(0, "settings.toml", b"incoming");
        let outside = fixture.path.join("outside");
        fs::create_dir_all(&outside).expect("create generated fixture directory");
        fs::write(outside.join("settings.toml"), b"sentinel").expect("write generated fixture");
        if ancestor {
            symlink(&outside, &fixture.roots.target[0])
                .expect("expected test value from symlink(&outside, &fixture.roots.target[0])");
        } else {
            fs::create_dir_all(&fixture.roots.target[0])
                .expect("create generated fixture directory");
            symlink(
                outside.join("settings.toml"),
                fixture.target(0, "settings.toml"),
            )
            .expect("expected test value from symlink( outside.join(\"settings.toml\"), fixture.target(0, \"settings.toml\"), )");
        }
        assert!(fixture.roots.decide(Choice::Import).is_err());
        assert_eq!(
            fs::read(outside.join("settings.toml")).expect("read generated fixture"),
            b"sentinel"
        );
    }
}

#[test]
fn replaced_destination_ancestor_cannot_redirect_an_open_handle() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.roots.target[0]).expect("create generated fixture directory");
    let directory = Directory::open(&fixture.roots.target[0], false)
        .expect("expected test value from Directory::open(&fixture.roots.target[0], false)");
    let moved = fixture.path.join("moved-original");
    fs::rename(&fixture.roots.target[0], &moved)
        .expect("expected test value from fs::rename(&fixture.roots.target[0], &moved)");
    let outside = fixture.path.join("outside");
    fs::create_dir(&outside).expect("create generated fixture directory");
    symlink(&outside, &fixture.roots.target[0])
        .expect("expected test value from symlink(&outside, &fixture.roots.target[0])");
    publish(&directory, Path::new("settings.toml"), b"safe").expect(
        "expected test value from publish(&directory, Path::new(\"settings.toml\"), b\"safe\")",
    );
    assert_eq!(
        fs::read(moved.join("settings.toml")).expect("read generated fixture"),
        b"safe"
    );
    assert!(!outside.join("settings.toml").exists());
}

#[test]
fn backup_recovers_crash_before_or_after_backup_checkpoint_without_original() {
    for checkpointed in [false, true] {
        let fixture = Fixture::new();
        fixture.source(0, "settings.toml", b"recover me");
        let (directory, mut journal) = fixture.prepared();
        let backups = directory
            .child(Path::new("backups"), true)
            .expect("expected test value from directory.child(Path::new(\"backups\"), true)");
        publish(&backups, Path::new("000.backup"), b"recover me").expect("expected test value from publish(&backups, Path::new(\"000.backup\"), b\"recover me\")");
        if checkpointed {
            journal.entries[0].state = EntryState::BackedUp;
            save(&directory, 1, &journal)
                .expect("expected test value from save(&directory, 1, &journal)");
        }
        // An interrupted unpublished staging file is never mistaken for a journal.
        let orphan = directory
            .child(Path::new(".import-stage-crashed"), true)
            .expect("expected test value from directory .child(Path::new(\".import-stage-crashed\"), true)");
        fs::write(orphan.path().join("content"), b"partial journal")
            .expect("write generated fixture");
        fs::remove_file(fixture.roots.source[0].join("settings.toml"))
            .expect("remove generated fixture");
        fixture
            .roots
            .decide(Choice::Import)
            .expect("expected test value from fixture.roots.decide(Choice::Import)");
        assert_eq!(
            fs::read(fixture.target(0, "settings.toml")).expect("read generated fixture"),
            b"recover me"
        );
        assert_eq!(
            fixture
                .roots
                .inspect()
                .expect("expected test value from fixture.roots.inspect()"),
            Status::Ready
        );
        assert_eq!(
            fs::read(backups.path().join("000.backup")).expect("read generated fixture"),
            b"recover me"
        );
    }
}

#[test]
fn crash_after_publication_before_checkpoint_preserves_newer_destination() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"original");
    let (directory, mut journal) = fixture.prepared();
    let backups = directory
        .child(Path::new("backups"), true)
        .expect("expected test value from directory.child(Path::new(\"backups\"), true)");
    publish(&backups, Path::new("000.backup"), b"original").expect(
        "expected test value from publish(&backups, Path::new(\"000.backup\"), b\"original\")",
    );
    journal.entries[0].state = EntryState::BackedUp;
    save(&directory, 1, &journal).expect("expected test value from save(&directory, 1, &journal)");
    fs::create_dir_all(&fixture.roots.target[0]).expect("create generated fixture directory");
    fs::write(fixture.target(0, "settings.toml"), b"newer after crash")
        .expect("write generated fixture");
    fixture
        .roots
        .decide(Choice::Import)
        .expect("expected test value from fixture.roots.decide(Choice::Import)");
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).expect("read generated fixture"),
        b"newer after crash"
    );
    assert_eq!(
        fs::read(backups.path().join("000.backup")).expect("read generated fixture"),
        b"original"
    );
}

#[test]
fn incomplete_v1_journal_recovers_and_is_backed_up_verbatim() {
    let fixture = Fixture::new();
    fixture.source(1, "ratings-v1.json", b"retry ratings");
    let old = fixture.old_journal(false);
    assert_eq!(
        fixture
            .roots
            .inspect()
            .expect("expected test value from fixture.roots.inspect()"),
        Status::Consent { recovering: true }
    );
    fixture
        .roots
        .decide(Choice::Import)
        .expect("expected test value from fixture.roots.decide(Choice::Import)");
    assert_eq!(
        fs::read(fixture.target(1, "ratings-v1.json")).expect("read generated fixture"),
        b"retry ratings"
    );
    assert_eq!(
        fs::read(fixture.target(2, "migration-v1.json")).expect("read generated fixture"),
        old
    );
    assert_eq!(
        read_bounded(
            fixture
                .transaction()
                .file(Path::new("previous-journal.json"))
                .expect("expected test value from fixture .transaction() .file(Path::new(\"previous-journal.json\"))"),
            MAX_JOURNAL
        )
        .expect("expected test value from read_bounded( fixture .transaction() .file(Path::new(\"previous-journal.json\")) .unwrap(), MAX_JOURNAL )"),
        old
    );
}

#[test]
fn completed_v1_journal_remains_compatible_without_reimport() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"legacy");
    fixture.old_journal(true);
    assert_eq!(
        fixture
            .roots
            .inspect()
            .expect("expected test value from fixture.roots.inspect()"),
        Status::Ready
    );
    assert!(!fixture.target(0, "settings.toml").exists());
    assert!(!fixture.target(2, TRANSACTION).exists());
}

#[test]
fn explicit_keep_hermes_is_durable_and_does_not_read_unsafe_sources() {
    let fixture = Fixture::new();
    symlink("/missing/legacy", &fixture.roots.source[0])
        .expect("expected test value from symlink(\"/missing/legacy\", &fixture.roots.source[0])");
    fixture
        .roots
        .decide(Choice::KeepHermes)
        .expect("expected test value from fixture.roots.decide(Choice::KeepHermes)");
    assert_eq!(
        fixture
            .roots
            .inspect()
            .expect("expected test value from fixture.roots.inspect()"),
        Status::Ready
    );
    assert!(!fixture.roots.target[0].exists());
    assert!(!fixture.roots.target[1].exists());
}

#[test]
fn backup_integrity_failure_is_reported_without_publishing() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"original");
    let (directory, mut journal) = fixture.prepared();
    let backups = directory
        .child(Path::new("backups"), true)
        .expect("expected test value from directory.child(Path::new(\"backups\"), true)");
    publish(&backups, Path::new("000.backup"), b"tampered").expect(
        "expected test value from publish(&backups, Path::new(\"000.backup\"), b\"tampered\")",
    );
    journal.entries[0].state = EntryState::BackedUp;
    save(&directory, 1, &journal).expect("expected test value from save(&directory, 1, &journal)");
    assert!(matches!(
        fixture.roots.decide(Choice::Import),
        Err(Error::Changed)
    ));
    assert!(!fixture.target(0, "settings.toml").exists());
    assert!(
        !load(&directory)
            .expect("expected test value from load(&directory)")
            .expect("expected test value from load(&directory).unwrap()")
            .1
            .completed
    );
}

#[test]
fn journal_symlink_and_invalid_plan_fail_closed() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"original");
    let (directory, mut journal) = fixture.prepared();
    let sentinel = fixture.path.join("sentinel");
    fs::write(&sentinel, b"untouched").expect("write generated fixture");
    symlink(&sentinel, directory.path().join("journal-0001.json")).expect(
        "expected test value from symlink(&sentinel, directory.path().join(\"journal-0001.json\"))",
    );
    assert!(save(&directory, 1, &journal).is_err());
    assert!(fixture.roots.decide(Choice::Import).is_err());
    assert_eq!(
        fs::read(sentinel).expect("read generated fixture"),
        b"untouched"
    );
    assert!(!fixture.target(0, "settings.toml").exists());
    journal.entries[0].relative = PathBuf::from("../../outside");
    assert!(matches!(validate(&journal), Err(Error::InvalidJournal)));
}

#[test]
fn concurrent_import_and_oversized_input_fail_without_data_publication() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"legacy");
    let directory = fixture.transaction();
    directory
        .0
        .try_lock()
        .expect("expected test value from directory.0.try_lock()");
    assert!(matches!(
        fixture.roots.decide(Choice::Import),
        Err(Error::Busy)
    ));
    assert!(!fixture.target(0, "settings.toml").exists());
    drop(directory);
    let file = OpenOptions::new()
        .write(true)
        .open(fixture.roots.source[0].join("settings.toml"))
        .expect("expected test value from OpenOptions::new() .write(true) .open(fixture.roots.source[0].join(\"settings.toml\"))");
    file.set_len(MAX_FILE + 1)
        .expect("expected test value from file.set_len(MAX_FILE + 1)");
    assert!(matches!(
        fixture.roots.decide(Choice::Import),
        Err(Error::Limit)
    ));
    assert!(!fixture.target(0, "settings.toml").exists());
}
