use super::*;

#[test]
fn ownership_excludes_independent_stores_and_releases_after_close() {
    let root = super::super::tests::temporary("ownership");
    let path = root.join("collections.json");
    let mut owner = Storage::open(&path).unwrap();
    owner.save(&Collections::default()).unwrap();
    assert!(Storage::open(&path).is_err());
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "services::music_collections::disk::tests::disk_ownership_probe_child",
            "--ignored",
        ])
        .env("HERMES_COLLECTION_LOCK_FIXTURE", &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    drop(owner);
    assert!(Storage::open(&path).is_ok());
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Private subprocess probe, run by ownership_excludes_independent_stores_and_releases_after_close"]
fn disk_ownership_probe_child() {
    let path = PathBuf::from(std::env::var_os("HERMES_COLLECTION_LOCK_FIXTURE").unwrap());
    assert!(path.starts_with(std::env::temp_dir()));
    assert!(Storage::open(&path).is_err());
}

#[test]
fn external_edits_corruption_and_symlinks_are_never_overwritten() {
    let root = super::super::tests::temporary("conflict");
    let path = root.join("collections.json");
    let mut storage = Storage::open(&path).unwrap();
    let mut data = Collections::default();
    storage.save(&data).unwrap();
    data.apply(Command::Create("Hermes change".into())).unwrap();
    let mut external = Collections::default();
    external
        .apply(Command::Create("External change".into()))
        .unwrap();
    for bytes in [
        serde_json::to_vec(&external).unwrap(),
        b"corrupted externally".to_vec(),
    ] {
        fs::write(&path, &bytes).unwrap();
        assert!(storage.save(&data).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!fs::read_dir(&root).unwrap().flatten().any(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "tmp")
        }));
    }
    fs::remove_file(&path).unwrap();
    let other = root.join("other.json");
    fs::write(&other, b"preserved outside file").unwrap();
    std::os::unix::fs::symlink(&other, &path).unwrap();
    assert!(storage.save(&data).is_err());
    assert!(fs::symlink_metadata(&path).unwrap().is_symlink());
    assert_eq!(fs::read(other).unwrap(), b"preserved outside file");
    drop(storage);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn second_store_is_read_only_instead_of_overwriting_first_instances_changes() {
    let root = super::super::tests::temporary("two-stores");
    let path = root.join("collections.json");
    let owner = Store::start(Some(path.clone()));
    super::super::tests::wait(&owner, |snapshot| snapshot.ready);
    owner
        .send(Command::Create("First instance".into()))
        .unwrap();
    owner.flush(Duration::from_secs(2)).unwrap();
    let second = Store::start(Some(path.clone()));
    let initial = super::super::tests::wait(&second, |snapshot| snapshot.ready);
    assert!(
        initial
            .error
            .as_ref()
            .unwrap()
            .contains("another Hermes instance")
    );
    second
        .send(Command::Favorite(vec!["/generated/song.mp3".into()], true))
        .unwrap();
    super::super::tests::wait(&second, |snapshot| snapshot.revision > initial.revision);
    assert!(second.flush(Duration::from_secs(2)).is_err());
    assert!(load(&path).unwrap().favorite_paths().is_empty());
    owner
        .send(Command::Favorite(vec!["/generated/song.mp3".into()], true))
        .unwrap();
    owner.flush(Duration::from_secs(2)).unwrap();
    assert_eq!(load(&path).unwrap().favorite_paths().len(), 1);
    drop(second);
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}
