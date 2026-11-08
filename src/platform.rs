// SPDX-License-Identifier: GPL-3.0-or-later

//! Narrow OS boundaries. Capability values describe implementation status only.

pub(crate) mod desktop_file;
mod preview;

pub(crate) use preview::preview_isolation;
