use super::*;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
};

struct Fixture {
    root: PathBuf,
    directory: DirectoryHandle,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hermes-stage-recovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let directory = DirectoryHandle::open(&root, true).expect("private generated directory");
        directory.private().expect("private directory permissions");
        lock_directory(&directory.0).expect("exclusive catalog acquisition");
        Self { root, directory }
    }

    fn orphan(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("generated orphan directory");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path.join("snapshot.sqlite"))
            .expect("generated orphan file");
        path
    }

    fn recover(&self) {
        recover(&self.directory, &Cancellation::default()).expect("recover disposable stages");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn recovery_removes_versioned_orphans_and_preserves_live_leases() {
    let fixture = Fixture::new();
    let stage = Stage::new(&fixture.directory).expect("live leased stage");
    let live = fixture.root.join(&stage.name);
    let orphan = fixture.orphan(".snapshot-v2-123-0");
    let empty = fixture.root.join(".snapshot-v2-123-1");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&empty)
        .expect("empty orphan");
    fixture.recover();
    assert!(live.join("snapshot.sqlite").exists());
    assert!(!orphan.exists());
    assert!(!empty.exists());
    drop(stage);
    assert!(!live.exists());
}

#[test]
fn older_stages_unknown_contents_and_unsafe_objects_are_preserved() {
    let fixture = Fixture::new();
    let old = fixture.orphan(".snapshot-123-0");
    let unknown = fixture.orphan(".snapshot-v2-123-0");
    fs::write(unknown.join("keep.txt"), b"unknown content").expect("unknown file");
    let linked = fixture.orphan(".snapshot-v2-123-1");
    fs::hard_link(
        linked.join("snapshot.sqlite"),
        fixture.root.join("linked-file"),
    )
    .expect("shared inode");
    let shared_mode = fixture.orphan(".snapshot-v2-123-2");
    fs::set_permissions(
        shared_mode.join("snapshot.sqlite"),
        fs::Permissions::from_mode(0o644),
    )
    .expect("unsafe permissions");
    let nested = fixture.orphan(".snapshot-v2-123-3");
    fs::create_dir(nested.join("nested")).expect("unknown directory");
    let symlink_stage = fixture.root.join(".snapshot-v2-123-4");
    symlink(&old, &symlink_stage).expect("stage symlink");
    let symlink_file = fixture.orphan(".snapshot-v2-123-5");
    fs::remove_file(symlink_file.join("snapshot.sqlite")).expect("remove fixture file");
    symlink(
        old.join("snapshot.sqlite"),
        symlink_file.join("snapshot.sqlite"),
    )
    .expect("payload symlink");
    fixture.recover();
    for path in [
        old,
        unknown,
        linked,
        shared_mode,
        nested,
        symlink_stage,
        symlink_file,
    ] {
        assert!(fs::symlink_metadata(path).is_ok());
    }
}

#[test]
fn only_strict_versioned_stage_names_are_eligible() {
    for name in [
        ".snapshot-v2-12-0",
        ".snapshot-v3-12-0",
        ".snapshot-v2-4294967295-18446744073709551615",
    ] {
        assert!(leased_stage_name(OsStr::new(name)));
    }
    for name in [
        ".snapshot-12-0",
        ".snapshot-v4-12-0",
        ".snapshot-v2-0-0",
        ".snapshot-v2--0",
        ".snapshot-v2-12-",
        ".snapshot-v2-12-0-extra",
        ".snapshot-v2-+12-0",
        ".snapshot-v2-4294967296-0",
        ".snapshot-v2-12-18446744073709551616",
    ] {
        assert!(!leased_stage_name(OsStr::new(name)), "{name}");
    }
}

#[test]
fn cancelled_recovery_preserves_orphan_files() {
    let fixture = Fixture::new();
    let orphan = fixture.orphan(".snapshot-v2-123-0");
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(recover(&fixture.directory, &cancellation).is_err());
    assert!(orphan.join("snapshot.sqlite").exists());
}

#[test]
fn recovery_budget_does_not_disable_normal_catalog_acquisition() {
    let fixture = Fixture::new();
    for number in 0..=MAX_DIRECTORY_ENTRIES {
        fs::write(fixture.root.join(format!("unmanaged-{number}")), []).expect("disposable entry");
    }
    fixture.recover();
    fixture
        .directory
        .0
        .unlock()
        .expect("release fixture acquisition");
    let path = fixture.root.join("general.sqlite");
    super::super::write(&path, &fixture.root, &[], None, &Cancellation::default())
        .expect("save catalog despite deferred cleanup");
    let (entries, _) = super::super::read(&path, &fixture.root, 10, 1024, &Cancellation::default())
        .expect("read catalog despite deferred cleanup");
    assert!(entries.is_empty());
}

#[test]
fn reader_drop_between_stage_open_and_lease_acquisition_is_harmless() {
    let fixture = Fixture::new();
    let stage = Stage::new(&fixture.directory).expect("live leased reader");
    let name = stage.name.clone();
    let opened =
        DirectoryHandle::at(&fixture.directory, &name).expect("recovery opens reader stage");
    drop(stage);
    recover_stage(&fixture.directory, &name, opened, &Cancellation::default())
        .expect("vanished stage does not reject cache acquisition");
}

#[test]
fn abrupt_child_exit_releases_the_lease_for_the_next_acquisition() {
    let fixture = Fixture::new();
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "services::music_library::catalog_disk::stage_recovery::tests::crash_fixture_child",
            "--nocapture",
        ])
        .env("HERMES_STAGE_CRASH_FIXTURE", &fixture.root)
        .output()
        .expect("private crash helper");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stages: Vec<_> = fs::read_dir(&fixture.root)
        .expect("crash fixture entries")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.file_name().is_some_and(leased_stage_name))
        .collect();
    assert_eq!(stages.len(), 1);
    assert!(stages[0].join("snapshot.sqlite").exists());
    assert!(stages[0].join("snapshot.sqlite-journal").exists());
    fixture.recover();
    assert!(!stages[0].exists());
}

#[test]
fn crash_fixture_child() {
    let Some(root) = std::env::var_os("HERMES_STAGE_CRASH_FIXTURE") else {
        return;
    };
    let directory = DirectoryHandle::open(Path::new(&root), false).expect("generated parent");
    let stage = Stage::new(&directory).expect("leased child stage");
    let connection = stage.connect(true).expect("private SQLite connection");
    connection.execute_batch(
        "CREATE TABLE generated(value INTEGER); INSERT INTO generated VALUES(1); BEGIN IMMEDIATE; UPDATE generated SET value=2;",
    ).expect("uncommitted journal fixture");
    // Deliberately skip destructors to exercise kernel lease release after exit.
    std::process::exit(0);
}
