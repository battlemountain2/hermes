// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn test_shortcuts_window_constructs_without_panics() {
    if gtk::init().is_err() {
        return;
    }
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
