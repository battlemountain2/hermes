use super::*;

#[test]
fn ownership_excludes_independent_stores_and_releases_after_close() {
    let root = super::super::tests::temporary("ownership");
    let path = root.join("collections.json");
    let mut owner = Storage::open(&path).expect("expected test value from Storage::open(&path)");
    owner
        .save(&Collections::default())
        .expect("expected test value from owner.save(&Collections::default())");
    assert!(Storage::open(&path).is_err());
    let output = std::process::Command::new(std::env::current_exe().expect("expected test value from std::env::current_exe()"))
        .args([
            "--exact",
            "services::music_collections::disk::tests::disk_ownership_probe_child",
            "--ignored",
        ])
        .env("HERMES_COLLECTION_LOCK_FIXTURE", &path)
        .output()
        .expect("expected test value from std::process::Command::new(std::env::current_exe().unwrap()) .args([ \"--exact\", \"services::music_collections::disk::tests::disk...");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    drop(owner);
    assert!(Storage::open(&path).is_ok());
    fs::remove_dir_all(root).expect("remove generated fixture");
}

#[test]
#[ignore = "Private subprocess probe, run by ownership_excludes_independent_stores_and_releases_after_close"]
fn disk_ownership_probe_child() {
    let path =
        PathBuf::from(std::env::var_os("HERMES_COLLECTION_LOCK_FIXTURE").expect(
            "expected test value from std::env::var_os(\"HERMES_COLLECTION_LOCK_FIXTURE\")",
        ));
    assert!(path.starts_with(std::env::temp_dir()));
    assert!(Storage::open(&path).is_err());
}

#[test]
fn external_edits_corruption_and_symlinks_are_never_overwritten() {
    let root = super::super::tests::temporary("conflict");
    let path = root.join("collections.json");
    let mut storage = Storage::open(&path).expect("expected test value from Storage::open(&path)");
    let mut data = Collections::default();
    storage
        .save(&data)
        .expect("expected test value from storage.save(&data)");
    data.apply(Command::Create("Hermes change".into()))
        .expect("expected test value from data.apply(Command::Create(\"Hermes change\".into()))");
    let mut external = Collections::default();
    external
        .apply(Command::Create("External change".into()))
        .expect(
            "expected test value from external .apply(Command::Create(\"External change\".into()))",
        );
    for bytes in [
        serde_json::to_vec(&external).expect("serialize generated JSON fixture"),
        b"corrupted externally".to_vec(),
    ] {
        fs::write(&path, &bytes).expect("write generated fixture");
        assert!(storage.save(&data).is_err());
        assert_eq!(fs::read(&path).expect("read generated fixture"), bytes);
        assert!(
            !fs::read_dir(&root)
                .expect("expected test value from fs::read_dir(&root)")
                .flatten()
                .any(|entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "tmp")
                })
        );
    }
    fs::remove_file(&path).expect("remove generated fixture");
    let other = root.join("other.json");
    fs::write(&other, b"preserved outside file").expect("write generated fixture");
    std::os::unix::fs::symlink(&other, &path)
        .expect("expected test value from std::os::unix::fs::symlink(&other, &path)");
    assert!(storage.save(&data).is_err());
    assert!(
        fs::symlink_metadata(&path)
            .expect("expected test value from fs::symlink_metadata(&path)")
            .is_symlink()
    );
    assert_eq!(
        fs::read(other).expect("read generated fixture"),
        b"preserved outside file"
    );
    drop(storage);
    fs::remove_dir_all(root).expect("remove generated fixture");
}

#[test]
fn second_store_is_read_only_instead_of_overwriting_first_instances_changes() {
    let root = super::super::tests::temporary("two-stores");
    let path = root.join("collections.json");
    let owner = Store::start(Some(path.clone()));
    super::super::tests::wait(&owner, |snapshot| snapshot.ready);
    owner
        .send(Command::Create("First instance".into()))
        .expect("expected test value from owner .send(Command::Create(\"First instance\".into()))");
    owner
        .flush(Duration::from_secs(2))
        .expect("expected test value from owner.flush(Duration::from_secs(2))");
    let second = Store::start(Some(path.clone()));
    let initial = super::super::tests::wait(&second, |snapshot| snapshot.ready);
    assert!(
        initial
            .error
            .as_ref()
            .expect("expected test value from initial .error .as_ref()")
            .contains("another Hermes instance")
    );
    second
        .send(Command::Favorite(vec!["/generated/song.mp3".into()], true))
        .expect("expected test value from second .send(Command::Favorite(vec![\"/generated/song.mp3\".into()], true))");
    super::super::tests::wait(&second, |snapshot| snapshot.revision > initial.revision);
    assert!(second.flush(Duration::from_secs(2)).is_err());
    assert!(
        load(&path)
            .expect("expected test value from load(&path)")
            .favorite_paths()
            .is_empty()
    );
    owner
        .send(Command::Favorite(vec!["/generated/song.mp3".into()], true))
        .expect("expected test value from owner .send(Command::Favorite(vec![\"/generated/song.mp3\".into()], true))");
    owner
        .flush(Duration::from_secs(2))
        .expect("expected test value from owner.flush(Duration::from_secs(2))");
    assert_eq!(
        load(&path)
            .expect("expected test value from load(&path)")
            .favorite_paths()
            .len(),
        1
    );
    drop(second);
    drop(owner);
    fs::remove_dir_all(root).expect("remove generated fixture");
}
