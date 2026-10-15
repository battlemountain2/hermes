use super::*;

#[test]
#[ignore = "Requires isolated GTK display; validates stable geometry and row reuse during streamed search"]
fn desktop_search_geometry_and_rows_stay_stable_during_updates() {
    fn settle() {
        let deadline = std::time::Instant::now() + Duration::from_millis(100);
        while std::time::Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let _theme = crate::ui::theme::ThemeManager::shared();
    let styles = gtk::CssProvider::new();
    styles.load_from_string(include_str!("../../style.css"));
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let dialog = SearchDialog::new(Rc::new(|_| {}), Rc::new(|| {}));
    dialog.state.root.replace(PathBuf::from("/synthetic"));
    dialog.state.field.set_text("needle");
    dialog.state.layer.set_visible(true);
    let window = gtk::Window::builder()
        .default_width(1000)
        .default_height(900)
        .child(&dialog.widget())
        .build();
    window.present();
    settle();
    let panel = dialog
        .state
        .field
        .ancestor(gtk::Box::static_type())
        .unwrap()
        .parent()
        .unwrap();
    let bounds = || panel.compute_bounds(&window).unwrap();
    let initial = bounds();
    dialog
        .state
        .scope_label
        .set_text(&"/a long folder name".repeat(30));
    settle();
    assert_eq!(bounds(), initial);
    let item = |number| {
        SearchItem::from_path(
            PathBuf::from(format!("/synthetic/needle-{number}.flac")),
            Path::new("/synthetic"),
            false,
            None,
        )
    };
    let mut items: Vec<_> = (0..3).map(item).collect();
    render_results(&dialog.state, items.clone(), true, Some("Partial index"));
    settle();
    assert_eq!(bounds(), initial);
    let retained_row = dialog.state.list.row_at_index(0).unwrap();
    let audio_height = dialog.state.list.row_at_index(1).unwrap().height();
    items[1].music_tags = Some(crate::model::MusicTags {
        title: Some("A long song title".repeat(12)),
        artist: Some("An artist".repeat(12)),
        album: Some("An album".repeat(12)),
        ..Default::default()
    });
    render_results(
        &dialog.state,
        items.clone(),
        true,
        Some(&"Long indexing notice ".repeat(100)),
    );
    settle();
    assert_eq!(bounds(), initial);
    assert_eq!(dialog.state.list.row_at_index(0).unwrap(), retained_row);
    assert_eq!(
        dialog.state.list.row_at_index(1).unwrap().height(),
        audio_height
    );
    let changed_row = dialog.state.list.row_at_index(1).unwrap();
    items.swap(0, 1);
    items.extend((3..100).map(item));
    render_results(&dialog.state, items, false, None);
    settle();
    assert_eq!(bounds(), initial);
    assert_eq!(dialog.state.list.row_at_index(0).unwrap(), changed_row);
    assert_eq!(dialog.state.list.row_at_index(1).unwrap(), retained_row);
    begin_query(&dialog.state, "another query");
    settle();
    assert_eq!(bounds(), initial);
    render_results(&dialog.state, Vec::new(), false, None);
    settle();
    assert_eq!(bounds(), initial);
    dialog.hide();
    window.close();
}

#[test]
#[ignore = "Requires isolated GTK display; verifies cumulative pages, stable geometry and stale revisions"]
fn desktop_search_pages_preserve_rows_selection_and_footer() {
    fn settle() {
        let deadline = std::time::Instant::now() + Duration::from_millis(100);
        while std::time::Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let _theme = crate::ui::theme::ThemeManager::shared();
    let styles = gtk::CssProvider::new();
    styles.load_from_string(include_str!("../../style.css"));
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let dialog = SearchDialog::new(Rc::new(|_| {}), Rc::new(|| {}));
    dialog.set_open_results(Rc::new(|_| {}));
    dialog.state.root.replace(PathBuf::from("/synthetic"));
    dialog.state.field.set_text("needle");
    dialog.state.layer.set_visible(true);
    dialog.state.session_generation.set(7);
    let (handle, _receiver) = index_tree(PathBuf::new(), Arc::new(LocalTextExtractionProvider));
    let generation = handle.query("needle");
    dialog.state.expected_query_generation.set(Some(generation));
    dialog.state.search.replace(Some(handle));
    let window = gtk::Window::builder()
        .default_width(1000)
        .default_height(900)
        .child(&dialog.widget())
        .build();
    window.present();
    settle();
    let footer = dialog.state.more_button.parent().unwrap();
    let panel = footer.parent().unwrap();
    let bounds = || {
        (
            panel.compute_bounds(&window).unwrap(),
            footer.compute_bounds(&window).unwrap(),
            dialog.state.more_button.compute_bounds(&window).unwrap(),
        )
    };
    let initial = bounds();
    assert!(dialog.state.more_button.is_visible());
    assert!(!dialog.state.more_button.is_sensitive());
    let items: Vec<_> = (0..205)
        .map(|number| {
            SearchItem::from_path(
                PathBuf::from(format!("/synthetic/needle-{number:03}.txt")),
                Path::new("/synthetic"),
                false,
                None,
            )
        })
        .collect();
    let event = |revision, shown, indexing, notice: Option<&str>| SearchEvent::Results {
        generation,
        query: "needle".into(),
        items: items[..shown].to_vec(),
        indexing,
        notice: notice.map(str::to_owned),
        groups: None,
        page: Some(SearchPage {
            generation,
            revision,
            shown,
            total: items.len(),
        }),
    };
    apply_search_event(
        &dialog.state,
        7,
        event(3, 100, true, Some("Partial coverage")),
    );
    settle();
    assert_eq!(bounds(), initial);
    assert!(dialog.state.notice.text().starts_with("100 of 205 matches"));
    assert!(dialog.state.notice.text().contains("Partial coverage"));
    assert!(dialog.state.more_button.is_sensitive());
    assert!(dialog.state.list.row_at_index(100).is_none());
    let retained: Vec<_> = (0..100)
        .map(|position| dialog.state.list.row_at_index(position).unwrap())
        .collect();
    dialog.state.list.select_row(Some(&retained[42]));
    dialog.state.more_button.emit_clicked();
    assert!(!dialog.state.more_button.is_sensitive());
    dialog.state.more_button.emit_clicked();
    apply_search_event(&dialog.state, 7, event(2, 205, false, Some("Stale notice")));
    assert!(!dialog.state.more_button.is_sensitive());
    assert_eq!(dialog.state.visible_results.borrow().len(), 100);
    assert!(!dialog.state.notice.text().contains("Stale notice"));
    let mut mismatched = event(4, 205, false, None);
    let SearchEvent::Results { page, .. } = &mut mismatched;
    page.as_mut().unwrap().generation += 1;
    apply_search_event(&dialog.state, 7, mismatched);
    assert_eq!(dialog.state.visible_results.borrow().len(), 100);
    apply_search_event(&dialog.state, 7, event(3, 200, false, None));
    settle();
    assert_eq!(bounds(), initial);
    assert_eq!(dialog.state.notice.text(), "200 of 205 matches");
    assert!(dialog.state.more_button.is_sensitive());
    for (position, row) in retained.iter().enumerate() {
        assert_eq!(
            dialog.state.list.row_at_index(position as i32).as_ref(),
            Some(row)
        );
    }
    assert_eq!(
        dialog.state.list.selected_row().as_ref(),
        Some(&retained[42])
    );
    assert_eq!(
        dialog.state.selected_key.borrow().as_ref(),
        Some(&SearchResultKey::File(items[42].path.clone()))
    );
    dialog.state.more_button.emit_clicked();
    assert!(!dialog.state.more_button.is_sensitive());
    apply_search_event(
        &dialog.state,
        7,
        event(3, 100, false, Some("Stale cumulative page")),
    );
    assert_eq!(dialog.state.visible_results.borrow().len(), 200);
    assert!(!dialog.state.more_button.is_sensitive());
    assert_eq!(dialog.state.notice.text(), "200 of 205 matches");
    apply_search_event(&dialog.state, 7, event(4, 100, true, None));
    assert!(dialog.state.more_button.is_sensitive());
    apply_search_event(&dialog.state, 7, event(3, 205, false, None));
    assert_eq!(dialog.state.visible_results.borrow().len(), 100);
    assert_eq!(dialog.state.page.get().unwrap().revision, 4);
    assert_eq!(
        dialog.state.list.selected_row().as_ref(),
        Some(&retained[42])
    );
    apply_search_event(&dialog.state, 7, event(4, 205, false, None));
    settle();
    assert_eq!(bounds(), initial);
    assert_eq!(
        *dialog.state.visible_results.borrow(),
        items
            .iter()
            .cloned()
            .map(SearchResult::File)
            .collect::<Vec<_>>()
    );
    assert_eq!(dialog.state.notice.text(), "205 of 205 matches");
    assert!(dialog.state.more_button.is_visible());
    assert!(!dialog.state.more_button.is_sensitive());
    assert_eq!(
        dialog.state.list.selected_row().as_ref(),
        Some(&retained[42])
    );
    begin_query(&dialog.state, "needle");
    assert_eq!(dialog.state.page.get(), None);
    assert!(!dialog.state.more_button.is_sensitive());
    assert!(!dialog.state.notice.is_visible());
    apply_search_event(&dialog.state, 7, event(5, 205, false, None));
    assert!(dialog.state.visible_results.borrow().is_empty());
    dialog.state.expected_query_generation.set(Some(generation));
    apply_search_event(&dialog.state, 7, event(5, 100, false, None));
    assert!(dialog.state.more_button.is_sensitive());
    dialog.state.layer.set_visible(false);
    dialog.state.current_folder.replace(PathBuf::new());
    dialog.state.scope.set_selected(0);
    dialog.state.layer.set_visible(true);
    restart_search(&dialog.state);
    assert_eq!(dialog.state.page.get(), None);
    assert!(!dialog.state.more_button.is_sensitive());
    assert!(!dialog.state.notice.is_visible());
    dialog.state.expected_query_generation.set(Some(generation));
    apply_search_event(
        &dialog.state,
        dialog.state.session_generation.get(),
        event(5, 100, false, None),
    );
    assert!(dialog.state.more_button.is_sensitive());
    dialog.hide();
    assert_eq!(dialog.state.page.get(), None);
    assert!(!dialog.state.more_button.is_sensitive());
    window.close();
}

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
fn initial_scope_is_home_and_music_remains_explicit() {
    assert_eq!(SearchScope::default(), SearchScope::Home);
    let music = glib::user_special_dir(glib::UserDirectory::Music)
        .unwrap_or_else(|| glib::home_dir().join("Music"));
    assert_eq!(selected_scope(2).root(Path::new("/tmp/an-album")), music);
    assert_eq!(selected_scope(u32::MAX), SearchScope::Home);
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
fn music_filter_quotes_preserve_phrases_and_punctuation() {
    let mut parts = Vec::new();
    push_filter(&mut parts, "artist", " Sinéad O'Connor ");
    push_filter(&mut parts, "album", "A \"quote\" \\ path");
    push_filter(&mut parts, "title", "Nothing Compares 2 U");
    let (terms, filters) = search_filter_values(&parts.join(" "));
    assert!(terms.is_empty());
    assert_eq!(filters.artist.as_deref(), Some("sinéad o'connor"));
    assert_eq!(filters.album.as_deref(), Some("a \"quote\" \\ path"));
    assert_eq!(filters.title.as_deref(), Some("nothing compares 2 u"));
    assert!(filters.content.is_none());
}

#[test]
fn query_generation_rejects_previous_requests_when_query_text_repeats() {
    let session = 7;
    let first_a = 1;
    let query_b = 2;
    let second_a = 3;
    assert!(!is_current_search_response(
        session,
        session,
        Some(second_a),
        first_a
    ));
    assert!(!is_current_search_response(
        session,
        session,
        Some(second_a),
        query_b
    ));
    assert!(is_current_search_response(
        session,
        session,
        Some(second_a),
        second_a
    ));
    assert!(!is_current_search_response(
        session, session, None, second_a
    ));
}

#[test]
fn session_generation_rejects_previous_sessions_even_if_query_ids_repeat() {
    assert!(!is_current_search_response(8, 7, Some(1), 1));
    assert!(is_current_search_response(8, 8, Some(1), 1));
}

#[test]
#[ignore = "Requires isolated GTK display; verifies friendly music filters and query round-trips"]
fn desktop_music_filters_survive_unrelated_changes_and_round_trip() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let dialog = SearchDialog::new(Rc::new(|_| {}), Rc::new(|| {}));
    let query = r#"needle artist:"Sinéad O'Connor" album:"A \"quote\" \\ path" title:"Nothing Compares 2 U" in:Music modified:today"#;
    dialog.state.field.set_text(query);
    let (_, original) = search_filter_values(query);
    let panel = &dialog.state.filters;
    assert_eq!(panel.artist.text(), original.artist.as_deref().unwrap());
    assert_eq!(panel.album.text(), original.album.as_deref().unwrap());
    assert_eq!(panel.title.text(), original.title.as_deref().unwrap());
    assert!(!panel.content_enabled.is_active());
    assert!(!panel.content.is_sensitive());
    panel.extension.set_text("flac");
    panel.file_type.set_selected(3);
    panel.apply.emit_clicked();
    dialog.state.categories[2].emit_clicked();
    let (terms, values) = search_filter_values(&dialog.state.field.text());
    assert_eq!(terms, "needle");
    assert_eq!(
        values,
        SearchFilterValues {
            category: Some("audio".into()),
            extension: Some("flac".into()),
            ..original.clone()
        }
    );
    let request = ScopedSearch::from_uri(&dialog.search_uri()).unwrap();
    assert_eq!(search_filter_values(&request.query), (terms, values));
    panel.artist.set_text("Björk");
    panel.album.set_text("Debut");
    panel.title.set_text("Human Behaviour");
    panel.apply.emit_clicked();
    let (_, values) = search_filter_values(&dialog.state.field.text());
    assert_eq!(values.artist.as_deref(), Some("björk"));
    assert_eq!(values.album.as_deref(), Some("debut"));
    assert_eq!(values.title.as_deref(), Some("human behaviour"));
    assert_eq!(values.extension.as_deref(), Some("flac"));
    assert!(values.content.is_none());
    panel.content.set_text("Liner notes");
    panel.content_enabled.set_active(true);
    panel.apply.emit_clicked();
    dialog.state.categories[0].emit_clicked();
    let (_, values) = search_filter_values(&dialog.state.field.text());
    assert_eq!(values.category.as_deref(), Some("audio"));
    assert_eq!(values.content.as_deref(), Some("liner notes"));
    assert_eq!(values.artist.as_deref(), Some("björk"));
    assert_eq!(values.album.as_deref(), Some("debut"));
    assert_eq!(values.title.as_deref(), Some("human behaviour"));
    panel.content_enabled.set_active(false);
    panel.apply.emit_clicked();
    let (_, values) = search_filter_values(&dialog.state.field.text());
    assert!(values.content.is_none());
    assert_eq!(values.artist.as_deref(), Some("björk"));
    panel.clear.emit_clicked();
    assert_eq!(dialog.state.field.text(), "needle");
    assert!(panel.artist.text().is_empty());
    assert!(panel.album.text().is_empty());
    assert!(panel.title.text().is_empty());
    dialog.hide();
}

#[test]
#[ignore = "Requires isolated GTK display; verifies stale results and notices cannot repaint the dialog"]
fn desktop_search_rejects_old_query_and_session_results() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let dialog = SearchDialog::new(Rc::new(|_| {}), Rc::new(|| {}));
    dialog.state.field.set_text("needle");
    dialog.state.layer.set_visible(true);
    dialog.state.session_generation.set(7);
    dialog.state.expected_query_generation.set(Some(3));
    let item = SearchItem::from_path(
        PathBuf::from("/tmp/needle.txt"),
        Path::new("/tmp"),
        false,
        None,
    );
    let event = |generation, indexing, notice: &str| SearchEvent::Results {
        query: "needle".into(),
        generation,
        items: vec![item.clone()],
        indexing,
        notice: Some(notice.into()),
        page: None,
        groups: None,
    };
    apply_search_event(&dialog.state, 7, event(1, false, "Old query"));
    apply_search_event(&dialog.state, 6, event(3, false, "Old session"));
    assert!(dialog.state.visible_results.borrow().is_empty());
    assert!(!dialog.state.notice.is_visible());
    apply_search_event(&dialog.state, 7, event(3, true, "Partial coverage"));
    assert_eq!(
        *dialog.state.visible_results.borrow(),
        vec![SearchResult::File(item.clone())]
    );
    assert!(dialog.state.notice.text().contains("Partial coverage"));
    apply_search_event(&dialog.state, 7, event(1, false, "Stale error"));
    assert!(dialog.state.notice.text().contains("Partial coverage"));
    apply_search_event(&dialog.state, 7, event(3, false, "Finished"));
    assert_eq!(dialog.state.notice.text(), "Finished");
    dialog.hide();
    assert_eq!(dialog.state.expected_query_generation.get(), None);
    apply_search_event(&dialog.state, 7, event(3, false, "Closed"));
    assert_eq!(dialog.state.notice.text(), "Finished");
    dialog.state.layer.set_visible(true);
    dialog.state.expected_query_generation.set(Some(3));
    apply_search_event(&dialog.state, 7, event(3, false, "Previous dialog"));
    assert_eq!(dialog.state.notice.text(), "Finished");
    dialog.hide();
}

#[test]
#[ignore = "Requires isolated GTK display; primary groups remain independent of type filters"]
fn desktop_search_groups_preserve_terms_and_file_filters() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let dialog = SearchDialog::new(Rc::new(|_| {}), Rc::new(|| {}));
    assert_eq!(dialog.state.scope.selected(), 1);
    assert_eq!(dialog.state.categories.len(), 5);
    dialog
        .state
        .field
        .set_text("cameron type:audio in:Music modified:today");
    dialog.state.categories[2].emit_clicked();
    let (terms, filters) = search_filter_values(&dialog.state.field.text());
    assert_eq!(terms, "cameron");
    assert_eq!(filters.category.as_deref(), Some("audio"));
    assert_eq!(filters.within.as_deref(), Some("music"));
    assert_eq!(filters.modified.as_deref(), Some("today"));
    assert!(dialog.state.categories[2].is_active());
    dialog.state.categories[2].emit_clicked();
    assert!(dialog.state.categories[2].is_active());
    dialog.state.categories[0].emit_clicked();
    let (_, filters) = search_filter_values(&dialog.state.field.text());
    assert_eq!(filters.category.as_deref(), Some("audio"));
    assert_eq!(filters.within.as_deref(), Some("music"));
    assert_eq!(dialog.state.group.get(), SearchGroup::All);
    dialog.state.field.set_text("type:image ext:heic");
    assert!(dialog.state.categories[0].is_active());
    assert_eq!(dialog.state.filters.file_type.selected(), 5);
    dialog.state.categories[4].emit_clicked();
    assert_eq!(dialog.state.group.get(), SearchGroup::Artists);
    assert_eq!(dialog.state.field.text(), "type:image ext:heic");
    assert_eq!(dialog.state.filters.file_type.selected(), 5);
    dialog.hide();
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
    let results = vec![
        SearchResult::File(second),
        SearchResult::File(first.clone()),
    ];
    let first_key = SearchResultKey::File(first.path.clone());
    assert_eq!(selection_position(&results, Some(&first_key)), Some(1));
    assert_eq!(
        selection_position(
            &results,
            Some(&SearchResultKey::File(PathBuf::from("/gone")))
        ),
        Some(0)
    );
    assert_eq!(selection_position(&[], Some(&first_key)), None);
}

#[test]
fn entity_selection_never_aliases_its_representative_file() {
    let item = SearchItem::from_path(
        PathBuf::from("/synthetic/Album.flac"),
        Path::new("/synthetic"),
        false,
        None,
    );
    let album = SearchResult::Album(crate::services::SearchEntity {
        key: SearchEntityKey::Album {
            artist: "Artist".into(),
            album: item.name.clone(),
        },
        title: item.name.clone(),
        artist: Some("Artist".into()),
        songs: 2,
        representative: item.clone(),
    });
    let artist = SearchResult::Artist(crate::services::SearchEntity {
        key: SearchEntityKey::Artist(item.name.clone()),
        title: item.name.clone(),
        artist: None,
        songs: 2,
        representative: item.clone(),
    });
    let file = SearchResult::File(item);
    assert_ne!(file.key(), album.key());
    assert_ne!(artist.key(), album.key());
    let results = vec![artist.clone(), file.clone(), album.clone()];
    assert_eq!(selection_position(&results, Some(&file.key())), Some(1));
    assert_eq!(selection_position(&results, Some(&album.key())), Some(2));
    assert_eq!(selection_position(&results, Some(&artist.key())), Some(0));
    assert!(album.file().is_none());
    assert!(artist.file().is_none());
}

#[test]
#[ignore = "Requires isolated GTK display; verifies grouped 205-result pages, entity drills, headers and exports"]
fn desktop_grouped_search_pages_entities_and_back_preserve_context() {
    fn settle() {
        let deadline = std::time::Instant::now() + Duration::from_millis(100);
        while std::time::Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let _theme = crate::ui::theme::ThemeManager::shared();
    let styles = gtk::CssProvider::new();
    styles.load_from_string(include_str!("../../style.css"));
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let activated = Rc::new(RefCell::new(Vec::new()));
    let captured = activated.clone();
    let dismissed = Rc::new(Cell::new(0));
    let dismissals = dismissed.clone();
    let dialog = SearchDialog::new(
        Rc::new(move |item| captured.borrow_mut().push(item)),
        Rc::new(move || dismissals.set(dismissals.get() + 1)),
    );
    let exported = Rc::new(RefCell::new(Vec::new()));
    let exports = exported.clone();
    dialog.set_open_results(Rc::new(move |uri| exports.borrow_mut().push(uri)));
    let query = r#"needle type:audio ext:flac in:Music artist:"Artist" album:"Album" title:"Title" modified:today content:"liner notes""#;
    dialog.state.field.set_text(query);
    dialog
        .state
        .current_folder
        .replace(PathBuf::from("/synthetic"));
    dialog.state.root.replace(PathBuf::from("/synthetic"));
    dialog.state.scope.set_selected(0);
    dialog.state.layer.set_visible(true);
    dialog.state.session_generation.set(7);
    let (handle, _receiver) = index_tree(PathBuf::new(), Arc::new(LocalTextExtractionProvider));
    let generation = handle.query_group(query, SearchGroup::All, None);
    dialog.state.expected_query_generation.set(Some(generation));
    dialog.state.search.replace(Some(handle));
    let reentrant = Rc::downgrade(&dialog.state);
    dialog.state.list.connect_row_selected(move |_, _| {
        if let Some(state) = reentrant.upgrade() {
            let _items = state.visible_results.borrow_mut();
            let _selection = state.selected_key.borrow_mut();
            let _entity = state.entity_scope.borrow_mut();
            let _root = state.root.borrow_mut();
        }
    });
    let window = gtk::Window::builder()
        .default_width(1000)
        .default_height(900)
        .child(&dialog.widget())
        .build();
    window.present();
    settle();
    let footer = dialog.state.more_button.parent().unwrap();
    let panel = footer.parent().unwrap();
    let geometry = || {
        (
            panel.compute_bounds(&window).unwrap(),
            footer.compute_bounds(&window).unwrap(),
            dialog.state.more_button.compute_bounds(&window).unwrap(),
            dialog.state.back_button.compute_bounds(&window).unwrap(),
            dialog
                .state
                .notice
                .parent()
                .unwrap()
                .compute_bounds(&window)
                .unwrap(),
        )
    };
    let initial = geometry();
    assert!(dialog.state.back_button.is_visible());
    assert!(!dialog.state.back_button.is_sensitive());
    let physical = |number, extension| {
        SearchResult::File(SearchItem::from_path(
            PathBuf::from(format!("/synthetic/needle-{number:03}.{extension}")),
            Path::new("/synthetic"),
            false,
            None,
        ))
    };
    let files: Vec<_> = (0..100).map(|number| physical(number, "txt")).collect();
    let songs: Vec<_> = (0..100).map(|number| physical(number, "flac")).collect();
    let representative = songs[0].file().unwrap().clone();
    let albums: Vec<_> = (0..3)
        .map(|number| {
            SearchResult::Album(crate::services::SearchEntity {
                key: SearchEntityKey::Album {
                    artist: "artist".into(),
                    album: format!("album {number}"),
                },
                title: representative.name.clone(),
                artist: Some("Artist".into()),
                songs: 100,
                representative: representative.clone(),
            })
        })
        .collect();
    let artists: Vec<_> = (0..2)
        .map(|number| {
            SearchResult::Artist(crate::services::SearchEntity {
                key: SearchEntityKey::Artist(format!("artist {number}")),
                title: format!("Artist {number}"),
                artist: None,
                songs: 100,
                representative: representative.clone(),
            })
        })
        .collect();
    let counts = SearchCounts {
        files: 100,
        songs: 100,
        albums: 3,
        artists: 2,
    };
    let page_results = |shown| {
        let (file_count, song_count) = match shown {
            100 => (48, 47),
            200 => (98, 97),
            205 => (100, 100),
            _ => unreachable!(),
        };
        files[..file_count]
            .iter()
            .chain(&songs[..song_count])
            .chain(&albums)
            .chain(&artists)
            .cloned()
            .collect::<Vec<_>>()
    };
    let event = |generation, group, results: Vec<SearchResult>, counts: SearchCounts, shown| {
        SearchEvent::Results {
            generation,
            query: query.into(),
            items: Vec::new(),
            indexing: false,
            notice: Some("Partial coverage".into()),
            page: Some(SearchPage {
                generation,
                revision: 3,
                shown,
                total: counts.total(group),
            }),
            groups: Some(SearchGroups {
                group,
                results,
                counts,
            }),
        }
    };
    apply_search_event(
        &dialog.state,
        7,
        event(generation, SearchGroup::All, page_results(100), counts, 100),
    );
    settle();
    assert_eq!(geometry(), initial);
    assert!(dialog.state.notice.text().contains("100 of 205 matches"));
    assert!(
        dialog
            .state
            .notice
            .text()
            .contains("100 files · 100 songs · 3 albums · 2 artists")
    );
    assert!(dialog.state.notice.text().contains("Partial coverage"));
    assert!(dialog.state.more_button.is_sensitive());
    let headers = || {
        (0..dialog.state.visible_results.borrow().len())
            .filter_map(|position| {
                let row = dialog.state.list.row_at_index(position as i32).unwrap();
                row.header()
                    .and_downcast::<gtk::Label>()
                    .map(|header| (position, header.text().to_string()))
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        headers(),
        vec![
            (0, "Files".into()),
            (48, "Songs".into()),
            (95, "Albums".into()),
            (98, "Artists".into())
        ]
    );
    let retained: HashMap<_, _> = dialog
        .state
        .visible_results
        .borrow()
        .iter()
        .enumerate()
        .map(|(position, result)| {
            (
                result.key(),
                dialog.state.list.row_at_index(position as i32).unwrap(),
            )
        })
        .collect();
    let album_row = retained[&albums[0].key()].clone();
    assert!(
        album_row
            .tooltip_text()
            .unwrap()
            .contains("100 matching songs")
    );
    assert!(!album_row.tooltip_text().unwrap().contains("/synthetic"));
    dialog
        .state
        .list
        .select_row(dialog.state.list.row_at_index(47).as_ref());
    move_selection(&dialog.state, 1);
    assert_eq!(dialog.state.list.selected_row().unwrap().index(), 48);
    dialog
        .state
        .list
        .select_row(dialog.state.list.row_at_index(94).as_ref());
    move_selection(&dialog.state, 1);
    assert_eq!(dialog.state.list.selected_row().as_ref(), Some(&album_row));
    assert_eq!(
        dialog.state.selected_key.borrow().as_ref(),
        Some(&albums[0].key())
    );
    for shown in [200, 205] {
        dialog.state.more_button.emit_clicked();
        assert!(!dialog.state.more_button.is_sensitive());
        apply_search_event(
            &dialog.state,
            7,
            event(
                generation,
                SearchGroup::All,
                page_results(shown),
                counts,
                shown,
            ),
        );
        settle();
        assert_eq!(geometry(), initial);
        assert!(dialog.state.list.row_at_index(shown as i32).is_none());
        for (position, result) in dialog.state.visible_results.borrow().iter().enumerate() {
            if let Some(row) = retained.get(&result.key()) {
                assert_eq!(
                    dialog.state.list.row_at_index(position as i32).as_ref(),
                    Some(row)
                );
            }
        }
        assert_eq!(dialog.state.list.selected_row().as_ref(), Some(&album_row));
    }
    assert!(!dialog.state.more_button.is_sensitive());
    assert_eq!(
        headers(),
        vec![
            (0, "Files".into()),
            (100, "Songs".into()),
            (200, "Albums".into()),
            (203, "Artists".into())
        ]
    );
    activate_position(&dialog.state, album_row.index());
    let drilled_generation = dialog.state.expected_query_generation.get().unwrap();
    assert!(drilled_generation > generation);
    assert_eq!(dialog.state.group.get(), SearchGroup::Songs);
    assert_eq!(dialog.state.field.text(), query);
    assert_eq!(dialog.state.scope.selected(), 0);
    assert_eq!(*dialog.state.root.borrow(), PathBuf::from("/synthetic"));
    assert!(activated.borrow().is_empty());
    assert_eq!(dismissed.get(), 0);
    assert!(dialog.is_visible());
    assert!(dialog.state.back_button.is_sensitive());
    assert!(dialog.state.scope_label.text().contains("Album: album 0"));
    assert!(dialog.state.open_results_button.is_sensitive());
    let drilled = ScopedSearch::from_uri(&dialog.search_uri()).unwrap();
    assert_eq!(drilled.group, SearchGroup::Songs);
    assert_eq!(
        drilled.entity,
        Some(match albums[0].key() {
            SearchResultKey::Entity(key) => key,
            _ => unreachable!(),
        })
    );
    assert_eq!(drilled.scope, SearchScope::ThisFolder);
    assert_eq!(drilled.root, PathBuf::from("/synthetic"));
    assert_eq!(
        search_filter_values(&drilled.query),
        search_filter_values(query)
    );
    apply_search_event(
        &dialog.state,
        7,
        event(generation, SearchGroup::All, page_results(205), counts, 205),
    );
    assert!(dialog.state.visible_results.borrow().is_empty());
    apply_search_event(
        &dialog.state,
        7,
        event(
            drilled_generation,
            SearchGroup::Songs,
            songs.clone(),
            counts,
            100,
        ),
    );
    settle();
    assert_eq!(geometry(), initial);
    assert!(headers().is_empty());
    let songs_row = dialog.state.list.row_at_index(0).unwrap();
    apply_search_event(
        &dialog.state,
        7,
        event(
            drilled_generation,
            SearchGroup::Albums,
            albums.clone(),
            counts,
            3,
        ),
    );
    apply_search_event(
        &dialog.state,
        6,
        event(
            drilled_generation,
            SearchGroup::Songs,
            Vec::new(),
            counts,
            0,
        ),
    );
    assert_eq!(dialog.state.list.row_at_index(0).as_ref(), Some(&songs_row));
    dialog.state.back_button.emit_clicked();
    let back_generation = dialog.state.expected_query_generation.get().unwrap();
    assert!(back_generation > drilled_generation);
    assert_eq!(dialog.state.group.get(), SearchGroup::All);
    assert!(dialog.state.entity_scope.borrow().is_none());
    assert_eq!(dialog.state.field.text(), query);
    assert!(!dialog.state.back_button.is_sensitive());
    assert!(!dialog.state.scope_label.text().contains("Album:"));
    assert_eq!(
        ScopedSearch::from_uri(&dialog.search_uri()).unwrap().entity,
        None
    );
    apply_search_event(
        &dialog.state,
        7,
        event(
            drilled_generation,
            SearchGroup::Songs,
            songs.clone(),
            counts,
            100,
        ),
    );
    assert!(dialog.state.visible_results.borrow().is_empty());
    apply_search_event(
        &dialog.state,
        7,
        event(
            back_generation,
            SearchGroup::All,
            page_results(100),
            counts,
            100,
        ),
    );
    assert_eq!(
        dialog.state.selected_key.borrow().as_ref(),
        Some(&albums[0].key())
    );
    activate_position(&dialog.state, 95);
    dialog.state.field.set_text("edited needle type:audio");
    assert!(dialog.state.entity_scope.borrow().is_none());
    assert_eq!(dialog.state.group.get(), SearchGroup::All);
    assert!(!dialog.state.back_button.is_sensitive());
    for position in [3, 4] {
        dialog.state.categories[position].emit_clicked();
        assert!(dialog.state.open_results_button.is_visible());
        assert!(!dialog.state.open_results_button.is_sensitive());
        dialog.state.open_results_button.emit_clicked();
        assert!(exported.borrow().is_empty());
        assert!(dialog.is_visible());
    }
    dialog.state.categories[1].emit_clicked();
    assert!(dialog.state.open_results_button.is_sensitive());
    assert_eq!(
        ScopedSearch::from_uri(&dialog.search_uri()).unwrap().group,
        SearchGroup::Files
    );
    dialog.state.categories[0].emit_clicked();
    let final_generation = dialog.state.expected_query_generation.get().unwrap();
    apply_search_event(
        &dialog.state,
        7,
        event(
            final_generation,
            SearchGroup::All,
            page_results(100),
            counts,
            100,
        ),
    );
    settle();
    assert_eq!(geometry(), initial);
    activate_position(&dialog.state, 48);
    assert_eq!(*activated.borrow(), vec![representative]);
    assert_eq!(dismissed.get(), 1);
    assert!(!dialog.is_visible());
    window.close();
}

#[test]
#[ignore = "Requires isolated GTK display; verifies guarded physical exports and album/artist song exports"]
fn desktop_search_tab_exports_preserve_groups_and_entity_keys() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let activated = Rc::new(Cell::new(0));
    let activations = activated.clone();
    let dialog = SearchDialog::new(
        Rc::new(move |_| activations.set(activations.get() + 1)),
        Rc::new(|| {}),
    );
    let exported = Rc::new(RefCell::new(Vec::new()));
    let exports = exported.clone();
    dialog.set_open_results(Rc::new(move |uri| exports.borrow_mut().push(uri)));
    dialog.state.scope.set_selected(0);
    dialog
        .state
        .current_folder
        .replace(PathBuf::from("/synthetic"));
    dialog.state.root.replace(PathBuf::from("/synthetic"));
    let query = r#"needle type:audio artist:"sinéad o'connor" album:"album" title:"title" in:Music ext:flac modified:yesterday content:"liner notes""#;
    dialog.state.field.set_text(query);
    for position in [0, 1, 2] {
        dialog.state.layer.set_visible(true);
        dialog.state.categories[position].emit_clicked();
        assert!(dialog.state.open_results_button.is_sensitive());
        dialog.state.open_results_button.emit_clicked();
        let uri = exported.borrow().last().unwrap().clone();
        let request = ScopedSearch::from_uri(&uri).unwrap();
        assert_eq!(request.group, SEARCH_GROUPS[position]);
        assert_eq!(request.entity, None);
        assert_eq!(request.scope, SearchScope::ThisFolder);
        assert_eq!(request.root, PathBuf::from("/synthetic"));
        assert_eq!(
            search_filter_values(&request.query),
            search_filter_values(query)
        );
    }
    dialog.state.layer.set_visible(true);
    dialog.state.open_results_button.set_sensitive(false);
    dialog.state.open_results_button.emit_clicked();
    assert_eq!(exported.borrow().len(), 3);
    assert!(dialog.is_visible());
    let representative = SearchItem::from_path(
        PathBuf::from("/synthetic/needle.flac"),
        Path::new("/synthetic"),
        false,
        None,
    );
    for (position, key) in [
        (
            3,
            SearchEntityKey::Album {
                artist: "sinéad o'connor".into(),
                album: "a \"quote\" \\ album".into(),
            },
        ),
        (4, SearchEntityKey::Artist("sinéad o'connor".into())),
    ] {
        dialog.state.layer.set_visible(true);
        dialog.state.categories[position].emit_clicked();
        let before = exported.borrow().len();
        assert!(!dialog.state.open_results_button.is_sensitive());
        assert!(dialog.state.open_results_button.is_visible());
        assert!(
            dialog
                .state
                .open_results_button
                .tooltip_text()
                .unwrap()
                .contains("Open an album or artist first")
        );
        dialog.state.open_results_button.emit_clicked();
        assert_eq!(exported.borrow().len(), before);
        assert!(dialog.is_visible());
        let entity = crate::services::SearchEntity {
            key: key.clone(),
            title: "Display title".into(),
            artist: Some("Display artist".into()),
            songs: 12,
            representative: representative.clone(),
        };
        let result = if position == 3 {
            SearchResult::Album(entity)
        } else {
            SearchResult::Artist(entity)
        };
        render_search_results(&dialog.state, vec![result.clone()], false, None, None);
        let row = dialog.state.list.row_at_index(0).unwrap();
        let image = row
            .child()
            .unwrap()
            .first_child()
            .unwrap()
            .downcast::<gtk::Image>()
            .unwrap();
        assert!(crate::assets::has_primary_icon(&image));
        assert!(row.tooltip_text().unwrap().contains("12 matching songs"));
        assert!(!row.tooltip_text().unwrap().contains("/synthetic"));
        activate_position(&dialog.state, 0);
        assert!(dialog.state.open_results_button.is_sensitive());
        assert_eq!(dialog.state.group.get(), SearchGroup::Songs);
        assert_eq!(
            dialog.state.entity_scope.borrow().as_ref().unwrap().key,
            key
        );
        assert_eq!(activated.get(), 0);
        dialog.state.back_button.emit_clicked();
        assert_eq!(dialog.state.group.get(), SEARCH_GROUPS[position]);
        assert!(dialog.state.entity_scope.borrow().is_none());
        assert!(!dialog.state.open_results_button.is_sensitive());
        render_search_results(&dialog.state, vec![result], false, None, None);
        activate_position(&dialog.state, 0);
        dialog.state.open_results_button.emit_clicked();
        assert_eq!(exported.borrow().len(), before + 1);
        let uri = exported.borrow().last().unwrap().clone();
        let request = ScopedSearch::from_uri(&uri).unwrap();
        assert_eq!(request.group, SearchGroup::Songs);
        assert_eq!(request.entity, Some(key));
        assert_eq!(request.root, PathBuf::from("/synthetic"));
        assert_eq!(
            search_filter_values(&request.query),
            search_filter_values(query)
        );
        assert_eq!(activated.get(), 0);
    }
    dialog.hide();
}

#[test]
#[ignore = "Requires a GTK desktop display; run explicitly"]
fn desktop_scope_selection_and_reentrant_rendering() {
    gtk::init().expect("Desktop display available");
    crate::assets::prepare().expect("Icons available");
    let dialog = SearchDialog::new(Rc::new(|_| {}), Rc::new(|| {}));
    assert_eq!(dialog.state.scope.selected(), 1);
    dialog
        .state
        .current_folder
        .replace(PathBuf::from("/tmp/current-album"));
    let request = ScopedSearch::from_uri(&dialog.search_uri()).expect("Dialog URI parses");
    assert_eq!(
        request.root,
        SearchScope::Home.root(Path::new("/tmp/current-album"))
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
            let _selection = state.selected_key.borrow_mut();
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
    render_search_results(&dialog.state, same, false, None, None);
    assert_eq!(
        dialog.state.list.row_at_index(0).unwrap(),
        row,
        "Unchanged results must not rebuild rows"
    );
    assert_eq!(dialog.state.list.selected_row().unwrap().index(), 0);
    assert_eq!(
        dialog.state.selected_key.borrow().as_ref(),
        Some(&SearchResultKey::File(second.path.clone()))
    );
    render_results(&dialog.state, Vec::new(), true, Some("Index limit"));
    assert_eq!(
        dialog.state.selected_key.borrow().as_ref(),
        Some(&SearchResultKey::File(second.path.clone()))
    );
    render_results(&dialog.state, vec![second.clone()], false, None);
    assert_eq!(
        dialog.state.selected_key.borrow().as_ref(),
        Some(&SearchResultKey::File(second.path.clone()))
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
    dialog.show_scoped(root.clone(), "needle", 0);
    let pump = |expected: &str| {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let context = glib::MainContext::default();
        loop {
            if dialog
                .state
                .visible_results
                .borrow()
                .iter()
                .filter_map(SearchResult::file)
                .any(|item| item.name == expected)
                && !dialog
                    .state
                    .notice
                    .text()
                    .contains("Indexing selected scope")
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
    let session = dialog.state.session_generation.get();
    let initial_query = dialog.state.expected_query_generation.get().unwrap();
    begin_query(&dialog.state, "needle");
    let repeated_query = dialog.state.expected_query_generation.get().unwrap();
    assert!(repeated_query > initial_query);
    dialog.state.field.set_text("another query");
    let other_query = dialog.state.expected_query_generation.get().unwrap();
    assert!(other_query > repeated_query);
    dialog.state.field.set_text("");
    let empty_query = dialog.state.expected_query_generation.get().unwrap();
    assert!(empty_query > other_query);
    dialog.state.field.set_text("needle");
    assert!(dialog.state.expected_query_generation.get().unwrap() > empty_query);
    assert_eq!(dialog.state.session_generation.get(), session);
    pump("needle-old.txt");
    std::fs::remove_file(root.join("needle-old.txt")).unwrap();
    std::fs::write(root.join("needle-new.txt"), "generated fixture").unwrap();
    let generation = dialog.state.session_generation.get();
    dialog.state.refresh.emit_clicked();
    assert_eq!(
        dialog.state.session_generation.get(),
        generation.wrapping_add(1)
    );
    assert_eq!(dialog.state.field.text(), "needle");
    pump("needle-new.txt");
    assert!(
        !dialog
            .state
            .visible_results
            .borrow()
            .iter()
            .filter_map(SearchResult::file)
            .any(|item| item.name == "needle-old.txt")
    );
    dialog.hide();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Requires isolated GTK display; verifies interpretation slot chips, dismiss action, and filter panel sections"]
fn desktop_interpretation_slot_and_filter_sections() {
    if gtk::init().is_err() {
        return;
    }
    crate::assets::prepare().unwrap();
    let dialog = SearchDialog::new(Rc::new(|_| {}), Rc::new(|| {}));

    assert_eq!(dialog.state.filters.modified.model().unwrap().n_items(), 5);
    dialog.state.filters.set_values(&SearchFilterValues {
        modified: Some("last-week".into()),
        ..SearchFilterValues::default()
    });
    assert_eq!(dialog.state.filters.modified.selected(), 3);
    assert!(dialog.state.filters.query("").contains("modified:last-week"));

    dialog.state.filters.set_values(&SearchFilterValues {
        modified: Some("past-7-days".into()),
        ..SearchFilterValues::default()
    });
    assert_eq!(dialog.state.filters.modified.selected(), 4);
    assert!(!dialog.state.interpretation_slot.is_visible());
    assert!(!dialog.state.notice_slot.is_visible());

    dialog.state.field.set_text("Beach House songs");
    assert!(dialog.state.interpretation_slot.is_visible());
    assert!(dialog.state.interpretation_slot.first_child().is_some());

    let chip_box = dialog.state.interpretation_slot.first_child().unwrap();
    let remove_btn = chip_box.last_child().unwrap().downcast::<gtk::Button>().unwrap();
    remove_btn.emit_clicked();

    assert!(dialog.state.field.text().contains("\"songs\""));
    assert!(dialog.state.interpretation_slot.first_child().is_none());
    assert!(!dialog.state.interpretation_slot.is_visible());
}

