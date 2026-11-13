use super::*;
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{OpenOptionsExt, symlink},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hermes-native-open-flags-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create generated fixture directory");
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove generated fixture directory");
    }
}

#[test]
fn descriptor_storage_is_linux_only() {
    assert_eq!(descriptor_storage_supported(), cfg!(target_os = "linux"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn native_flags_allow_reopening_and_reject_symlinks() {
    let fixture = Fixture::new();
    let path = fixture.0.join("store.lock");
    fs::write(&path, b"preserve").expect("write generated fixture");

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(&path)
        .expect("native nonblocking flag must allow reopening an existing lock");
    drop(file);

    let alias = fixture.0.join("alias");
    symlink(&path, &alias).expect("create generated symlink");
    assert!(
        OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | NON_BLOCK)
            .open(&alias)
            .is_err(),
        "native no-follow flag must reject the symlink"
    );
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW | DIRECTORY | NON_BLOCK)
        .open(&fixture.0)
        .expect("native directory flag must allow opening a directory");
    drop(directory);
    assert!(
        OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | DIRECTORY | NON_BLOCK)
            .open(&path)
            .is_err(),
        "native directory flag must reject a regular file"
    );
    assert_eq!(
        fs::read(&path).expect("read generated fixture"),
        b"preserve"
    );
}
