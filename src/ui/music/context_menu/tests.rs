use super::*;

#[test]
#[ignore = "Requires isolated GTK display; generated library only, no personal library or session"]
fn desktop_context_menus_target_clicked_rows_and_preserve_queue_playback() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let _theme = super::super::super::theme::ThemeManager::shared();
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../../../style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let player = MusicPlayer::with_session(None);
    let first = super::super::tests::tagged_track(
        PathBuf::from("/generated/Album/z.flac"),
        Some(1),
        Some(1),
        "First",
    );
    let second = super::super::tests::tagged_track(
        PathBuf::from("/generated/Album/a.flac"),
        Some(1),
        Some(2),
        "Second",
    );
    let third = super::super::tests::tagged_track(
        PathBuf::from("/generated/Album/b.flac"),
        Some(2),
        Some(1),
        "Third",
    );
    let other = super::super::tests::tagged_track(
        PathBuf::from("/generated/Other/01.flac"),
        None,
        Some(1),
        "Other",
    );
    player.state.library.replace(vec![
        third.clone(),
        other.clone(),
        second.clone(),
        first.clone(),
    ]);
    player.state.render_library();
    player.state.queue.borrow_mut().append(vec![other.clone()]);
    player.state.queue.borrow_mut().select(0);
    player.state.queue_changed();
    let window = gtk::Window::new();
    window.set_default_size(1000, 700);
    window.set_child(Some(&player.view()));
    window.present();
    for _ in 0..100 {
        if !glib::MainContext::default().iteration(false) {
            break;
        }
    }
    let view = super::super::tests::descendants(player.view().upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ListView>().ok())
        .find(|view| {
            view.model()
                .is_some_and(|model| model == player.state.album_selection)
        })
        .unwrap();
    player.state.album_selection.set_selected(1);
    player.state.filter.set_text("Third");
    let target = Target::resolve(&player.state, Kind::Album, 0).unwrap();
    assert_eq!(
        target.tracks,
        vec![first.clone(), second.clone(), third.clone()]
    );
    let selected = player.state.selected_album.borrow().clone();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let row = loop {
        if let Some(row) = super::super::tests::descendants(view.upcast_ref())
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
            .find(|label| label.text().starts_with("Album\n"))
            .and_then(|label| label.parent())
        {
            break row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Album row did not bind"
        );
        glib::MainContext::default().iteration(false);
    };
    let controllers = row.observe_controllers();
    let gesture = (0..controllers.n_items())
        .filter_map(|position| {
            controllers
                .item(position)
                .and_downcast::<gtk::GestureClick>()
        })
        .find(|gesture| gesture.button() == 3)
        .unwrap();
    gesture.emit_by_name::<()>("pressed", &[&1_i32, &24.0_f64, &24.0_f64]);
    let menu = super::super::tests::descendants(&row)
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::Popover>().ok())
        .unwrap();
    super::super::tests::desktop_screenshot(
        &player,
        menu.upcast_ref(),
        "music-album-context-menu.png",
    );
    drop((controllers, gesture, row));
    assert_eq!(player.state.queue.borrow().tracks, vec![other.clone()]);
    assert_eq!(*player.state.selected_album.borrow(), selected);
    assert_eq!(player.state.filter.text(), "Third");
    let controls: Vec<_> = super::super::tests::descendants(menu.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .collect();
    assert_eq!(controls.len(), 3);
    player.state.library.borrow_mut().reverse();
    player.state.render_library();
    controls[2].emit_clicked();
    assert_eq!(
        player.state.queue.borrow().tracks,
        vec![other.clone(), first.clone(), second.clone(), third.clone()]
    );
    assert_eq!(player.state.queue.borrow().current, Some(0));
    assert!(player.state.stream.borrow().is_none());
    assert!(menu.parent().is_none());
    let menu_probe = menu.downgrade();
    drop(controls);
    drop(menu);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while menu_probe.upgrade().is_some() && std::time::Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(menu_probe.upgrade().is_none());
    let target = Target::resolve(&player.state, Kind::Track, 0).unwrap();
    assert_eq!(target.tracks, vec![third.clone()]);
    let menu = show(
        view.upcast_ref(),
        Rc::downgrade(&player.state),
        target,
        24.0,
        24.0,
    );
    super::super::tests::desktop_screenshot(
        &player,
        menu.upcast_ref(),
        "music-track-context-menu.png",
    );
    let controls: Vec<_> = super::super::tests::descendants(menu.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .collect();
    player.state.filter.set_text("Other");
    controls[1].emit_clicked();
    assert_eq!(player.state.queue.borrow().ordered()[1].1, &third);
    assert_eq!(player.state.queue.borrow().current, Some(0));
    assert!(player.state.stream.borrow().is_none());
    assert!(Target::resolve(&player.state, Kind::Track, gtk::INVALID_LIST_POSITION).is_none());
    assert!(Target::resolve(&player.state, Kind::Album, 99).is_none());
    let target = Target::resolve(&player.state, Kind::Album, 0).unwrap();
    let menu = show(
        view.upcast_ref(),
        Rc::downgrade(&player.state),
        target,
        24.0,
        24.0,
    );
    dismiss(view.upcast_ref());
    assert!(menu.parent().is_none());
    let keys = view
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|keys| keys.name().as_deref() == Some("hermes-music-context-keys"))
        .unwrap();
    assert!(!keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::F10,
            &0_u32,
            &gtk::gdk::ModifierType::empty()
        ]
    ));
    let queued = player.state.queue.borrow().tracks.clone();
    for (key, modifiers) in [
        (gtk::gdk::Key::Menu, gtk::gdk::ModifierType::empty()),
        (gtk::gdk::Key::F10, gtk::gdk::ModifierType::SHIFT_MASK),
    ] {
        assert!(keys.emit_by_name::<bool>("key-pressed", &[&key, &0_u32, &modifiers]));
        let menu = super::super::tests::descendants(view.upcast_ref())
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::Popover>().ok())
            .unwrap();
        assert!(
            super::super::tests::descendants(menu.upcast_ref())
                .iter()
                .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
                .any(|label| label.text() == "Other")
        );
        assert_eq!(player.state.queue.borrow().tracks, queued);
        dismiss(view.upcast_ref());
        assert!(menu.parent().is_none());
    }
    let target = Target::resolve(&player.state, Kind::Album, 1).unwrap();
    let menu = show(
        view.upcast_ref(),
        Rc::downgrade(&player.state),
        target,
        24.0,
        24.0,
    );
    view.set_visible(false);
    assert!(menu.parent().is_none());
    window.destroy();
    player.dispose_view();
}
