use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "strata-waveform-cache-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("fixture directory");
        Self(directory)
    }

    fn source(&self) -> PathBuf {
        let path = self.0.join("source.wav");
        fs::write(&path, b"audio fixture").expect("source");
        path
    }

    fn cache(&self) -> PathBuf {
        self.0.join("cache")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

fn numeric_json() -> Vec<u8> {
    serde_json::to_vec(&vec![0.5_f32; 1024]).expect("JSON")
}

#[test]
fn identity_keys_include_every_field_and_analyzer_version() {
    let fixture = Fixture::new();
    let original = Identity::read(&fixture.source()).expect("identity");
    assert!(owned_name(&original.filename()));
    for field in 0..8 {
        let mut changed = original.clone();
        match field {
            0 => changed.version += 1,
            1 => changed.device += 1,
            2 => changed.inode += 1,
            3 => changed.size += 1,
            4 => changed.modified.0 += 1,
            5 => changed.modified.1 += 1,
            6 => changed.changed.0 += 1,
            _ => changed.changed.1 += 1,
        }
        assert_ne!(changed, original);
        assert_ne!(changed.filename(), original.filename());
    }
}

#[test]
fn disk_cache_roundtrip_avoids_analyzer_and_invalidates_on_source_change() {
    let fixture = Fixture::new();
    let source = fixture.source();
    let cancellation = Cancellation::default();
    let peaks = load_with(&source, &cancellation, &fixture.cache(), |_, _| {
        Ok(numeric_json())
    })
    .expect("load");
    assert_eq!(
        load_with(&source, &cancellation, &fixture.cache(), |_, _| panic!(
            "cache miss"
        ))
        .expect("cached"),
        peaks
    );
    fs::write(&source, b"different audio bytes").expect("change source");
    assert!(
        load_with(&source, &cancellation, &fixture.cache(), |_, _| Err(
            "cache invalidated".to_owned()
        ))
        .expect_err("changed")
        .contains("invalidated")
    );
}

#[test]
fn hardlinks_share_identity_and_same_size_rewrites_change_ctime() {
    let fixture = Fixture::new();
    let source = fixture.source();
    let alias = fixture.0.join("alias.wav");
    fs::hard_link(&source, &alias).expect("hardlink");
    let before = Identity::read(&source).expect("identity");
    assert_eq!(Identity::read(&alias).expect("alias identity"), before);
    std::thread::sleep(std::time::Duration::from_millis(2));
    fs::write(&source, b"other fixture").expect("same size rewrite");
    let after = Identity::read(&source).expect("new identity");
    assert_eq!(after.size, before.size);
    assert_ne!(after.changed, before.changed);
}

#[test]
fn source_changed_during_analysis_is_never_cached() {
    let fixture = Fixture::new();
    let source = fixture.source();
    let identity = Identity::read(&source).expect("identity");
    let result = load_with(
        &source,
        &Cancellation::default(),
        &fixture.cache(),
        |path, _| {
            fs::write(path, b"changed").expect("source changed");
            Ok(numeric_json())
        },
    );
    assert!(result.expect_err("changed").contains("source changed"));
    assert!(!fixture.cache().join(identity.filename()).exists());
}

#[test]
fn cancellation_is_checked_before_cache_hits_and_after_analysis() {
    let fixture = Fixture::new();
    let source = fixture.source();
    let cancellation = Cancellation::default();
    load_with(&source, &cancellation, &fixture.cache(), |_, _| {
        Ok(numeric_json())
    })
    .expect("populate");
    cancellation.cancel();
    assert!(
        load_with(&source, &cancellation, &fixture.cache(), |_, _| panic!(
            "cancelled"
        ))
        .is_err()
    );
    let cancellation = Cancellation::default();
    let result = load_with(
        &source,
        &cancellation,
        &fixture.0.join("other-cache"),
        |_, token| {
            token.cancel();
            Ok(numeric_json())
        },
    );
    assert!(result.expect_err("cancelled").contains("cancelled"));
}

#[test]
fn numeric_validation_rejects_wrong_sizes_and_invalid_values() {
    for count in [0, 1, 511, 513, 1025] {
        assert!(validate_peaks(&vec![0.0; count]).is_err());
    }
    for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(validate_peaks(&vec![value; 1024]).is_err());
    }
    assert!(validate_peaks(&vec![0.0; 512]).is_ok());
    assert!(validate_peaks(&vec![1.0; 1024]).is_ok());
}

#[test]
fn analyzer_errors_and_invalid_payloads_are_never_cached() {
    let fixture = Fixture::new();
    let source = fixture.source();
    let identity = Identity::read(&source).expect("identity");
    let directory = fixture.cache();
    for result in [
        Err("Unsupported long track".to_owned()),
        Ok(b"[0.5]".to_vec()),
        Ok(b"not JSON".to_vec()),
        Ok(serde_json::to_vec(&vec![1.1_f32; 1024]).expect("out of range")),
        Ok(vec![b' '; MAX_ENTRY_BYTES as usize + 1]),
    ] {
        assert!(load_with(&source, &Cancellation::default(), &directory, |_, _| result).is_err());
        assert!(!directory.join(identity.filename()).exists());
    }
}

#[test]
fn oversized_owned_entries_are_removed_even_below_entry_cap() {
    let fixture = Fixture::new();
    let directory = fixture.cache();
    prepare_cache(&directory).expect("cache");
    let identity = Identity::read(&fixture.source()).expect("identity");
    let oversized = directory.join(identity.filename());
    fs::write(&oversized, vec![0_u8; MAX_ENTRY_BYTES as usize + 1]).expect("oversized entry");
    cleanup(&directory, 0).expect("cleanup");
    assert!(!oversized.exists());
}

#[test]
fn corrupt_oversized_mismatched_and_symlink_caches_are_misses() {
    let fixture = Fixture::new();
    let identity = Identity::read(&fixture.source()).expect("identity");
    let directory = fixture.cache();
    prepare_cache(&directory).expect("cache");
    let path = directory.join(identity.filename());
    for data in [
        b"not JSON".to_vec(),
        vec![b' '; MAX_ENTRY_BYTES as usize + 1],
    ] {
        fs::write(&path, data).expect("bad entry");
        assert!(read_cache(&directory, &identity).is_none());
    }
    let mut mismatched = identity.clone();
    mismatched.changed.1 += 1;
    fs::write(
        &path,
        serde_json::to_vec(&CacheEntry {
            identity: mismatched,
            peaks: vec![0.0; 1024],
        })
        .expect("serialize"),
    )
    .expect("mismatched cache");
    assert!(read_cache(&directory, &identity).is_none());
    fs::remove_file(&path).expect("remove");
    let outside = fixture.0.join("outside.json");
    fs::write(&outside, b"outside").expect("outside");
    symlink(&outside, &path).expect("symlink");
    assert!(read_cache(&directory, &identity).is_none());
    cleanup(&directory, MAX_CACHE_BYTES).expect("cleanup");
    assert!(
        fs::symlink_metadata(&path)
            .expect("symlink preserved")
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&outside).expect("outside"), b"outside");
}

#[test]
fn cache_cleanup_caps_entries_and_bytes_but_preserves_unowned_files() {
    let fixture = Fixture::new();
    let directory = fixture.cache();
    prepare_cache(&directory).expect("cache");
    let mut identity = Identity::read(&fixture.source()).expect("identity");
    for index in 0..MAX_CACHE_ENTRIES + 12 {
        identity.inode = index as u64;
        fs::write(
            directory.join(identity.filename()),
            vec![b' '; MAX_ENTRY_BYTES as usize],
        )
        .expect("entry");
    }
    let unrelated = directory.join("keep.json");
    fs::write(&unrelated, b"user file").expect("unowned");
    write_cache(&directory, identity, &vec![0.5; 1024]).expect("write");
    let entries: Vec<_> = fs::read_dir(&directory)
        .expect("cache")
        .map(|entry| entry.expect("entry"))
        .filter(|entry| entry.file_name().to_str().is_some_and(owned_name))
        .collect();
    assert!(entries.len() <= MAX_CACHE_ENTRIES);
    assert!(
        entries
            .iter()
            .map(|entry| entry.metadata().expect("metadata").len())
            .sum::<u64>()
            <= MAX_CACHE_BYTES
    );
    assert_eq!(fs::read(unrelated).expect("preserved"), b"user file");
    assert_eq!(
        fs::metadata(directory.join(".waveform-cache.lock"))
            .expect("lock")
            .len(),
        0
    );
}

#[test]
fn writer_lock_is_reusable_and_stale_owned_temporaries_are_cleaned() {
    let fixture = Fixture::new();
    let directory = fixture.cache();
    let identity = Identity::read(&fixture.source()).expect("identity");
    prepare_cache(&directory).expect("cache");
    let stale = directory.join(".waveform-123-456.tmp");
    fs::write(&stale, b"interrupted cache write").expect("temporary");
    let unrelated = directory.join(".waveform-not-owned.tmp");
    fs::write(&unrelated, b"preserve").expect("unowned");
    write_cache(&directory, identity.clone(), &vec![0.5; 1024]).expect("write");
    assert!(!stale.exists());
    assert!(unrelated.exists());
    write_cache(&directory, identity.clone(), &vec![0.25; 1024]).expect("second write");
    assert_eq!(
        read_cache(&directory, &identity).expect("latest"),
        vec![0.25; 1024]
    );
    let lock = OpenOptions::new()
        .write(true)
        .open(directory.join(".waveform-cache.lock"))
        .expect("lock");
    lock.try_lock().expect("hold lock");
    assert!(write_cache(&directory, identity.clone(), &vec![1.0; 1024]).is_err());
    assert_eq!(
        read_cache(&directory, &identity).expect("not overwritten"),
        vec![0.25; 1024]
    );
}

#[test]
fn unavailable_or_nonprivate_cache_does_not_prevent_analysis() {
    let fixture = Fixture::new();
    let source = fixture.source();
    let directory = fixture.cache();
    fs::create_dir(&directory).expect("directory");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o777)).expect("permissions");
    assert!(prepare_cache(&directory).is_err());
    assert!(
        load_with(&source, &Cancellation::default(), &directory, |_, _| Ok(
            numeric_json()
        ))
        .is_ok()
    );
    let linked = fixture.0.join("linked-cache");
    symlink(&directory, &linked).expect("symlink directory");
    assert!(prepare_cache(&linked).is_err());
}

#[test]
fn ownership_names_are_strict() {
    assert!(owned_name("waveform-v1-0-1-2-3-4-5-6.json"));
    for name in [
        "waveform-v1-a.json",
        "waveform-v1-0-1-2-3-4-5-z.json",
        "other-v1-0-1-2-3-4-5-6.json",
        "waveform-v1-0-1-2-3-4-5-6.json.tmp",
        "waveform-v1-0-1-2-3-4-5-../6.json",
    ] {
        assert!(!owned_name(name));
    }
}
