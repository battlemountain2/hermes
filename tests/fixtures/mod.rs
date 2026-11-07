// SPDX-License-Identifier: GPL-3.0-or-later
#![expect(
    dead_code,
    reason = "Each integration executable uses a different subset of shared test fixtures and oracles"
)]

pub mod archives;
pub mod audio;
pub mod geotiff;
pub mod models;
pub mod pdf;
pub mod spreadsheets;
pub mod zip_util;
