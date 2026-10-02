// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn item_count_formatting_handles_singular_and_plural() {
    assert_eq!(format_item_count(0), "0 items");
    assert_eq!(format_item_count(1), "1 item");
    assert_eq!(format_item_count(2), "2 items");
    assert_eq!(format_item_count(42), "42 items");
}

#[test]
fn selection_formatting_handles_empty_and_populated_selections() {
    assert_eq!(format_selection(0, 0), "");
    assert_eq!(format_selection(0, 1024), "");
    assert_eq!(format_selection(1, 1_200), "1 selected, 1.2 kB");
    assert_eq!(format_selection(3, 2_500_000_000), "3 selected, 2.5 GB");
}

#[test]
fn free_space_formatting_appends_free_suffix() {
    assert_eq!(format_free_space(0), "0 B free");
    assert_eq!(format_free_space(1_200), "1.2 kB free");
    assert_eq!(format_free_space(1_000_000), "1 MB free");
    assert_eq!(format_free_space(500_000_000_000), "500 GB free");
}
