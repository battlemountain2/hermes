// SPDX-License-Identifier: GPL-3.0-or-later

//! Narrow OS boundaries. Capability values describe implementation status only.

pub(crate) mod desktop_file;
pub(crate) mod filesystem;
mod preview;

pub(crate) use preview::preview_isolation;
