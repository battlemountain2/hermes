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
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
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
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn target(&self, root: usize, name: &str) -> PathBuf {
        self.roots.target[root].join(name)
    }

    fn transaction(&self) -> Directory {
        Directory::open(&self.roots.target[2], true)
            .unwrap()
            .child(Path::new(TRANSACTION), true)
            .unwrap()
    }

    fn prepared(&self) -> (Directory, Journal) {
        let directory = self.transaction();
        let journal = Journal {
            version: 2,
            completed: false,
            keep_hermes: false,
            entries: self.roots.plan().unwrap(),
        };
        save(&directory, 0, &journal).unwrap();
        (directory, journal)
    }

    fn old_journal(&self, completed: bool) -> Vec<u8> {
        fs::create_dir_all(&self.roots.target[2]).unwrap();
        // Untrusted old paths must never be used for recovery.
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": 1, "completed": completed,
            "entries": [{"source": "/untrusted/source", "destination": "/untrusted/destination", "bytes": 0, "success": false}]
        })).unwrap();
        fs::write(self.target(2, "migration-v1.json"), &bytes).unwrap();
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
        fixture.roots.inspect().unwrap(),
        Status::Consent { recovering: false }
    );
    assert!(fixture.roots.target.iter().all(|path| !path.exists()));
}

#[test]
fn development_does_not_even_inspect_legacy_and_cannot_import() {
    let mut fixture = Fixture::new();
    symlink("/missing/legacy", &fixture.roots.source[0]).unwrap();
    fixture.roots.development = true;
    assert_eq!(fixture.roots.inspect().unwrap(), Status::Ready);
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
    fixture.roots.decide(Choice::Import).unwrap();
    let directory = fixture.transaction();
    let (_, journal) = load(&directory).unwrap().unwrap();
    assert!(journal.completed);
    assert_eq!(journal.entries.len(), 5);
    assert_eq!(
        fs::metadata(directory.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for (index, entry) in journal.entries.iter().enumerate() {
        let original = fs::read(fixture.roots.source[entry.root].join(&entry.relative)).unwrap();
        assert_eq!(
            fs::read(fixture.roots.target[entry.root].join(&entry.relative)).unwrap(),
            original
        );
        let backup = directory.path().join(format!("backups/{index:03}.backup"));
        assert_eq!(fs::read(&backup).unwrap(), original);
        assert_eq!(
            fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(!fixture.target(0, "folder-integration.json").exists());
    assert!(!fixture.target(0, "picker-integration.json").exists());
    assert!(!fixture.target(1, "music-library").exists());
    assert_eq!(fixture.roots.inspect().unwrap(), Status::Ready);
    fixture.roots.decide(Choice::Import).unwrap();
}

#[test]
fn partial_failure_resumes_and_preserves_newer_completed_destination() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"legacy settings");
    fixture.source(1, "ratings-v1.json", b"legacy ratings");
    fs::write(&fixture.roots.target[1], b"blocked parent").unwrap();
    assert!(fixture.roots.decide(Choice::Import).is_err());
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).unwrap(),
        b"legacy settings"
    );
    assert_eq!(
        fixture.roots.inspect().unwrap(),
        Status::Consent { recovering: true }
    );
    let (_, journal) = load(&fixture.transaction()).unwrap().unwrap();
    assert!(!journal.completed);
    assert_eq!(journal.entries[0].state, EntryState::Published);
    fs::write(fixture.target(0, "settings.toml"), b"newer Hermes settings").unwrap();
    fs::remove_file(&fixture.roots.target[1]).unwrap();
    fixture.roots.decide(Choice::Import).unwrap();
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).unwrap(),
        b"newer Hermes settings"
    );
    assert_eq!(
        fs::read(fixture.target(1, "ratings-v1.json")).unwrap(),
        b"legacy ratings"
    );
    assert_eq!(
        fs::read(fixture.roots.source[0].join("settings.toml")).unwrap(),
        b"legacy settings"
    );
    assert_eq!(fixture.roots.inspect().unwrap(), Status::Ready);
}

#[test]
fn old_predictable_staging_symlink_cannot_change_sentinel() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"incoming");
    fs::create_dir_all(&fixture.roots.target[0]).unwrap();
    let sentinel = fixture.path.join("sentinel");
    fs::write(&sentinel, b"untouched").unwrap();
    let trap = fixture
        .target(0, "settings.toml")
        .with_extension("hermes-migrating-tmp");
    symlink(&sentinel, &trap).unwrap();
    fixture.roots.decide(Choice::Import).unwrap();
    assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");
    assert!(fs::symlink_metadata(trap).unwrap().file_type().is_symlink());
    assert!(
        !fs::symlink_metadata(fixture.target(0, "settings.toml"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).unwrap(),
        b"incoming"
    );
}

#[test]
fn existing_and_raced_in_destinations_are_never_replaced() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"old");
    fs::create_dir_all(&fixture.roots.target[0]).unwrap();
    fs::write(fixture.target(0, "settings.toml"), b"keep existing").unwrap();
    fixture.roots.decide(Choice::Import).unwrap();
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).unwrap(),
        b"keep existing"
    );
    let directory = Directory::open(&fixture.roots.target[0], false).unwrap();
    assert!(matches!(
        publish(&directory, Path::new("settings.toml"), b"replace"),
        Err(Error::Io(io::ErrorKind::AlreadyExists))
    ));
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).unwrap(),
        b"keep existing"
    );
    let sentinel = fixture.path.join("sentinel");
    fs::write(&sentinel, b"sentinel").unwrap();
    symlink(&sentinel, fixture.target(0, "trails.toml")).unwrap();
    assert!(matches!(
        publish(&directory, Path::new("trails.toml"), b"replace"),
        Err(Error::Io(io::ErrorKind::AlreadyExists))
    ));
    assert_eq!(fs::read(sentinel).unwrap(), b"sentinel");
}

#[test]
fn source_leaf_and_ancestor_symlinks_are_rejected() {
    for ancestor in [false, true] {
        let fixture = Fixture::new();
        let outside = fixture.path.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("settings.toml"), b"sentinel").unwrap();
        if ancestor {
            symlink(&outside, &fixture.roots.source[0]).unwrap();
        } else {
            fs::create_dir_all(&fixture.roots.source[0]).unwrap();
            symlink(
                outside.join("settings.toml"),
                fixture.roots.source[0].join("settings.toml"),
            )
            .unwrap();
        }
        assert!(fixture.roots.inspect().is_err());
        assert!(fixture.roots.decide(Choice::Import).is_err());
        assert!(!fixture.target(0, "settings.toml").exists());
        assert_eq!(
            fs::read(outside.join("settings.toml")).unwrap(),
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
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("settings.toml"), b"sentinel").unwrap();
        if ancestor {
            symlink(&outside, &fixture.roots.target[0]).unwrap();
        } else {
            fs::create_dir_all(&fixture.roots.target[0]).unwrap();
            symlink(
                outside.join("settings.toml"),
                fixture.target(0, "settings.toml"),
            )
            .unwrap();
        }
        assert!(fixture.roots.decide(Choice::Import).is_err());
        assert_eq!(
            fs::read(outside.join("settings.toml")).unwrap(),
            b"sentinel"
        );
    }
}

#[test]
fn replaced_destination_ancestor_cannot_redirect_an_open_handle() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.roots.target[0]).unwrap();
    let directory = Directory::open(&fixture.roots.target[0], false).unwrap();
    let moved = fixture.path.join("moved-original");
    fs::rename(&fixture.roots.target[0], &moved).unwrap();
    let outside = fixture.path.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, &fixture.roots.target[0]).unwrap();
    publish(&directory, Path::new("settings.toml"), b"safe").unwrap();
    assert_eq!(fs::read(moved.join("settings.toml")).unwrap(), b"safe");
    assert!(!outside.join("settings.toml").exists());
}

#[test]
fn backup_recovers_crash_before_or_after_backup_checkpoint_without_original() {
    for checkpointed in [false, true] {
        let fixture = Fixture::new();
        fixture.source(0, "settings.toml", b"recover me");
        let (directory, mut journal) = fixture.prepared();
        let backups = directory.child(Path::new("backups"), true).unwrap();
        publish(&backups, Path::new("000.backup"), b"recover me").unwrap();
        if checkpointed {
            journal.entries[0].state = EntryState::BackedUp;
            save(&directory, 1, &journal).unwrap();
        }
        // An interrupted unpublished staging file is never mistaken for a journal.
        let orphan = directory
            .child(Path::new(".import-stage-crashed"), true)
            .unwrap();
        fs::write(orphan.path().join("content"), b"partial journal").unwrap();
        fs::remove_file(fixture.roots.source[0].join("settings.toml")).unwrap();
        fixture.roots.decide(Choice::Import).unwrap();
        assert_eq!(
            fs::read(fixture.target(0, "settings.toml")).unwrap(),
            b"recover me"
        );
        assert_eq!(fixture.roots.inspect().unwrap(), Status::Ready);
        assert_eq!(
            fs::read(backups.path().join("000.backup")).unwrap(),
            b"recover me"
        );
    }
}

#[test]
fn crash_after_publication_before_checkpoint_preserves_newer_destination() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"original");
    let (directory, mut journal) = fixture.prepared();
    let backups = directory.child(Path::new("backups"), true).unwrap();
    publish(&backups, Path::new("000.backup"), b"original").unwrap();
    journal.entries[0].state = EntryState::BackedUp;
    save(&directory, 1, &journal).unwrap();
    fs::create_dir_all(&fixture.roots.target[0]).unwrap();
    fs::write(fixture.target(0, "settings.toml"), b"newer after crash").unwrap();
    fixture.roots.decide(Choice::Import).unwrap();
    assert_eq!(
        fs::read(fixture.target(0, "settings.toml")).unwrap(),
        b"newer after crash"
    );
    assert_eq!(
        fs::read(backups.path().join("000.backup")).unwrap(),
        b"original"
    );
}

#[test]
fn incomplete_v1_journal_recovers_and_is_backed_up_verbatim() {
    let fixture = Fixture::new();
    fixture.source(1, "ratings-v1.json", b"retry ratings");
    let old = fixture.old_journal(false);
    assert_eq!(
        fixture.roots.inspect().unwrap(),
        Status::Consent { recovering: true }
    );
    fixture.roots.decide(Choice::Import).unwrap();
    assert_eq!(
        fs::read(fixture.target(1, "ratings-v1.json")).unwrap(),
        b"retry ratings"
    );
    assert_eq!(
        fs::read(fixture.target(2, "migration-v1.json")).unwrap(),
        old
    );
    assert_eq!(
        read_bounded(
            fixture
                .transaction()
                .file(Path::new("previous-journal.json"))
                .unwrap(),
            MAX_JOURNAL
        )
        .unwrap(),
        old
    );
}

#[test]
fn completed_v1_journal_remains_compatible_without_reimport() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"legacy");
    fixture.old_journal(true);
    assert_eq!(fixture.roots.inspect().unwrap(), Status::Ready);
    assert!(!fixture.target(0, "settings.toml").exists());
    assert!(!fixture.target(2, TRANSACTION).exists());
}

#[test]
fn explicit_keep_hermes_is_durable_and_does_not_read_unsafe_sources() {
    let fixture = Fixture::new();
    symlink("/missing/legacy", &fixture.roots.source[0]).unwrap();
    fixture.roots.decide(Choice::KeepHermes).unwrap();
    assert_eq!(fixture.roots.inspect().unwrap(), Status::Ready);
    assert!(!fixture.roots.target[0].exists());
    assert!(!fixture.roots.target[1].exists());
}

#[test]
fn backup_integrity_failure_is_reported_without_publishing() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"original");
    let (directory, mut journal) = fixture.prepared();
    let backups = directory.child(Path::new("backups"), true).unwrap();
    publish(&backups, Path::new("000.backup"), b"tampered").unwrap();
    journal.entries[0].state = EntryState::BackedUp;
    save(&directory, 1, &journal).unwrap();
    assert!(matches!(
        fixture.roots.decide(Choice::Import),
        Err(Error::Changed)
    ));
    assert!(!fixture.target(0, "settings.toml").exists());
    assert!(!load(&directory).unwrap().unwrap().1.completed);
}

#[test]
fn journal_symlink_and_invalid_plan_fail_closed() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"original");
    let (directory, mut journal) = fixture.prepared();
    let sentinel = fixture.path.join("sentinel");
    fs::write(&sentinel, b"untouched").unwrap();
    symlink(&sentinel, directory.path().join("journal-0001.json")).unwrap();
    assert!(save(&directory, 1, &journal).is_err());
    assert!(fixture.roots.decide(Choice::Import).is_err());
    assert_eq!(fs::read(sentinel).unwrap(), b"untouched");
    assert!(!fixture.target(0, "settings.toml").exists());
    journal.entries[0].relative = PathBuf::from("../../outside");
    assert!(matches!(validate(&journal), Err(Error::InvalidJournal)));
}

#[test]
fn concurrent_import_and_oversized_input_fail_without_data_publication() {
    let fixture = Fixture::new();
    fixture.source(0, "settings.toml", b"legacy");
    let directory = fixture.transaction();
    directory.0.try_lock().unwrap();
    assert!(matches!(
        fixture.roots.decide(Choice::Import),
        Err(Error::Busy)
    ));
    assert!(!fixture.target(0, "settings.toml").exists());
    drop(directory);
    let file = OpenOptions::new()
        .write(true)
        .open(fixture.roots.source[0].join("settings.toml"))
        .unwrap();
    file.set_len(MAX_FILE + 1).unwrap();
    assert!(matches!(
        fixture.roots.decide(Choice::Import),
        Err(Error::Limit)
    ));
    assert!(!fixture.target(0, "settings.toml").exists());
}
