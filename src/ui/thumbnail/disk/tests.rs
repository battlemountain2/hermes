use super::*;
use std::{
    os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    },
    sync::atomic::AtomicU64,
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hermes-disk-thumbnail-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create generated fixture directory");
        Self(path)
    }

    fn key(&self, name: &Path) -> ThumbnailKey {
        let path = self.0.join(name);
        fs::write(
            &path,
            b"generated source, intentionally not decodable as an image",
        )
        .expect("write generated fixture");
        let identity = SourceIdentity::read(&path)
            .expect("expected test value from SourceIdentity::read(&path)");
        ThumbnailKey {
            path: path.clone(),
            source_path: path,
            identity,
            source_identity: identity,
            thumbnail_size: 64,
        }
    }

    fn cache(&self) -> PathBuf {
        self.0.join("cache")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove generated fixture");
    }
}

fn item(key: ThumbnailKey) -> Save {
    Save {
        key,
        pixels: glib::Bytes::from_owned([240, 20, 80, 255].repeat(64 * 32)),
        width: 64,
        height: 32,
        stride: 256,
    }
}

#[test]
fn clear_cache_removes_only_owned_records_without_following_links() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    assert_eq!(
        clear_at(&fixture.cache()).expect("expected test value from clear_at(&fixture.cache())"),
        0
    );
    save(&fixture.cache(), item(key.clone()))
        .expect("expected test value from save(&fixture.cache(), item(key.clone()))");
    let unrelated = fixture.cache().join("unrelated.txt");
    fs::write(&unrelated, b"keep").expect("write generated fixture");
    fs::write(fixture.cache().join(".tmp-123-456"), b"staged").expect("write generated fixture");
    let outside = fixture.0.join("outside.txt");
    fs::write(&outside, b"never delete").expect("write generated fixture");
    symlink(&outside, fixture.cache().join(format!("{:064x}.rgba", 1))).expect("expected test value from symlink(&outside, fixture.cache().join(format!(\"{:064x}.rgba\", 1)))");
    assert_eq!(
        clear_at(&fixture.cache()).expect("expected test value from clear_at(&fixture.cache())"),
        3
    );
    assert!(load_at(&fixture.cache(), &key).is_none());
    assert_eq!(
        fs::read(&unrelated).expect("read generated fixture"),
        b"keep"
    );
    assert_eq!(
        fs::read(&outside).expect("read generated fixture"),
        b"never delete"
    );
    assert_eq!(
        clear_at(&fixture.cache()).expect("expected test value from clear_at(&fixture.cache())"),
        0
    );
    save(&fixture.cache(), item(key.clone()))
        .expect("expected test value from save(&fixture.cache(), item(key.clone()))");
    assert!(load_at(&fixture.cache(), &key).is_some());
}

#[test]
fn clear_cache_reports_unsafe_directories_and_removal_errors() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.cache()).expect("create generated fixture directory");
    fs::set_permissions(fixture.cache(), fs::Permissions::from_mode(0o755)).expect("expected test value from fs::set_permissions(fixture.cache(), fs::Permissions::from_mode(0o755))");
    assert!(clear_at(&fixture.cache()).is_err());
    fs::set_permissions(fixture.cache(), fs::Permissions::from_mode(0o700)).expect("expected test value from fs::set_permissions(fixture.cache(), fs::Permissions::from_mode(0o700))");
    let alias = fixture.0.join("alias");
    symlink(fixture.cache(), &alias)
        .expect("expected test value from symlink(fixture.cache(), &alias)");
    assert!(clear_at(&alias).is_err());
    fs::create_dir(fixture.cache().join(format!("{:064x}.rgba", 1)))
        .expect("create generated fixture directory");
    assert!(clear_at(&fixture.cache()).is_err());
}

#[test]
fn clear_cache_is_ordered_after_queued_saves_and_allows_later_writes() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    let root = fixture.cache();
    let (sender, receiver) = mpsc::sync_channel(8);
    let worker_root = root.clone();
    let worker = std::thread::spawn(move || run_writer(&worker_root, receiver));
    sender
        .send(Command::Save(item(key.clone())))
        .expect("expected test value from sender.send(Command::Save(item(key.clone())))");
    let (reply, result) = mpsc::channel();
    sender
        .send(Command::Clear(reply))
        .expect("expected test value from sender.send(Command::Clear(reply))");
    assert_eq!(
        result
            .recv()
            .expect("expected test value from result.recv()")
            .expect("expected test value from result.recv().unwrap()"),
        1
    );
    assert!(load_at(&root, &key).is_none());
    sender
        .send(Command::Save(item(key.clone())))
        .expect("expected test value from sender.send(Command::Save(item(key.clone())))");
    let (reply, result) = mpsc::channel();
    sender
        .send(Command::Clear(reply))
        .expect("expected test value from sender.send(Command::Clear(reply))");
    assert_eq!(
        result
            .recv()
            .expect("expected test value from result.recv()")
            .expect("expected test value from result.recv().unwrap()"),
        1
    );
    drop(sender);
    worker
        .join()
        .expect("expected test value from worker.join()");
    assert!(load_at(&root, &key).is_none());
}

#[test]
fn cache_survives_a_fresh_process_without_redecoding_the_source() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    save(&fixture.cache(), item(key.clone()))
        .expect("expected test value from save(&fixture.cache(), item(key.clone()))");
    let output = std::process::Command::new(std::env::current_exe().expect("expected test value from std::env::current_exe()"))
        .args([
            "--exact",
            "ui::thumbnail::disk::tests::child_reads_only_a_generated_raw_cache",
            "--ignored",
            "--nocapture",
        ])
        .env("HERMES_DISK_THUMBNAIL_FIXTURE", &fixture.0)
        .output()
        .expect("expected test value from std::process::Command::new(std::env::current_exe().unwrap()) .args([ \"--exact\", \"ui::thumbnail::disk::tests::child_reads_only_a...");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    assert_eq!(
        fs::metadata(fixture.cache())
            .expect("expected test value from fs::metadata(fixture.cache())")
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(fixture.cache().join(filename(&key.path).expect("expected test value from filename(&key.path)")))
            .expect("expected test value from fs::metadata(fixture.cache().join(filename(&key.path).unwrap()))")
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
#[ignore = "Child of isolated generated-thumbnail test; never supply personal paths"]
fn child_reads_only_a_generated_raw_cache() {
    let root = PathBuf::from(
        std::env::var_os("HERMES_DISK_THUMBNAIL_FIXTURE")
            .expect("expected test value from std::env::var_os(\"HERMES_DISK_THUMBNAIL_FIXTURE\")"),
    );
    assert!(
        root.file_name()
            .expect("expected test value from root.file_name()")
            .to_string_lossy()
            .starts_with("hermes-disk-thumbnail-")
    );
    assert!(root.starts_with(std::env::temp_dir()));
    let source = root.join("source.jpg");
    let identity = SourceIdentity::read(&source)
        .expect("expected test value from SourceIdentity::read(&source)");
    let key = ThumbnailKey {
        path: source.clone(),
        source_path: source,
        identity,
        source_identity: identity,
        thumbnail_size: 64,
    };
    let started = std::time::Instant::now();
    let png = load_at(&root.join("cache"), &key)
        .expect("expected test value from load_at(&root.join(\"cache\"), &key)");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    eprintln!(
        "Fresh-process disk thumbnail (no source decode): {:?}",
        started.elapsed()
    );
}

#[test]
fn raw_cache_rejects_corruption_changed_sources_bad_sizes_and_symlinks() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    let path = fixture
        .cache()
        .join(filename(&key.path).expect("expected test value from filename(&key.path)"));
    save(&fixture.cache(), item(key.clone()))
        .expect("expected test value from save(&fixture.cache(), item(key.clone()))");
    assert!(load_at(&fixture.cache(), &key).is_some());
    assert!(audio_candidate_at(&fixture.cache(), &key.path, 64).is_some());
    let original = fs::read(&path).expect("read generated fixture");
    for length in [0, MAGIC.len(), original.len() - 1] {
        fs::write(&path, &original[..length]).expect("write generated fixture");
        assert!(load_at(&fixture.cache(), &key).is_none());
    }
    let mut corrupt = original.clone();
    *corrupt
        .last_mut()
        .expect("expected test value from corrupt.last_mut()") ^= 1;
    fs::write(&path, corrupt).expect("write generated fixture");
    assert!(load_at(&fixture.cache(), &key).is_none());
    fs::write(&path, &original).expect("write generated fixture");
    let mut bigger = key.clone();
    bigger.thumbnail_size = 128;
    assert!(load_at(&fixture.cache(), &bigger).is_none());
    fs::write(&key.path, b"changed source").expect("write generated fixture");
    assert!(load_at(&fixture.cache(), &key).is_none());
    assert!(save(&fixture.cache(), item(key.clone())).is_none());
    fs::remove_file(&path).expect("remove generated fixture");
    symlink(&key.path, &path).expect("expected test value from symlink(&key.path, &path)");
    assert!(read(&fixture.cache(), &key.path).is_none());
    assert_eq!(
        fs::read(&key.path).expect("read generated fixture"),
        b"changed source"
    );
    for (width, height) in [(0, 4), (257, 4), (4, i32::MAX)] {
        assert!(dimensions(width, height).is_none());
    }
}

#[test]
fn cache_checks_adjacent_art_identity_non_utf8_paths_and_private_directories() {
    let fixture = Fixture::new();
    let mut key = fixture.key(Path::new(&std::ffi::OsString::from_vec(
        b"source-\xff.flac".to_vec(),
    )));
    let cover = fixture.key(Path::new("cover.jpg"));
    key.source_path = cover.path;
    key.source_identity = cover.identity;
    save(&fixture.cache(), item(key.clone()))
        .expect("expected test value from save(&fixture.cache(), item(key.clone()))");
    assert!(load_at(&fixture.cache(), &key).is_some());
    fs::write(&key.source_path, b"changed cover").expect("write generated fixture");
    assert!(load_at(&fixture.cache(), &key).is_none());
    assert!(audio_candidate_at(&fixture.cache(), &key.path, 64).is_none());
    fs::set_permissions(fixture.cache(), fs::Permissions::from_mode(0o755)).expect("expected test value from fs::set_permissions(fixture.cache(), fs::Permissions::from_mode(0o755))");
    assert!(DirectoryHandle::open(&fixture.cache(), false).is_none());
    let alias = fixture.0.join("symlink-cache");
    symlink(fixture.cache(), &alias)
        .expect("expected test value from symlink(fixture.cache(), &alias)");
    assert!(DirectoryHandle::open(&alias, false).is_none());
}

#[test]
fn cache_prunes_only_its_own_records_and_bounds_disk_entries() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    save(&fixture.cache(), item(key))
        .expect("expected test value from save(&fixture.cache(), item(key))");
    fs::write(fixture.cache().join("unrelated.txt"), b"leave alone")
        .expect("write generated fixture");
    fs::write(fixture.cache().join(".tmp-123-456"), b"abandoned staging")
        .expect("write generated fixture");
    for index in 0..MAX_FILES + 2 {
        fs::write(
            fixture.cache().join(format!("{index:064x}.rgba")),
            b"generated garbage cache",
        )
        .expect("write generated fixture");
    }
    let directory = DirectoryHandle::open(&fixture.cache(), false)
        .expect("expected test value from DirectoryHandle::open(&fixture.cache(), false)");
    prune(&directory, "not-yet-present", 128)
        .expect("expected test value from prune(&directory, \"not-yet-present\", 128)");
    assert!(!fixture.cache().join(".tmp-123-456").exists());
    assert!(
        fs::read_dir(fixture.cache())
            .expect("expected test value from fs::read_dir(fixture.cache())")
            .count()
            <= MAX_FILES + 1
    );
    assert_eq!(
        fs::read(fixture.cache().join("unrelated.txt")).expect("read generated fixture"),
        b"leave alone"
    );
}

#[test]
fn a_busy_cache_writer_is_skipped_without_blocking_or_truncating_records() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    save(&fixture.cache(), item(key.clone()))
        .expect("expected test value from save(&fixture.cache(), item(key.clone()))");
    let directory = DirectoryHandle::open(&fixture.cache(), false)
        .expect("expected test value from DirectoryHandle::open(&fixture.cache(), false)");
    directory
        .0
        .try_lock()
        .expect("expected test value from directory.0.try_lock()");
    assert!(save(&fixture.cache(), item(key.clone())).is_none());
    assert!(load_at(&fixture.cache(), &key).is_some());
    directory
        .0
        .unlock()
        .expect("expected test value from directory.0.unlock()");
}

#[test]
fn unrelated_unicode_cache_names_never_panic_or_get_pruned() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    save(&fixture.cache(), item(key))
        .expect("expected test value from save(&fixture.cache(), item(key))");
    let name = format!("{}é.rgba", "a".repeat(63));
    fs::write(fixture.cache().join(&name), b"unrelated file").expect("write generated fixture");
    let directory = DirectoryHandle::open(&fixture.cache(), false)
        .expect("expected test value from DirectoryHandle::open(&fixture.cache(), false)");
    prune(&directory, "not-yet-present", 128)
        .expect("expected test value from prune(&directory, \"not-yet-present\", 128)");
    assert_eq!(
        fs::read(fixture.cache().join(name)).expect("read generated fixture"),
        b"unrelated file"
    );
}

#[test]
#[ignore = "Private generated cache FIFO safety test requires mkfifo"]
fn fifo_cache_entries_do_not_block_readers() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    let _directory = DirectoryHandle::open(&fixture.cache(), true)
        .expect("expected test value from DirectoryHandle::open(&fixture.cache(), true)");
    let path = fixture
        .cache()
        .join(filename(&key.path).expect("expected test value from filename(&key.path)"));
    assert!(
        std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("expected test value from std::process::Command::new(\"mkfifo\") .arg(path) .status()")
            .success()
    );
    let start = std::time::Instant::now();
    assert!(load_at(&fixture.cache(), &key).is_none());
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
}
