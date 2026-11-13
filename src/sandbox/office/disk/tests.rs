use super::*;
use std::{
    cell::Cell,
    fs::Permissions,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    process::Command,
    time::{Duration, UNIX_EPOCH},
};

const PDF: &[u8] = b"%PDF-1.7\ngenerated fixture\n%%EOF\n";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "hermes-office-disk-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("expected test value from SystemTime::now() .duration_since(UNIX_EPOCH)")
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("expected test value from fs::DirBuilder::new().mode(0o700).create(&path)");
        Self(path)
    }

    fn root(&self) -> PathBuf {
        self.0.join("cache")
    }

    fn key(&self, name: &str) -> Key {
        let path = self.0.join(name);
        fs::write(&path, b"generated Office fixture").expect("write generated fixture");
        let (path, source) = source(&path).expect("expected test value from source(&path)");
        Key {
            path: path.as_os_str().as_bytes().to_vec(),
            source,
            environment: "isolated-test-environment-v1".into(),
        }
    }

    fn store(&self, key: &Key, bytes: &[u8]) {
        store_at(&self.root(), key, bytes, &Cancellation::default(), || {
            source_current(key)
        })
        .expect("expected test value from store_at(&self.root(), key, bytes, &Cancellation::default(), || { source_current(key) })");
    }

    fn load(&self, key: &Key) -> Option<Vec<u8>> {
        load_at(&self.root(), key, &Cancellation::default(), || {
            source_current(key)
        })
    }

    fn record(&self, key: &Key) -> PathBuf {
        self.root()
            .join(filename(key).expect("expected test value from filename(key)"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

fn source_current(key: &Key) -> bool {
    source(key.input())
        .is_ok_and(|(path, stamp)| path.as_os_str().as_bytes() == key.path && stamp == key.source)
}

fn records(root: &Path) -> Vec<PathBuf> {
    let mut records = fs::read_dir(root)
        .expect("expected test value from fs::read_dir(root)")
        .map(|entry| entry.expect("expected test value from entry").path())
        .filter(|path| {
            path.file_name()
                .expect("expected test value from path.file_name()")
                .to_str()
                .is_some_and(owned_record)
        })
        .collect::<Vec<_>>();
    records.sort();
    records
}

#[test]
fn private_cache_round_trip_is_independent_of_open_handles() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    assert!(fixture.load(&key).is_none());
    fixture.store(&key, PDF);
    let encoded = serde_json::to_vec(&key).expect("serialize generated JSON fixture");
    let reloaded: Key = serde_json::from_slice(&encoded).expect("decode generated JSON fixture");
    assert_eq!(fixture.load(&reloaded).as_deref(), Some(PDF));
    assert_eq!(
        fs::metadata(fixture.root())
            .expect("expected test value from fs::metadata(fixture.root())")
            .mode()
            & 0o7777,
        0o700
    );
    let metadata = fs::metadata(fixture.record(&key))
        .expect("expected test value from fs::metadata(fixture.record(&key))");
    assert_eq!(metadata.mode() & 0o7777, 0o600);
    assert_eq!(metadata.nlink(), 1);
    assert_eq!(
        metadata.uid(),
        user().expect("expected test value from user()")
    );
    assert_eq!(
        fs::read(key.input()).expect("read generated fixture"),
        b"generated Office fixture"
    );
}

#[test]
fn changed_source_is_not_admitted_under_a_freshly_read_identity() {
    let fixture = Fixture::new();
    let key = fixture.key("source.xlsx");
    fs::write(key.input(), b"changed after conversion started").expect("write generated fixture");
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            source_current(&key)
        })
        .is_err()
    );
    assert!(!fixture.root().exists());
}

#[test]
fn same_size_rewrite_with_restored_mtime_is_rejected_by_ctime() {
    let fixture = Fixture::new();
    let key = fixture.key("source.docx");
    fixture.store(&key, PDF);
    let modified = fs::metadata(key.input())
        .expect("expected test value from fs::metadata(key.input())")
        .modified()
        .expect("expected test value from fs::metadata(key.input()).unwrap().modified()");
    fs::write(key.input(), vec![b'x'; key.source.bytes as usize]).expect("write generated fixture");
    File::open(key.input())
        .expect("expected test value from File::open(key.input())")
        .set_modified(modified)
        .expect(
            "expected test value from File::open(key.input()) .unwrap() .set_modified(modified)",
        );
    assert!(!source_current(&key));
    assert!(fixture.load(&key).is_none());
}

#[test]
fn atomic_source_replacement_is_rejected_by_inode() {
    let fixture = Fixture::new();
    let key = fixture.key("source.ods");
    fixture.store(&key, PDF);
    let replacement = fixture.0.join("replacement");
    fs::write(&replacement, b"generated Office fixture").expect("write generated fixture");
    fs::rename(replacement, key.input())
        .expect("expected test value from fs::rename(replacement, key.input())");
    assert!(fixture.load(&key).is_none());
}

#[test]
fn cancelled_load_and_store_do_not_create_records() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(store_at(&fixture.root(), &key, PDF, &cancellation, || true).is_err());
    assert!(!fixture.root().exists());
    fixture.store(&key, PDF);
    assert!(load_at(&fixture.root(), &key, &cancellation, || true).is_none());
}

#[test]
fn cancellation_or_source_change_during_write_removes_staged_and_published_data() {
    for fail_at in [2, 3, 4] {
        let fixture = Fixture::new();
        let key = fixture.key("source.pptx");
        let calls = Cell::new(0);
        let result = store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            calls.set(calls.get() + 1);
            calls.get() < fail_at
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read_dir(fixture.root())
                .expect("expected test value from fs::read_dir(fixture.root())")
                .count(),
            0
        );
    }
    let fixture = Fixture::new();
    let key = fixture.key("source.xlsx");
    let cancellation = Cancellation::default();
    let calls = Cell::new(0);
    assert!(
        store_at(&fixture.root(), &key, PDF, &cancellation, || {
            calls.set(calls.get() + 1);
            if calls.get() == 3 {
                cancellation.cancel();
            }
            true
        })
        .is_err()
    );
    assert_eq!(
        fs::read_dir(fixture.root())
            .expect("expected test value from fs::read_dir(fixture.root())")
            .count(),
        0
    );
}

#[test]
fn source_change_while_loading_is_a_miss() {
    let fixture = Fixture::new();
    let key = fixture.key("source.docx");
    fixture.store(&key, PDF);
    let calls = Cell::new(0);
    assert!(
        load_at(&fixture.root(), &key, &Cancellation::default(), || {
            calls.set(calls.get() + 1);
            calls.get() == 1
        })
        .is_none()
    );
}

#[test]
fn invalid_and_oversized_pdfs_are_rejected_without_creating_a_cache() {
    let fixture = Fixture::new();
    let key = fixture.key("source.docx");
    for bytes in [
        b"not a PDF".as_slice(),
        b"%PDF-1.7\ntruncated",
        b"%PDF-X.7\n%%EOF\n",
    ] {
        assert!(
            store_at(
                &fixture.root(),
                &key,
                bytes,
                &Cancellation::default(),
                || true
            )
            .is_err()
        );
    }
    let mut oversized = vec![b' '; MAX_PDF + 1];
    oversized[..9].copy_from_slice(b"%PDF-1.7\n");
    let length = oversized.len();
    oversized[length - 6..].copy_from_slice(b"%%EOF\n");
    assert!(
        store_at(
            &fixture.root(),
            &key,
            &oversized,
            &Cancellation::default(),
            || true
        )
        .is_err()
    );
    assert!(!fixture.root().exists());
}

#[test]
fn checksum_magic_header_length_truncation_and_trailing_data_fail_closed() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    fixture.store(&key, PDF);
    let original = fs::read(fixture.record(&key)).expect("read generated fixture");
    let mut damaged = original.clone();
    damaged[0] ^= 1;
    fs::write(fixture.record(&key), &damaged).expect("write generated fixture");
    assert!(fixture.load(&key).is_none());
    damaged = original.clone();
    damaged[MAGIC.len()..MAGIC.len() + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(fixture.record(&key), &damaged).expect("write generated fixture");
    assert!(fixture.load(&key).is_none());
    damaged = original.clone();
    let content = damaged.len() - PDF.len() + 10;
    damaged[content] ^= 1;
    fs::write(fixture.record(&key), &damaged).expect("write generated fixture");
    assert!(fixture.load(&key).is_none());
    fs::write(fixture.record(&key), &original[..original.len() - 1])
        .expect("write generated fixture");
    assert!(fixture.load(&key).is_none());
    damaged = original;
    damaged.push(0);
    fs::write(fixture.record(&key), damaged).expect("write generated fixture");
    assert!(fixture.load(&key).is_none());
}

#[test]
fn wrong_key_and_schema_are_never_loaded() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    fixture.store(&key, PDF);
    let mut changed = key.clone();
    changed.environment.push_str("-new-renderer");
    fs::copy(fixture.record(&key), fixture.record(&changed)).expect(
        "expected test value from fs::copy(fixture.record(&key), fixture.record(&changed))",
    );
    assert!(fixture.load(&changed).is_none());
    let mut bytes = fs::read(fixture.record(&key)).expect("read generated fixture");
    bytes[MAGIC.len() - 2] = b'2';
    fs::write(fixture.record(&key), bytes).expect("write generated fixture");
    assert!(fixture.load(&key).is_none());
}

#[test]
fn symlinked_cache_directory_and_ancestor_are_not_followed() {
    let fixture = Fixture::new();
    let key = fixture.key("source.docx");
    let outside = fixture.0.join("outside");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&outside)
        .expect("expected test value from fs::DirBuilder::new().mode(0o700).create(&outside)");
    std::os::unix::fs::symlink(&outside, fixture.root())
        .expect("expected test value from std::os::unix::fs::symlink(&outside, fixture.root())");
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            true
        })
        .is_err()
    );
    assert!(fixture.load(&key).is_none());
    assert_eq!(
        fs::read_dir(&outside)
            .expect("expected test value from fs::read_dir(&outside)")
            .count(),
        0
    );
    let alias = fixture.0.join("alias");
    std::os::unix::fs::symlink(&outside, &alias)
        .expect("expected test value from std::os::unix::fs::symlink(&outside, &alias)");
    assert!(Directory::open(&alias.join("child"), true).is_err());
    assert!(!outside.join("child").exists());
}

#[test]
fn symlinked_and_hardlinked_records_never_modify_the_target() {
    let fixture = Fixture::new();
    let key = fixture.key("source.docx");
    fixture.store(&key, PDF);
    let outside = fixture.0.join("outside");
    fs::rename(fixture.record(&key), &outside)
        .expect("expected test value from fs::rename(fixture.record(&key), &outside)");
    let original = fs::read(&outside).expect("read generated fixture");
    std::os::unix::fs::symlink(&outside, fixture.record(&key)).expect(
        "expected test value from std::os::unix::fs::symlink(&outside, fixture.record(&key))",
    );
    assert!(fixture.load(&key).is_none());
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            true
        })
        .is_err()
    );
    fs::remove_file(fixture.record(&key)).expect("remove generated fixture");
    fs::hard_link(&outside, fixture.record(&key))
        .expect("expected test value from fs::hard_link(&outside, fixture.record(&key))");
    assert!(fixture.load(&key).is_none());
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            true
        })
        .is_err()
    );
    assert_eq!(fs::read(outside).expect("read generated fixture"), original);
}

#[test]
fn nonprivate_directories_and_records_are_not_repaired_or_used() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    fixture.store(&key, PDF);
    fs::set_permissions(fixture.root(), Permissions::from_mode(0o755)).expect("expected test value from fs::set_permissions(fixture.root(), Permissions::from_mode(0o755))");
    assert!(fixture.load(&key).is_none());
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            true
        })
        .is_err()
    );
    assert_eq!(
        fs::metadata(fixture.root())
            .expect("expected test value from fs::metadata(fixture.root())")
            .mode()
            & 0o777,
        0o755
    );
    fs::set_permissions(fixture.root(), Permissions::from_mode(0o700)).expect("expected test value from fs::set_permissions(fixture.root(), Permissions::from_mode(0o700))");
    fs::set_permissions(fixture.record(&key), Permissions::from_mode(0o644)).expect("expected test value from fs::set_permissions(fixture.record(&key), Permissions::from_mode(0o644))");
    assert!(fixture.load(&key).is_none());
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            true
        })
        .is_err()
    );
}

#[test]
fn directory_lock_contention_is_a_miss_not_an_unbounded_wait() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    fixture.store(&key, PDF);
    let _locked = Directory::open(&fixture.root(), false)
        .expect("expected test value from Directory::open(&fixture.root(), false)");
    assert!(fixture.load(&key).is_none());
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            true
        })
        .is_err()
    );
}

#[test]
fn lru_bounds_entries_and_persists_access_order() {
    let fixture = Fixture::new();
    let mut keys = Vec::new();
    for index in 0..MAX_ENTRIES {
        let key = fixture.key(&format!("source-{index}.pptx"));
        fixture.store(&key, PDF);
        File::open(fixture.record(&key))
            .expect("expected test value from File::open(fixture.record(&key))")
            .set_modified(UNIX_EPOCH + Duration::from_secs(index as u64 + 1))
            .expect("expected test value from File::open(fixture.record(&key)) .unwrap() .set_modified(UNIX_EPOCH + Duration::from_secs(index as u64 + 1))");
        keys.push(key);
    }
    assert!(fixture.load(&keys[0]).is_some());
    let added = fixture.key("new.pptx");
    fixture.store(&added, PDF);
    assert_eq!(records(&fixture.root()).len(), MAX_ENTRIES);
    assert!(fixture.load(&keys[0]).is_some());
    assert!(fixture.load(&keys[1]).is_none());
    assert!(fixture.load(&added).is_some());
}

#[test]
fn byte_budget_prunes_independently_of_entry_budget() {
    let fixture = Fixture::new();
    let key = fixture.key("incoming.xlsx");
    let directory = Directory::open(&fixture.root(), true)
        .expect("expected test value from Directory::open(&fixture.root(), true)");
    for index in 0..5 {
        let path = directory.path().join(format!("{index:064x}.pdfcache"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .expect("expected test value from OpenOptions::new() .write(true) .create_new(true) .mode(0o600) .open(path)");
        file.set_len(MAX_PDF as u64)
            .expect("expected test value from file.set_len(MAX_PDF as u64)");
        file.set_modified(UNIX_EPOCH + Duration::from_secs(index + 1))
            .expect("expected test value from file.set_modified(UNIX_EPOCH + Duration::from_secs(index + 1))");
    }
    prune(
        &directory,
        &filename(&key).expect("expected test value from filename(&key)"),
        MAX_PDF as u64,
    )
    .expect("expected test value from prune(&directory, &filename(&key).unwrap(), MAX_PDF as u64)");
    assert_eq!(records(&fixture.root()).len(), 3);
    let size: u64 = records(&fixture.root())
        .iter()
        .map(|path| {
            fs::metadata(path)
                .expect("expected test value from fs::metadata(path)")
                .len()
        })
        .sum();
    assert!(size + MAX_PDF as u64 <= MAX_BYTES);
}

#[test]
fn orphan_staging_files_are_reclaimed_but_unrelated_files_are_preserved() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    let directory = Directory::open(&fixture.root(), true)
        .expect("expected test value from Directory::open(&fixture.root(), true)");
    let temporary = directory.path().join(".tmp-123-456");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .expect("expected test value from OpenOptions::new() .write(true) .create_new(true) .mode(0o600) .open(&temporary)");
    fs::write(directory.path().join("unrelated"), b"preserve").expect("write generated fixture");
    drop(directory);
    fixture.store(&key, PDF);
    assert!(!fixture.root().join(".tmp-123-456").exists());
    assert_eq!(
        fs::read(fixture.root().join("unrelated")).expect("read generated fixture"),
        b"preserve"
    );
}

#[test]
fn directory_scan_and_environment_walk_are_bounded() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    let directory = Directory::open(&fixture.root(), true)
        .expect("expected test value from Directory::open(&fixture.root(), true)");
    for index in 0..=MAX_DIRECTORY_ENTRIES {
        fs::write(directory.path().join(format!("unrelated-{index}")), [])
            .expect("write generated fixture");
    }
    assert!(
        prune(
            &directory,
            &filename(&key).expect("expected test value from filename(&key)"),
            PDF.len() as u64
        )
        .is_err()
    );
    let mut remaining = 1;
    assert!(
        walk_environment(
            &fixture.0,
            0,
            &mut remaining,
            &mut BTreeSet::new(),
            &mut Vec::new()
        )
        .is_err()
    );
    let mut remaining = 10;
    assert!(
        walk_environment(
            &fixture.0,
            MAX_ENVIRONMENT_DEPTH + 1,
            &mut remaining,
            &mut BTreeSet::new(),
            &mut Vec::new()
        )
        .is_err()
    );
}

#[test]
fn raw_non_utf8_canonical_source_names_remain_distinct() {
    let fixture = Fixture::new();
    let path = fixture
        .0
        .join(std::ffi::OsString::from_vec(b"source-\xff.docx".to_vec()));
    fs::write(&path, b"fixture").expect("write generated fixture");
    let (path, stamp) = source(&path).expect("expected test value from source(&path)");
    let key = Key {
        path: path.as_os_str().as_bytes().to_vec(),
        source: stamp,
        environment: "fixture".into(),
    };
    fixture.store(&key, PDF);
    assert_eq!(fixture.load(&key).as_deref(), Some(PDF));
    assert_eq!(key.input(), path);
}

#[test]
fn font_renderer_and_helper_revisions_change_the_deterministic_fingerprint() {
    let fixture = Fixture::new();
    let program = fixture.0.join("program");
    let fonts = fixture.0.join("fonts");
    fs::create_dir(&program).expect("create generated fixture directory");
    fs::create_dir(&fonts).expect("create generated fixture directory");
    let renderer = program.join("soffice");
    let helper = fixture.0.join("helper");
    fs::write(&renderer, b"renderer wrapper").expect("write generated fixture");
    fs::write(&helper, b"helper revision").expect("write generated fixture");
    let fingerprint = || {
        environment_at(&renderer, &helper, &[fonts.as_path()]).expect(
            "expected test value from environment_at(&renderer, &helper, &[fonts.as_path()])",
        )
    };
    let first = fingerprint();
    assert_eq!(first, fingerprint());
    let binary = program.join("soffice.bin");
    fs::write(&binary, b"renderer binary").expect("write generated fixture");
    let second = fingerprint();
    assert_ne!(first, second);
    let modified = fs::metadata(&binary)
        .expect("expected test value from fs::metadata(&binary)")
        .modified()
        .expect("expected test value from fs::metadata(&binary).unwrap().modified()");
    fs::write(&binary, b"changed! binary").expect("write generated fixture");
    File::open(&binary)
        .expect("expected test value from File::open(&binary)")
        .set_modified(modified)
        .expect("expected test value from File::open(&binary).unwrap().set_modified(modified)");
    let third = fingerprint();
    assert_ne!(second, third);
    let nested = fonts.join("nested");
    fs::create_dir(&nested).expect("create generated fixture directory");
    let font = nested.join("fixture.ttf");
    fs::write(&font, b"font revision one").expect("write generated fixture");
    let fourth = fingerprint();
    assert_ne!(third, fourth);
    fs::write(&font, b"font revision two").expect("write generated fixture");
    let fifth = fingerprint();
    assert_ne!(fourth, fifth);
    fs::write(&helper, b"new helper revision").expect("write generated fixture");
    assert_ne!(fifth, fingerprint());
}

#[test]
fn font_symlink_cycles_are_bounded_and_font_cache_changes_invalidate() {
    let fixture = Fixture::new();
    let program = fixture.0.join("program");
    fs::create_dir(&program).expect("create generated fixture directory");
    let renderer = program.join("soffice");
    let helper = fixture.0.join("helper");
    let fonts = fixture.0.join("fonts");
    fs::write(&renderer, b"renderer").expect("write generated fixture");
    fs::write(&helper, b"helper").expect("write generated fixture");
    fs::create_dir(&fonts).expect("create generated fixture directory");
    std::os::unix::fs::symlink(&fonts, fonts.join("loop")).expect(
        "expected test value from std::os::unix::fs::symlink(&fonts, fonts.join(\"loop\"))",
    );
    let first = environment_at(&renderer, &helper, &[fonts.as_path()])
        .expect("expected test value from environment_at(&renderer, &helper, &[fonts.as_path()])");
    fs::write(fonts.join("fixture.cache-8"), b"font cache").expect("write generated fixture");
    assert_ne!(
        first,
        environment_at(&renderer, &helper, &[fonts.as_path()]).expect(
            "expected test value from environment_at(&renderer, &helper, &[fonts.as_path()])"
        )
    );
}

#[test]
#[ignore = "Optional isolated child-process persistent Office cache regression"]
fn persistent_cache_survives_a_fresh_process() {
    const CHILD: &str = "HERMES_OFFICE_DISK_TEST_CHILD";
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        assert_eq!(
            fs::metadata(&root)
                .expect("expected test value from fs::metadata(&root)")
                .uid(),
            user().expect("expected test value from user()")
        );
        assert_eq!(
            fs::metadata(&root)
                .expect("expected test value from fs::metadata(&root)")
                .mode()
                & 0o7777,
            0o700
        );
        let key: Key = serde_json::from_slice(
            &fs::read(root.join("key.json")).expect("read generated fixture"),
        )
        .expect("read generated fixture");
        assert!(key.input().starts_with(&root));
        assert_eq!(
            load_at(&root.join("cache"), &key, &Cancellation::default(), || {
                source_current(&key)
            })
            .as_deref(),
            Some(PDF)
        );
        return;
    }
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    fixture.store(&key, PDF);
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(fixture.0.join("key.json"))
        .expect("expected test value from OpenOptions::new() .write(true) .create_new(true) .mode(0o600) .open(fixture.0.join(\"key.json\"))");
    output
        .write_all(&serde_json::to_vec(&key).expect("serialize generated JSON fixture"))
        .expect("serialize generated JSON fixture");
    let status = Command::new(std::env::current_exe().expect("expected test value from std::env::current_exe()"))
        .args([
            "--ignored",
            "--exact",
            "sandbox::office::disk::tests::persistent_cache_survives_a_fresh_process",
            "--test-threads=1",
        ])
        .env(CHILD, &fixture.0)
        .status()
        .expect("expected test value from Command::new(std::env::current_exe().unwrap()) .args([ \"--ignored\", \"--exact\", \"sandbox::office::disk::tests::persistent_cache_...");
    assert!(status.success());
}
