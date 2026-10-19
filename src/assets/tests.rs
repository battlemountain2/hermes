// SPDX-License-Identifier: GPL-3.0-or-later

use super::recolor_icon_source;

#[test]
#[ignore = "Requires an isolated GTK display; validates filled control icon colors"]
fn desktop_filled_control_icons_keep_contrast_across_theme_and_playback_changes() {
    use gtk::prelude::*;
    gtk::init().unwrap();
    super::prepare().unwrap();
    super::set_on_accent_icon_color("#102030");
    let image = super::on_accent_icon(super::icons::PLAY, 20);
    let original = image.paintable().unwrap();
    super::set_primary_icon_color("#aabbcc");
    assert_eq!(image.paintable().as_ref(), Some(&original));
    super::set_on_accent_icon_color("#f0e0d0");
    assert_ne!(image.paintable().as_ref(), Some(&original));
    super::set_on_accent_icon(&image, super::icons::PAUSE);
    let expected = super::primary_icon_texture(super::icons::PAUSE, "#f0e0d0").unwrap();
    assert_eq!(image.paintable(), Some(expected.upcast()));
    super::set_primary_icon_color("#778899");
    super::set_text_icon_color("#223344");
    assert_eq!(
        image.paintable(),
        super::primary_icon_texture(super::icons::PAUSE, "#f0e0d0").map(|texture| texture.upcast())
    );
}

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
