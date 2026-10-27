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
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn key(&self, name: &Path) -> ThumbnailKey {
        let path = self.0.join(name);
        fs::write(
            &path,
            b"generated source, intentionally not decodable as an image",
        )
        .unwrap();
        let identity = SourceIdentity::read(&path).unwrap();
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
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn item(key: ThumbnailKey) -> Save {
    Save {
        key,
        pixels: glib::Bytes::from_owned(vec![240, 20, 80, 255].repeat(64 * 32)),
        width: 64,
        height: 32,
        stride: 256,
    }
}

#[test]
fn clear_cache_removes_only_owned_records_without_following_links() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    assert_eq!(clear_at(&fixture.cache()).unwrap(), 0);
    save(&fixture.cache(), item(key.clone())).unwrap();
    let unrelated = fixture.cache().join("unrelated.txt");
    fs::write(&unrelated, b"keep").unwrap();
    fs::write(fixture.cache().join(".tmp-123-456"), b"staged").unwrap();
    let outside = fixture.0.join("outside.txt");
    fs::write(&outside, b"never delete").unwrap();
    symlink(&outside, fixture.cache().join(format!("{:064x}.rgba", 1))).unwrap();
    assert_eq!(clear_at(&fixture.cache()).unwrap(), 3);
    assert!(load_at(&fixture.cache(), &key).is_none());
    assert_eq!(fs::read(&unrelated).unwrap(), b"keep");
    assert_eq!(fs::read(&outside).unwrap(), b"never delete");
    assert_eq!(clear_at(&fixture.cache()).unwrap(), 0);
    save(&fixture.cache(), item(key.clone())).unwrap();
    assert!(load_at(&fixture.cache(), &key).is_some());
}

#[test]
fn clear_cache_reports_unsafe_directories_and_removal_errors() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.cache()).unwrap();
    fs::set_permissions(fixture.cache(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(clear_at(&fixture.cache()).is_err());
    fs::set_permissions(fixture.cache(), fs::Permissions::from_mode(0o700)).unwrap();
    let alias = fixture.0.join("alias");
    symlink(fixture.cache(), &alias).unwrap();
    assert!(clear_at(&alias).is_err());
    fs::create_dir(fixture.cache().join(format!("{:064x}.rgba", 1))).unwrap();
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
    sender.send(Command::Save(item(key.clone()))).unwrap();
    let (reply, result) = mpsc::channel();
    sender.send(Command::Clear(reply)).unwrap();
    assert_eq!(result.recv().unwrap().unwrap(), 1);
    assert!(load_at(&root, &key).is_none());
    sender.send(Command::Save(item(key.clone()))).unwrap();
    let (reply, result) = mpsc::channel();
    sender.send(Command::Clear(reply)).unwrap();
    assert_eq!(result.recv().unwrap().unwrap(), 1);
    drop(sender);
    worker.join().unwrap();
    assert!(load_at(&root, &key).is_none());
}

#[test]
fn cache_survives_a_fresh_process_without_redecoding_the_source() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    save(&fixture.cache(), item(key.clone())).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "ui::thumbnail::disk::tests::child_reads_only_a_generated_raw_cache",
            "--ignored",
            "--nocapture",
        ])
        .env("HERMES_DISK_THUMBNAIL_FIXTURE", &fixture.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    assert_eq!(fs::metadata(fixture.cache()).unwrap().mode() & 0o777, 0o700);
    assert_eq!(
        fs::metadata(fixture.cache().join(filename(&key.path).unwrap()))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
#[ignore = "Child of isolated generated-thumbnail test; never supply personal paths"]
fn child_reads_only_a_generated_raw_cache() {
    let root = PathBuf::from(std::env::var_os("HERMES_DISK_THUMBNAIL_FIXTURE").unwrap());
    assert!(
        root.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hermes-disk-thumbnail-")
    );
    assert!(root.starts_with(std::env::temp_dir()));
    let source = root.join("source.jpg");
    let identity = SourceIdentity::read(&source).unwrap();
    let key = ThumbnailKey {
        path: source.clone(),
        source_path: source,
        identity,
        source_identity: identity,
        thumbnail_size: 64,
    };
    let started = std::time::Instant::now();
    let png = load_at(&root.join("cache"), &key).unwrap();
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
    let path = fixture.cache().join(filename(&key.path).unwrap());
    save(&fixture.cache(), item(key.clone())).unwrap();
    assert!(load_at(&fixture.cache(), &key).is_some());
    assert!(audio_candidate_at(&fixture.cache(), &key.path, 64).is_some());
    let original = fs::read(&path).unwrap();
    for length in [0, MAGIC.len(), original.len() - 1] {
        fs::write(&path, &original[..length]).unwrap();
        assert!(load_at(&fixture.cache(), &key).is_none());
    }
    let mut corrupt = original.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    fs::write(&path, corrupt).unwrap();
    assert!(load_at(&fixture.cache(), &key).is_none());
    fs::write(&path, &original).unwrap();
    let mut bigger = key.clone();
    bigger.thumbnail_size = 128;
    assert!(load_at(&fixture.cache(), &bigger).is_none());
    fs::write(&key.path, b"changed source").unwrap();
    assert!(load_at(&fixture.cache(), &key).is_none());
    assert!(save(&fixture.cache(), item(key.clone())).is_none());
    fs::remove_file(&path).unwrap();
    symlink(&key.path, &path).unwrap();
    assert!(read(&fixture.cache(), &key.path).is_none());
    assert_eq!(fs::read(&key.path).unwrap(), b"changed source");
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
    save(&fixture.cache(), item(key.clone())).unwrap();
    assert!(load_at(&fixture.cache(), &key).is_some());
    fs::write(&key.source_path, b"changed cover").unwrap();
    assert!(load_at(&fixture.cache(), &key).is_none());
    assert!(audio_candidate_at(&fixture.cache(), &key.path, 64).is_none());
    fs::set_permissions(fixture.cache(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(DirectoryHandle::open(&fixture.cache(), false).is_none());
    let alias = fixture.0.join("symlink-cache");
    symlink(fixture.cache(), &alias).unwrap();
    assert!(DirectoryHandle::open(&alias, false).is_none());
}

#[test]
fn cache_prunes_only_its_own_records_and_bounds_disk_entries() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    save(&fixture.cache(), item(key)).unwrap();
    fs::write(fixture.cache().join("unrelated.txt"), b"leave alone").unwrap();
    fs::write(fixture.cache().join(".tmp-123-456"), b"abandoned staging").unwrap();
    for index in 0..MAX_FILES + 2 {
        fs::write(
            fixture.cache().join(format!("{index:064x}.rgba")),
            b"generated garbage cache",
        )
        .unwrap();
    }
    let directory = DirectoryHandle::open(&fixture.cache(), false).unwrap();
    prune(&directory, "not-yet-present", 128).unwrap();
    assert!(!fixture.cache().join(".tmp-123-456").exists());
    assert!(fs::read_dir(fixture.cache()).unwrap().count() <= MAX_FILES + 1);
    assert_eq!(
        fs::read(fixture.cache().join("unrelated.txt")).unwrap(),
        b"leave alone"
    );
}

#[test]
fn a_busy_cache_writer_is_skipped_without_blocking_or_truncating_records() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    save(&fixture.cache(), item(key.clone())).unwrap();
    let directory = DirectoryHandle::open(&fixture.cache(), false).unwrap();
    directory.0.try_lock().unwrap();
    assert!(save(&fixture.cache(), item(key.clone())).is_none());
    assert!(load_at(&fixture.cache(), &key).is_some());
    directory.0.unlock().unwrap();
}

#[test]
fn unrelated_unicode_cache_names_never_panic_or_get_pruned() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    save(&fixture.cache(), item(key)).unwrap();
    let name = format!("{}é.rgba", "a".repeat(63));
    fs::write(fixture.cache().join(&name), b"unrelated file").unwrap();
    let directory = DirectoryHandle::open(&fixture.cache(), false).unwrap();
    prune(&directory, "not-yet-present", 128).unwrap();
    assert_eq!(
        fs::read(fixture.cache().join(name)).unwrap(),
        b"unrelated file"
    );
}

#[test]
#[ignore = "Private generated cache FIFO safety test requires mkfifo"]
fn fifo_cache_entries_do_not_block_readers() {
    let fixture = Fixture::new();
    let key = fixture.key(Path::new("source.jpg"));
    let _directory = DirectoryHandle::open(&fixture.cache(), true).unwrap();
    let path = fixture.cache().join(filename(&key.path).unwrap());
    assert!(
        std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
    let start = std::time::Instant::now();
    assert!(load_at(&fixture.cache(), &key).is_none());
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
}
