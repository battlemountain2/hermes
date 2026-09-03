// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::gio;

use super::application_key;

#[test]
fn commandline_applications_have_stable_fallback_keys() {
    let first = gio::AppInfo::create_from_commandline(
        "example-editor %f",
        Some("Example Editor"),
        gio::AppInfoCreateFlags::SUPPORTS_STARTUP_NOTIFICATION,
    )
    .expect("fixture application should be created");
    let same = gio::AppInfo::create_from_commandline(
        "example-editor %f",
        Some("Example Editor"),
        gio::AppInfoCreateFlags::SUPPORTS_STARTUP_NOTIFICATION,
    )
    .expect("fixture application should be created");
    let other = gio::AppInfo::create_from_commandline(
        "other-editor %f",
        Some("Other Editor"),
        gio::AppInfoCreateFlags::SUPPORTS_STARTUP_NOTIFICATION,
    )
    .expect("fixture application should be created");

    assert_eq!(application_key(&first), application_key(&same));
    assert_ne!(application_key(&first), application_key(&other));
}
