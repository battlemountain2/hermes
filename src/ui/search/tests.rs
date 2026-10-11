use super::*;

#[test]
fn visual_identities_distinguish_songs_images_video_and_applications() {
    let item = |name| SearchItem::from_path(PathBuf::from(name), Path::new("/tmp"), false, None);
    assert_eq!(
        results::identity(&item("/tmp/song.MP3")),
        (crate::assets::icons::MUSIC, "Song")
    );
    assert_eq!(
        results::identity(&item("/tmp/cover.jpg")),
        (crate::assets::icons::PICTURES, "Image")
    );
    assert_eq!(
        results::identity(&item("/tmp/movie.MOV")),
        (crate::assets::icons::VIDEOS, "Video")
    );
    assert_eq!(
        results::identity(&item("/tmp/game.exe")),
        (crate::assets::icons::MONITOR, "Application")
    );
    assert_eq!(
        results::identity(&item("/tmp/game.desktop")),
        (crate::assets::icons::EXTERNAL_LINK, "Launcher")
    );
    assert_eq!(
        results::identity(&item("/tmp/backup.zip")),
        (crate::assets::icons::FILE_ARCHIVE, "Archive")
    );
    assert_eq!(results::identity(&item("/tmp/paper.pdf")).1, "PDF");
}

#[test]
fn artist_and_album_detail_is_separate_from_paths_and_uses_album_artist_fallback() {
    let mut item = SearchItem::from_path(
        PathBuf::from("/Music/album/song.flac"),
        Path::new("/Music"),
        false,
        None,
    );
    item.music_tags = Some(crate::model::MusicTags {
        artist: Some("Cameron Winter".into()),
        album_artist: Some("Fallback".into()),
        album: Some("Heavy Metal".into()),
        ..Default::default()
    });
    assert_eq!(results::music_detail(&item), "Cameron Winter · Heavy Metal");
    item.music_tags.as_mut().unwrap().artist = None;
    assert_eq!(results::music_detail(&item), "Fallback · Heavy Metal");
    assert_eq!(item.file_entry().music_tags, item.music_tags);
}

#[test]
#[ignore = "Requires GTK desktop; verifies lazy launcher icons without running applications"]
fn desktop_search_artwork_loads_only_visible_rows_and_never_launches_games() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let folder = std::env::temp_dir().join(format!("hermes-search-visuals-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let sentinel = folder.join("must-not-exist");
    let first = folder.join("first.desktop");
    let last = folder.join("last.desktop");
    let launcher = format!(
        "[Desktop Entry]\nType=Application\nName=Fixture game\nExec=/bin/touch {}\nIcon=folder\n",
        sentinel.display()
    );
    std::fs::write(&first, &launcher).unwrap();
    std::fs::write(&last, &launcher).unwrap();
    let dialog = SearchDialog::new(Rc::new(|_| {}), Rc::new(|| {}));
    dialog.state.root.replace(folder.clone());
    let mut items = vec![SearchItem::from_path(first, &folder, false, None)];
    for index in 0..35 {
        items.push(SearchItem::from_path(
            folder.join(format!("text-{index}.txt")),
            &folder,
            false,
            None,
        ));
    }
    items.push(SearchItem::from_path(last, &folder, false, None));
    render_results(&dialog.state, items, false, None);
    let image = |index| {
        dialog
            .state
            .list
            .row_at_index(index)
            .unwrap()
            .child()
            .unwrap()
            .first_child()
            .unwrap()
            .downcast::<gtk::Image>()
            .unwrap()
    };
    let first_image = image(0);
    let last_image = image(36);
    assert!(crate::assets::has_primary_icon(&first_image));
    assert!(crate::assets::has_primary_icon(&last_image));
    dialog.state.layer.set_visible(true);
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(640)
        .child(&dialog.state.layer)
        .build();
    window.present();
    let pump = |predicate: &dyn Fn() -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !predicate() && std::time::Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(predicate());
    };
    pump(&|| !crate::assets::has_primary_icon(&first_image));
    assert!(
        crate::assets::has_primary_icon(&last_image),
        "Offscreen rows must not load artwork yet: last={:?}, scroll height={}, upper={}, page={}",
        dialog.state.list.row_at_index(36).unwrap().compute_bounds(
            &dialog
                .state
                .list
                .ancestor(gtk::ScrolledWindow::static_type())
                .unwrap()
        ),
        dialog
            .state
            .list
            .ancestor(gtk::ScrolledWindow::static_type())
            .unwrap()
            .height(),
        dialog
            .state
            .list
            .ancestor(gtk::ScrolledWindow::static_type())
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap()
            .vadjustment()
            .upper(),
        dialog
            .state
            .list
            .ancestor(gtk::ScrolledWindow::static_type())
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap()
            .vadjustment()
            .page_size()
    );
    let scroll = dialog
        .state
        .list
        .ancestor(gtk::ScrolledWindow::static_type())
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    let adjustment = scroll.vadjustment();
    adjustment.set_value(adjustment.upper() - adjustment.page_size());
    pump(&|| !crate::assets::has_primary_icon(&last_image));
    assert!(
        !sentinel.exists(),
        "Search must never execute launcher files"
    );
    dialog.hide();
    window.close();
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn initial_and_fallback_scope_is_the_global_music_library() {
    assert_eq!(SearchScope::default(), SearchScope::MusicLibrary);
    let music = glib::user_special_dir(glib::UserDirectory::Music)
        .unwrap_or_else(|| glib::home_dir().join("Music"));
    assert_eq!(selected_scope(2).root(Path::new("/tmp/an-album")), music);
    assert_eq!(selected_scope(u32::MAX), SearchScope::MusicLibrary);
    assert_eq!(
        selected_scope(0).root(Path::new("/tmp/an-album")),
        Path::new("/tmp/an-album")
    );
    assert_eq!(selected_scope(1), SearchScope::Home);
}

#[test]
fn filter_quotes_preserve_punctuation() {
    let mut parts = Vec::new();
    push_filter(&mut parts, "content", " A \"quote\" \\ path ");
    let (_, filters) = search_filter_values(&parts.join(" "));
    assert_eq!(filters.content.as_deref(), Some("a \"quote\" \\ path"));
}

#[test]
fn selection_tracks_song_path_across_index_updates_and_reranking() {
    let first = SearchItem::from_path(
        PathBuf::from("/Music/One/first.flac"),
        Path::new("/Music"),
        false,
        None,
    );
    let second = SearchItem::from_path(
        PathBuf::from("/Music/Two/second.flac"),
        Path::new("/Music"),
        false,
        None,
    );
    let results = vec![second, first.clone()];
    assert_eq!(selection_position(&results, Some(&first.path)), Some(1));
    assert_eq!(
        selection_position(&results, Some(Path::new("/gone"))),
        Some(0)
    );
    assert_eq!(selection_position(&[], Some(&first.path)), None);
}

#[test]
#[ignore = "Requires a GTK desktop display; run explicitly"]
fn desktop_scope_selection_and_reentrant_rendering() {
    gtk::init().expect("Desktop display available");
    crate::assets::prepare().expect("Icons available");
    let dialog = SearchDialog::new(Rc::new(|_| {}), Rc::new(|| {}));
    assert_eq!(dialog.state.scope.selected(), 2);
    dialog
        .state
        .current_folder
        .replace(PathBuf::from("/tmp/current-album"));
    let request = ScopedSearch::from_uri(&dialog.search_uri()).expect("Dialog URI parses");
    assert_eq!(
        request.root,
        SearchScope::MusicLibrary.root(Path::new("/tmp/current-album"))
    );
    dialog.state.scope.set_selected(1);
    assert_eq!(
        ScopedSearch::from_uri(&dialog.search_uri()).unwrap().scope,
        SearchScope::Home
    );

    let reentrant = Rc::downgrade(&dialog.state);
    dialog.state.list.connect_row_selected(move |_, _| {
        if let Some(state) = reentrant.upgrade() {
            let _root = state.root.borrow_mut();
            let _results = state.visible_results.borrow_mut();
            let _selection = state.selected_path.borrow_mut();
        }
    });
    let first = SearchItem::from_path(
        PathBuf::from("/Music/First/one.flac"),
        Path::new("/Music"),
        false,
        None,
    );
    let second = SearchItem::from_path(
        PathBuf::from("/Music/Second/two.flac"),
        Path::new("/Music"),
        false,
        None,
    );
    render_results(
        &dialog.state,
        vec![first.clone(), second.clone()],
        true,
        Some("Index limit"),
    );
    dialog
        .state
        .list
        .select_row(dialog.state.list.row_at_index(1).as_ref());
    render_results(&dialog.state, vec![second.clone(), first], true, None);
    let row = dialog.state.list.row_at_index(0).unwrap();
    let same = dialog.state.visible_results.borrow().clone();
    render_results(&dialog.state, same, false, None);
    assert_eq!(
        dialog.state.list.row_at_index(0).unwrap(),
        row,
        "Unchanged results must not rebuild rows"
    );
    assert_eq!(dialog.state.list.selected_row().unwrap().index(), 0);
    assert_eq!(
        dialog.state.selected_path.borrow().as_ref(),
        Some(&second.path)
    );
    render_results(&dialog.state, Vec::new(), true, Some("Index limit"));
    assert_eq!(
        dialog.state.selected_path.borrow().as_ref(),
        Some(&second.path)
    );
    render_results(&dialog.state, vec![second.clone()], false, None);
    assert_eq!(
        dialog.state.selected_path.borrow().as_ref(),
        Some(&second.path)
    );
    render_results(&dialog.state, Vec::new(), true, Some("Index limit"));
    assert!(dialog.state.notice.property::<bool>("visible"));
    assert!(dialog.state.notice.text().contains("Index limit"));
    dialog.hide();
    assert_eq!(dialog.state.scope.selected(), 1);

    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let reentrant = Rc::downgrade(&dialog.state);
    dialog.set_open_results(Rc::new(move |uri| {
        if let Some(state) = reentrant.upgrade() {
            state.open_results.borrow_mut().take();
        }
        capture.replace(Some(uri));
    }));
    dialog.state.open_results_button.emit_clicked();
    let uri = captured
        .borrow()
        .clone()
        .expect("Results action sends a scoped URI");
    assert_eq!(
        ScopedSearch::from_uri(&uri).unwrap().scope,
        SearchScope::Home
    );

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("hermes-search-refresh-{nonce}"));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("needle-old.txt"), "generated fixture").unwrap();
    dialog.state.scope.set_selected(0);
    dialog.show_query(root.clone(), "needle");
    let pump = |expected: &str| {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let context = glib::MainContext::default();
        loop {
            if dialog
                .state
                .visible_results
                .borrow()
                .iter()
                .any(|item| item.name == expected)
                && !dialog.state.notice.is_visible()
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Missing refreshed result {expected}"
            );
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    pump("needle-old.txt");
    std::fs::remove_file(root.join("needle-old.txt")).unwrap();
    std::fs::write(root.join("needle-new.txt"), "generated fixture").unwrap();
    let generation = dialog.state.generation.get();
    dialog.state.refresh.emit_clicked();
    assert_eq!(dialog.state.generation.get(), generation.wrapping_add(1));
    assert_eq!(dialog.state.field.text(), "needle");
    pump("needle-new.txt");
    assert!(
        !dialog
            .state
            .visible_results
            .borrow()
            .iter()
            .any(|item| item.name == "needle-old.txt")
    );
    dialog.hide();
    std::fs::remove_dir_all(root).unwrap();
}
