// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::prelude::*;

#[cfg(test)]
mod tests;

pub fn show_shortcuts_window(parent: &impl IsA<gtk::Window>) {
    let window = gtk::ShortcutsWindow::builder()
        .transient_for(parent)
        .modal(true)
        .build();

    let section = gtk::ShortcutsSection::builder().build();

    // Navigation group
    let nav_group = gtk::ShortcutsGroup::builder().title("Navigation").build();
    add_shortcut(&nav_group, "New tab", "<Primary>t");
    add_shortcut(&nav_group, "Close tab", "<Primary>w");
    add_shortcut(&nav_group, "Reopen closed tab", "<Primary><Shift>t");
    add_shortcut(&nav_group, "Switch to tab 1-9", "<Primary>1...9");
    add_shortcut(&nav_group, "Enclosing folder", "<Alt>Up");
    add_shortcut(&nav_group, "Back in history", "<Alt>Left");
    add_shortcut(&nav_group, "Forward in history", "<Alt>Right");
    add_shortcut(&nav_group, "Location / address bar", "<Primary>l");
    add_shortcut(&nav_group, "Toggle hidden files", "<Primary>h");
    section.append(&nav_group);

    // Views group
    let view_group = gtk::ShortcutsGroup::builder().title("Views").build();
    add_shortcut(&view_group, "Column view", "<Primary><Shift>1");
    add_shortcut(&view_group, "Grid view", "<Primary><Shift>2");
    add_shortcut(&view_group, "Explorer view", "<Primary><Shift>3");
    add_shortcut(&view_group, "Toggle sidebar", "<Primary>b");
    add_shortcut(&view_group, "Preview / Quick Look", "space");
    add_shortcut(&view_group, "Zoom in / Airy density", "<Primary>plus");
    add_shortcut(&view_group, "Zoom out / Compact density", "<Primary>minus");
    add_shortcut(&view_group, "Reset zoom & density", "<Primary>0");
    section.append(&view_group);

    // Search & Omnibar group
    let search_group = gtk::ShortcutsGroup::builder().title("Search").build();
    add_shortcut(&search_group, "Unified search / Omnibar", "<Primary>f");
    add_shortcut(&search_group, "Quick filter", "slash");
    add_shortcut(&search_group, "Close search or dialog", "Escape");
    section.append(&search_group);

    // File Operations group
    let ops_group = gtk::ShortcutsGroup::builder().title("File Operations").build();
    add_shortcut(&ops_group, "Copy files", "<Primary>c");
    add_shortcut(&ops_group, "Copy path", "<Primary><Shift>c");
    add_shortcut(&ops_group, "Cut files", "<Primary>x");
    add_shortcut(&ops_group, "Paste files", "<Primary>v");
    add_shortcut(&ops_group, "Duplicate selection", "<Primary>d");
    add_shortcut(&ops_group, "Invert selection", "<Primary><Shift>i");
    add_shortcut(&ops_group, "Open in Terminal", "F4");
    add_shortcut(&ops_group, "New folder", "<Primary><Shift>n");
    add_shortcut(&ops_group, "New empty file", "<Primary><Alt>n");
    add_shortcut(&ops_group, "Rename file or folder", "F2");
    add_shortcut(&ops_group, "Move to Trash", "Delete");
    add_shortcut(&ops_group, "Context menu", "Menu");
    add_shortcut(&ops_group, "Context menu (alternative)", "<Shift>F10");
    section.append(&ops_group);

    // General group
    let gen_group = gtk::ShortcutsGroup::builder().title("General").build();
    add_shortcut(&gen_group, "Keyboard shortcuts", "F1");
    add_shortcut(&gen_group, "Preferences", "<Primary>comma");
    add_shortcut(&gen_group, "Quit", "<Primary>q");
    section.append(&gen_group);

    window.set_child(Some(&section));
    window.present();
}

fn add_shortcut(group: &gtk::ShortcutsGroup, title: &str, accelerator: &str) {
    let shortcut = gtk::ShortcutsShortcut::builder()
        .title(title)
        .accelerator(accelerator)
        .build();
    group.append(&shortcut);
}
