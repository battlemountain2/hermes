// SPDX-License-Identifier: GPL-3.0-or-later

use super::recolor_icon_source;

#[test]
fn themed_icons_replace_every_legacy_fallback_color() {
    for fallback in ["#8bc9eb", "#22d3ee", "#2e3436"] {
        let source = format!(r##"<svg stroke="{fallback}"/>"##);
        assert_eq!(
            recolor_icon_source(&source, "#ab6a57"),
            r##"<svg stroke="#ab6a57"/>"##
        );
    }
}

#[test]
fn on_primary_icons_keep_their_contrast_color() {
    assert_eq!(
        recolor_icon_source(r##"<svg stroke="#ffffff"/>"##, "#ab6a57"),
        r##"<svg stroke="#ffffff"/>"##
    );
}

#[test]
#[ignore = "Requires an isolated GTK display; validates icon registration and live-theme cleanup"]
fn desktop_icon_registration_rebinds_and_releases_dead_widgets() {
    use gtk::{glib::object::ObjectType, prelude::ObjectExt};
    use std::cell::RefCell;

    gtk::init().unwrap();
    super::prepare().unwrap();
    let registry = RefCell::new(super::IconRegistry::default());
    let image = gtk::Image::new();
    let key = image.as_ptr() as usize;
    super::register_icon(&registry, &image, super::icons::FOLDER);
    super::register_icon(&registry, &image, super::icons::MUSIC);
    assert_eq!(registry.borrow().entries.len(), 1);
    assert_eq!(registry.borrow().entries[&key].name, super::icons::MUSIC);
    let weak_image = image.downgrade();
    drop(image);
    assert!(weak_image.upgrade().is_none());

    for _ in 0..1_024 {
        let transient = gtk::Image::new();
        super::register_icon(&registry, &transient, super::icons::DOCUMENTS);
    }
    assert!(registry.borrow().entries.len() <= 256);
    let image = gtk::Image::new();
    super::register_icon(&registry, &image, super::icons::MUSIC);
    super::recolor_registered_icons(&registry, "#ab6a57");
    assert_eq!(registry.borrow().entries.len(), 1);
    assert!(image.paintable().is_some());
    drop(image);
    super::recolor_registered_icons(&registry, "#8bc9eb");
    assert!(registry.borrow().entries.is_empty());
}
