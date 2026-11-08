// SPDX-License-Identifier: GPL-3.0-or-later

use super::{PreviewIsolation, preview_isolation};

#[test]
fn pending_and_unsupported_backends_never_advertise_isolation() {
    assert!(PreviewIsolation::LinuxBubblewrap.is_implemented());
    assert!(!PreviewIsolation::MacXpcPending.is_implemented());
    assert!(!PreviewIsolation::Unsupported.is_implemented());
}

#[test]
fn capability_matches_the_compilation_target() {
    #[cfg(target_os = "linux")]
    assert_eq!(preview_isolation(), PreviewIsolation::LinuxBubblewrap);
    #[cfg(target_os = "macos")]
    assert_eq!(preview_isolation(), PreviewIsolation::MacXpcPending);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    assert_eq!(preview_isolation(), PreviewIsolation::Unsupported);
}
