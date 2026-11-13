// SPDX-License-Identifier: GPL-3.0-or-later

//! Native open flags and availability of Linux descriptor-backed persistence.

#[cfg(target_os = "macos")]
mod native {
    pub(crate) const NO_FOLLOW: i32 = 0x100;
    pub(crate) const NON_BLOCK: i32 = 0x4;
    pub(crate) const DIRECTORY: i32 = 0x100000;
}

// Retain the existing flags on Linux and other previously unsupported targets.
#[cfg(not(target_os = "macos"))]
mod native {
    pub(crate) const NO_FOLLOW: i32 = 0x20000;
    pub(crate) const NON_BLOCK: i32 = 0x800;
    pub(crate) const DIRECTORY: i32 = 0x10000;
}

pub(crate) use native::{DIRECTORY, NO_FOLLOW, NON_BLOCK};

/// These stores require Linux /proc/self/fd directory traversal.
/// Persistence on macOS stays pending until a native adapter is available.
pub(crate) const fn descriptor_storage_supported() -> bool {
    cfg!(target_os = "linux")
}

#[cfg(test)]
mod tests;
