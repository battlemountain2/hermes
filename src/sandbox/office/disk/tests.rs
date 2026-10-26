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
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        Self(path)
    }

    fn root(&self) -> PathBuf {
        self.0.join("cache")
    }

    fn key(&self, name: &str) -> Key {
        let path = self.0.join(name);
        fs::write(&path, b"generated Office fixture").unwrap();
        let (path, source) = source(&path).unwrap();
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
        .unwrap();
    }

    fn load(&self, key: &Key) -> Option<Vec<u8>> {
        load_at(&self.root(), key, &Cancellation::default(), || {
            source_current(key)
        })
    }

    fn record(&self, key: &Key) -> PathBuf {
        self.root().join(filename(key).unwrap())
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
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.file_name().unwrap().to_str().is_some_and(owned_record))
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
    let encoded = serde_json::to_vec(&key).unwrap();
    let reloaded: Key = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(fixture.load(&reloaded).as_deref(), Some(PDF));
    assert_eq!(fs::metadata(fixture.root()).unwrap().mode() & 0o7777, 0o700);
    let metadata = fs::metadata(fixture.record(&key)).unwrap();
    assert_eq!(metadata.mode() & 0o7777, 0o600);
    assert_eq!(metadata.nlink(), 1);
    assert_eq!(metadata.uid(), user().unwrap());
    assert_eq!(fs::read(key.input()).unwrap(), b"generated Office fixture");
}

#[test]
fn changed_source_is_not_admitted_under_a_freshly_read_identity() {
    let fixture = Fixture::new();
    let key = fixture.key("source.xlsx");
    fs::write(key.input(), b"changed after conversion started").unwrap();
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
    let modified = fs::metadata(key.input()).unwrap().modified().unwrap();
    fs::write(key.input(), vec![b'x'; key.source.bytes as usize]).unwrap();
    File::open(key.input())
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert!(!source_current(&key));
    assert!(fixture.load(&key).is_none());
}

#[test]
fn atomic_source_replacement_is_rejected_by_inode() {
    let fixture = Fixture::new();
    let key = fixture.key("source.ods");
    fixture.store(&key, PDF);
    let replacement = fixture.0.join("replacement");
    fs::write(&replacement, b"generated Office fixture").unwrap();
    fs::rename(replacement, key.input()).unwrap();
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
        assert_eq!(fs::read_dir(fixture.root()).unwrap().count(), 0);
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
    assert_eq!(fs::read_dir(fixture.root()).unwrap().count(), 0);
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
    let original = fs::read(fixture.record(&key)).unwrap();
    let mut damaged = original.clone();
    damaged[0] ^= 1;
    fs::write(fixture.record(&key), &damaged).unwrap();
    assert!(fixture.load(&key).is_none());
    damaged = original.clone();
    damaged[MAGIC.len()..MAGIC.len() + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(fixture.record(&key), &damaged).unwrap();
    assert!(fixture.load(&key).is_none());
    damaged = original.clone();
    let content = damaged.len() - PDF.len() + 10;
    damaged[content] ^= 1;
    fs::write(fixture.record(&key), &damaged).unwrap();
    assert!(fixture.load(&key).is_none());
    fs::write(fixture.record(&key), &original[..original.len() - 1]).unwrap();
    assert!(fixture.load(&key).is_none());
    damaged = original;
    damaged.push(0);
    fs::write(fixture.record(&key), damaged).unwrap();
    assert!(fixture.load(&key).is_none());
}

#[test]
fn wrong_key_and_schema_are_never_loaded() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    fixture.store(&key, PDF);
    let mut changed = key.clone();
    changed.environment.push_str("-new-renderer");
    fs::copy(fixture.record(&key), fixture.record(&changed)).unwrap();
    assert!(fixture.load(&changed).is_none());
    let mut bytes = fs::read(fixture.record(&key)).unwrap();
    bytes[MAGIC.len() - 2] = b'2';
    fs::write(fixture.record(&key), bytes).unwrap();
    assert!(fixture.load(&key).is_none());
}

#[test]
fn symlinked_cache_directory_and_ancestor_are_not_followed() {
    let fixture = Fixture::new();
    let key = fixture.key("source.docx");
    let outside = fixture.0.join("outside");
    fs::DirBuilder::new().mode(0o700).create(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, fixture.root()).unwrap();
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            true
        })
        .is_err()
    );
    assert!(fixture.load(&key).is_none());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    let alias = fixture.0.join("alias");
    std::os::unix::fs::symlink(&outside, &alias).unwrap();
    assert!(Directory::open(&alias.join("child"), true).is_err());
    assert!(!outside.join("child").exists());
}

#[test]
fn symlinked_and_hardlinked_records_never_modify_the_target() {
    let fixture = Fixture::new();
    let key = fixture.key("source.docx");
    fixture.store(&key, PDF);
    let outside = fixture.0.join("outside");
    fs::rename(fixture.record(&key), &outside).unwrap();
    let original = fs::read(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, fixture.record(&key)).unwrap();
    assert!(fixture.load(&key).is_none());
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            true
        })
        .is_err()
    );
    fs::remove_file(fixture.record(&key)).unwrap();
    fs::hard_link(&outside, fixture.record(&key)).unwrap();
    assert!(fixture.load(&key).is_none());
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            true
        })
        .is_err()
    );
    assert_eq!(fs::read(outside).unwrap(), original);
}

#[test]
fn nonprivate_directories_and_records_are_not_repaired_or_used() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    fixture.store(&key, PDF);
    fs::set_permissions(fixture.root(), Permissions::from_mode(0o755)).unwrap();
    assert!(fixture.load(&key).is_none());
    assert!(
        store_at(&fixture.root(), &key, PDF, &Cancellation::default(), || {
            true
        })
        .is_err()
    );
    assert_eq!(fs::metadata(fixture.root()).unwrap().mode() & 0o777, 0o755);
    fs::set_permissions(fixture.root(), Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(fixture.record(&key), Permissions::from_mode(0o644)).unwrap();
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
    let _locked = Directory::open(&fixture.root(), false).unwrap();
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
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(index as u64 + 1))
            .unwrap();
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
    let directory = Directory::open(&fixture.root(), true).unwrap();
    for index in 0..5 {
        let path = directory.path().join(format!("{index:064x}.pdfcache"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.set_len(MAX_PDF as u64).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(index + 1))
            .unwrap();
    }
    prune(&directory, &filename(&key).unwrap(), MAX_PDF as u64).unwrap();
    assert_eq!(records(&fixture.root()).len(), 3);
    let size: u64 = records(&fixture.root())
        .iter()
        .map(|path| fs::metadata(path).unwrap().len())
        .sum();
    assert!(size + MAX_PDF as u64 <= MAX_BYTES);
}

#[test]
fn orphan_staging_files_are_reclaimed_but_unrelated_files_are_preserved() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    let directory = Directory::open(&fixture.root(), true).unwrap();
    let temporary = directory.path().join(".tmp-123-456");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .unwrap();
    fs::write(directory.path().join("unrelated"), b"preserve").unwrap();
    drop(directory);
    fixture.store(&key, PDF);
    assert!(!fixture.root().join(".tmp-123-456").exists());
    assert_eq!(
        fs::read(fixture.root().join("unrelated")).unwrap(),
        b"preserve"
    );
}

#[test]
fn directory_scan_and_environment_walk_are_bounded() {
    let fixture = Fixture::new();
    let key = fixture.key("source.pptx");
    let directory = Directory::open(&fixture.root(), true).unwrap();
    for index in 0..=MAX_DIRECTORY_ENTRIES {
        fs::write(directory.path().join(format!("unrelated-{index}")), []).unwrap();
    }
    assert!(prune(&directory, &filename(&key).unwrap(), PDF.len() as u64).is_err());
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
    fs::write(&path, b"fixture").unwrap();
    let (path, stamp) = source(&path).unwrap();
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
    fs::create_dir(&program).unwrap();
    fs::create_dir(&fonts).unwrap();
    let renderer = program.join("soffice");
    let helper = fixture.0.join("helper");
    fs::write(&renderer, b"renderer wrapper").unwrap();
    fs::write(&helper, b"helper revision").unwrap();
    let fingerprint = || environment_at(&renderer, &helper, &[fonts.as_path()]).unwrap();
    let first = fingerprint();
    assert_eq!(first, fingerprint());
    let binary = program.join("soffice.bin");
    fs::write(&binary, b"renderer binary").unwrap();
    let second = fingerprint();
    assert_ne!(first, second);
    let modified = fs::metadata(&binary).unwrap().modified().unwrap();
    fs::write(&binary, b"changed! binary").unwrap();
    File::open(&binary).unwrap().set_modified(modified).unwrap();
    let third = fingerprint();
    assert_ne!(second, third);
    let nested = fonts.join("nested");
    fs::create_dir(&nested).unwrap();
    let font = nested.join("fixture.ttf");
    fs::write(&font, b"font revision one").unwrap();
    let fourth = fingerprint();
    assert_ne!(third, fourth);
    fs::write(&font, b"font revision two").unwrap();
    let fifth = fingerprint();
    assert_ne!(fourth, fifth);
    fs::write(&helper, b"new helper revision").unwrap();
    assert_ne!(fifth, fingerprint());
}

#[test]
fn font_symlink_cycles_are_bounded_and_font_cache_changes_invalidate() {
    let fixture = Fixture::new();
    let program = fixture.0.join("program");
    fs::create_dir(&program).unwrap();
    let renderer = program.join("soffice");
    let helper = fixture.0.join("helper");
    let fonts = fixture.0.join("fonts");
    fs::write(&renderer, b"renderer").unwrap();
    fs::write(&helper, b"helper").unwrap();
    fs::create_dir(&fonts).unwrap();
    std::os::unix::fs::symlink(&fonts, fonts.join("loop")).unwrap();
    let first = environment_at(&renderer, &helper, &[fonts.as_path()]).unwrap();
    fs::write(fonts.join("fixture.cache-8"), b"font cache").unwrap();
    assert_ne!(
        first,
        environment_at(&renderer, &helper, &[fonts.as_path()]).unwrap()
    );
}

#[test]
#[ignore = "Optional isolated child-process persistent Office cache regression"]
fn persistent_cache_survives_a_fresh_process() {
    const CHILD: &str = "HERMES_OFFICE_DISK_TEST_CHILD";
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        assert_eq!(fs::metadata(&root).unwrap().uid(), user().unwrap());
        assert_eq!(fs::metadata(&root).unwrap().mode() & 0o7777, 0o700);
        let key: Key = serde_json::from_slice(&fs::read(root.join("key.json")).unwrap()).unwrap();
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
        .unwrap();
    output
        .write_all(&serde_json::to_vec(&key).unwrap())
        .unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "sandbox::office::disk::tests::persistent_cache_survives_a_fresh_process",
            "--test-threads=1",
        ])
        .env(CHILD, &fixture.0)
        .status()
        .unwrap();
    assert!(status.success());
}
