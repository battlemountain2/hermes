use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

const GTK: &[u8] = b"# retain exact bytes\r\n[preferred]\r\ndefault=gtk;\r\norg.freedesktop.impl.portal.FileChooser=gtk\r\norg.freedesktop.impl.portal.ScreenCast=wlr\r\n";
const KEY: &str = "org.freedesktop.impl.portal.FileChooser=hermes\n";

struct Fixture {
    root: PathBuf,
    integration: Integration,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hermes-picker-restoration-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self {
            integration: Integration {
                config: root.join("config"),
                data: root.join("data"),
                state: root.join("state"),
                binary: root.join("bin with spaces/hermes"),
                desktop: "Hyprland:GNOME".into(),
                config_dirs: root.join("system-config").into_os_string(),
                data_dirs: root.join("system-data").into_os_string(),
                portal_dir: None,
                development: false,
            },
            root,
        };
        fixture.seed(
            &fixture.integration.binary,
            b"\x7fELFfixture-HERMES_FILE_CHOOSER_V1",
        );
        fs::set_permissions(
            &fixture.integration.binary,
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        fixture.seed(&fixture.inherited(), GTK);
        fixture
    }

    fn seed(&self, path: &Path, content: &[u8]) {
        assert!(path.starts_with(&self.root));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn portal(&self) -> PathBuf {
        self.integration
            .config
            .join("xdg-desktop-portal/hyprland-portals.conf")
    }

    fn inherited(&self) -> PathBuf {
        self.root
            .join("system-config/xdg-desktop-portal/hyprland-portals.conf")
    }

    fn marker(&self) -> PathBuf {
        self.integration
            .config
            .join("hermes/picker-integration.json")
    }

    fn journal(&self) -> PathBuf {
        self.integration
            .state
            .join("hermes/picker-integration-transaction.json")
    }

    fn service(&self) -> PathBuf {
        self.integration
            .data
            .join("dbus-1/services/org.freedesktop.impl.portal.desktop.hermes.service")
    }

    fn manifest(&self) -> PathBuf {
        self.integration
            .data
            .join("xdg-desktop-portal/portals/hermes.portal")
    }

    fn enable(&self) {
        self.integration.set_enabled(true).unwrap();
        assert!(self.integration.is_enabled().unwrap());
        assert!(fs::read_to_string(self.portal()).unwrap().contains(KEY));
    }

    fn restore(&self) {
        self.integration.set_enabled(false).unwrap();
        assert!(!self.integration.is_enabled().unwrap());
        assert!(!self.journal().exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn gtk_hermes_gtk_roundtrip_is_exact_and_repeated_enable_keeps_original_snapshot() {
    let fixture = Fixture::new();
    fixture.seed(&fixture.portal(), GTK);
    fs::set_permissions(fixture.portal(), fs::Permissions::from_mode(0o640)).unwrap();
    let unrelated = fixture.integration.config.join("mimeapps.list");
    fixture.seed(&unrelated, b"# unrelated folder defaults\n");
    fixture.enable();
    let content = fs::read_to_string(fixture.portal()).unwrap();
    assert!(content.contains("default=gtk;\r\n"));
    assert!(content.contains("org.freedesktop.impl.portal.ScreenCast=wlr\r\n"));
    let journal = fs::read(fixture.journal()).unwrap();
    fixture.enable();
    assert_eq!(fs::read(fixture.journal()).unwrap(), journal);
    fixture.restore();
    assert_eq!(fs::read(fixture.portal()).unwrap(), GTK);
    assert_eq!(
        fs::metadata(fixture.portal()).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        fs::read(unrelated).unwrap(),
        b"# unrelated folder defaults\n"
    );
    assert!(!fixture.marker().exists());
    assert!(!fixture.service().exists());
    assert!(!fixture.manifest().exists());
}

#[test]
fn absent_user_file_and_marker_are_removed_after_inherited_config_roundtrip() {
    let fixture = Fixture::new();
    assert!(!fixture.integration.is_enabled().unwrap());
    assert!(!fixture.integration.state.exists()); // Status is read only.
    fixture.enable();
    fixture.restore();
    assert!(!fixture.portal().exists());
    assert!(!fixture.marker().exists());
    assert_eq!(fs::read(fixture.inherited()).unwrap(), GTK);
    fixture.restore(); // Opt-out is idempotent after restoration.
}

#[test]
fn missing_assignment_is_inserted_and_its_absence_is_restored() {
    let fixture = Fixture::new();
    let original = b"# no chooser override\n[preferred]\ndefault=gtk\n";
    fixture.seed(&fixture.portal(), original);
    fixture.seed(&fixture.marker(), b"{\"version\":1,\"enabled\":false}\n");
    fixture.enable();
    fixture.restore();
    assert_eq!(fs::read(fixture.portal()).unwrap(), original);
    assert_eq!(
        fs::read(fixture.marker()).unwrap(),
        b"{\"version\":1,\"enabled\":false}\n"
    );
}

#[test]
fn generic_user_config_beats_inherited_desktop_config_and_stays_untouched() {
    let fixture = Fixture::new();
    let generic = fixture
        .integration
        .config
        .join("xdg-desktop-portal/portals.conf");
    let original = b"[preferred]\ndefault=gtk\n";
    fixture.seed(&generic, original);
    fixture.enable();
    assert!(
        fs::read_to_string(fixture.portal())
            .unwrap()
            .contains("default=gtk\n")
    );
    fixture.restore();
    assert!(!fixture.portal().exists());
    assert_eq!(fs::read(generic).unwrap(), original);
}

#[test]
fn externally_changed_provider_is_never_overwritten_and_snapshot_is_retained() {
    let fixture = Fixture::new();
    fixture.seed(&fixture.portal(), GTK);
    fixture.enable();
    let enabled = fs::read(fixture.portal()).unwrap();
    let external = b"[preferred]\ndefault=gtk\norg.freedesktop.impl.portal.FileChooser=gnome\n";
    fixture.seed(&fixture.portal(), external);
    let journal = fs::read(fixture.journal()).unwrap();
    let marker = fs::read(fixture.marker()).unwrap();
    for requested in [false, true] {
        let error = fixture
            .integration
            .set_enabled(requested)
            .unwrap_err()
            .to_string();
        assert!(error.contains("changed outside Hermes"), "{error}");
        assert_eq!(fs::read(fixture.portal()).unwrap(), external);
        assert_eq!(fs::read(fixture.journal()).unwrap(), journal);
        assert_eq!(fs::read(fixture.marker()).unwrap(), marker);
    }
    assert!(fixture.integration.is_enabled().is_err());
    fixture.seed(&fixture.portal(), &enabled);
    fixture.restore();
    assert_eq!(fs::read(fixture.portal()).unwrap(), GTK);
}

#[test]
fn unrelated_external_portal_edits_are_preserved_by_refusing_restoration() {
    let fixture = Fixture::new();
    fixture.enable();
    let mut external = fs::read(fixture.portal()).unwrap();
    external.extend_from_slice(b"# another program updated this file\n");
    fixture.seed(&fixture.portal(), &external);
    assert!(fixture.integration.set_enabled(false).is_err());
    assert_eq!(fs::read(fixture.portal()).unwrap(), external);
    assert!(fixture.service().exists()); // Preflight refuses any partial restore.
}

#[test]
fn journal_write_failure_leaves_all_desktop_outputs_unchanged() {
    let fixture = Fixture::new();
    fixture.seed(&fixture.portal(), GTK);
    fixture.seed(&fixture.integration.state, b"blocked state directory");
    let error = fixture
        .integration
        .set_enabled(true)
        .unwrap_err()
        .to_string();
    assert!(error.contains("non-directory parent"), "{error}");
    assert_eq!(fs::read(fixture.portal()).unwrap(), GTK);
    assert!(!fixture.marker().exists());
    assert!(!fixture.service().exists());
    assert!(!fixture.manifest().exists());
}

#[test]
fn marker_write_failure_is_reported_without_switching_the_provider() {
    let fixture = Fixture::new();
    fixture.seed(&fixture.portal(), GTK);
    fs::create_dir_all(fixture.marker()).unwrap();
    let error = fixture
        .integration
        .set_enabled(true)
        .unwrap_err()
        .to_string();
    assert!(error.contains("picker-integration.json"), "{error}");
    assert_eq!(fs::read(fixture.portal()).unwrap(), GTK);
    assert!(!fixture.service().exists());
    assert!(!fixture.journal().exists());
}

#[test]
fn interrupted_atomic_publish_retains_original_and_can_be_restored() {
    // Fail the final rename after the real shared writer has staged/flushed the
    // provider file. Prior outputs and the durable journal must remain recoverable.
    let fixture = Fixture::new();
    fixture.seed(&fixture.portal(), GTK);
    let injection = r#"
original_replace = scope["os"].replace
def failing_replace(source, destination):
    if scope["pathlib"].Path(destination).name == "hyprland-portals.conf":
        raise OSError("Generated provider publication failure")
    return original_replace(source, destination)
scope["os"].replace = failing_replace
"#;
    let bridge = BRIDGE.replacen("try:\n", &format!("{injection}\ntry:\n"), 1);
    let mut child = Command::new("python3")
        .args(["-I", "-c", &bridge])
        .arg(&fixture.integration.config)
        .arg(&fixture.integration.data)
        .arg(&fixture.integration.state)
        .arg(&fixture.integration.desktop)
        .arg(&fixture.integration.binary)
        .arg("enable")
        .env("XDG_CONFIG_DIRS", &fixture.integration.config_dirs)
        .env("XDG_DATA_DIRS", &fixture.integration.data_dirs)
        .env_remove("XDG_DESKTOP_PORTAL_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(SETUP.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Generated provider publication failure"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(fixture.portal()).unwrap(), GTK);
    assert!(fixture.journal().exists());
    assert!(fixture.marker().exists());
    let error = fixture.integration.is_enabled().unwrap_err().to_string();
    assert!(error.contains("incomplete"), "{error}");
    fixture.restore();
    assert_eq!(fs::read(fixture.portal()).unwrap(), GTK);
    assert!(!fixture.marker().exists());
    assert!(!fixture.service().exists());
}

#[test]
fn legacy_enabled_marker_without_snapshot_is_refused_instead_of_guessing_kde() {
    let fixture = Fixture::new();
    let legacy = b"[preferred]\norg.freedesktop.impl.portal.FileChooser=hermes\n";
    fixture.seed(&fixture.portal(), legacy);
    fixture.seed(&fixture.marker(), b"{\"version\":1,\"enabled\":true}\n");
    assert!(
        fixture
            .integration
            .is_enabled()
            .unwrap_err()
            .to_string()
            .contains("previous provider is unknown")
    );
    for requested in [false, true] {
        assert!(
            fixture
                .integration
                .set_enabled(requested)
                .unwrap_err()
                .to_string()
                .contains("recovery journal is missing")
        );
        assert_eq!(fs::read(fixture.portal()).unwrap(), legacy);
        assert!(!fixture.journal().exists());
    }
}

#[test]
fn development_enable_and_restore_cannot_touch_shared_defaults_or_create_a_lock() {
    let mut fixture = Fixture::new();
    fixture.enable();
    let portal = fs::read(fixture.portal()).unwrap();
    let journal = fs::read(fixture.journal()).unwrap();
    fs::remove_file(fixture.journal().with_extension("lock")).unwrap();
    fixture.integration.development = true;
    for requested in [true, false] {
        assert_eq!(
            fixture
                .integration
                .set_enabled(requested)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    assert!(fixture.integration.is_enabled().is_err());
    assert_eq!(fs::read(fixture.portal()).unwrap(), portal);
    assert_eq!(fs::read(fixture.journal()).unwrap(), journal);
    assert!(!fixture.journal().with_extension("lock").exists());
}

#[test]
fn symlink_target_is_not_written_and_custom_discovery_is_refused() {
    let mut fixture = Fixture::new();
    fs::create_dir_all(fixture.portal().parent().unwrap()).unwrap();
    symlink(fixture.inherited(), fixture.portal()).unwrap();
    assert!(fixture.integration.set_enabled(true).is_err());
    assert_eq!(fs::read(fixture.inherited()).unwrap(), GTK);
    assert!(!fixture.journal().exists());
    fs::remove_file(fixture.portal()).unwrap();
    fixture.integration.portal_dir = Some(fixture.root.join("custom").into_os_string());
    assert!(
        fixture
            .integration
            .set_enabled(true)
            .unwrap_err()
            .to_string()
            .contains("overrides discovery")
    );
    assert!(!fixture.marker().exists());
}
