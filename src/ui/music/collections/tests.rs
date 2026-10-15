use super::*;

fn library() -> Vec<Track> {
    let mut first = super::super::tests::tagged_track(
        "/generated/Album/z.flac".into(),
        Some(1),
        Some(1),
        "First",
    );
    first.tags.artist = Some("Artist one".into());
    let mut second = super::super::tests::tagged_track(
        "/generated/Album/a.flac".into(),
        Some(1),
        Some(2),
        "Second",
    );
    second.tags.artist = Some("Artist one".into());
    let mut third = super::super::tests::tagged_track(
        "/generated/Other/b.mp3".into(),
        Some(1),
        Some(1),
        "Another",
    );
    third.tags.artist = Some("Artist two".into());
    vec![second, third, first]
}

fn data(commands: Vec<CollectionCommand>) -> Collections {
    let store = CollectionStore::start(None);
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !store.snapshot().ready {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    for command in commands {
        let revision = store.snapshot().revision;
        store.send(command).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while store.snapshot().revision <= revision || !store.snapshot().ready {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    store.snapshot().data.clone()
}

#[test]
fn pending_final_observation_retries_without_advancing_or_losing_first_seen_time() {
    let paths: Vec<_> = (0..1025)
        .map(|index| PathBuf::from(format!("/generated/{index}.flac")))
        .collect();
    let mut pending = PendingObservation::new(paths.clone(), 123);
    assert!(pending.try_submit(|_, _| Err("queue full".into())).is_err());
    assert_eq!(pending.next, 0);
    let mut accepted = Vec::new();
    assert!(
        !pending
            .try_submit(|batch, timestamp| {
                assert_eq!(timestamp, 123);
                accepted.extend(batch);
                Ok(())
            })
            .unwrap()
    );
    assert_eq!(pending.next, 512);
    assert!(
        pending
            .try_submit(|_, _| Err("queue full again".into()))
            .is_err()
    );
    assert_eq!(pending.next, 512);
    assert!(
        !pending
            .try_submit(|batch, timestamp| {
                assert_eq!(timestamp, 123);
                accepted.extend(batch);
                Ok(())
            })
            .unwrap()
    );
    assert!(
        pending
            .try_submit(|batch, timestamp| {
                assert_eq!(timestamp, 123);
                accepted.extend(batch);
                Ok(())
            })
            .unwrap()
    );
    assert_eq!(accepted, paths);
}

#[test]
fn pending_latest_snapshot_coalesces_and_each_submission_is_bounded() {
    let mut pending = PendingObservation::new(vec!["/generated/old.flac".into()], 100);
    assert!(pending.try_submit(|_, _| Err("queue full".into())).is_err());
    pending = pending.with_latest(vec![PathBuf::from("/generated/latest.flac"); 1024], 200);
    assert!(
        !pending
            .try_submit(|paths, timestamp| {
                assert_eq!(paths.len(), 512);
                assert_eq!(timestamp, 200);
                assert!(
                    paths
                        .iter()
                        .all(|path| path == std::path::Path::new("/generated/latest.flac"))
                );
                Ok(())
            })
            .unwrap()
    );
    assert_eq!(pending.next, 512);
}

#[test]
fn coalescing_preserves_earliest_unsubmitted_dates_and_dates_new_paths_independently() {
    let old = PathBuf::from("/generated/old.flac");
    let newest = PathBuf::from("/generated/new.flac");
    let mut pending = PendingObservation::new(vec![old.clone()], 100)
        .with_latest(vec![old.clone(), newest.clone()], 200);
    assert!(
        !pending
            .try_submit(|paths, timestamp| {
                assert_eq!(paths, vec![old]);
                assert_eq!(timestamp, 100);
                Ok(())
            })
            .unwrap()
    );
    assert!(
        pending
            .try_submit(|paths, timestamp| {
                assert_eq!(paths, vec![newest]);
                assert_eq!(timestamp, 200);
                Ok(())
            })
            .unwrap()
    );
}

#[test]
fn dedicated_sections_filter_without_changing_original_album_order() {
    let library = library();
    let collections = data(vec![CollectionCommand::Favorite(
        vec![library[2].path.clone()],
        true,
    )]);
    assert_eq!(
        visible_tracks(
            &library,
            Section::Albums,
            Some(std::path::Path::new("/generated/Album")),
            None,
            &collections,
            ""
        ),
        vec![library[2].clone(), library[0].clone()]
    );
    assert_eq!(
        visible_tracks(
            &library,
            Section::Artists,
            None,
            Some(&Group::Artist("Artist two".into())),
            &collections,
            ""
        ),
        vec![library[1].clone()]
    );
    assert_eq!(
        visible_tracks(&library, Section::Songs, None, None, &collections, ""),
        vec![library[1].clone(), library[2].clone(), library[0].clone()]
    );
    assert_eq!(
        visible_tracks(&library, Section::Favorites, None, None, &collections, ""),
        vec![library[2].clone()]
    );
    assert!(
        visible_tracks(
            &library,
            Section::Favorites,
            None,
            None,
            &collections,
            "Another"
        )
        .is_empty()
    );
    assert_eq!(
        visible_tracks(
            &library,
            Section::Albums,
            Some(std::path::Path::new("/generated/Album")),
            None,
            &collections,
            "Another"
        ),
        vec![library[1].clone()]
    );
}

#[test]
fn playlist_order_and_unavailable_placeholders_survive_search_and_catalog_removal() {
    let library = library();
    let missing = PathBuf::from("/generated/Missing/gone.flac");
    let collections = data(vec![
        CollectionCommand::Create("Mix".into()),
        CollectionCommand::Add(
            1,
            vec![
                library[0].path.clone(),
                missing.clone(),
                library[2].path.clone(),
            ],
        ),
    ]);
    let tracks = visible_tracks(
        &library,
        Section::Playlists,
        None,
        Some(&Group::Playlist(1)),
        &collections,
        "",
    );
    assert_eq!(
        tracks.iter().map(|track| &track.path).collect::<Vec<_>>(),
        vec![&library[0].path, &missing, &library[2].path]
    );
    assert!(tracks[1].title().starts_with("Unavailable:"));
    assert_eq!(
        visible_tracks(
            &library,
            Section::Playlists,
            None,
            Some(&Group::Playlist(1)),
            &collections,
            "First"
        ),
        vec![library[2].clone()]
    );
    assert_eq!(
        visible_tracks(
            &[],
            Section::Playlists,
            None,
            Some(&Group::Playlist(1)),
            &collections,
            ""
        )
        .len(),
        3
    );
    assert_eq!(collections.playlist(1).unwrap().tracks.len(), 3);
}

#[test]
fn smart_collections_rank_dates_and_positive_counts_with_stable_path_ties() {
    let library = library();
    let collections = data(vec![
        CollectionCommand::Observe(vec![library[0].path.clone()], 100),
        CollectionCommand::Observe(vec![library[2].path.clone(), library[1].path.clone()], 200),
        CollectionCommand::Played(library[2].path.clone()),
        CollectionCommand::Played(library[2].path.clone()),
        CollectionCommand::Played(library[0].path.clone()),
    ]);
    assert_eq!(
        visible_tracks(
            &library,
            Section::Playlists,
            None,
            Some(&Group::RecentlyAdded),
            &collections,
            ""
        ),
        vec![library[2].clone(), library[1].clone(), library[0].clone()]
    );
    assert_eq!(
        visible_tracks(
            &library,
            Section::Playlists,
            None,
            Some(&Group::MostPlayed),
            &collections,
            ""
        ),
        vec![library[2].clone(), library[0].clone()]
    );
}

#[test]
#[ignore = "Requires isolated GTK display; generated catalog only, no personal library or collection data"]
fn desktop_navigation_favorites_separate_editor_drag_and_missing_memberships() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let _theme = super::super::super::theme::ThemeManager::shared();
    let player = MusicPlayer::with_session(None);
    let state = &player.state;
    state.library.replace(library());
    state.render_library();
    let window = gtk::Window::builder()
        .default_width(1100)
        .default_height(650)
        .child(&player.view())
        .build();
    window.present();
    let pump = |condition: &dyn Fn() -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            state.scan.borrow_mut().take();
            state.poll_collections();
            if condition() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "GTK collection fixture timed out"
            );
            for _ in 0..32 {
                if !glib::MainContext::default().iteration(false) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    for (section, control) in &state.navigation {
        control.set_active(true);
        assert_eq!(state.section.get(), *section);
        assert_eq!(state.album_scroll.is_visible(), *section == Section::Albums);
        assert_eq!(
            state.playlist_editor.is_visible(),
            *section == Section::Playlists
        );
        assert!(state.stream.borrow().is_none());
    }
    state.navigation[2].1.set_active(true);
    pump(&|| !state.favorite_buttons.borrow().is_empty());
    let heart = state.favorite_buttons.borrow()[0].0.upgrade().unwrap();
    let heart_path = state.favorite_buttons.borrow()[0].1.clone();
    assert!(!heart.is_active());
    assert!(
        heart
            .child()
            .and_downcast::<gtk::Image>()
            .unwrap()
            .paintable()
            .is_some()
    );
    assert!(
        gtk::gio::resources_lookup_data(
            "/io/github/lgse/Strata/icons/scalable/actions/strata-heart.svg",
            gtk::gio::ResourceLookupFlags::NONE
        )
        .is_ok()
    );
    heart.emit_clicked();
    pump(&|| state.favorite_paths.borrow().contains(&heart_path));
    assert!(heart.is_active());
    state.navigation[3].1.set_active(true);
    assert_eq!(state.tracks.borrow().len(), 1);
    assert!(state.stream.borrow().is_none());
    state.navigation[4].1.set_active(true);
    let tracks = library();
    state.collection_command(CollectionCommand::Create("Fixture mix".into()));
    pump(&|| {
        state
            .collection_snapshot
            .borrow()
            .data
            .playlist(1)
            .is_some()
    });
    state.collection_command(CollectionCommand::Add(
        1,
        vec![
            tracks[0].path.clone(),
            tracks[1].path.clone(),
            tracks[2].path.clone(),
            "/generated/Missing/gone.flac".into(),
        ],
    ));
    pump(&|| {
        state
            .collection_snapshot
            .borrow()
            .data
            .playlist(1)
            .is_some_and(|playlist| playlist.tracks.len() == 4)
    });
    let position = state
        .groups
        .borrow()
        .iter()
        .position(|group| *group == Group::Playlist(1))
        .unwrap();
    state.group_selection.set_selected(position as u32);
    assert_eq!(state.tracks.borrow().len(), 4);
    let playback = state.track_playback(&tracks[1].path);
    assert_eq!(playback, vec![tracks[1].clone(), tracks[2].clone()]);
    let dialog = editor::open(state, Some(1));
    let descendants = super::super::tests::descendants;
    let rows = || {
        descendants(dialog.upcast_ref())
            .into_iter()
            .filter(|widget| widget.has_css_class("music-playlist-row"))
            .collect::<Vec<_>>()
    };
    pump(&|| rows().len() == 4);
    let rows = rows();
    let source = rows[0]
        .first_child()
        .unwrap()
        .observe_controllers()
        .item(0)
        .and_downcast::<gtk::DragSource>()
        .unwrap();
    let provider = source
        .emit_by_name::<Option<gtk::gdk::ContentProvider>>("prepare", &[&0.0_f64, &0.0_f64])
        .unwrap();
    let value = provider.value(String::static_type()).unwrap();
    let changed = Rc::new(Cell::new(0));
    let changed_groups = changed.clone();
    state
        .group_model
        .connect_items_changed(move |_, _, _, _| changed_groups.set(changed_groups.get() + 1));
    let editor_models: Vec<_> = descendants(dialog.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::ListView>().ok())
        .filter_map(|view| view.model().and_downcast::<gtk::SingleSelection>())
        .filter_map(|selection| selection.model().and_downcast::<gtk::StringList>())
        .collect();
    assert_eq!(editor_models.len(), 2);
    for model in &editor_models {
        let changed = changed.clone();
        model.connect_items_changed(move |_, _, _, _| changed.set(changed.get() + 1));
    }
    state.collection_command(CollectionCommand::Favorite(
        vec![tracks[0].path.clone()],
        true,
    ));
    pump(&|| state.favorite_paths.borrow().contains(&tracks[0].path));
    state.render_groups();
    state.render_library();
    let deadline = std::time::Instant::now() + Duration::from_millis(300);
    while std::time::Instant::now() < deadline {
        state.poll_collections();
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(3));
    }
    assert_eq!(
        changed.get(),
        0,
        "unrelated favorite/save/unchanged catalog must not splice editor or group models"
    );
    let target = rows[2]
        .observe_controllers()
        .item(0)
        .and_downcast::<gtk::DropTarget>()
        .unwrap();
    assert!(target.emit_by_name::<bool>(
        "drop",
        &[
            &glib::BoxedValue(value.clone()),
            &0.0_f64,
            &f64::from(rows[2].height())
        ]
    ));
    pump(&|| {
        state
            .collection_snapshot
            .borrow()
            .data
            .playlist(1)
            .unwrap()
            .tracks[2]
            .native_path()
            == Some(tracks[0].path.as_path())
    });
    for _ in 0..50 {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(3));
    }
    assert!(!target.emit_by_name::<bool>("drop", &[&glib::BoxedValue(value), &0.0_f64, &0.0_f64]));
    assert!(state.stream.borrow().is_none());
    assert!(state.queue.borrow().tracks.is_empty());
    let entry = descendants(dialog.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Entry>().ok())
        .find(|entry| entry.placeholder_text().as_deref() == Some("Playlist name"))
        .unwrap();
    entry.set_text("Renamed fixture");
    let control = |label: &str| {
        descendants(dialog.upcast_ref())
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
            .find(|button| button.label().as_deref() == Some(label))
            .unwrap()
    };
    control("Rename").emit_clicked();
    pump(&|| {
        state
            .collection_snapshot
            .borrow()
            .data
            .playlist(1)
            .unwrap()
            .name
            == "Renamed fixture"
    });
    let remove = descendants(dialog.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| {
            button
                .tooltip_text()
                .is_some_and(|tooltip| tooltip.starts_with("Remove membership"))
        })
        .unwrap();
    remove.emit_clicked();
    pump(&|| {
        state
            .collection_snapshot
            .borrow()
            .data
            .playlist(1)
            .unwrap()
            .tracks
            .len()
            == 3
    });
    let add = descendants(dialog.upcast_ref())
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| {
            button
                .tooltip_text()
                .is_some_and(|tooltip| tooltip.starts_with("Add selected library song"))
        })
        .unwrap();
    add.emit_clicked();
    pump(&|| {
        state
            .collection_snapshot
            .borrow()
            .data
            .playlist(1)
            .unwrap()
            .tracks
            .len()
            == 4
    });
    dialog.close();
    window.close();
    player.dispose_view();
}
