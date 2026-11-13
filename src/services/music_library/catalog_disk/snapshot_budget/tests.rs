use super::*;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    process::Command,
    sync::{Arc, Barrier},
};

struct Fixture {
    root: PathBuf,
    directory: DirectoryHandle,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hermes-snapshot-budget-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        let directory = DirectoryHandle::open(&root, true).expect("private generated namespace");
        lock_directory(&directory.0).expect("caller acquisition lock");
        Self { root, directory }
    }

    fn normalized(&self, id: u64) -> DirectoryHandle {
        let path = self.root.join(format!("normalized-{id:064x}"));
        DirectoryHandle::open(&path, true).expect("private normalized root")
    }

    fn admit(&self, bytes: u64, limit: u64) -> Result<Admission, String> {
        Admission::with_limit(&self.directory, bytes, limit, &Cancellation::default())
    }

    fn database(&self) -> Stage {
        let mut stage = Stage::build(&self.directory, &Cancellation::default()).expect("builder");
        let connection = stage.connect(true).expect("private SQLite");
        connection
            .execute_batch("CREATE TABLE value(number INTEGER); INSERT INTO value VALUES(7)")
            .expect("generated database");
        connection.close().expect("close writer before sealing");
        stage
            .seal(&Cancellation::default())
            .expect("seal completed database");
        stage
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn four_full_build_reservations_exhaust_the_shared_budget_and_drop_releases_it() {
    let fixture = Fixture::new();
    let mut stages: Vec<_> = (0..4)
        .map(|_| Stage::new(&fixture.directory).expect("admitted builder"))
        .collect();
    let failure = Stage::new(&fixture.directory)
        .err()
        .expect("fifth builder must fail");
    assert!(failure.contains("snapshot budget"), "{failure}");
    assert!(stages.iter().all(|stage| stage.file().exists()));
    drop(stages.pop());
    let next = Stage::new(&fixture.directory).expect("freed reservation can be reused");
    assert_eq!(
        read_record(&record_file(&next.directory).expect("marker")).expect("amount"),
        MAX_DATABASE_BYTES
    );
}

#[test]
fn sealed_readers_charge_actual_bytes_and_reject_reopening_for_writes() {
    let fixture = Fixture::new();
    let stage = fixture.database();
    let size = fs::metadata(stage.file()).expect("snapshot metadata").len();
    assert!(size < MAX_DATABASE_BYTES);
    assert_eq!(stage.allowance, size);
    assert_eq!(
        read_record(&record_file(&stage.directory).expect("marker")).expect("amount"),
        size
    );
    assert!(stage.open_connection(true).is_err());
    let connection = stage.connect(false).expect("sealed reader");
    assert_eq!(
        connection
            .query_row("SELECT number FROM value", [], |row| row.get::<_, i64>(0))
            .expect("value"),
        7
    );
    drop(connection);
    assert!(fixture.admit(1, size).is_err());
    drop(fixture.admit(0, size).expect("exact occupied boundary"));
    let copied = Stage::copied(
        &fixture.directory,
        File::open(stage.file()).expect("source"),
        &Cancellation::default(),
    )
    .expect("exact-size copy");
    assert_eq!(copied.allowance, size);
    assert!(!copied.writable);
    assert!(copied.open_connection(true).is_err());
    assert!(fixture.admit(1, 2 * size).is_err());
}

#[test]
fn read_copy_rejects_source_growth_before_exceeding_its_reservation() {
    let fixture = Fixture::new();
    let stage = Stage::reserved(&fixture.directory, 1, false, &Cancellation::default())
        .expect("small reservation");
    let path = fixture.root.join("source");
    let mut source = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .expect("private source");
    source.write_all(b"too large").expect("generated data");
    assert!(stage.copy(source, &Cancellation::default()).is_err());
    assert_eq!(fs::metadata(stage.file()).expect("bounded file").len(), 0);
}

#[test]
fn failed_reservation_shrink_still_seals_the_writer() {
    let fixture = Fixture::new();
    let mut stage = Stage::new(&fixture.directory).expect("builder");
    fs::write(stage.directory.pinned().join(RECORD), [0u8; 16]).expect("corrupt disposable marker");
    assert!(stage.seal(&Cancellation::default()).is_err());
    assert!(!stage.writable);
    assert!(stage.open_connection(true).is_err());
}

#[test]
fn dropped_stage_binding_is_zero_even_when_discovery_already_opened_it() {
    let fixture = Fixture::new();
    let stage = Stage::new(&fixture.directory).expect("live stage");
    let name = stage.name.clone();
    let opened = DirectoryHandle::at(&fixture.directory, &name).expect("second discovery open");
    drop(stage);
    assert_eq!(
        reservation_charge(&fixture.directory, &name, &opened).expect("detached accounting"),
        0
    );
}

#[test]
fn live_prefix_does_not_starve_recovery_of_a_later_malformed_orphan() {
    let fixture = Fixture::new();
    let live: Vec<_> = (0..MAX_RECLAIMED_STAGES)
        .map(|_| fixture.database())
        .collect();
    let orphan = OsStr::new(".snapshot-v3-123-900");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(fixture.root.join(orphan))
        .expect("abandoned stage");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(fixture.root.join(orphan).join(RECORD))
        .expect("private malformed record");
    file.write_all(&[0u8; 16]).expect("malformed contents");
    let mut attempts = MAX_RECLAIMED_STAGES;
    let mut used = 0;
    for stage in &live {
        used = add_charge(
            used,
            &fixture.directory,
            &fixture.directory,
            &stage.name,
            &mut attempts,
            &Cancellation::default(),
        )
        .expect("live prefix remains readable");
    }
    assert_eq!(attempts, MAX_RECLAIMED_STAGES);
    let total = add_charge(
        used,
        &fixture.directory,
        &fixture.directory,
        orphan,
        &mut attempts,
        &Cancellation::default(),
    )
    .expect("later malformed orphan is reclaimed");
    assert_eq!(total, used);
    assert!(!fixture.root.join(orphan).exists());
    assert_eq!(attempts, MAX_RECLAIMED_STAGES - 1);
    for _ in 0..3 {
        drop(
            fixture
                .admit(1, TOTAL_BYTES)
                .expect("repeated admission makes progress"),
        );
    }
}

#[test]
fn interrupted_marker_write_and_post_write_failure_leave_the_stage_sealed() {
    for partial in [true, false] {
        let fixture = Fixture::new();
        let mut stage = Stage::new(&fixture.directory).expect("writable builder");
        let connection = stage.connect(true).expect("private writer");
        connection
            .execute_batch("CREATE TABLE value(number INTEGER); INSERT INTO value VALUES(7)")
            .expect("schema");
        connection.close().expect("close writer");
        let failure = stage
            .seal_with(&Cancellation::default(), |stage, previous, cancel| {
                reduce_with_write(stage, previous, cancel, |file, bytes| {
                    if partial {
                        file.write_all_at(b"BAD", 0).map_err(error)?;
                    } else {
                        write_record(file, bytes)?;
                    }
                    Err("generated marker write/sync failure".into())
                })
            })
            .expect_err("injected failure must be reported");
        assert!(failure.contains("generated marker"));
        assert!(!stage.writable);
        assert!(stage.open_connection(true).is_err());
        if partial {
            assert!(fixture.admit(1, TOTAL_BYTES).is_err());
        } else {
            assert_eq!(
                read_record(&record_file(&stage.directory).expect("record"))
                    .expect("committed amount"),
                stage.allowance
            );
            drop(
                fixture
                    .admit(1, stage.allowance + 1)
                    .expect("committed smaller allowance"),
            );
        }
    }
}

#[test]
fn malformed_live_reservations_fail_admission_without_deleting_the_reader() {
    let fixture = Fixture::new();
    let stage = Stage::new(&fixture.directory).expect("live revision");
    fs::write(stage.directory.pinned().join(RECORD), b"unknown record").expect("malformed marker");
    assert!(fixture.admit(1, TOTAL_BYTES).is_err());
    assert!(stage.file().exists());
    drop(stage);
    drop(
        fixture
            .admit(1, TOTAL_BYTES)
            .expect("admission recovers after owner closes"),
    );
}

#[test]
fn older_lease_free_stages_are_preserved_and_conservatively_charged() {
    let fixture = Fixture::new();
    let old = fixture.root.join(".snapshot-123-0");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&old)
        .expect("legacy stage");
    assert!(fixture.admit(1, MAX_DATABASE_BYTES).is_err());
    assert!(old.exists());
}

#[test]
fn version_two_leases_are_charged_until_the_owner_releases_them() {
    let fixture = Fixture::new();
    let name = OsStr::new(".snapshot-v2-123-0");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(fixture.root.join(name))
        .expect("older leased stage");
    let directory = DirectoryHandle::at(&fixture.directory, name).expect("pin stage");
    directory.0.try_lock().expect("live older lease");
    assert!(fixture.admit(1, MAX_DATABASE_BYTES).is_err());
    drop(directory);
    drop(
        fixture
            .admit(1, MAX_DATABASE_BYTES)
            .expect("reclaim older abandoned lease"),
    );
    assert!(!fixture.root.join(name).exists());
}

#[test]
fn namespace_sweep_recovers_an_abandoned_stage_from_another_scope() {
    let fixture = Fixture::new();
    let current = fixture.normalized(1);
    let peer = fixture.normalized(2);
    let peer_path = fs::read_link(peer.pinned()).expect("peer path");
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "services::music_library::catalog_disk::stage_recovery::tests::crash_fixture_child",
            "--nocapture",
        ])
        .env("HERMES_STAGE_CRASH_FIXTURE", &peer_path)
        .output()
        .expect("crash helper");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_dir(&peer_path)
            .expect("peer contents")
            .next()
            .is_some()
    );
    let stage = Stage::new(&current).expect("new scope triggers namespace maintenance");
    assert!(stage.file().exists());
    assert!(
        fs::read_dir(&peer_path)
            .expect("cleaned peer")
            .next()
            .is_none()
    );
}

#[test]
fn concurrent_scope_builders_cannot_admit_beyond_the_namespace_limit() {
    let fixture = Fixture::new();
    let occupied: Vec<_> = (0..3)
        .map(|_| Stage::new(&fixture.directory).expect("existing builder"))
        .collect();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [fixture.normalized(1), fixture.normalized(2)]
        .into_iter()
        .map(|parent| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                lock_directory(&parent.0).expect("peer acquisition");
                barrier.wait();
                Stage::build(&parent, &Cancellation::default())
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().expect("builder thread"))
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    assert!(occupied.iter().all(|stage| stage.file().exists()));
}

#[test]
fn unsafe_control_objects_and_cancelled_admission_preserve_existing_data() {
    let fixture = Fixture::new();
    let outside = fixture.root.join("outside");
    fs::write(&outside, b"keep").expect("unrelated file");
    let lock = fixture.root.join(LOCK);
    symlink(&outside, &lock).expect("unsafe lock");
    assert!(fixture.admit(1, TOTAL_BYTES).is_err());
    assert_eq!(fs::read(&outside).expect("preserved"), b"keep");
    fs::remove_file(&lock).expect("remove fixture symlink");
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(Admission::acquire(&fixture.directory, 1, &cancellation).is_err());
    assert!(!lock.exists());
    let orphan = fixture.root.join(".snapshot-v3-123-1");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&orphan)
        .expect("orphan");
    fs::set_permissions(&orphan, fs::Permissions::from_mode(0o755)).expect("unsafe stage");
    assert!(fixture.admit(1, TOTAL_BYTES).is_err());
    assert!(orphan.exists());
}

#[test]
fn discovery_budget_is_shared_across_scope_directories() {
    let fixture = Fixture::new();
    for id in 0..2 {
        let directory = fixture.normalized(id);
        for number in 0..INSPECTION_ENTRIES / 2 {
            fs::write(directory.pinned().join(format!("unmanaged-{number}")), [])
                .expect("generated entry");
        }
    }
    assert!(
        fixture
            .admit(1, TOTAL_BYTES)
            .err()
            .expect("bounded discovery")
            .contains("inspection budget")
    );
}
