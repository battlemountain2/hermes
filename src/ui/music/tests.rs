use super::*;

#[test]
fn elapsed_time_is_readable_and_never_negative() {
    assert_eq!(time_label(-1), "0:00");
    assert_eq!(time_label(61_000_000), "1:01");
    assert_eq!(time_label(3_600_000_000), "60:00");
}

pub(super) fn tagged_track(
    path: PathBuf,
    disc: Option<u32>,
    track: Option<u32>,
    title: &str,
) -> Track {
    Track {
        path,
        tags: crate::model::MusicTags {
            disc,
            track,
            title: Some(title.into()),
            ..Default::default()
        },
    }
}

#[test]
fn album_playback_includes_earlier_tracks_and_sorts_all_discs_by_metadata() {
    let album = PathBuf::from("/generated/music/Album");
    let first = tagged_track(album.join("z.flac"), Some(1), Some(1), "First");
    let second = tagged_track(album.join("a.flac"), Some(1), Some(2), "Second");
    let third = tagged_track(album.join("b.flac"), Some(2), Some(1), "Third");
    let fourth = tagged_track(album.join("c.flac"), Some(2), Some(2), "Fourth");
    let other = tagged_track(
        PathBuf::from("/generated/music/Other/01.flac"),
        None,
        Some(1),
        "Other",
    );
    let library = vec![
        fourth.clone(),
        other,
        second.clone(),
        third.clone(),
        first.clone(),
    ];
    assert_eq!(
        album_tracks(&library, &album),
        vec![first.clone(), second.clone(), third.clone(), fourth.clone()]
    );
    assert_eq!(
        crate::music::folder_queue(library, &third.path),
        vec![third, fourth]
    );
    assert!(album_tracks(&[], &album).is_empty());
}

#[test]
fn album_playback_falls_back_to_filename_numbers_and_excludes_other_folders() {
    let album = PathBuf::from("/generated/music/Album");
    let first = tagged_track(album.join("02.flac"), None, None, "First");
    let second = tagged_track(album.join("10.flac"), None, None, "Second");
    let unknown = tagged_track(album.join("untagged.flac"), None, None, "Unknown");
    let other = tagged_track(album.join("nested/01.flac"), None, None, "Other");
    assert_eq!(
        album_tracks(
            &[unknown.clone(), other, second.clone(), first.clone()],
            &album
        ),
        vec![first, second, unknown]
    );
}

#[test]
fn album_tracks_empty_and_matching_keys() {
    let album = PathBuf::from("/music/Album");
    let track1 = tagged_track(album.join("01.flac"), None, Some(1), "Song 1");
    let track2 = tagged_track(album.join("02.flac"), None, Some(2), "Song 2");
    let other = tagged_track(PathBuf::from("/music/Other/01.flac"), None, Some(1), "Other Song");
    let library = vec![track2.clone(), other, track1.clone()];
    assert_eq!(album_tracks(&library, &album), vec![track1, track2]);
}

#[test]
#[ignore = "Requires isolated GTK display, FFmpeg and a built worker; generated media only, no personal library or session"]
fn desktop_album_browse_activation_filtered_results_and_queue_visibility() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let _theme = super::super::theme::ThemeManager::shared();
    let provider = gtk::CssProvider::new();
    let css_errors = Rc::new(RefCell::new(Vec::new()));
    let errors = css_errors.clone();
    provider.connect_parsing_error(move |_, _, error| errors.borrow_mut().push(error.to_string()));
    provider.load_from_string(include_str!("../../style.css"));
    assert!(css_errors.borrow().is_empty(), "{css_errors:?}");
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let root = std::env::temp_dir().join(format!("hermes-music-cleanup-{}", std::process::id()));
    let empty_library = root.join("empty-library");
    let album = root.join("Album");
    let other_album = root.join("Other");
    for directory in [&empty_library, &album, &other_album] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let specs = [
        ("z.flac", 1, 1, "First"),
        ("a.flac", 1, 2, "Second"),
        ("b.flac", 2, 1, "Third"),
        ("c.flac", 2, 2, "Fourth"),
    ];
    let mut tracks: Vec<_> = specs
        .into_iter()
        .map(|(name, disc, number, title)| {
            tagged_track(album.join(name), Some(disc), Some(number), title)
        })
        .collect();
    tracks.push(tagged_track(
        other_album.join("01.flac"),
        None,
        Some(1),
        "Other",
    ));
    for track in &tracks {
        assert!(
            std::process::Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-nostdin",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    "anullsrc=r=48000:cl=stereo",
                    "-t",
                    "30"
                ])
                .arg(&track.path)
                .status()
                .unwrap()
                .success()
        );
    }
    let player = MusicPlayer::with_session(None);
    player.state.volume.set_value(0.0);
    player
        .state
        .scan
        .replace(Some(crate::music::library::Scan::start(empty_library)));
    let contents = gtk::Box::new(gtk::Orientation::Vertical, 0);
    contents.append(&player.view());
    contents.append(&player.bar());
    let window = gtk::Window::builder()
        .default_width(1400)
        .default_height(800)
        .child(&contents)
        .build();
    window.present();
    desktop_pump(&player, || {
        player.state.status.text().contains("Library ready")
    });
    assert!(!player.state.waveform.is_visible());
    assert!(!player.state.waveform_status.is_visible());
    assert!(!player.state.play_album.is_sensitive());
    assert!(!player.state.selection_options.is_sensitive());
    player.state.library.replace(vec![
        tracks[3].clone(),
        tracks[4].clone(),
        tracks[1].clone(),
        tracks[2].clone(),
        tracks[0].clone(),
    ]);
    player.state.render_library();
    desktop_screenshot(&player, contents.upcast_ref(), "music-idle.png");
    let lists: Vec<_> = descendants(player.state.view.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ListView>().ok())
        .collect();
    let album_view = &lists[0];
    let track_view = &lists[1];
    assert!(!album_view.is_single_click_activate());
    assert!(track_view.is_single_click_activate());
    player.state.album_selection.set_selected(1);
    assert_eq!(player.state.tracks.borrow().as_slice(), &tracks[4..]);
    assert!(player.state.stream.borrow().is_none());
    assert!(player.state.queue.borrow().tracks.is_empty());
    assert!(!player.state.bar.is_visible());
    player.state.album_selection.set_selected(0);
    player.state.filter.set_text("Fourth");
    assert_eq!(player.state.track_model.n_items(), 1);
    player.state.set_shuffle(true);
    album_view.emit_by_name::<()>("activate", &[&0_u32]);
    desktop_pump(&player, || player.is_playing());
    assert_eq!(player.state.queue.borrow().tracks.as_slice(), &tracks[..4]);
    assert!(!player.state.controller.shuffle());
    assert!(player.state.filter.text().is_empty());
    assert_eq!(
        player.state.queue_indices.borrow().as_slice(),
        &[0, 1, 2, 3]
    );
    let stream = player.state.stream.borrow().clone().unwrap();
    assert!(player.state.bar.is_visible());
    desktop_pump(&player, || {
        track_view.width() > 0 && player.state.queue_panel.width() > 0
    });
    let with_queue = track_view.width();
    desktop_screenshot(&player, contents.upcast_ref(), "music-queue-visible.png");
    player.state.queue_toggle.set_active(false);
    assert!(!player.state.queue_panel.is_visible());
    desktop_pump(&player, || track_view.width() > with_queue);
    desktop_screenshot(&player, contents.upcast_ref(), "music-queue-hidden.png");
    assert_eq!(player.state.stream.borrow().as_ref(), Some(&stream));
    assert!(stream.is_playing());
    assert!(player.state.bar.is_visible());
    assert!(
        descendants(player.state.queue_toggle.upcast_ref())
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
            .any(|label| label.text() == "Show queue")
    );
    player.state.album_selection.set_selected(1);
    assert_eq!(player.state.stream.borrow().as_ref(), Some(&stream));
    assert_eq!(player.state.queue.borrow().tracks.as_slice(), &tracks[..4]);
    assert!(player.state.selection_options.is_sensitive());
    assert_eq!(player.state.selected_actions.len(), 3);
    player.state.selection_options.popup();
    desktop_pump(&player, || {
        player
            .state
            .selection_options
            .popover()
            .unwrap()
            .is_visible()
    });
    desktop_screenshot(
        &player,
        contents.upcast_ref(),
        "music-selection-actions.png",
    );
    desktop_screenshot(
        &player,
        player
            .state
            .selection_options
            .popover()
            .unwrap()
            .upcast_ref(),
        "music-selection-popover.png",
    );
    player.state.selected_actions[1].emit_clicked();
    assert!(
        !player
            .state
            .selection_options
            .popover()
            .unwrap()
            .is_visible()
    );
    assert_eq!(player.state.queue.borrow().tracks.last(), Some(&tracks[4]));
    assert_eq!(player.state.stream.borrow().as_ref(), Some(&stream));
    player.state.selected_actions[2].emit_clicked();
    assert_eq!(player.state.queue.borrow().ordered()[1].1, &tracks[4]);
    assert_eq!(player.state.stream.borrow().as_ref(), Some(&stream));
    player.state.repeat.emit_clicked();
    assert_eq!(player.controller().repeat(), Repeat::All);
    assert!(player.state.repeat.has_css_class("repeat-enabled"));
    player.state.repeat.emit_clicked();
    assert_eq!(player.controller().repeat(), Repeat::One);
    player.state.repeat.emit_clicked();
    assert_eq!(player.controller().repeat(), Repeat::Off);
    assert!(!player.state.repeat.has_css_class("repeat-enabled"));
    player.state.filter.set_text("Third Fourth");
    assert_eq!(player.state.track_model.n_items(), 0);
    assert!(!player.state.play_album.is_sensitive());
    assert!(!player.state.selection_options.is_sensitive());
    player.state.filter.set_text("Fourth");
    assert!(
        descendants(player.state.play_album.upcast_ref())
            .iter()
            .filter_map(|widget| widget.downcast_ref::<gtk::Label>())
            .any(|label| label.text() == "Play results")
    );
    player.state.play_album.emit_clicked();
    desktop_pump(&player, || player.is_playing());
    assert_eq!(player.state.queue.borrow().tracks.as_slice(), &tracks[3..4]);
    player.state.album_selection.set_selected(0);
    player.state.filter.set_text("Fourth");
    album_view.emit_by_name::<()>("activate", &[&0_u32]);
    desktop_pump(&player, || player.is_playing());
    assert_eq!(player.state.queue.borrow().tracks.as_slice(), &tracks[..4]);
    track_view.emit_by_name::<()>("activate", &[&2_u32]);
    desktop_pump(&player, || player.is_playing());
    assert_eq!(player.state.queue.borrow().tracks.as_slice(), &tracks[2..4]);
    desktop_pump(&player, || {
        !player
            .state
            .waveform_notice
            .borrow()
            .starts_with("Preparing")
    });
    player.state.peaks.replace(vec![0.25; 512]);
    player.state.sync_controller();
    assert!(player.state.waveform.is_visible());
    let stream = player.state.stream.borrow().clone();
    let queue = player.state.queue.borrow().tracks.clone();
    player.state.waveform_toggle.set_active(false);
    assert!(!player.state.waveform.is_visible());
    assert!(!player.state.waveform_status.is_visible());
    player.state.sync_controller();
    assert!(!player.state.waveform.is_visible());
    assert_eq!(*player.state.stream.borrow(), stream);
    assert_eq!(player.state.queue.borrow().tracks, queue);
    player.state.waveform_toggle.set_active(true);
    assert!(player.state.waveform.is_visible());
    desktop_screenshot(&player, contents.upcast_ref(), "music-waveform.png");
    player.state.peaks.borrow_mut().clear();
    player.state.sync_controller();
    assert!(!player.state.waveform.is_visible());
    assert!(!player.state.waveform_status.is_visible());
    desktop_screenshot(&player, contents.upcast_ref(), "music-no-waveform.png");
    player.state.queue_toggle.set_active(true);
    assert!(player.state.queue_panel.is_visible());
    let stream = player.state.stream.borrow().clone().unwrap();
    let more = descendants(player.state.bar.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::MenuButton>().ok())
        .find(|button| button.has_css_class("music-transport-more"))
        .unwrap();
    more.popup();
    desktop_pump(&player, || more.popover().unwrap().is_visible());
    desktop_screenshot(&player, contents.upcast_ref(), "music-transport-more.png");
    desktop_screenshot(
        &player,
        more.popover().unwrap().upcast_ref(),
        "music-transport-stop-popover.png",
    );
    assert_eq!(player.state.stream.borrow().as_ref(), Some(&stream));
    player.state.view.set_visible(false);
    assert_eq!(player.state.stream.borrow().as_ref(), Some(&stream));
    assert!(player.state.bar.is_visible());
    let stop = descendants(more.popover().unwrap().upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| {
            button
                .tooltip_text()
                .is_some_and(|text| text.starts_with("Stop music"))
        })
        .unwrap();
    stop.emit_clicked();
    assert!(!more.popover().unwrap().is_visible());
    assert!(player.state.stream.borrow().is_none());
    assert!(!player.state.bar.is_visible());
    assert!(!player.state.waveform.is_visible());
    player.shutdown();
    window.close();
    drop(player);
    std::fs::remove_dir_all(root).unwrap();
}

pub(super) fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut result = vec![widget.clone()];
    let mut child = widget.first_child();
    while let Some(widget) = child {
        result.extend(descendants(&widget));
        child = widget.next_sibling();
    }
    result
}

pub(super) fn desktop_screenshot(_player: &MusicPlayer, widget: &gtk::Widget, name: &str) {
    let Some(directory) = std::env::var_os("HERMES_TEST_SCREENSHOT_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    if let Err(error) = std::fs::create_dir_all(&directory) {
        eprintln!("Skipping optional screenshot {name}: {error}");
        return;
    }
    let frames = Rc::new(Cell::new(0));
    let frame_count = frames.clone();
    let tick = widget.add_tick_callback(move |_, _| {
        frame_count.set(frame_count.get() + 1);
        if frame_count.get() >= 2 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    widget.queue_draw();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while frames.get() < 2 && std::time::Instant::now() < deadline {
        for _ in 0..128 {
            if frames.get() >= 2 || !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    if frames.get() < 2 || widget.width() <= 0 || widget.height() <= 0 {
        if frames.get() < 2 {
            tick.remove();
        }
        eprintln!(
            "Skipping optional screenshot {name}: no allocated frame; connect a Broadway browser client"
        );
        return;
    }
    let width = widget.width() as f32;
    let height = widget.height() as f32;
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let Some(node) = snapshot.to_node() else {
        eprintln!(
            "Skipping optional screenshot {name}: no render node; connect a Broadway browser client"
        );
        return;
    };
    let renderer = gtk::gsk::CairoRenderer::new();
    if let Err(error) = renderer.realize(None::<&gtk::gdk::Surface>) {
        eprintln!("Skipping optional screenshot {name}: {error}");
        return;
    }
    let texture = renderer.render_texture(
        &node,
        Some(&gtk::graphene::Rect::new(0.0, 0.0, width, height)),
    );
    if let Err(error) = texture.save_to_png(directory.join(name)) {
        eprintln!("Skipping optional screenshot {name}: {error}");
    }
    renderer.unrealize();
}

fn desktop_pump(player: &MusicPlayer, predicate: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !predicate() && std::time::Instant::now() < deadline {
        for _ in 0..128 {
            if predicate() || !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        predicate(),
        "Timed out: {} / {}",
        player.state.status.text(),
        player.state.detail.text()
    );
}

#[test]
#[ignore = "Requires desktop, FFmpeg and a built worker; uses only generated media and temporary session state"]
fn desktop_queue_drag_buttons_removal_and_restart_resume() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let folder = std::env::temp_dir().join(format!("hermes-queue-desktop-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let session_path = folder.join("session.json");
    let tracks: Vec<_> = (1..=3)
        .map(|number| {
            let path = folder.join(format!("{number}.flac"));
            assert!(
                std::process::Command::new("ffmpeg")
                    .args([
                        "-v",
                        "error",
                        "-nostdin",
                        "-y",
                        "-f",
                        "lavfi",
                        "-i",
                        "sine=frequency=440",
                        "-t",
                        "45",
                    ])
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
            Track {
                path,
                tags: crate::model::MusicTags {
                    title: Some(format!("Queue fixture {number}")),
                    ..Default::default()
                },
            }
        })
        .collect();
    let mut queue = Queue::default();
    queue.replace(tracks.clone(), 1);
    queue.set_shuffle(true, 42);
    queue.move_before(2, 1);
    queue.repeat = Repeat::All;
    let mut store = crate::music::session::Store::new(session_path.clone());
    store.save(crate::music::session::Session::capture(
        &queue, 12_000_000, 0.0,
    ));
    store.finish();
    let player = MusicPlayer::with_session(Some(session_path.clone()));
    let opened = Rc::new(Cell::new(false));
    let captured = opened.clone();
    player.connect_open_player(Rc::new(move || captured.set(true)));
    let window = gtk::Window::builder().child(&player.bar()).build();
    window.present();
    desktop_pump(&player, || player.state.queue_model.n_items() == 3);
    assert!(player.state.stream.borrow().is_none());
    assert!(!player.state.bar.is_visible());
    assert!(player.state.shuffle.is_active());
    assert!(
        player
            .state
            .repeat
            .tooltip_text()
            .unwrap()
            .starts_with("Repeat: all")
    );
    assert_eq!(player.state.queue_indices.borrow().as_slice(), &[1, 2, 0]);
    assert!(player.state.queue_resume.is_sensitive());
    player.state.queue_resume.emit_clicked();
    desktop_pump(&player, || {
        player.is_playing()
            && player
                .state
                .stream
                .borrow()
                .as_ref()
                .is_some_and(|stream| !stream.is_seeking() && stream.timestamp() > 12_000_000)
    });
    let stream = player.state.stream.borrow().clone().unwrap();
    player.state.cover_button.emit_clicked();
    assert!(opened.get());
    assert_eq!(player.state.stream.borrow().as_ref().unwrap(), &stream);
    player.state.queue_button.popup();
    let popover = player.state.queue_button.popover().unwrap();
    desktop_pump(&player, || {
        descendants(popover.upcast_ref())
            .iter()
            .filter(|widget| widget.has_css_class("music-queue-row"))
            .count()
            == 3
    });
    let rows = || {
        descendants(popover.upcast_ref())
            .into_iter()
            .filter(|widget| widget.has_css_class("music-queue-row"))
            .collect::<Vec<_>>()
    };
    let first_rows = rows();
    let handle = first_rows[1].first_child().unwrap();
    let source = handle
        .observe_controllers()
        .item(0)
        .and_downcast::<gtk::DragSource>()
        .unwrap();
    let provider = source
        .emit_by_name::<Option<gtk::gdk::ContentProvider>>("prepare", &[&0.0_f64, &0.0_f64])
        .unwrap();
    let value = provider.value(String::static_type()).unwrap();
    let stale_token = value.get::<String>().unwrap();
    let target = first_rows[2]
        .observe_controllers()
        .item(0)
        .and_downcast::<gtk::DropTarget>()
        .unwrap();
    assert!(target.emit_by_name::<bool>(
        "drop",
        &[
            &glib::BoxedValue(value),
            &0.0_f64,
            &f64::from(first_rows[2].height())
        ]
    ));
    assert_eq!(player.state.queue_indices.borrow().as_slice(), &[1, 0, 2]);
    assert_eq!(player.state.stream.borrow().as_ref().unwrap(), &stream);
    assert!(stream.is_playing());
    assert!(player.state.drag_index(&stale_token).is_none());
    let after_move = rows();
    let remove = after_move[1]
        .last_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    remove.emit_clicked();
    assert_eq!(player.state.queue_model.n_items(), 2);
    assert_eq!(
        player.state.queue.borrow().playing().unwrap().path,
        tracks[1].path
    );
    assert_eq!(player.state.stream.borrow().as_ref().unwrap(), &stream);
    let after_remove = rows();
    let down = after_remove[0]
        .last_child()
        .unwrap()
        .prev_sibling()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    down.emit_clicked();
    assert_eq!(player.state.queue_indices.borrow().as_slice(), &[1, 0]);
    let after_down = rows();
    let up = after_down[1]
        .last_child()
        .unwrap()
        .prev_sibling()
        .unwrap()
        .prev_sibling()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    up.emit_clicked();
    assert_eq!(player.state.queue_indices.borrow().as_slice(), &[0, 1]);
    assert_eq!(player.state.stream.borrow().as_ref().unwrap(), &stream);
    let queue_view = descendants(popover.upcast_ref())
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::ListView>().ok())
        .unwrap();
    let selection = queue_view
        .model()
        .and_downcast::<gtk::SingleSelection>()
        .unwrap();
    let keys = queue_view
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .find(|keys| keys.name().as_deref() == Some("hermes-queue-keys"))
        .unwrap();
    selection.set_selected(1);
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Up,
            &0_u32,
            &gtk::gdk::ModifierType::ALT_MASK
        ]
    ));
    assert_eq!(player.state.queue_indices.borrow().as_slice(), &[1, 0]);
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Down,
            &0_u32,
            &gtk::gdk::ModifierType::ALT_MASK
        ]
    ));
    assert_eq!(player.state.queue_indices.borrow().as_slice(), &[0, 1]);
    player
        .state
        .queue
        .borrow_mut()
        .append(vec![tracks[0].clone()]);
    player.state.queue_changed();
    selection.set_selected(2);
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Delete,
            &0_u32,
            &gtk::gdk::ModifierType::empty()
        ]
    ));
    assert_eq!(player.state.queue_model.n_items(), 2);
    assert_eq!(player.state.stream.borrow().as_ref().unwrap(), &stream);
    player.state.play.emit_clicked();
    let paused_rows = rows();
    paused_rows[0]
        .last_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    desktop_pump(&player, || {
        player
            .state
            .stream
            .borrow()
            .as_ref()
            .is_some_and(|next| next.is_prepared())
    });
    assert!(!player.is_playing());
    assert_eq!(
        player.state.queue.borrow().playing().unwrap().path,
        tracks[2].path
    );
    player.state.play.emit_clicked();
    desktop_pump(&player, || player.is_playing());
    player
        .state
        .queue
        .borrow_mut()
        .append(vec![tracks[0].clone()]);
    player.state.queue_changed();
    player.state.remove_queue(0);
    desktop_pump(&player, || {
        player.is_playing()
            && player
                .state
                .queue
                .borrow()
                .playing()
                .is_some_and(|track| track.path == tracks[0].path)
    });
    let next = player.state.stream.borrow().clone().unwrap();
    next.seek(20_000_000);
    desktop_pump(&player, || {
        !next.is_seeking() && next.timestamp() > 20_000_000
    });
    player.shutdown();
    window.close();
    let restored = MusicPlayer::with_session(Some(session_path.clone()));
    desktop_pump(&restored, || restored.state.queue_model.n_items() == 1);
    assert!(restored.state.stream.borrow().is_none());
    assert!(!restored.state.bar.is_visible());
    assert!(restored.state.resume_position.get() > 20_000_000);
    assert_eq!(
        restored.state.queue.borrow().playing().unwrap().path,
        tracks[0].path
    );
    restored.state.queue_clear.emit_clicked();
    assert_eq!(restored.state.queue_model.n_items(), 0);
    restored.shutdown();
    let final_session = crate::music::session::load(&session_path)
        .unwrap()
        .unwrap()
        .into_queue();
    assert!(final_session.tracks.is_empty());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
#[ignore = "Requires desktop, FFmpeg and a built target/debug/strata sandbox worker"]
fn desktop_full_track_queue_navigation_seek_and_stop() {
    use std::time::Instant;
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let folder = std::env::temp_dir().join(format!("hermes-music-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let earlier = folder.join("01.flac");
    let first = folder.join("03.flac");
    let second = folder.join("04.mp3");
    for path in [&earlier, &first, &second] {
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440",
                "-t",
                "45",
                "-metadata",
                "title=Fixture",
                "-metadata",
                "artist=Hermes test",
            ])
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let player = MusicPlayer::with_session(None);
    player.state.volume.set_value(0.0);
    assert!(!player.state.bar.is_visible());
    let window = gtk::Window::builder().child(&player.bar()).build();
    window.present();
    player.play_folder(first.clone());
    let pump = |predicate: &dyn Fn() -> bool| {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !predicate() && Instant::now() < deadline {
            for _ in 0..128 {
                if !glib::MainContext::default().iteration(false) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(predicate(), "Timed out: {}", player.state.detail.text());
    };
    pump(&|| player.is_playing());
    let library = crate::services::music_library::tests::Fixture::new();
    library.song("First/01.flac", "First library fixture");
    let mut subscription = library.subscribe();
    crate::services::music_library::tests::ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .any(|entry| entry.music_tags.is_some())
    });
    player
        .state
        .scan
        .replace(Some(crate::music::library::Scan::start(
            library.root.clone(),
        )));
    pump(&|| player.state.library.borrow().len() == 1);
    let playing = player.state.stream.borrow().clone().unwrap();
    library.song("Second/01.flac", "Second library fixture");
    pump(&|| player.state.library.borrow().len() == 2);
    assert_eq!(player.state.stream.borrow().as_ref().unwrap(), &playing);
    assert!(
        playing.is_playing(),
        "Library updates must not interrupt playback"
    );
    player
        .state
        .selected_album
        .replace(Some(library.root.join("Second")));
    player.state.render_library();
    std::fs::rename(library.root.join("Second"), library.root.join("Renamed")).unwrap();
    pump(&|| {
        player
            .state
            .library
            .borrow()
            .iter()
            .any(|track| track.path.starts_with(library.root.join("Renamed")))
    });
    assert!(
        player
            .state
            .selected_album
            .borrow()
            .as_ref()
            .is_some_and(|path| player
                .state
                .albums
                .borrow()
                .iter()
                .any(|album| &album.path == path))
    );
    assert_eq!(
        player.state.track_model.n_items(),
        1,
        "Renamed albums must not leave a stale empty view"
    );
    assert_eq!(player.state.queue.borrow().current, Some(0));
    assert_eq!(
        player
            .state
            .queue
            .borrow()
            .tracks
            .iter()
            .map(|track| &track.path)
            .collect::<Vec<_>>(),
        vec![&first, &second]
    );
    assert!(player.state.bar.is_visible());
    assert_eq!(player.state.queue_model.n_items(), 2);
    assert!(!player.state.queue_empty.is_visible());
    assert!(player.state.queue_summary.text().contains("2 songs"));
    player.state.queue_button.popup();
    pump(&|| player.state.queue_button.popover().unwrap().is_visible());
    let queue_popup = player.state.queue_button.popover().unwrap();
    let queue_contents = queue_popup.child().unwrap();
    let queue_scroll = queue_contents
        .last_child()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    let queue_view = queue_scroll
        .child()
        .unwrap()
        .downcast::<gtk::ListView>()
        .unwrap();
    assert_eq!(queue_view.model().unwrap().n_items(), 2);
    assert!(!queue_view.is_single_click_activate());
    queue_view.emit_by_name::<()>("activate", &[&0_u32]);
    pump(&|| player.is_playing());
    assert!(!queue_popup.is_visible());
    let stream = player.state.stream.borrow().clone().unwrap();
    assert!(stream.has_audio() && !stream.has_video());
    assert!(stream.duration() > 44_000_000);
    stream.seek(31_000_000);
    pump(&|| !stream.is_seeking() && stream.timestamp() > 31_100_000);
    player.state.view.set_visible(false);
    assert!(player.is_playing());
    stream.pause();
    assert!(!player.is_playing());
    stream.play();
    stream.seek(43_500_000);
    pump(&|| player.state.queue.borrow().current == Some(1));
    pump(&|| player.is_playing());
    assert!(player.state.stream.borrow().as_ref().unwrap().duration() > 44_000_000);
    let failed_stream = player.state.stream.borrow().clone().unwrap();
    failed_stream.stop();
    failed_stream.set_error(glib::Error::new(
        gtk::gio::IOErrorEnum::Failed,
        "Injected decoder failure",
    ));
    assert!(player.state.play.tooltip_text().unwrap().contains("Retry"));
    player.state.play.emit_clicked();
    pump(&|| player.is_playing());
    assert_eq!(player.state.queue.borrow().current, Some(1));
    assert_eq!(player.state.queue_model.n_items(), 2);
    assert!(player.state.detail.tooltip_text().is_none());
    player.shutdown();
    assert!(!player.state.bar.is_visible());
    assert!(player.state.stream.borrow().is_none());
    assert!(player.state.peaks.borrow().is_empty());
    window.close();
    std::fs::remove_file(earlier).unwrap();
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
    std::fs::remove_dir(folder).unwrap();
}

#[test]
#[ignore = "Requires HERMES_AUDIO_TEST_FILE, desktop and a built worker; plays the entire track silently"]
fn desktop_real_folder_full_song_and_auto_next() {
    use std::time::Instant;
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let path = std::env::var_os("HERMES_AUDIO_TEST_FILE").expect("Audio path");
    let player = MusicPlayer::with_session(None);
    player.state.volume.set_value(0.0);
    let window = gtk::Window::builder().child(&player.bar()).build();
    window.present();
    player.play_folder(PathBuf::from(path));
    let started = Instant::now();
    let mut duration = None;
    let mut next_started = None;
    loop {
        for _ in 0..128 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        if let Some(stream) = player.state.stream.borrow().as_ref() {
            assert!(stream.error().is_none(), "{:?}", stream.error());
            if stream.is_prepared() && duration.is_none() {
                duration = Some(Duration::from_micros(stream.duration() as u64));
                eprintln!(
                    "Real track duration: {} seconds",
                    stream.duration() / 1_000_000
                );
            }
            if player.state.queue.borrow().current == Some(1) && stream.is_playing() {
                let next_started = next_started.get_or_insert(Instant::now());
                if next_started.elapsed() > Duration::from_secs(2) {
                    break;
                }
            }
        }
        let limit = duration.unwrap_or(Duration::ZERO) + Duration::from_secs(30);
        assert!(
            started.elapsed() < limit,
            "Timed out: {}",
            player.state.detail.text()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(player.state.bar.is_visible());
    assert_eq!(player.state.queue.borrow().current, Some(1));
    player.shutdown();
    window.close();
}

#[test]
#[ignore = "Requires isolated GTK display"]
fn desktop_workspace_switching_and_layout_changes_update_bar_visibility() {
    gtk::init().unwrap();
    let player = MusicPlayer::with_session(None);
    player.controller().transport_visible.set(true);

    // Initial state in files workspace with top layout
    player.set_workspace("files");
    assert!(player.bar().is_visible(), "bottom dock visible in files view");

    // Switching to music with top layout hides bottom dock and shows hero deck
    super::super::theme::ThemeManager::shared().set_music_player_layout("top");
    player.set_workspace("music");
    assert!(!player.bar().is_visible(), "bottom dock hidden in music view with top layout");
    assert!(player.state.hero_deck.is_visible(), "hero deck visible in music view with top layout");

    // Switching to bottom layout shows bottom dock and hides hero deck
    super::super::theme::ThemeManager::shared().set_music_player_layout("bottom");
    assert!(player.bar().is_visible(), "bottom dock visible with bottom layout");
    assert!(!player.state.hero_deck.is_visible(), "hero deck hidden with bottom layout");

    // Entering immersive mode hides bottom dock regardless of layout
    player.set_workspace("immersive");
    assert!(!player.bar().is_visible(), "bottom dock hidden in immersive mode");

    // Restore top layout
    super::super::theme::ThemeManager::shared().set_music_player_layout("top");
    player.shutdown();
}
