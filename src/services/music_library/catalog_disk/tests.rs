use super::*;
use std::{
    os::unix::fs::{PermissionsExt, symlink},
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

struct Fixture {
    parent: PathBuf,
    root: PathBuf,
    cache: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = std::env::temp_dir().join(format!(
            "hermes-catalog-disk-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&parent).unwrap();
        let root = parent.join("Library");
        fs::create_dir(&root).unwrap();
        Self {
            cache: parent.join("cache/general.sqlite"),
            parent,
            root,
        }
    }

    fn entry(&self, name: impl AsRef<Path>) -> Entry {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"synthetic catalog source").unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();
        Entry {
            path,
            is_directory: false,
            modified_unix_seconds: Some(metadata.mtime()),
            identity: Some(FileIdentity::from_metadata(&metadata)),
            music_tags: None,
        }
    }

    fn save(&self, entries: &[Entry], notice: Option<&str>) {
        write(
            &self.cache,
            &self.root,
            entries,
            notice,
            &Cancellation::default(),
        )
        .unwrap();
    }

    fn load(&self) -> Result<(Vec<Entry>, Option<String>), String> {
        read(
            &self.cache,
            &self.root,
            super::super::MAX_ENTRIES,
            super::super::MAX_INDEX_BYTES,
            &Cancellation::default(),
        )
    }

    fn database(&self) -> Connection {
        Connection::open_with_flags(
            &self.cache,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .unwrap()
    }

    fn assert_no_stages(&self) {
        let names: Vec<_> = fs::read_dir(self.cache.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![self.cache.file_name().unwrap().to_owned()]);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.parent);
    }
}

#[test]
fn restart_preserves_unicode_native_paths_types_identities_tags_and_notice() {
    let fixture = Fixture::new();
    let mut song = fixture.entry("Bj\u{f6}rk/\u{6771}\u{4eac}.flac");
    song.music_tags = Some(MusicTags {
        title: Some("\u{6771}\u{4eac}".into()),
        artist: Some("Bj\u{f6}rk".into()),
        album_artist: Some("Various artists".into()),
        album: Some("Unicode album".into()),
        disc: Some(2),
        track: Some(17),
    });
    let native = fixture.entry(OsString::from_vec(b"opaque-\xff.txt".to_vec()));
    let directory_path = fixture.root.join("Bj\u{f6}rk");
    let metadata = fs::symlink_metadata(&directory_path).unwrap();
    let directory = Entry {
        path: directory_path,
        is_directory: true,
        modified_unix_seconds: None,
        identity: Some(FileIdentity::from_metadata(&metadata)),
        music_tags: None,
    };
    let entries = vec![song, native, directory];
    fixture.save(
        &entries,
        Some("Partial scan: unreadable folders \u{b7} entry limit"),
    );
    assert_eq!(
        fixture.load().unwrap(),
        (
            entries.clone(),
            Some("Partial scan: unreadable folders \u{b7} entry limit".into())
        )
    );
    assert_eq!(fixture.load().unwrap().0, entries);
    assert_eq!(
        fs::metadata(fixture.cache.parent().unwrap())
            .unwrap()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(fs::metadata(&fixture.cache).unwrap().mode() & 0o777, 0o600);
    fixture.assert_no_stages();
}

#[test]
fn empty_snapshot_and_absent_notice_survive_restart() {
    let fixture = Fixture::new();
    fixture.save(&[], None);
    assert_eq!(fixture.load().unwrap(), (Vec::new(), None));
    fixture.assert_no_stages();
}

#[test]
fn root_specific_cache_path_is_separate_from_music_json() {
    let fixture = Fixture::new();
    let first = path(&fixture.root);
    assert_eq!(first, path(&fixture.root));
    assert_ne!(first, path(&fixture.parent));
    assert_ne!(first, super::super::cache::path(&fixture.root));
    assert_eq!(first.extension(), Some(OsStr::new("sqlite")));
}

#[test]
fn changed_and_deleted_sources_are_removed_with_an_additional_notice() {
    let fixture = Fixture::new();
    let changed = fixture.entry("changed.txt");
    let kept = fixture.entry("kept.txt");
    let deleted = fixture.entry("deleted.txt");
    fixture.save(
        &[changed.clone(), kept.clone(), deleted.clone()],
        Some("Original partial scan"),
    );
    fs::write(&changed.path, b"different fixture data").unwrap();
    fs::remove_file(&deleted.path).unwrap();
    let (entries, notice) = fixture.load().unwrap();
    assert_eq!(entries, vec![kept]);
    let notice = notice.unwrap();
    assert!(notice.contains("Original partial scan"));
    assert!(notice.contains("Cached sources changed"));
}

#[test]
fn same_size_source_replacement_is_rejected_by_identity() {
    let fixture = Fixture::new();
    let entry = fixture.entry("replaced.txt");
    fixture.save(std::slice::from_ref(&entry), None);
    fs::rename(&entry.path, fixture.root.join("old.txt")).unwrap();
    fs::write(&entry.path, b"synthetic catalog source").unwrap();
    assert!(fixture.load().unwrap().0.is_empty());
}

#[test]
fn changed_root_and_mismatched_root_cannot_reuse_a_snapshot() {
    let fixture = Fixture::new();
    let entry = fixture.entry("source.txt");
    fixture.save(std::slice::from_ref(&entry), None);
    assert!(
        read(
            &fixture.cache,
            &fixture.parent,
            40_000,
            32 * 1024 * 1024,
            &Cancellation::default()
        )
        .is_err()
    );
    fs::rename(&fixture.root, fixture.parent.join("OldLibrary")).unwrap();
    fs::create_dir(&fixture.root).unwrap();
    assert!(fixture.load().is_err());
    let replacement = fixture.entry("replacement.txt");
    fixture.save(std::slice::from_ref(&replacement), None);
    assert_eq!(fixture.load().unwrap().0, vec![replacement]);
}

#[test]
fn malformed_and_oversized_databases_fall_back_without_deleting_them() {
    let fixture = Fixture::new();
    let entry = fixture.entry("source.txt");
    fixture.save(&[entry], None);
    fs::write(&fixture.cache, b"not sqlite").unwrap();
    assert!(fixture.load().is_err());
    assert_eq!(fs::read(&fixture.cache).unwrap(), b"not sqlite");
    let output = OpenOptions::new().write(true).open(&fixture.cache).unwrap();
    output.set_len(MAX_DATABASE_BYTES + 1).unwrap();
    assert!(fixture.load().is_err());
    assert_eq!(
        fs::metadata(&fixture.cache).unwrap().len(),
        MAX_DATABASE_BYTES + 1
    );
    fixture.assert_no_stages();
}

#[test]
fn entry_and_memory_limits_fail_instead_of_silently_truncating() {
    let fixture = Fixture::new();
    let entries = vec![fixture.entry("one.txt"), fixture.entry("two.txt")];
    fixture.save(&entries, None);
    let loaded = fixture.load().unwrap().0;
    let cost: usize = loaded.iter().map(super::super::entry_bytes).sum();
    assert!(
        read(
            &fixture.cache,
            &fixture.root,
            1,
            cost,
            &Cancellation::default()
        )
        .is_err()
    );
    assert!(
        read(
            &fixture.cache,
            &fixture.root,
            2,
            cost - 1,
            &Cancellation::default()
        )
        .is_err()
    );
    assert_eq!(
        read(
            &fixture.cache,
            &fixture.root,
            2,
            cost,
            &Cancellation::default()
        )
        .unwrap()
        .0,
        entries
    );
}

#[test]
fn oversized_rows_invalid_tags_and_notice_are_rejected() {
    let fixture = Fixture::new();
    let entry = fixture.entry("song.flac");
    fixture.save(std::slice::from_ref(&entry), None);
    let mut invalid = entry.clone();
    invalid.music_tags = Some(MusicTags {
        title: Some("invalid\ncontrol".into()),
        ..MusicTags::default()
    });
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            &[invalid],
            None,
            &Cancellation::default()
        )
        .is_err()
    );
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            std::slice::from_ref(&entry),
            Some(&"x".repeat(MAX_NOTICE_BYTES + 1)),
            &Cancellation::default()
        )
        .is_err()
    );
    let connection = fixture.database();
    connection
        .execute(
            "UPDATE entries SET title=?1, tagged=1",
            ["x".repeat(MAX_ROW_BYTES + 1)],
        )
        .unwrap();
    drop(connection);
    assert!(fixture.load().is_err());
}

#[test]
fn malformed_identity_and_directory_values_are_rejected() {
    let fixture = Fixture::new();
    fixture.save(&[fixture.entry("source.txt")], None);
    let connection = fixture.database();
    connection
        .execute("UPDATE entries SET identity=?1", [b"invalid".as_slice()])
        .unwrap();
    drop(connection);
    assert!(fixture.load().is_err());
    let connection = fixture.database();
    connection
        .execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE entries SET directory=9;")
        .unwrap();
    drop(connection);
    assert!(fixture.load().is_err());
}

#[test]
fn stored_sql_triggers_views_and_altered_schema_are_never_executed() {
    let fixture = Fixture::new();
    let entry = fixture.entry("source.txt");
    fixture.save(std::slice::from_ref(&entry), None);
    let connection = fixture.database();
    connection.execute_batch("CREATE TRIGGER trap BEFORE DELETE ON entries BEGIN SELECT RAISE(ABORT, 'stored trigger executed'); END;").unwrap();
    drop(connection);
    let original = fs::read(&fixture.cache).unwrap();
    let failure = fixture.load().unwrap_err();
    assert!(failure.contains("schema"));
    assert!(!failure.contains("stored trigger executed"));
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            &[entry],
            None,
            &Cancellation::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&fixture.cache).unwrap(), original);
    let connection = fixture.database();
    connection.execute_batch("DROP TRIGGER trap; ALTER TABLE entries RENAME TO old_entries; CREATE VIEW entries AS SELECT * FROM old_entries;").unwrap();
    drop(connection);
    assert!(fixture.load().is_err());
}

#[test]
fn future_version_is_preserved_on_both_read_and_write() {
    let fixture = Fixture::new();
    let entry = fixture.entry("source.txt");
    fixture.save(std::slice::from_ref(&entry), Some("Future notice"));
    let connection = fixture.database();
    connection.execute_batch("PRAGMA user_version=2; CREATE TABLE future_data (value TEXT); INSERT INTO future_data VALUES('must survive');").unwrap();
    drop(connection);
    let original = fs::read(&fixture.cache).unwrap();
    assert!(fixture.load().unwrap_err().contains("newer version"));
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            &[entry],
            None,
            &Cancellation::default()
        )
        .unwrap_err()
        .contains("newer version")
    );
    assert_eq!(fs::read(&fixture.cache).unwrap(), original);
    fixture.assert_no_stages();
}

#[test]
fn symlink_fifo_hardlink_and_insecure_cache_permissions_are_rejected() {
    let fixture = Fixture::new();
    let entry = fixture.entry("source.txt");
    fixture.save(std::slice::from_ref(&entry), None);
    let target = fixture.parent.join("original.sqlite");
    fs::rename(&fixture.cache, &target).unwrap();
    symlink(&target, &fixture.cache).unwrap();
    assert!(fixture.load().is_err());
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            std::slice::from_ref(&entry),
            None,
            &Cancellation::default()
        )
        .is_err()
    );
    fs::remove_file(&fixture.cache).unwrap();
    fs::hard_link(&target, &fixture.cache).unwrap();
    assert!(fixture.load().is_err());
    fs::remove_file(&fixture.cache).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg("-m")
            .arg("600")
            .arg(&fixture.cache)
            .status()
            .unwrap()
            .success()
    );
    assert!(fixture.load().is_err());
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            std::slice::from_ref(&entry),
            None,
            &Cancellation::default()
        )
        .is_err()
    );
    fs::remove_file(&fixture.cache).unwrap();
    fs::rename(&target, &fixture.cache).unwrap();
    fs::set_permissions(&fixture.cache, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(fixture.load().is_err());
    fs::set_permissions(&fixture.cache, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(
        fixture.cache.parent().unwrap(),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(fixture.load().is_err());
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            &[entry],
            None,
            &Cancellation::default()
        )
        .is_err()
    );
}

#[test]
fn symlink_cache_ancestor_and_root_are_rejected() {
    let fixture = Fixture::new();
    let entry = fixture.entry("source.txt");
    fixture.save(&[entry], None);
    let alias = fixture.parent.join("alias");
    symlink(fixture.cache.parent().unwrap(), &alias).unwrap();
    assert!(
        read(
            &alias.join("general.sqlite"),
            &fixture.root,
            40_000,
            32 * 1024 * 1024,
            &Cancellation::default()
        )
        .is_err()
    );
    let root_alias = fixture.parent.join("LibraryAlias");
    symlink(&fixture.root, &root_alias).unwrap();
    assert!(
        read(
            &fixture.cache,
            &root_alias,
            40_000,
            32 * 1024 * 1024,
            &Cancellation::default()
        )
        .is_err()
    );
}

#[test]
fn hidden_escaped_and_symlinked_sources_cannot_be_published() {
    let fixture = Fixture::new();
    let kept = fixture.entry("kept.txt");
    fixture.save(std::slice::from_ref(&kept), None);
    let hidden = fixture.entry(".hidden/source.txt");
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            &[hidden],
            None,
            &Cancellation::default()
        )
        .is_err()
    );
    let mut escaped = kept.clone();
    escaped.path = fixture.root.join("../Library/kept.txt");
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            &[escaped],
            None,
            &Cancellation::default()
        )
        .is_err()
    );
    let actual = fixture.entry("actual/source.txt");
    symlink(fixture.root.join("actual"), fixture.root.join("alias")).unwrap();
    let mut linked = actual.clone();
    linked.path = fixture.root.join("alias/source.txt");
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            &[linked],
            None,
            &Cancellation::default()
        )
        .is_err()
    );
    let mut linked = actual;
    symlink(&linked.path, fixture.root.join("link.txt")).unwrap();
    linked.path = fixture.root.join("link.txt");
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            &[linked],
            None,
            &Cancellation::default()
        )
        .is_err()
    );
    assert_eq!(fixture.load().unwrap().0, vec![kept]);
}

#[test]
fn injected_hidden_and_escape_rows_fail_closed() {
    let fixture = Fixture::new();
    let entry = fixture.entry("source.txt");
    fixture.save(&[entry], None);
    for path in [
        fixture.root.join(".hidden.txt"),
        fixture.root.join("../escaped.txt"),
        fixture.parent.join("outside.txt"),
    ] {
        let connection = fixture.database();
        connection
            .execute("UPDATE entries SET path=?1", [path.as_os_str().as_bytes()])
            .unwrap();
        drop(connection);
        assert!(fixture.load().is_err());
    }
}

#[test]
fn source_turning_into_symlink_or_symlink_ancestor_is_not_restored() {
    let fixture = Fixture::new();
    let file = fixture.entry("source.txt");
    let nested = fixture.entry("folder/nested.txt");
    fixture.save(&[file.clone(), nested], None);
    fs::rename(&file.path, fixture.root.join("original.txt")).unwrap();
    symlink(fixture.root.join("original.txt"), &file.path).unwrap();
    fs::rename(
        fixture.root.join("folder"),
        fixture.root.join("real-folder"),
    )
    .unwrap();
    symlink(
        fixture.root.join("real-folder"),
        fixture.root.join("folder"),
    )
    .unwrap();
    assert!(fixture.load().unwrap().0.is_empty());
}

#[test]
fn cancelled_operations_and_mid_transaction_failure_leave_old_snapshot_unchanged() {
    let fixture = Fixture::new();
    let original = fixture.entry("original.txt");
    fixture.save(std::slice::from_ref(&original), Some("Original notice"));
    let bytes = fs::read(&fixture.cache).unwrap();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        read(
            &fixture.cache,
            &fixture.root,
            40_000,
            32 * 1024 * 1024,
            &cancellation
        )
        .is_err()
    );
    assert!(write(&fixture.cache, &fixture.root, &[], None, &cancellation).is_err());
    let first = fixture.entry("first.txt");
    let stale = fixture.entry("stale.txt");
    fs::remove_file(&stale.path).unwrap();
    assert!(
        write(
            &fixture.cache,
            &fixture.root,
            &[first, stale],
            None,
            &Cancellation::default()
        )
        .is_err()
    );
    assert_eq!(fs::read(&fixture.cache).unwrap(), bytes);
    assert_eq!(
        fixture.load().unwrap(),
        (vec![original], Some("Original notice".into()))
    );
    fixture.assert_no_stages();
}

#[test]
fn cancellation_during_transaction_does_not_publish_or_leave_staging_files() {
    let fixture = Fixture::new();
    let original = fixture.entry("original.txt");
    fixture.save(std::slice::from_ref(&original), None);
    let bytes = fs::read(&fixture.cache).unwrap();
    let entries: Vec<_> = (0..1024)
        .map(|index| fixture.entry(format!("replacement-{index}.txt")))
        .collect();
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancellation = Cancellation::from_shared(cancelled.clone());
    let cache = fixture.cache.clone();
    let root = fixture.root.clone();
    let writer = std::thread::spawn(move || write(&cache, &root, &entries, None, &cancellation));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let journal = fs::read_dir(fixture.cache.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.path().join("snapshot.sqlite-journal").is_file());
        if journal {
            cancelled.store(true, Ordering::Release);
            break;
        }
        assert!(
            !writer.is_finished(),
            "Writer finished before the cancellation fixture saw its transaction"
        );
        assert!(Instant::now() < deadline, "Transaction did not start");
        std::thread::yield_now();
    }
    assert!(writer.join().unwrap().is_err());
    assert_eq!(fs::read(&fixture.cache).unwrap(), bytes);
    assert_eq!(fixture.load().unwrap().0, vec![original]);
    fixture.assert_no_stages();
}

#[test]
fn connection_uses_small_cache_and_disables_stored_code_and_attachments() {
    let fixture = Fixture::new();
    fixture.save(&[], None);
    let (directory, _) = location(&fixture.cache, false).unwrap();
    let stage = Stage::new(&directory).unwrap();
    let connection = stage.connect(true).unwrap();
    assert_eq!(
        connection.limit(Limit::SQLITE_LIMIT_LENGTH).unwrap(),
        MAX_ROW_BYTES as i32
    );
    assert_eq!(connection.limit(Limit::SQLITE_LIMIT_ATTACHED).unwrap(), 0);
    assert!(
        !connection
            .db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA)
            .unwrap()
    );
    assert!(
        !connection
            .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER)
            .unwrap()
    );
    assert!(
        !connection
            .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW)
            .unwrap()
    );
    assert_eq!(
        connection
            .query_row("PRAGMA cache_size", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        -2048
    );
    assert!(
        connection
            .execute_batch("SELECT load_extension('fixture-does-not-exist');")
            .is_err()
    );
}
