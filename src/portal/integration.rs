// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    ffi::OsString,
    io::{self, Write},
    path::PathBuf,
    process::{Command, Stdio},
};

// Embed the existing owner of the journal, lock, snapshots and atomic writes. This
// works in a packaged binary without depending on a source checkout at runtime.
const SETUP: &str = include_str!("../../scripts/desktop-integration.py");
const BRIDGE: &str = r#"
import sys
scope = {"__name__": "hermes_desktop_setup"}
exec(compile(sys.stdin.read(), "hermes-desktop-setup", "exec"), scope)
try:
    config, data, state, desktop, binary, action = sys.argv[1:]
    setup = scope["PickerSetup"](config, data, state, desktop)
    def read_marker():
        try:
            return scope["read"](setup.preference)
        except OSError as error:
            raise ValueError("Unable to read picker-integration.json as a regular preference file") from error
    if action == "status":
        # Status must not create directories/locks, or claim marker-only legacy
        # changes are reversible when their previous provider was never recorded.
        journal = setup.load()
        if journal:
            setup.verify(journal, restoring=journal["phase"] == "prepared")
            if journal["phase"] != "enabled":
                raise ValueError("Picker setup is incomplete; restore previous picker settings before retrying")
        else:
            marker = read_marker()
            if marker and scope["json"].loads(marker[0]).get("enabled"):
                raise ValueError("Picker recovery journal is missing; the previous provider is unknown. Review desktop setup manually")
        print("enabled" if journal else "disabled")
    else:
        with setup.lock():
            journal = setup.load()
            if journal is None:
                marker = read_marker()
                if marker and scope["json"].loads(marker[0]).get("enabled"):
                    raise ValueError("Picker recovery journal is missing; refusing to overwrite an unknown previous provider")
            if action == "enable":
                setup.enable(binary)
            elif action == "restore":
                setup.restore()
            else:
                raise ValueError("Unknown picker integration action")
except (OSError, ValueError, KeyError, TypeError, AttributeError, scope["configparser"].Error) as error:
    sys.exit(f"Hermes picker setup stopped: {error}. Recovery records are kept; restore previous picker settings before retrying.")
"#;

pub(super) struct Integration {
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub binary: PathBuf,
    pub desktop: OsString,
    pub config_dirs: OsString,
    pub data_dirs: OsString,
    pub portal_dir: Option<OsString>,
    pub development: bool,
}

impl Integration {
    pub fn is_enabled(&self) -> io::Result<bool> {
        match self.invoke("status")?.as_str() {
            "enabled" => Ok(true),
            "disabled" => Ok(false),
            _ => Err(io::Error::other("Unexpected picker setup status")),
        }
    }

    pub fn set_enabled(&self, enabled: bool) -> io::Result<()> {
        self.invoke(if enabled { "enable" } else { "restore" })?;
        Ok(())
    }

    fn invoke(&self, action: &str) -> io::Result<String> {
        // Check before spawning or taking the script's lock: even a Dev opt-out
        // must never restore/write the stable desktop's shared configuration.
        if self.development {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Desktop picker integration is unavailable in development mode",
            ));
        }
        let mut command = Command::new("python3");
        command
            .args(["-I", "-c", BRIDGE])
            .arg(&self.config)
            .arg(&self.data)
            .arg(&self.state)
            .arg(&self.desktop)
            .arg(&self.binary)
            .arg(action)
            .env("XDG_CONFIG_DIRS", &self.config_dirs)
            .env("XDG_DATA_DIRS", &self.data_dirs)
            .env_remove("XDG_DESKTOP_PORTAL_DIR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(value) = &self.portal_dir {
            command.env("XDG_DESKTOP_PORTAL_DIR", value);
        }
        let mut child = command.spawn().map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Unable to run picker setup (requires Python 3): {error}"),
            )
        })?;
        let written = child
            .stdin
            .take()
            .expect("piped setup input")
            .write_all(SETUP.as_bytes());
        // Always reap, including when the child closes its input early.
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "Picker setup failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        written?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
}

#[cfg(test)]
#[path = "integration/tests.rs"]
mod tests;
