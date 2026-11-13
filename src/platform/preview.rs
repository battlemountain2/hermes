// SPDX-License-Identifier: GPL-3.0-or-later

/// Preview decoder isolation implemented for the compilation target.
///
/// This is not a runtime readiness probe. Linux still requires working Bubblewrap,
/// namespaces and decoder dependencies. macOS needs a separate XPC implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewIsolation {
    #[cfg(any(target_os = "linux", test))]
    LinuxBubblewrap,
    #[cfg(any(target_os = "macos", test))]
    MacXpcPending,
    #[cfg(any(not(any(target_os = "linux", target_os = "macos")), test))]
    Unsupported,
}

impl PreviewIsolation {
    pub(crate) fn require_implemented(self) -> Result<(), String> {
        if self.is_implemented() {
            Ok(())
        } else {
            Err("Media previews and playback require the macOS isolated decoder, which is not available in this Dev build".to_owned())
        }
    }

    /// Whether this target has an implemented isolation backend.
    /// Callers must still handle sandbox startup failures without decoding in-process.
    pub(crate) const fn is_implemented(self) -> bool {
        match self {
            #[cfg(any(target_os = "linux", test))]
            Self::LinuxBubblewrap => true,
            #[cfg(any(target_os = "macos", test))]
            Self::MacXpcPending => false,
            #[cfg(any(not(any(target_os = "linux", target_os = "macos")), test))]
            Self::Unsupported => false,
        }
    }
}

pub(crate) const fn preview_isolation() -> PreviewIsolation {
    #[cfg(target_os = "linux")]
    {
        PreviewIsolation::LinuxBubblewrap
    }
    #[cfg(target_os = "macos")]
    {
        PreviewIsolation::MacXpcPending
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        PreviewIsolation::Unsupported
    }
}

#[cfg(test)]
mod tests;
