// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use gtk::gio;
#[cfg(target_os = "linux")]
use gtk::{glib, prelude::*};

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DesktopFileError {
    pub(crate) title: &'static str,
    pub(crate) detail: &'static str,
}

/// Discover a desktop-file application only after applying the existing launch policy.
/// Launching and working-directory handling remain with the caller.
#[cfg(target_os = "linux")]
pub(crate) fn application(path: &Path) -> Result<gio::AppInfo, DesktopFileError> {
    use std::os::unix::fs::PermissionsExt;

    let trusted_location = path.starts_with("/usr/share/applications")
        || path.starts_with(glib::home_dir().join(".local/share/applications"));
    let executable = std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false);
    if !trusted_location && !executable {
        return Err(DesktopFileError {
            title: "Desktop entry is not trusted",
            detail: "Mark the file as executable before launching it.",
        });
    }
    gio::DesktopAppInfo::from_filename(path)
        .map(|app| app.upcast())
        .ok_or(DesktopFileError {
            title: "Unable to open desktop entry",
            detail: "The desktop entry is invalid.",
        })
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn application(_path: &Path) -> Result<gio::AppInfo, DesktopFileError> {
    Err(DesktopFileError {
        title: "Desktop entries are unsupported",
        detail: "Launching Linux .desktop files is unavailable on this platform.",
    })
}

/// Read the desktop-file icon without granting permission to launch its application.
#[cfg(target_os = "linux")]
pub(crate) fn icon(path: &Path) -> Option<String> {
    gio::DesktopAppInfo::from_filename(path)?
        .icon()?
        .to_string()
        .map(|icon| icon.to_string())
}

/// Keep the caller's generic icon, without parsing a desktop file or its icon.
#[cfg(not(target_os = "linux"))]
pub(crate) fn icon(_path: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests;
