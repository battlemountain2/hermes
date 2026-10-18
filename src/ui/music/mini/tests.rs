// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::path::PathBuf;

#[test]
#[ignore = "Requires isolated GTK display"]
fn test_mini_player_constructs_with_proper_title_and_dimensions() {
    gtk::init().expect("Isolated GTK display available");
    let controller = Controller::with_session(Some(PathBuf::from("/tmp/dummy-mini-session.json")));
    let mini = MiniPlayer::new(controller, Rc::new(|| {}));

    assert_eq!(mini.window().title().as_deref(), Some("Hermes Mini"));
    assert_eq!(mini.window().default_width(), 380);
    assert_eq!(mini.window().default_height(), 128);
    assert!(mini.window().has_css_class("hermes-mini-window"));
    assert!(!mini.is_visible());
}
