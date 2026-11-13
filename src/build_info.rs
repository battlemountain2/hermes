// SPDX-License-Identifier: GPL-3.0-or-later

pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("HERMES_BUILD_COMMIT");
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");

#[cfg(test)]
mod tests;
