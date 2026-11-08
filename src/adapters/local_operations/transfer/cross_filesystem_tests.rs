//! Real cross-device failures confined to separately bounded private tmpfs mounts.

use std::{
    cell::Cell,
    io::{self, Write},
    process::{Command, Stdio},
};

use super::*;

const CHILD_ROOT: &str = "HERMES_CROSS_FILESYSTEM_ROOT";
const CHILD_CASE: &str = "HERMES_CROSS_FILESYSTEM_CASE";
const MOUNT_BYTES: u64 = 4 * 1024 * 1024;
const PAYLOAD_BYTES: usize = 1024 * 1024;

fn in_namespace(
    name: &str,
    test: impl FnOnce(&Path) -> Result<(), Box<dyn Error>>,
) -> Result<(), Box<dyn Error>> {
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        assert_eq!(std::env::var(CHILD_CASE)?, name);
        let root = PathBuf::from(root);
        assert!(
            root.file_name().is_some_and(|name| {
                name.to_string_lossy().starts_with("hermes-safe-transfer-")
            })
        );
        // Validate every writable fixture mount before creating or filling files.
        // The magic is Linux TMPFS_MAGIC from linux/magic.h.
        for (path, bytes) in [
            (root.clone(), 65536),
            (root.join("source"), MOUNT_BYTES),
            (root.join("destination"), MOUNT_BYTES),
        ] {
            assert_eq!(rustix::fs::statfs(&path)?.f_type, 0x0102_1994);
            let stats = rustix::fs::statvfs(&path)?;
            assert_eq!(stats.f_blocks.checked_mul(stats.f_frsize), Some(bytes));
        }
        let source_device = fs::metadata(root.join("source"))?.dev();
        let target_device = fs::metadata(root.join("destination"))?.dev();
        assert_ne!(
            source_device, target_device,
            "require actual cross-device IO"
        );
        assert_ne!(source_device, fs::metadata(&root)?.dev());
        assert_ne!(target_device, fs::metadata(&root)?.dev());
        // Prove that rename cannot accidentally satisfy a staged-move test.
        let probe = root.join("source/rename-probe");
        fs::write(&probe, b"private probe")?;
        assert_eq!(
            fs::rename(&probe, root.join("destination/rename-probe"))
                .expect_err("separate tmpfs mounts must reject rename")
                .kind(),
            io::ErrorKind::CrossesDevices
        );
        fs::remove_file(probe)?;
        return test(&root);
    }

    // Only tiny reservation/sentinel files exist outside the namespace. All
    // payloads, filling and transfer writes go to the three private mounts.
    let fixture = Fixture::new()?;
    let mut child = Command::new("bwrap")
        .args([
            "--die-with-parent",
            "--unshare-user",
            "--unshare-pid",
            "--unshare-net",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--size",
            "65536",
            "--tmpfs",
        ])
        .arg(&fixture.0)
        .arg("--dir")
        .arg(fixture.0.join("source"))
        .arg("--dir")
        .arg(fixture.0.join("destination"))
        .args(["--size", &MOUNT_BYTES.to_string(), "--tmpfs"])
        .arg(fixture.0.join("source"))
        .args(["--size", &MOUNT_BYTES.to_string(), "--tmpfs"])
        .arg(fixture.0.join("destination"))
        .arg("--dir")
        .arg(fixture.0.join("home"))
        .arg("--setenv")
        .arg("HOME")
        .arg(fixture.0.join("home"))
        .arg("--setenv")
        .arg("TMPDIR")
        .arg(&fixture.0)
        .arg("--")
        .arg(std::env::current_exe()?)
        .arg(format!(
            "{}::{name}",
            module_path!()
                .split_once("::")
                .expect("crate-prefixed test module")
                .1
        ))
        .args(["--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ROOT, &fixture.0)
        .env(CHILD_CASE, name)
        .env("GIO_USE_VFS", "local")
        .env("GSETTINGS_BACKEND", "memory")
        .env("G_DEBUG", "fatal-criticals")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            child.kill()?;
            let output = child.wait_with_output()?;
            panic!(
                "bounded transfer case {name} exceeded 20 seconds\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "bounded transfer case {name} failed ({})\n{stdout}{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    fixture.assert_clean()?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Destination {
    Absent,
    File,
    Link,
    DanglingLink,
}

const DESTINATIONS: [Destination; 4] = [
    Destination::Absent,
    Destination::File,
    Destination::Link,
    Destination::DanglingLink,
];

struct CrossDeviceFixture {
    root: PathBuf,
    original: Destination,
    payload: Vec<u8>,
}

impl CrossDeviceFixture {
    fn new(root: &Path, original: Destination) -> Result<Self, Box<dyn Error>> {
        let fixture = Self {
            root: root.to_path_buf(),
            original,
            payload: (0..PAYLOAD_BYTES)
                .map(|index| (index % 251) as u8)
                .collect(),
        };
        for path in [fixture.source(), fixture.target()] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        fs::write(fixture.source(), &fixture.payload)?;
        fs::write(root.join("source/untouched"), b"source original")?;
        fs::write(root.join("destination/untouched"), b"target original")?;
        match original {
            Destination::Absent => {}
            Destination::File => fs::write(fixture.target(), b"old contents")?,
            Destination::Link => symlink(root.join("destination/untouched"), fixture.target())?,
            Destination::DanglingLink => {
                symlink(root.join("destination/missing"), fixture.target())?
            }
        }
        Ok(fixture)
    }

    fn source(&self) -> PathBuf {
        self.root.join("source/item")
    }

    fn target(&self) -> PathBuf {
        self.root.join("destination/item")
    }

    fn overwrite(&self) -> bool {
        !matches!(self.original, Destination::Absent)
    }

    fn assert_destination_original(&self) -> Result<(), Box<dyn Error>> {
        match self.original {
            Destination::Absent => assert_eq!(
                fs::symlink_metadata(self.target())
                    .expect_err("failed transfer must not publish a destination")
                    .kind(),
                io::ErrorKind::NotFound
            ),
            Destination::File => assert_eq!(fs::read(self.target())?, b"old contents"),
            Destination::Link => assert_eq!(
                fs::read_link(self.target())?,
                self.root.join("destination/untouched")
            ),
            Destination::DanglingLink => assert_eq!(
                fs::read_link(self.target())?,
                self.root.join("destination/missing")
            ),
        }
        Ok(())
    }

    fn assert_clean(&self) -> Result<(), Box<dyn Error>> {
        assert_eq!(
            fs::read(self.root.join("source/untouched"))?,
            b"source original"
        );
        assert_eq!(
            fs::read(self.root.join("destination/untouched"))?,
            b"target original"
        );
        assert!(!self.root.join("destination/missing").try_exists()?);
        for directory in ["source", "destination"] {
            for entry in fs::read_dir(self.root.join(directory))? {
                let name = entry?.file_name();
                assert!(
                    name == "item" || name == "untouched",
                    "unexpected leftover in {directory}: {name:?}"
                );
            }
        }
        Ok(())
    }

    fn assert_private_stage(&self, stage: &Path) -> Result<u64, Box<dyn Error>> {
        let parent = stage.parent().expect("private staging directory");
        assert_eq!(parent.parent(), self.target().parent());
        assert!(
            parent
                .file_name()
                .expect("stage name")
                .to_string_lossy()
                .starts_with(".hermes-transfer-")
        );
        assert_eq!(fs::metadata(parent)?.permissions().mode() & 0o777, 0o700);
        let metadata = fs::symlink_metadata(stage)?;
        assert_eq!(
            metadata.dev(),
            fs::metadata(self.root.join("destination"))?.dev()
        );
        assert_ne!(metadata.dev(), fs::symlink_metadata(self.source())?.dev());
        Ok(metadata.len())
    }

    fn fill_destination(&self) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.root.join("destination/space-filler");
        let mut file = fs::File::create(&path)?;
        let mut exhausted = false;
        // Even if the mount guard regresses, at most 5 MiB may be written.
        for _ in 0..1280 {
            if let Err(error) = file.write_all(&[b'x'; 4096]) {
                assert_eq!(error.kind(), io::ErrorKind::StorageFull);
                exhausted = true;
                break;
            }
        }
        assert!(exhausted, "private 4 MiB tmpfs must exhaust within 5 MiB");
        // Allow a real partial copy, but never enough room for the 1 MiB file.
        file.set_len(
            file.metadata()?
                .len()
                .checked_sub(128 * 1024)
                .expect("bounded filler size"),
        )?;
        Ok(path)
    }
}

#[test]
fn cross_device_copy_move_and_overwrite_preserve_symlinks() -> Result<(), Box<dyn Error>> {
    in_namespace(
        "cross_device_copy_move_and_overwrite_preserve_symlinks",
        |root| {
            for moving in [false, true] {
                for original in DESTINATIONS {
                    for link in [None, Some("untouched"), Some("missing")] {
                        let fixture = CrossDeviceFixture::new(root, original)?;
                        if let Some(referent) = link {
                            fs::remove_file(fixture.source())?;
                            symlink(root.join("source").join(referent), fixture.source())?;
                        }
                        transfer(
                            &fixture.source(),
                            &fixture.target(),
                            moving,
                            fixture.overwrite(),
                        )?;
                        assert_eq!(fs::symlink_metadata(fixture.source()).is_ok(), !moving);
                        assert_eq!(
                            fs::symlink_metadata(fixture.target())?.dev(),
                            fs::metadata(root.join("destination"))?.dev()
                        );
                        if let Some(referent) = link {
                            assert_eq!(
                                fs::read_link(fixture.target())?,
                                root.join("source").join(referent)
                            );
                            if !moving {
                                assert_eq!(
                                    fs::read_link(fixture.source())?,
                                    root.join("source").join(referent)
                                );
                            }
                        } else {
                            assert!(fs::symlink_metadata(fixture.target())?.is_file());
                            assert_eq!(fs::read(fixture.target())?, fixture.payload);
                            if !moving {
                                assert_eq!(fs::read(fixture.source())?, fixture.payload);
                            }
                        }
                        fixture.assert_clean()?;
                    }
                }
            }
            Ok(())
        },
    )
}

#[test]
fn cross_device_enospc_preserves_originals_and_retries() -> Result<(), Box<dyn Error>> {
    in_namespace(
        "cross_device_enospc_preserves_originals_and_retries",
        |root| {
            for moving in [false, true] {
                for original in DESTINATIONS {
                    let fixture = CrossDeviceFixture::new(root, original)?;
                    let filler = fixture.fill_destination()?;
                    let copied_partially = Cell::new(false);
                    let cancellable = gio::Cancellable::new();
                    let result = native_transfer_with_copy(
                        &fixture.source(),
                        &fixture.target(),
                        moving,
                        fixture.overwrite(),
                        &cancellable,
                        |stage| {
                            // Same real GIO copy as staged_native_transfer, with inspection
                            // before its error drops the private staging directory.
                            let result = gio::File::for_path(fixture.source()).copy(
                                &gio::File::for_path(stage),
                                gio::FileCopyFlags::ALL_METADATA
                                    | gio::FileCopyFlags::NOFOLLOW_SYMLINKS,
                                Some(&cancellable),
                                None,
                            );
                            assert!(
                                result
                                    .as_ref()
                                    .is_err_and(|error| error.matches(gio::IOErrorEnum::NoSpace))
                            );
                            let bytes = fixture
                                .assert_private_stage(stage)
                                .expect("inspect partial stage");
                            assert!(bytes > 0 && bytes < PAYLOAD_BYTES as u64);
                            copied_partially.set(true);
                            result
                        },
                    );
                    assert!(
                        copied_partially.get(),
                        "cross-device move must use the copy fallback"
                    );
                    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::NoSpace)));
                    assert_eq!(fs::read(fixture.source())?, fixture.payload);
                    fixture.assert_destination_original()?;
                    // Repeat through the normal async entry point while still full.
                    assert!(
                        transfer(
                            &fixture.source(),
                            &fixture.target(),
                            moving,
                            fixture.overwrite()
                        )
                        .is_err_and(|error| error.matches(gio::IOErrorEnum::NoSpace))
                    );
                    assert_eq!(fs::read(fixture.source())?, fixture.payload);
                    fixture.assert_destination_original()?;
                    fs::remove_file(filler)?;
                    fixture.assert_clean()?;
                    transfer(
                        &fixture.source(),
                        &fixture.target(),
                        moving,
                        fixture.overwrite(),
                    )?;
                    assert_eq!(fs::read(fixture.target())?, fixture.payload);
                    assert_eq!(fs::symlink_metadata(fixture.source()).is_ok(), !moving);
                    fixture.assert_clean()?;
                }
            }
            Ok(())
        },
    )
}

#[test]
fn cross_device_cancellation_preserves_originals_and_cleans_staging() -> Result<(), Box<dyn Error>>
{
    in_namespace(
        "cross_device_cancellation_preserves_originals_and_cleans_staging",
        |root| {
            for moving in [false, true] {
                for original in DESTINATIONS {
                    let fixture = CrossDeviceFixture::new(root, original)?;
                    let cancelled = gio::Cancellable::new();
                    cancelled.cancel();
                    assert!(
                        staged_native_transfer(
                            &fixture.source(),
                            &fixture.target(),
                            moving,
                            fixture.overwrite(),
                            &cancelled,
                            None
                        )
                        .is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled))
                    );
                    fixture.assert_destination_original()?;
                    fixture.assert_clean()?;
                    let cancellable = gio::Cancellable::new();
                    let observed = Cell::new(false);
                    let mut update = |current, _| {
                        if current > 0 {
                            assert_eq!(
                                fs::read(fixture.source()).expect("source intact during copy"),
                                fixture.payload
                            );
                            fixture
                                .assert_destination_original()
                                .expect("destination intact during copy");
                            observed.set(true);
                            cancellable.cancel();
                        }
                    };
                    let result = staged_native_transfer(
                        &fixture.source(),
                        &fixture.target(),
                        moving,
                        fixture.overwrite(),
                        &cancellable,
                        Some(&mut update),
                    );
                    assert!(
                        observed.get(),
                        "real GIO progress must trigger cancellation"
                    );
                    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
                    assert_eq!(fs::read(fixture.source())?, fixture.payload);
                    fixture.assert_destination_original()?;
                    fixture.assert_clean()?;
                    transfer(
                        &fixture.source(),
                        &fixture.target(),
                        moving,
                        fixture.overwrite(),
                    )?;
                    assert_eq!(fs::read(fixture.target())?, fixture.payload);
                    assert_eq!(fs::symlink_metadata(fixture.source()).is_ok(), !moving);
                    fixture.assert_clean()?;
                }
            }
            Ok(())
        },
    )
}
