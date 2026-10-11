// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use super::{
    dropped_folder_bookmarks, insert_pinned_places, is_standard_place_location, mouse_navigation_delta,
    parse_pinned_places, preferred_pin_drop_action, preferred_sidebar_drop_action,
    remove_pinned_place, reorder_places,
    should_show_standard_place, sidebar_reorder_formats,
};

#[test]
fn pins_can_be_inserted_and_reordered_without_duplicates() {
    use crate::model::Location;
    let first = (Location::local("/test/first"), "First".into());
    let second = (Location::local("/test/second"), "Second".into());
    let third = (Location::local("/test/third"), "Third".into());
    let mut places = vec![first.clone(), third.clone()];
    assert!(insert_pinned_places(&mut places, vec![second.clone()], Some(&third.0)));
    assert_eq!(places, vec![first.clone(), second.clone(), third.clone()]);
    assert!(insert_pinned_places(&mut places, vec![third.clone()], Some(&first.0)));
    assert_eq!(places, vec![third.clone(), first.clone(), second.clone()]);
    assert!(!insert_pinned_places(&mut places, vec![first.clone()], Some(&first.0)));
    assert!(insert_pinned_places(&mut places, vec![third.clone(), third.clone()], None));
    assert_eq!(places, vec![first, second, third]);
    assert!(!insert_pinned_places(&mut places, Vec::new(), None));
}

#[test]
fn view_mode_action_names_round_trip_and_reject_unknown_values() {
    use crate::ui::browser_modes::BrowserMode;
    for mode in [BrowserMode::Columns, BrowserMode::Grid, BrowserMode::Explorer] {
        assert_eq!(BrowserMode::from_action_name(mode.action_name()), Some(mode));
    }
    assert_eq!(BrowserMode::from_action_name("unknown"), None);
}

#[test]
fn pin_drops_negotiate_copy_without_moving_the_source() {
    use gtk::gdk::DragAction;
    assert_eq!(
        preferred_pin_drop_action(DragAction::COPY | DragAction::MOVE, true),
        DragAction::COPY
    );
    assert_eq!(preferred_pin_drop_action(DragAction::COPY, false), DragAction::COPY);
    assert_eq!(preferred_pin_drop_action(DragAction::MOVE, true), DragAction::MOVE);
    assert_eq!(preferred_pin_drop_action(DragAction::MOVE, false), DragAction::empty());
    assert_eq!(
        preferred_pin_drop_action(DragAction::empty(), true),
        DragAction::empty()
    );
}

#[test]
fn sidebar_file_drops_cannot_be_consumed_by_place_reordering() {
    use gtk::glib::prelude::*;
    let formats = gtk::gdk::ContentFormats::builder()
        .add_type(String::static_type())
        .add_type(gtk::gdk::FileList::static_type())
        .build();
    assert!(!sidebar_reorder_formats(&formats));
    let formats = gtk::gdk::ContentFormats::builder()
        .add_type(String::static_type())
        .add_mime_type("text/uri-list")
        .build();
    assert!(!sidebar_reorder_formats(&formats));
    assert!(sidebar_reorder_formats(&gtk::gdk::ContentFormats::for_type(
        String::static_type()
    )));
}

#[test]
fn sidebar_drops_honor_copy_and_move_modifiers() {
    use gtk::gdk::{DragAction, ModifierType};
    let actions = DragAction::COPY | DragAction::MOVE;
    assert_eq!(
        preferred_sidebar_drop_action(actions, true, ModifierType::empty(), false),
        DragAction::MOVE
    );
    assert_eq!(
        preferred_sidebar_drop_action(actions, false, ModifierType::empty(), false),
        DragAction::COPY
    );
    assert_eq!(
        preferred_sidebar_drop_action(actions, true, ModifierType::CONTROL_MASK, false),
        DragAction::COPY
    );
    assert_eq!(
        preferred_sidebar_drop_action(actions, false, ModifierType::SHIFT_MASK, false),
        DragAction::MOVE
    );
    assert_eq!(
        preferred_sidebar_drop_action(DragAction::COPY, false, ModifierType::SHIFT_MASK, false),
        DragAction::empty()
    );
}

#[test]
fn sidebar_trash_drops_only_accept_move() {
    use gtk::gdk::{DragAction, ModifierType};
    let actions = DragAction::COPY | DragAction::MOVE;
    assert_eq!(
        preferred_sidebar_drop_action(actions, false, ModifierType::empty(), true),
        DragAction::MOVE
    );
    assert_eq!(
        preferred_sidebar_drop_action(actions, true, ModifierType::CONTROL_MASK, true),
        DragAction::empty()
    );
    assert_eq!(
        preferred_sidebar_drop_action(DragAction::COPY, false, ModifierType::empty(), true),
        DragAction::empty()
    );
    assert_eq!(
        preferred_sidebar_drop_action(DragAction::LINK, true, ModifierType::empty(), false),
        DragAction::empty()
    );
}

#[test]
fn sidebar_pinning_filters_files_missing_paths_and_duplicates()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!("hermes-sidebar-pins-{}", std::process::id()));
    std::fs::create_dir_all(directory.join("Folder"))?;
    std::fs::write(directory.join("file.txt"), "fixture")?;
    let folder = crate::model::Location::local(directory.join("Folder"));
    let context = gtk::glib::MainContext::new();
    let folders = context.with_thread_default(|| {
        context.block_on(dropped_folder_bookmarks(vec![
            folder.clone(),
            crate::model::Location::local(directory.join("file.txt")),
            crate::model::Location::local(directory.join("missing")),
            crate::model::Location::uri("search://modified:today"),
            folder.clone(),
        ]))
    })?;
    assert_eq!(folders, vec![(folder, "Folder".to_owned())]);
    std::fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn mouse_side_buttons_map_to_history_navigation() {
    assert_eq!(mouse_navigation_delta(8), Some(-1));
    assert_eq!(mouse_navigation_delta(9), Some(1));
    assert_eq!(mouse_navigation_delta(1), None);
    assert_eq!(mouse_navigation_delta(2), None);
    assert_eq!(mouse_navigation_delta(3), None);
}

#[test]
fn places_can_move_before_an_earlier_item() {
    let mut places = vec!["desktop", "documents", "downloads", "pictures", "videos"];

    assert!(reorder_places(&mut places, "videos", "documents", false));
    assert_eq!(
        places,
        vec!["desktop", "videos", "documents", "downloads", "pictures"]
    );
}

#[test]
fn places_can_move_after_a_later_item() {
    let mut places = vec!["desktop", "documents", "downloads", "pictures", "videos"];

    assert!(reorder_places(&mut places, "documents", "pictures", true));
    assert_eq!(
        places,
        vec!["desktop", "downloads", "pictures", "documents", "videos"]
    );
}

#[test]
fn invalid_place_reorders_leave_the_order_unchanged() {
    let original = vec!["desktop", "documents", "downloads"];
    let mut places = original.clone();

    assert!(!reorder_places(&mut places, "missing", "desktop", false));
    assert!(!reorder_places(&mut places, "desktop", "missing", false));
    assert!(!reorder_places(&mut places, "desktop", "desktop", false));
    assert_eq!(places, original);
}

#[test]
fn gtk_bookmarks_become_native_and_remote_pinned_places() {
    let places = parse_pinned_places(
        "file:///home/user/Projects Work\nsftp://host.example/home/user Remote\nfile:///home/user/Projects Duplicate\n",
    );

    assert_eq!(
        places[0].0.native_path(),
        Some(Path::new("/home/user/Projects"))
    );
    assert_eq!(places[0].1, "Work");
    assert_eq!(
        places[1].0.uri_value(),
        Some("sftp://host.example/home/user")
    );
    assert_eq!(places[1].1, "Remote");
    assert_eq!(places.len(), 2);
}

#[test]
fn pinned_places_can_be_removed_by_location() {
    let removed = crate::model::Location::local("/home/user/Removed");
    let retained = crate::model::Location::local("/home/user/Retained");
    let mut places = vec![
        (removed.clone(), "Removed".to_owned()),
        (retained.clone(), "Retained".to_owned()),
    ];

    assert!(remove_pinned_place(&mut places, &removed));
    assert_eq!(places, vec![(retained, "Retained".to_owned())]);
    assert!(!remove_pinned_place(&mut places, &removed));
}

#[test]
fn home_is_already_a_standard_sidebar_location() {
    assert!(is_standard_place_location(&crate::model::Location::local(
        super::home_directory()
    )));
}

#[test]
fn desktop_is_hidden_when_it_points_to_home() {
    let home = Path::new("/home/user");

    assert!(!should_show_standard_place("desktop", home, home));
    assert!(should_show_standard_place(
        "desktop",
        Path::new("/home/user/Desktop"),
        home
    ));
    assert!(should_show_standard_place("documents", home, home));
}
