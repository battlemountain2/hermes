// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn breadcrumbs_preserve_each_native_ancestor() {
    let location = Location::local("/home/user/project");

    assert_eq!(
        location.breadcrumbs(),
        vec![
            Location::local("/"),
            Location::local("/home"),
            Location::local("/home/user"),
            Location::local("/home/user/project"),
        ]
    );
}

#[test]
fn uri_locations_remain_explicit_and_have_one_breadcrumb() {
    let trash = Location::uri("trash:///");

    assert_eq!(trash.native_path(), None);
    assert_eq!(trash.uri_value(), Some("trash:///"));
    assert_eq!(trash.display_name(), "Trash");
    assert_eq!(trash.display_path(), "trash:///");
    assert_eq!(trash.breadcrumbs(), vec![trash.clone()]);
    assert_eq!(trash.parent(), None);
}

#[test]
fn root_has_one_breadcrumb() {
    assert_eq!(
        Location::local("/").breadcrumbs(),
        vec![Location::local("/")]
    );
}

#[test]
fn test_archive_location_breadcrumbs_parent_and_names() {
    let root = Location::uri("archive:///home/user/test.zip/");
    assert_eq!(root.display_name(), "test.zip");
    assert_eq!(root.parent(), Some(Location::local("/home/user")));

    let sub = Location::uri("archive:///home/user/test.zip/folder/sub/");
    assert_eq!(sub.display_name(), "sub");
    assert_eq!(
        sub.parent(),
        Some(Location::uri("archive:///home/user/test.zip/folder/"))
    );

    let crumbs = sub.breadcrumbs();
    assert_eq!(
        crumbs,
        vec![
            Location::local("/"),
            Location::local("/home"),
            Location::local("/home/user"),
            Location::uri("archive:///home/user/test.zip/"),
            Location::uri("archive:///home/user/test.zip/folder/"),
            Location::uri("archive:///home/user/test.zip/folder/sub/"),
        ]
    );
}
