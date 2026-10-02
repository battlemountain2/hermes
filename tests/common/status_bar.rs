// SPDX-License-Identifier: GPL-3.0-or-later
#![allow(dead_code)]

use gio::prelude::*;
use std::path::Path;

/// Formats byte count using compact decimal units, matching Hermes browser::format_file_size.
pub fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1000;
    const MB: u64 = KB * 1000;
    const GB: u64 = MB * 1000;
    const TB: u64 = GB * 1000;

    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes < TB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    }
}

/// Computes the expected item count label text.
pub fn format_item_count(count: usize) -> String {
    if count == 1 {
        "1 item".to_owned()
    } else {
        format!("{count} items")
    }
}

/// Computes the expected selection info label text.
pub fn format_selection_info(count: usize, total_bytes: u64) -> String {
    if count == 0 {
        String::new()
    } else {
        let size_str = format_file_size(total_bytes);
        format!("{count} selected, {size_str}")
    }
}

/// Computes the expected free space label text.
pub fn format_free_space(free_bytes: u64) -> String {
    let size_str = format_file_size(free_bytes);
    format!("{size_str} free")
}

/// Synchronously queries the GIO filesystem free space for a path.
pub fn query_gio_free_space(path: &Path) -> Option<u64> {
    let file = gio::File::for_path(path);
    let info = file.query_filesystem_info("filesystem::free", gio::Cancellable::NONE).ok()?;
    if info.has_attribute("filesystem::free") {
        Some(info.attribute_uint64("filesystem::free"))
    } else {
        None
    }
}
