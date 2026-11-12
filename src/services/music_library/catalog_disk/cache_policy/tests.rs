use super::*;
use rusqlite::Connection;
use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hermes-public-quota-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("private fixture root");
        Self(path)
    }

    fn general(&self, id: u64) -> PathBuf {
        self.0.join(format!("general-{id:064x}.sqlite"))
    }

    fn normalized(&self, id: u64) -> PathBuf {
        let directory = self.0.join(format!("normalized-{id:064x}"));
        if !directory.exists() {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .expect("normalized directory");
        }
        directory.join("source.sqlite")
    }

    fn stage(&self, path: &Path, kind: Kind, extra: usize) -> (DirectoryHandle, Stage) {
        self.try_stage(path, kind, extra)
            .expect("private incoming stage")
    }

    fn try_stage(
        &self,
        path: &Path,
        kind: Kind,
        extra: usize,
    ) -> Result<(DirectoryHandle, Stage), String> {
        let directory = DirectoryHandle::open(path.parent().expect("target parent"), false)?;
        lock_directory(&directory.0)?;
        let stage = Stage::new(&directory)?;
        database(&stage.file(), kind, extra);
        let connection = Connection::open(stage.file()).expect("distinct incoming revision");
        connection
            .execute("UPDATE marker SET value=x'08' WHERE rowid=1", [])
            .expect("incoming marker differs from saved cache");
        connection.close().expect("close incoming revision");
        Ok((directory, stage))
    }

    fn save(&self, path: &Path, kind: Kind, budget: u64) -> Result<(), String> {
        let (directory, stage) = self.try_stage(path, kind, 0)?;
        publish_with_budget(
            &directory,
            path.file_name().expect("target name"),
            &stage,
            &Cancellation::default(),
            budget,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

fn database(path: &Path, kind: Kind, extra: usize) {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .expect("private database file");
    let connection = Connection::open(path).expect("SQLite fixture");
    connection.execute_batch("PRAGMA page_size=512; CREATE TABLE marker(value BLOB); INSERT INTO marker VALUES(x'07');").expect("fixture schema");
    if extra > 0 {
        connection
            .execute(
                "INSERT INTO marker VALUES(zeroblob(?1))",
                [i64::try_from(extra).expect("bounded generated payload")],
            )
            .expect("fixture payload");
    }
    let (version, application) = match kind {
        Kind::General => (1, 0),
        Kind::Normalized => (2, 0x48515331),
    };
    connection
        .pragma_update(None, "user_version", version)
        .expect("fixture version");
    connection
        .pragma_update(None, "application_id", application)
        .expect("fixture application");
    connection.close().expect("close fixture SQLite");
    file.sync_all().expect("sync fixture database");
}

fn length(path: &Path) -> u64 {
    fs::metadata(path).expect("database length").len()
}

fn modified(path: &Path, seconds: u64) {
    File::open(path)
        .expect("timestamp fixture")
        .set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
        .expect("publication timestamp");
}

fn seed(path: &Path, kind: Kind, seconds: u64) -> u64 {
    database(path, kind, 0);
    modified(path, seconds);
    length(path)
}

#[test]
fn both_catalog_kinds_share_a_budget_in_a_custom_named_root() {
    let fixture = Fixture::new();
    let general = fixture.general(1);
    let normalized = fixture.normalized(2);
    let size = seed(&general, Kind::General, 1);
    fixture
        .save(&normalized, Kind::Normalized, size)
        .expect("normalized evicts general");
    assert!(!general.exists());
    assert!(normalized.exists());
    fixture
        .save(&general, Kind::General, size)
        .expect("general evicts normalized");
    assert!(general.exists());
    assert!(!normalized.exists());
    assert!(normalized.parent().expect("normalized parent").is_dir());
}

#[test]
fn replacement_subtracts_the_existing_target_and_does_not_relock_current_inode() {
    let fixture = Fixture::new();
    for (path, kind) in [
        (fixture.general(1), Kind::General),
        (fixture.normalized(2), Kind::Normalized),
    ] {
        let size = seed(&path, kind, 1);
        let peer = fixture.general(3);
        let peer_size = seed(&peer, Kind::General, 2);
        fixture
            .save(&path, kind, size + peer_size)
            .expect("exact replacement fits");
        assert!(peer.exists());
        assert_eq!(length(&path), size);
        fs::remove_file(peer).expect("reset peer");
        fs::remove_file(path).expect("reset target");
    }
}

#[test]
fn failed_publication_preserves_target_after_prior_eligible_evictions() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    let eligible = fixture.normalized(2);
    let protected = fixture.general(3);
    let budget = seed(&target, Kind::General, 1);
    seed(&eligible, Kind::Normalized, 2);
    seed(&protected, Kind::General, 3);
    let connection = Connection::open(&protected).expect("generated future cache");
    connection
        .pragma_update(None, "user_version", 99)
        .expect("future schema fixture");
    connection.close().expect("close future cache");
    let original = fs::read(&target).expect("original target bytes");
    let identity = inode(&fs::metadata(&target).expect("target identity"));
    assert!(fixture.save(&target, Kind::General, budget).is_err());
    assert_eq!(fs::read(&target).expect("preserved target"), original);
    assert_eq!(
        inode(&fs::metadata(&target).expect("target identity")),
        identity
    );
    assert!(protected.exists());
    assert!(!eligible.exists());
}

#[test]
fn publication_cannot_bypass_an_independently_held_shared_lock() {
    let fixture = Fixture::new();
    let root = DirectoryHandle::open(&fixture.0, false).expect("namespace root");
    let lock = publication_lock(&root).expect("independent quota lock");
    lock.try_lock().expect("hold quota lock");
    let target = fixture.general(1);
    let failure = fixture
        .save(&target, Kind::General, PUBLIC_BYTES)
        .expect_err("publication must honor the independent lock");
    assert!(failure.contains("lock acquisition failed"), "{failure}");
    assert!(!target.exists());
    lock.unlock().expect("release quota lock");
    fixture
        .save(&target, Kind::General, PUBLIC_BYTES)
        .expect("publication resumes after lock release");
}

#[test]
fn post_rename_sync_failure_reports_error_with_new_target_and_released_lock() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    seed(&target, Kind::General, 1);
    let old_identity = inode(&fs::metadata(&target).expect("original identity"));
    let (directory, stage) = fixture.stage(&target, Kind::General, 0);
    let failure = publish_with_budget_and_sync(
        &directory,
        target.file_name().expect("target name"),
        &stage,
        &Cancellation::default(),
        PUBLIC_BYTES,
        |_| Err(std::io::Error::other("generated directory sync failure")),
    )
    .expect_err("final sync failure must be reported");
    assert!(failure.contains("generated directory sync failure"));
    assert_ne!(
        inode(&fs::metadata(&target).expect("new target identity")),
        old_identity
    );
    assert!(!stage.file().exists());
    let connection = Connection::open(&target).expect("complete new database");
    let marker: Vec<u8> = connection
        .query_row("SELECT value FROM marker WHERE rowid=1", [], |row| {
            row.get(0)
        })
        .expect("read new revision");
    assert_eq!(marker, [8]);
    let lock = publication_lock(&directory).expect("quota lock after failure");
    lock.try_lock()
        .expect("failed publication released shared lock");
}

#[test]
fn oversized_incoming_preserves_target_and_every_eviction_candidate() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    let peer = fixture.normalized(2);
    seed(&target, Kind::General, 1);
    seed(&peer, Kind::Normalized, 2);
    let original = fs::read(&target).expect("original public bytes");
    let (directory, stage) = fixture.stage(&target, Kind::General, 8192);
    let result = publish_with_budget(
        &directory,
        target.file_name().expect("target name"),
        &stage,
        &Cancellation::default(),
        length(&target),
    );
    assert!(
        result
            .expect_err("oversized incoming must fail")
            .contains("incoming database")
    );
    assert_eq!(fs::read(&target).expect("preserved bytes"), original);
    assert!(peer.exists());
    assert!(stage.file().exists());
}

#[test]
fn eviction_orders_publication_mtime_across_catalog_kinds_and_ignores_reads() {
    let fixture = Fixture::new();
    let oldest = fixture.normalized(1);
    let middle = fixture.general(2);
    let newest = fixture.general(3);
    let size = seed(&oldest, Kind::Normalized, 10);
    seed(&middle, Kind::General, 20);
    seed(&newest, Kind::General, 30);
    let _recent_read = fs::read(&oldest).expect("read old publication");
    let incoming = fixture.normalized(4);
    fixture
        .save(&incoming, Kind::Normalized, 3 * size)
        .expect("evict oldest publication");
    assert!(!oldest.exists());
    assert!(middle.exists());
    assert!(newest.exists());
    assert!(incoming.exists());
}

#[test]
fn a_busy_oldest_directory_is_skipped_without_waiting() {
    let fixture = Fixture::new();
    let busy = fixture.normalized(1);
    let available = fixture.normalized(2);
    let size = seed(&busy, Kind::Normalized, 1);
    seed(&available, Kind::Normalized, 2);
    let held =
        DirectoryHandle::open(busy.parent().expect("busy parent"), false).expect("busy directory");
    lock_directory(&held.0).expect("simulate active builder");
    fixture
        .save(&fixture.general(3), Kind::General, 2 * size)
        .expect("skip busy and evict available");
    assert!(busy.exists());
    assert!(!available.exists());
}

#[test]
fn busy_exhaustion_is_visible_and_preserves_replacement_target() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    let busy = fixture.normalized(2);
    let size = seed(&target, Kind::General, 1);
    seed(&busy, Kind::Normalized, 2);
    let original = fs::read(&target).expect("original target");
    let held = DirectoryHandle::open(busy.parent().expect("busy parent"), false)
        .expect("pin busy directory");
    lock_directory(&held.0).expect("active builder");
    let failure = fixture
        .save(&target, Kind::General, size)
        .expect_err("busy cache cannot fit");
    assert!(failure.contains("busy or protected"));
    assert_eq!(fs::read(target).expect("preserved target"), original);
    assert!(busy.exists());
}

#[test]
fn a_normalized_publisher_skips_general_files_when_the_root_lock_is_busy() {
    let fixture = Fixture::new();
    let general = fixture.general(1);
    let size = seed(&general, Kind::General, 1);
    let root = DirectoryHandle::open(&fixture.0, false).expect("pin root");
    lock_directory(&root.0).expect("general builder root lock");
    let target = fixture.normalized(2);
    assert!(
        fixture
            .save(&target, Kind::Normalized, size)
            .expect_err("general publication is protected")
            .contains("busy or protected")
    );
    assert!(general.exists());
    assert!(!target.exists());
}

#[test]
fn unknown_future_and_wrong_application_databases_are_counted_but_preserved() {
    let fixture = Fixture::new();
    let general_future = fixture.general(1);
    let normalized_future = fixture.normalized(2);
    let wrong_application = fixture.normalized(3);
    let size = seed(&general_future, Kind::General, 1);
    seed(&normalized_future, Kind::Normalized, 2);
    seed(&wrong_application, Kind::Normalized, 3);
    for path in [&general_future, &normalized_future] {
        Connection::open(path)
            .expect("future fixture")
            .pragma_update(None, "user_version", 99)
            .expect("future schema version");
    }
    Connection::open(&wrong_application)
        .expect("foreign fixture")
        .pragma_update(None, "application_id", 17)
        .expect("foreign application");
    let incoming = fixture.general(4);
    assert!(
        fixture
            .save(&incoming, Kind::General, 3 * size)
            .expect_err("protected bytes count toward quota")
            .contains("busy or protected")
    );
    for path in [&general_future, &normalized_future, &wrong_application] {
        assert!(path.exists());
    }
    assert!(!incoming.exists());
}

#[test]
fn corrupt_headers_and_unsafe_regular_files_are_counted_but_never_evicted() {
    let fixture = Fixture::new();
    let corrupt = fixture.general(1);
    let shared = fixture.general(2);
    let permissive = fixture.general(3);
    let size = seed(&corrupt, Kind::General, 1);
    seed(&shared, Kind::General, 2);
    seed(&permissive, Kind::General, 3);
    OpenOptions::new()
        .write(true)
        .open(&corrupt)
        .expect("corrupt header fixture")
        .write_all_at(b"unknown format!!", 0)
        .expect("invalidate SQLite magic");
    fs::hard_link(&shared, fixture.0.join("unrelated-hardlink")).expect("multiply linked file");
    fs::set_permissions(&permissive, fs::Permissions::from_mode(0o644))
        .expect("public mode fixture");
    assert!(
        fixture
            .save(&fixture.general(4), Kind::General, 3 * size)
            .is_err()
    );
    assert!(corrupt.exists());
    assert!(shared.exists());
    assert!(permissive.exists());
}

#[test]
fn private_stages_journals_artwork_and_rank_files_are_excluded() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    let size = seed(&target, Kind::General, 1);
    let private = fixture.0.join(".snapshot-v2-999999-1");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&private)
        .expect("unknown private stage");
    let private_file = private.join("snapshot.sqlite");
    database(&private_file, Kind::General, 16384);
    let private_lease = DirectoryHandle::open(&private, false).expect("active private revision");
    private_lease.0.try_lock().expect("hold the revision lease");
    for name in [
        "general-artwork.sqlite",
        "query-ranks.sqlite",
        "artwork.db",
        "source.sqlite-journal",
        "general-FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF.sqlite",
    ] {
        fs::write(fixture.0.join(name), vec![7; 16384]).expect("unmanaged fixture");
    }
    let journal = target.with_extension("sqlite-journal");
    fs::write(&journal, vec![3; 16384]).expect("public journal excluded");
    let (directory, stage) = fixture.stage(&target, Kind::General, 0);
    let reader = Stage::new(&directory).expect("active private reader");
    database(&reader.file(), Kind::General, 16384);
    publish_with_budget(
        &directory,
        target.file_name().expect("target name"),
        &stage,
        &Cancellation::default(),
        size,
    )
    .expect("only main public bytes count");
    assert!(private_file.exists());
    assert!(reader.file().exists());
    assert!(journal.exists());
}

#[test]
fn an_active_private_sqlite_reader_survives_public_eviction() {
    let fixture = Fixture::new();
    let public = fixture.normalized(1);
    let size = seed(&public, Kind::Normalized, 1);
    let directory = DirectoryHandle::open(public.parent().expect("public parent"), false)
        .expect("reader parent");
    lock_directory(&directory.0).expect("private copy acquisition lock");
    let reader = Stage::new(&directory).expect("reader stage");
    reader
        .copy(
            open_file(&public).expect("public source"),
            &Cancellation::default(),
        )
        .expect("reader private copy");
    let connection = reader.connect(false).expect("independent SQLite revision");
    directory.0.unlock().expect("release acquisition lock");
    fixture
        .save(&fixture.general(2), Kind::General, size)
        .expect("evict only public revision");
    assert!(!public.exists());
    let value: Vec<u8> = connection
        .query_row("SELECT value FROM marker", [], |row| row.get(0))
        .expect("private revision still readable");
    assert_eq!(value, [7]);
    drop(connection);
    assert!(reader.file().exists());
}

#[test]
fn cancelled_publication_and_discovery_preserve_public_bytes() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    let size = seed(&target, Kind::General, 1);
    let peer = fixture.normalized(2);
    seed(&peer, Kind::Normalized, 2);
    let original = fs::read(&target).expect("original public bytes");
    let (directory, stage) = fixture.stage(&target, Kind::General, 0);
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        publish_with_budget(
            &directory,
            target.file_name().expect("target name"),
            &stage,
            &cancellation,
            size
        )
        .expect_err("cancelled publication")
        .contains("cancelled")
    );
    let namespace = namespace(&directory, target.file_name().expect("target name"))
        .expect("recognize namespace")
        .expect("canonical target");
    assert!(
        discover(
            &namespace,
            target.file_name().expect("target name"),
            &cancellation
        )
        .err()
        .expect("cancelled discovery")
        .contains("cancelled")
    );
    assert_eq!(fs::read(target).expect("preserved target"), original);
    assert!(peer.exists());
}

#[test]
fn malformed_or_symlinked_lock_files_are_rejected_without_publication() {
    for attack in [
        "nonempty",
        "mode",
        "hardlink",
        "symlink",
        "directory",
        "fifo",
    ] {
        let fixture = Fixture::new();
        let target = fixture.general(1);
        let size = seed(&target, Kind::General, 1);
        let original = fs::read(&target).expect("original target");
        let lock = fixture.0.join(LOCK_NAME);
        match attack {
            "directory" => fs::create_dir(&lock).expect("lock directory attack"),
            "symlink" => symlink(&target, &lock).expect("lock symlink attack"),
            "fifo" => rustix::fs::mknodat(
                rustix::fs::CWD,
                &lock,
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::from_bits_truncate(0o600),
                0,
            )
            .expect("lock FIFO attack"),
            _ => {
                let file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&lock)
                    .expect("lock attack fixture");
                match attack {
                    "nonempty" => file.set_len(1).expect("nonempty lock"),
                    "mode" => file
                        .set_permissions(fs::Permissions::from_mode(0o644))
                        .expect("permissive lock"),
                    "hardlink" => fs::hard_link(&lock, fixture.0.join("other-lock-link"))
                        .expect("linked lock"),
                    _ => unreachable!("known lock fixture"),
                }
            }
        }
        assert!(
            fixture.save(&target, Kind::General, size).is_err(),
            "accepted {attack} lock"
        );
        assert_eq!(fs::read(&target).expect("target intact"), original);
    }
}

#[test]
fn symlinks_fifos_and_nonprivate_managed_directories_fail_discovery() {
    for attack in [
        "database-symlink",
        "database-fifo",
        "directory-symlink",
        "directory-mode",
    ] {
        let fixture = Fixture::new();
        let target = fixture.general(1);
        let size = seed(&target, Kind::General, 1);
        let original = fs::read(&target).expect("original target");
        match attack {
            "database-symlink" => symlink(&target, fixture.general(2)).expect("database symlink"),
            "database-fifo" => rustix::fs::mknodat(
                rustix::fs::CWD,
                fixture.general(2),
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::from_bits_truncate(0o600),
                0,
            )
            .expect("database FIFO"),
            "directory-symlink" => {
                symlink(&fixture.0, fixture.0.join(format!("normalized-{:064x}", 2)))
                    .expect("directory symlink")
            }
            "directory-mode" => {
                let path = fixture.normalized(2);
                fs::set_permissions(
                    path.parent().expect("normalized parent"),
                    fs::Permissions::from_mode(0o755),
                )
                .expect("permissive directory");
            }
            _ => unreachable!("known discovery fixture"),
        }
        assert!(
            fixture.save(&target, Kind::General, size).is_err(),
            "accepted {attack}"
        );
        assert_eq!(fs::read(&target).expect("preserved target"), original);
    }
}

#[test]
fn discovery_limit_is_an_error_before_any_eviction_or_replacement() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    let peer = fixture.normalized(2);
    let size = seed(&target, Kind::General, 1);
    seed(&peer, Kind::Normalized, 2);
    let original = fs::read(&target).expect("original target");
    for index in 0..DISCOVERY_ENTRIES {
        File::create(fixture.0.join(format!("unmanaged-{index}")))
            .expect("bounded-discovery fixture");
    }
    assert!(
        fixture
            .save(&target, Kind::General, size)
            .expect_err("partial discovery must fail")
            .contains("discovery exceeds")
    );
    assert_eq!(fs::read(target).expect("preserved target"), original);
    assert!(peer.exists());
}

#[test]
fn arbitrary_fixture_paths_use_checked_rename_without_creating_a_quota_lock() {
    let fixture = Fixture::new();
    let target = fixture.0.join("fixture.sqlite");
    seed(&target, Kind::General, 1);
    fixture
        .save(&target, Kind::General, 0)
        .expect("noncanonical fixture bypasses quota");
    assert!(!fixture.0.join(LOCK_NAME).exists());
    let (directory, stage) = fixture.stage(&target, Kind::General, 0);
    assert!(
        publish_with_budget(
            &directory,
            OsStr::new("../escaped.sqlite"),
            &stage,
            &Cancellation::default(),
            0
        )
        .is_err()
    );
    assert!(target.exists());
}

#[test]
fn publication_rejects_unsafe_target_and_incoming_stage() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    let other = fixture.general(2);
    let size = seed(&other, Kind::General, 1);
    symlink(&other, &target).expect("symlink target");
    assert!(fixture.save(&target, Kind::General, size).is_err());
    fs::remove_file(&target).expect("remove malicious target");
    let (directory, stage) = fixture.stage(&target, Kind::General, 0);
    fs::remove_file(stage.file()).expect("replace incoming stage");
    symlink(&other, stage.file()).expect("symlink incoming stage");
    assert!(
        publish_with_budget(
            &directory,
            target.file_name().expect("target name"),
            &stage,
            &Cancellation::default(),
            size
        )
        .is_err()
    );
    assert!(!target.exists());
    assert!(other.exists());
}

#[test]
fn candidate_identity_is_revalidated_before_unlink() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    let peer = fixture.normalized(2);
    seed(&peer, Kind::Normalized, 1);
    let (directory, _stage) = fixture.stage(&target, Kind::General, 0);
    let namespace = namespace(&directory, target.file_name().expect("target name"))
        .expect("namespace")
        .expect("canonical namespace");
    let (_, candidates) = discover(
        &namespace,
        target.file_name().expect("target name"),
        &Cancellation::default(),
    )
    .expect("discover peer");
    let original = open_file(&peer).expect("retain old inode");
    fs::remove_file(&peer).expect("replace discovered peer");
    database(&peer, Kind::Normalized, 0);
    let replacement = fs::read(&peer).expect("replacement bytes");
    assert!(
        evict(
            &namespace,
            &directory,
            &candidates[0],
            &Cancellation::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&peer).expect("replacement preserved"), replacement);
    drop(original);
}

#[test]
fn directory_identity_is_revalidated_before_eviction() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    let peer = fixture.normalized(2);
    seed(&peer, Kind::Normalized, 1);
    let (directory, _stage) = fixture.stage(&target, Kind::General, 0);
    let namespace = namespace(&directory, target.file_name().expect("target name"))
        .expect("namespace")
        .expect("canonical namespace");
    let (_, candidates) = discover(
        &namespace,
        target.file_name().expect("target name"),
        &Cancellation::default(),
    )
    .expect("discover peer");
    let old = fixture.0.join("moved-peer");
    fs::rename(peer.parent().expect("peer parent"), &old).expect("move discovered directory");
    let replacement = fixture.normalized(2);
    seed(&replacement, Kind::Normalized, 1);
    assert!(
        evict(
            &namespace,
            &directory,
            &candidates[0],
            &Cancellation::default()
        )
        .is_err()
    );
    assert!(replacement.exists());
    assert!(old.join("source.sqlite").exists());
}

#[test]
fn descriptor_pinned_publication_survives_a_root_path_move() {
    let fixture = Fixture::new();
    let root = fixture.0.join("original-root");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .expect("initial namespace");
    let target = root.join(format!("general-{:064x}.sqlite", 1));
    let peer = root.join(format!("general-{:064x}.sqlite", 2));
    let size = seed(&peer, Kind::General, 1);
    let (directory, stage) = fixture.stage(&target, Kind::General, 0);
    let moved = fixture.0.join("moved-root");
    fs::rename(&root, &moved).expect("move pinned namespace");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .expect("replacement pathname");
    fs::write(root.join("unrelated"), b"keep").expect("unrelated new root content");
    publish_with_budget(
        &directory,
        target.file_name().expect("target name"),
        &stage,
        &Cancellation::default(),
        size,
    )
    .expect("publish in pinned namespace");
    assert!(
        moved
            .join(target.file_name().expect("target name"))
            .exists()
    );
    assert!(!moved.join(peer.file_name().expect("peer name")).exists());
    assert!(!target.exists());
    assert_eq!(
        fs::read(root.join("unrelated")).expect("replacement root intact"),
        b"keep"
    );
}

#[test]
fn simultaneous_normalized_publishers_serialize_on_the_shared_quota_lock() {
    let fixture = Fixture::new();
    let first = fixture.normalized(1);
    let second = fixture.normalized(2);
    let sample = fixture.general(3);
    let size = seed(&sample, Kind::General, 1);
    fs::remove_file(sample).expect("only incoming publications remain");
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [first.clone(), second.clone()]
        .into_iter()
        .map(|path| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let directory = DirectoryHandle::open(path.parent().expect("target parent"), false)
                    .expect("publisher directory");
                lock_directory(&directory.0).expect("caller acquisition lock");
                let stage = Stage::new(&directory).expect("publisher stage");
                database(&stage.file(), Kind::Normalized, 0);
                barrier.wait();
                // Both fit, even while the other publisher retains its directory
                // lock. Each publication must pass through the same root lock file.
                publish_with_budget(
                    &directory,
                    path.file_name().expect("target name"),
                    &stage,
                    &Cancellation::default(),
                    2 * size,
                )
            })
        })
        .collect();
    for handle in handles {
        handle
            .join()
            .expect("publisher thread")
            .expect("serialized publication");
    }
    assert_eq!(length(&first) + length(&second), 2 * size);
    let root = DirectoryHandle::open(&fixture.0, false).expect("namespace root");
    let lock = publication_lock(&root).expect("secure shared lock");
    lock.try_lock().expect("publication lock released");
}

#[test]
fn publication_timestamp_is_refreshed_at_commit() {
    let fixture = Fixture::new();
    let target = fixture.general(1);
    let (directory, stage) = fixture.stage(&target, Kind::General, 0);
    modified(&stage.file(), 1);
    publish_with_budget(
        &directory,
        target.file_name().expect("target name"),
        &stage,
        &Cancellation::default(),
        length(&stage.file()),
    )
    .expect("publish an old prepared stage");
    assert!(fs::metadata(target).expect("published metadata").mtime() > 1);
}
