// SPDX-License-Identifier: GPL-3.0-or-later

use super::{application, icon};

#[cfg(target_os = "linux")]
use gtk::prelude::*;
#[cfg(target_os = "linux")]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(target_os = "linux")]
struct Fixture(PathBuf);

#[cfg(target_os = "linux")]
impl Fixture {
    fn new(contents: &str, mode: u32) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let directory = std::env::temp_dir().join(format!(
            "hermes-platform-desktop-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("create private desktop-file fixture");
        let path = directory.join("generated.desktop");
        fs::write(&path, contents).expect("write generated desktop file");
        fs::set_permissions(&path, fs::Permissions::from_mode(mode))
            .expect("set generated launcher permissions");
        Self(path)
    }
}

#[cfg(target_os = "linux")]
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(self.0.parent().expect("fixture has a private parent"))
            .expect("remove private desktop-file fixture");
    }
}

#[cfg(target_os = "linux")]
const DESKTOP_ENTRY: &str = "[Desktop Entry]\nType=Application\nName=Hermes generated fixture\nExec=/bin/true\nIcon=hermes-platform-fixture\n";

#[cfg(target_os = "linux")]
#[test]
fn untrusted_file_can_supply_an_icon_but_cannot_supply_a_launchable_app() {
    let fixture = Fixture::new(DESKTOP_ENTRY, 0o600);
    assert_eq!(icon(&fixture.0).as_deref(), Some("hermes-platform-fixture"));
    let error = application(&fixture.0).expect_err("nonexecutable fixture must be untrusted");
    assert_eq!(error.title, "Desktop entry is not trusted");
    assert_eq!(
        error.detail,
        "Mark the file as executable before launching it."
    );
}

#[cfg(target_os = "linux")]
#[test]
fn executable_file_returns_portable_app_info() {
    let fixture = Fixture::new(DESKTOP_ENTRY, 0o700);
    let app = application(&fixture.0).expect("discover executable fixture");
    assert_eq!(app.name(), "Hermes generated fixture");
    assert_eq!(
        app.icon().and_then(|icon| icon.to_string()).as_deref(),
        Some("hermes-platform-fixture")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn invalid_executable_desktop_file_fails_and_has_no_icon() {
    let fixture = Fixture::new("generated invalid desktop file\n", 0o700);
    assert!(icon(&fixture.0).is_none());
    let error = application(&fixture.0).expect_err("invalid fixture must be rejected");
    assert_eq!(error.title, "Unable to open desktop entry");
    assert_eq!(error.detail, "The desktop entry is invalid.");
}

#[cfg(not(target_os = "linux"))]
#[test]
fn unsupported_desktop_files_fail_before_any_filesystem_or_launch_handling() {
    for path in [
        "/usr/share/applications/generated.desktop",
        "/nonexistent/hermes-platform/generated.desktop",
    ] {
        let path = std::path::Path::new(path);
        assert!(icon(path).is_none());
        let error = application(path).expect_err("desktop files are unsupported on this target");
        assert_eq!(error.title, "Desktop entries are unsupported");
        assert_eq!(
            error.detail,
            "Launching Linux .desktop files is unavailable on this platform."
        );
    }
}
