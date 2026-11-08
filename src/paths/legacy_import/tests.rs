// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

thread_local! {
    static SYNC_FAILURE: std::cell::Cell<Option<(u64, u64)>> = const { std::cell::Cell::new(None) };
}

pub(super) fn check_directory_sync(directory: &Directory) -> Result<()> {
    let metadata = directory.0.metadata()?;
    if SYNC_FAILURE.get() == Some((metadata.dev(), metadata.ino())) {
        SYNC_FAILURE.set(None);
        return Err(Error::Io(io::ErrorKind::Other));
    }
    Ok(())
}

fn fail_next_sync(directory: &Directory) {
    let metadata = directory.0.metadata().expect("fixture directory metadata");
    SYNC_FAILURE.set(Some((metadata.dev(), metadata.ino())));
}

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
fn stage_lifetime_keeps_private_bytes_and_preserves_unrelated_entries() {
    let fixture = Fixture::new();
    let directory = fixture.transaction();
    let sentinel = directory.path().join(".import-stage-old");
    fs::create_dir(&sentinel).expect("create legacy stage sentinel");
    fs::write(sentinel.join("content"), b"preserve legacy bytes").expect("write sentinel");
    let mut stage = Stage::new(&directory).expect("create private stage");
    let anonymous = stage._reservation.is_none();
    assert_eq!(
        stage.file.metadata().expect("stage metadata").mode() & 0o777,
        0o600
    );
    assert_eq!(
        stage.file.metadata().expect("stage metadata").nlink(),
        u64::from(!anonymous)
    );
    stage
        .file
        .write_all(b"complete bytes")
        .expect("write private stage");
    stage.file.sync_all().expect("sync private stage");
    stage
        .publish(&directory, Path::new("published"))
        .expect("publish stage without replacement");
    drop(stage);
    assert_eq!(
        fs::read(directory.path().join("published")).expect("read published file"),
        b"complete bytes"
    );
    assert_eq!(
        fs::read(sentinel.join("content")).expect("read legacy sentinel"),
        b"preserve legacy bytes"
    );
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("list fixture")
            .count(),
        2
    );
}

#[test]
fn named_fallback_cleans_only_its_own_reservation() {
    let fixture = Fixture::new();
    let directory = fixture.transaction();
    let mut stage = Stage::named(&directory).expect("create named fallback");
    stage
        .file
        .write_all(b"partial")
        .expect("write partial fallback data");
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("list reserved stage")
            .count(),
        1
    );
    drop(stage);
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("list cleaned stage")
            .count(),
        0
    );
}

#[test]
fn named_fallback_does_not_remove_a_replacement_directory() {
    let fixture = Fixture::new();
    let directory = fixture.transaction();
    let stage = Stage::named(&directory).expect("create named fallback");
    let reservation = stage._reservation.as_ref().expect("named reservation");
    let path = directory.path().join(&reservation.name);
    let moved = directory.path().join("moved-owned-reservation");
    fs::rename(&path, &moved).expect("move owned directory");
    fs::create_dir(&path).expect("create replacement directory");
    drop(stage);
    assert!(path.is_dir(), "replacement directory must survive cleanup");
    assert!(
        !moved.join("content").exists(),
        "owned partial bytes should be cleaned through the retained handle"
    );
}

#[test]
fn named_fallback_publication_preserves_an_existing_destination() {
    let fixture = Fixture::new();
    let directory = fixture.transaction();
    let mut stage = Stage::named(&directory).expect("create named fallback");
    stage
        .file
        .write_all(b"incoming")
        .expect("write fallback data");
    fs::write(directory.path().join("keep"), b"existing").expect("write destination sentinel");
    assert!(matches!(
        stage.publish(&directory, Path::new("keep")),
        Err(Error::Io(io::ErrorKind::AlreadyExists))
    ));
    drop(stage);
    assert_eq!(
        fs::read(directory.path().join("keep")).expect("read sentinel"),
        b"existing"
    );
    assert_eq!(
        fs::read_dir(directory.path())
            .expect("list fixture")
            .count(),
        1
    );
}

#[test]
fn named_fallback_preserves_replaced_or_unowned_content() {
    let fixture = Fixture::new();
    let directory = fixture.transaction();
    let stage = Stage::named(&directory).expect("create named fallback");
    let path = stage
        ._reservation
        .as_ref()
        .expect("named reservation")
        .directory
        .path()
        .join("content");
    fs::remove_file(&path).expect("remove generated staged file");
    fs::write(&path, b"replacement must survive").expect("create replacement sentinel");
    let retained_path = fs::read_link(
        stage
            ._reservation
            .as_ref()
            .expect("named reservation")
            .directory
            .path(),
    )
    .expect("resolve fixture directory handle")
    .join("content");
    drop(stage);
    assert_eq!(
        fs::read(retained_path).expect("read replacement sentinel"),
        b"replacement must survive"
    );
    let reservation = NamedStage::new(&directory).expect("reserve empty fallback");
    let retained_path = directory.path().join(&reservation.name).join("content");
    fs::write(&retained_path, b"created before exclusive open").expect("write unowned sentinel");
    drop(reservation);
    assert_eq!(
        fs::read(retained_path).expect("read unowned sentinel"),
        b"created before exclusive open"
    );
}

#[test]
fn retry_resyncs_backup_and_destination_before_checkpointing_reused_entries() {
    for fail_backup in [false, true] {
        let fixture = Fixture::new();
        fixture.source(0, "settings.toml", b"complete original");
        let transaction = fixture.transaction();
        let directory = if fail_backup {
            transaction
                .child(Path::new("backups"), true)
                .expect("create backup fixture")
        } else {
            Directory::open(&fixture.roots.target[0], true).expect("create destination fixture")
        };
        fail_next_sync(&directory);
        assert!(matches!(
            fixture.roots.decide(Choice::Import),
            Err(Error::Io(io::ErrorKind::Other))
        ));
        let (generation, first) = load(&transaction)
            .expect("load durable fixture checkpoint")
            .expect("checkpoint exists");
        assert!(!first.completed);
        fail_next_sync(&directory);
        assert!(matches!(
            fixture.roots.decide(Choice::Import),
            Err(Error::Io(io::ErrorKind::Other))
        ));
        let (after_retry, second) = load(&transaction)
            .expect("load retry checkpoint")
            .expect("checkpoint exists");
        assert_eq!(
            generation, after_retry,
            "retry must sync reused publication before advancing recovery"
        );
        assert!(!second.completed);
        fixture
            .roots
            .decide(Choice::Import)
            .expect("resume after storage sync recovers");
        assert_eq!(
            fs::read(fixture.target(0, "settings.toml")).expect("read final data"),
            b"complete original"
        );
        assert_eq!(
            fs::read(fixture.roots.source[0].join("settings.toml")).expect("read original"),
            b"complete original"
        );
    }
}

#[test]
fn storage_exhaustion_preserves_originals_and_recovers_after_space_is_freed() {
    const CHILD: &str = "HERMES_IMPORT_STORAGE_FIXTURE";
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        assert!(
            root.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("hermes-import-"))
        );
        let index: usize = std::env::var("HERMES_IMPORT_STORAGE_INDEX")
            .expect("fixture index")
            .parse()
            .expect("numeric fixture index");
        assert!(index < 3);
        let roots = Roots {
            source: [
                root.join("old-config"),
                root.join("old-data"),
                root.join("old-state"),
            ],
            target: [root.join("config"), root.join("data"), root.join("state")],
            development: false,
        };
        for target in &roots.target {
            fs::create_dir_all(target).expect("create generated target");
            fs::write(target.join("unrelated"), b"keep").expect("write unrelated sentinel");
        }
        fs::write(roots.target[1].join("ratings-v1.json"), b"newer ratings")
            .expect("write conflict sentinel");
        let filler = roots.target[index].join("fixture-space-filler");
        let mut file = File::create(&filler).expect("create bounded filler");
        let mut exhausted = false;
        for _ in 0..256 {
            if let Err(error) = file.write_all(&[0; 4096]) {
                assert_eq!(error.kind(), io::ErrorKind::StorageFull);
                exhausted = true;
                break;
            }
        }
        drop(file);
        assert!(
            exhausted,
            "fixture must exhaust its bounded tmpfs within 1 MiB"
        );
        assert!(matches!(
            roots.decide(Choice::Import),
            Err(Error::Io(io::ErrorKind::StorageFull))
        ));
        for (index, name) in [
            "settings.toml",
            "music-collections-v1.json",
            "music-queue-v1.json",
        ]
        .into_iter()
        .enumerate()
        {
            let original =
                fs::read(roots.source[index].join(name)).expect("read original after exhaustion");
            assert_eq!(original, vec![b'x'; 8192]);
            if let Ok(published) = fs::read(roots.target[index].join(name)) {
                assert_eq!(published, original, "only complete data may be published");
            }
        }
        fs::remove_file(filler).expect("release bounded fixture space");
        roots
            .decide(Choice::Import)
            .expect("resume with available storage");
        assert_eq!(
            roots.inspect().expect("inspect recovered import"),
            Status::Ready
        );
        for (index, name) in [
            "settings.toml",
            "music-collections-v1.json",
            "music-queue-v1.json",
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                fs::read(roots.target[index].join(name)).expect("read recovered data"),
                vec![b'x'; 8192]
            );
            assert_eq!(
                fs::read(roots.target[index].join("unrelated")).expect("read unrelated sentinel"),
                b"keep"
            );
        }
        assert_eq!(
            fs::read(roots.target[1].join("ratings-v1.json")).expect("read preserved conflict"),
            b"newer ratings"
        );
        return;
    }
    for index in 0..3 {
        let fixture = Fixture::new();
        for (root, name) in [
            "settings.toml",
            "music-collections-v1.json",
            "music-queue-v1.json",
        ]
        .into_iter()
        .enumerate()
        {
            fixture.source(root, name, &vec![b'x'; 8192]);
        }
        fixture.source(1, "ratings-v1.json", b"older ratings");
        fs::create_dir_all(&fixture.roots.target[index]).expect("create bounded mount target");
        let output = std::process::Command::new("bwrap")
            .args(["--die-with-parent", "--unshare-pid", "--unshare-net", "--ro-bind", "/", "/", "--bind"])
            .arg(&fixture.path).arg(&fixture.path)
            .args(["--proc", "/proc", "--dev", "/dev", "--size", "131072", "--tmpfs"])
            .arg(&fixture.roots.target[index]).arg("--")
            .arg(std::env::current_exe().expect("current test executable"))
            .args(["paths::legacy_import::tests::storage_exhaustion_preserves_originals_and_recovers_after_space_is_freed", "--exact", "--nocapture"])
            .env(CHILD, &fixture.path).env("HERMES_IMPORT_STORAGE_INDEX", index.to_string())
            .output().expect("run private bounded filesystem fixture");
        assert!(
            output.status.success(),
            "bounded root {index} failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
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
        assert_eq!(
            fs::read(orphan.path().join("content")).expect("read unknown legacy stage"),
            b"partial journal"
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
