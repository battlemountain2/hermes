// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
#[ignore = "Private GTK display; execute with scripts/check-test-suite.py"]
fn test_shortcuts_window_constructs_without_panics() {
    gtk::init().expect("initialize the isolated GTK display");
    let parent = gtk::Window::new();
    let window = gtk::ShortcutsWindow::builder()
        .transient_for(&parent)
        .modal(true)
        .build();

    let section = gtk::ShortcutsSection::builder().build();
    let nav_group = gtk::ShortcutsGroup::builder().title("Navigation").build();
    add_shortcut(&nav_group, "New tab", "<Primary>t");
    add_shortcut(&nav_group, "Close tab", "<Primary>w");
    section.append(&nav_group);

    window.set_child(Some(&section));
    assert!(window.child().is_some());
}
