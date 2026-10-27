// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
#[ignore = "Requires isolated GTK display"]
fn test_mini_player_constructs_with_proper_title_and_dimensions() {
    gtk::init().expect("Isolated GTK display available");
    let controller = Controller::with_session(None);
    let mini = MiniPlayer::new(controller, Rc::new(|| {}));

    assert_eq!(mini.window().title().as_deref(), Some("Hermes Mini"));
    assert_eq!(mini.window().default_width(), 380);
    assert_eq!(mini.window().default_height(), 128);
    assert!(mini.window().has_css_class("hermes-mini-window"));
    assert!(!mini.is_visible());
    let header = mini
        .window()
        .titlebar()
        .unwrap()
        .downcast::<gtk::WindowHandle>()
        .unwrap();
    assert!(header.has_css_class("mini-player-header"));
    assert!(header.child().unwrap().is::<gtk::Box>());
    mini.close();
}

#[test]
#[ignore = "Requires isolated GTK display; no audio, personal files or preferences"]
fn desktop_mini_escape_and_close_hide_without_changing_transport() {
    gtk::init().unwrap();
    let controller = Controller::with_session(None);
    controller.play_intent.set(true);
    controller.resume_position.set(7_000_000);
    let generation = controller.generation.get();
    let revision = controller.session_revision.get();
    let mini = MiniPlayer::new(controller.clone(), Rc::new(|| {}));
    let keys = mini.window().observe_controllers();
    let keys = (0..keys.n_items())
        .find_map(|index| keys.item(index)?.downcast::<gtk::EventControllerKey>().ok())
        .unwrap();
    assert_eq!(keys.propagation_phase(), gtk::PropagationPhase::Capture);
    mini.window().set_visible(true);
    mini.state.volume.grab_focus();
    assert!(!keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::space,
            &0_u32,
            &gtk::gdk::ModifierType::empty()
        ],
    ));
    assert!(mini.is_visible());
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Escape,
            &0_u32,
            &gtk::gdk::ModifierType::empty()
        ],
    ));
    assert!(!mini.is_visible());
    assert!(controller.play_intent.get());
    assert_eq!(controller.generation.get(), generation);
    assert_eq!(controller.position(), 7_000_000);
    assert_eq!(controller.session_revision.get(), revision);
    let child = mini.window().child().unwrap();
    mini.toggle();
    assert!(mini.is_visible());
    mini.window().close();
    assert!(!mini.is_visible());
    assert_eq!(mini.window().child().as_ref(), Some(&child));
    assert!(controller.play_intent.get());
    assert_eq!(controller.generation.get(), generation);
    assert_eq!(controller.position(), 7_000_000);
    assert_eq!(controller.session_revision.get(), revision);
    mini.close();
    controller.shutdown();
}

#[test]
#[ignore = "Requires isolated GTK display; resource-only icons and no personal files or preferences"]
fn desktop_mini_play_pause_icons_keep_on_accent_colors_across_theme_changes() {
    gtk::init().unwrap();
    gtk::gio::resources_register_include!("strata.gresource").unwrap();
    assets::set_on_accent_icon_color("#102030");
    let mini = MiniPlayer::new(Controller::with_session(None), Rc::new(|| {}));
    for playing in [false, true, false] {
        mini.state.sync_play_icon(playing);
        let image = mini
            .state
            .play
            .child()
            .unwrap()
            .downcast::<gtk::Image>()
            .unwrap();
        let icon = if playing { icons::PAUSE } else { icons::PLAY };
        assert_eq!(
            image.paintable(),
            assets::on_accent_icon(icon, 16).paintable()
        );
        let original = image.paintable().unwrap();
        assets::set_primary_icon_color("#aabbcc");
        assets::set_text_icon_color("#ddeeff");
        assert_eq!(image.paintable().as_ref(), Some(&original));
        assets::set_on_accent_icon_color("#f0e0d0");
        assert_eq!(
            image.paintable(),
            assets::on_accent_icon(icon, 16).paintable()
        );
        if playing {
            mini.state.sync_play_icon(true);
            assert_eq!(mini.state.play.child().as_ref(), Some(image.upcast_ref()));
        }
        assets::set_on_accent_icon_color("#102030");
    }
    mini.close();
}
