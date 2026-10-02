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

#[test]
fn edge_case_boundary_values_for_item_count_and_selection() {
    // 0 items boundary
    assert_eq!(format_item_count(0), "0 items");
    assert_eq!(format_selection(0, 0), "");
    assert_eq!(format_selection(0, 999_999), "");

    // 1 item boundary
    assert_eq!(format_item_count(1), "1 item");
    assert_eq!(format_selection(1, 0), "1 selected, 0 B");
    assert_eq!(format_selection(1, 1024), "1 selected, 1 kB");

    // max usize items boundary
    let max_str = format!("{} items", usize::MAX);
    assert_eq!(format_item_count(usize::MAX), max_str);
    let max_sel_str = format!("{} selected, 1 MB", usize::MAX);
    assert_eq!(format_selection(usize::MAX, 1_000_000), max_sel_str);
}

#[test]
fn edge_case_byte_sizes_for_free_space() {
    // 0 B
    assert_eq!(format_free_space(0), "0 B free");

    // 1023 B
    assert_eq!(format_free_space(1023), "1 kB free");

    // 1024 B
    assert_eq!(format_free_space(1024), "1 kB free");

    // 1 MB (decimal 1,000,000 B)
    assert_eq!(format_free_space(1_000_000), "1 MB free");

    // 1 MB (binary 1,048,576 B)
    assert_eq!(format_free_space(1_048_576), "1 MB free");

    // 1 GB (decimal 1,000,000,000 B)
    assert_eq!(format_free_space(1_000_000_000), "1 GB free");

    // 10 TB (decimal 10,000,000,000,000 B)
    assert_eq!(format_free_space(10_000_000_000_000), "10 TB free");

    // u64::MAX boundary
    assert_eq!(format_free_space(u64::MAX), "18446744.1 TB free");
    assert_eq!(format_selection(1, u64::MAX), "1 selected, 18446744.1 TB");
}

#[test]
fn test_status_bar_clear_free_space_widget() {
    if gtk::init().is_err() {
        eprintln!("GTK init skipped (no display server)");
        return;
    }
    let bar = StatusBar::new();
    // Simulate updating free space
    bar.free_space.set_text("500 GB free");
    assert_eq!(bar.free_space.text(), "500 GB free");

    // Verify clear_free_space clears the label
    bar.clear_free_space();
    assert_eq!(bar.free_space.text(), "");
}

