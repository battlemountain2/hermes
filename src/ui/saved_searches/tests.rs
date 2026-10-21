use super::*;

#[test]
fn default_names_use_query_or_physical_group_and_bound_unicode() {
    let mut search = ScopedSearch::new(
        SearchScope::ThisFolder,
        std::path::Path::new("/synthetic"),
        "",
    );
    for (group, expected) in [
        (SearchGroup::All, "All"),
        (SearchGroup::Files, "Files"),
        (SearchGroup::Songs, "Songs"),
    ] {
        search.group = group;
        assert_eq!(default_name(&search), expected);
    }
    search.query = "  title:\"A song\" type:audio  ".into();
    assert_eq!(default_name(&search), "title:\"A song\" type:audio");
    search.query = "音🎵".repeat(60);
    assert_eq!(default_name(&search).chars().count(), 80);
    assert!(default_name(&search).starts_with("音🎵"));
    search.query = "one\ntwo\tthree".into();
    assert!(!default_name(&search).chars().any(char::is_control));
}

#[test]
fn selectable_groups_never_offer_virtual_entities() {
    assert_eq!(
        GROUPS,
        [SearchGroup::All, SearchGroup::Files, SearchGroup::Songs]
    );
    assert!(POLL_INTERVAL <= Duration::from_millis(150));
}

#[test]
#[ignore = "Requires a private GTK display; uses only synthetic temporary saved-search storage"]
fn desktop_saved_search_manager_preserves_fields_and_waits_for_durable_ack() {
    use std::{
        fs,
        os::unix::fs::DirBuilderExt,
        sync::atomic::{AtomicU64, Ordering},
        time::Instant,
    };

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "strata-saved-search-ui-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _removed = fs::remove_dir_all(&self.0);
        }
    }

    fn settle() {
        let deadline = Instant::now() + Duration::from_millis(120);
        while Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn ack(dialog: &SavedSearchDialog, store: &Store, success: bool) {
        assert_eq!(store.flush(Duration::from_secs(5)), success);
        poll(&dialog.state);
        assert!(dialog.state.pending.borrow().is_none());
    }

    gtk::init().unwrap();
    assets::prepare().unwrap();
    let fixture = Fixture::new();
    let store = Store::start(fixture.0.join("queries-v1.json"));
    assert!(store.flush(Duration::from_secs(5)));
    let tokens: crate::ui::theme::ThemeTokens =
        toml::from_str(include_str!("../../../data/themes/azure-glow.toml")).unwrap();
    let styles = gtk::CssProvider::new();
    styles.load_from_string(&format!(
        "@define-color theme_bg {}; @define-color theme_surface {}; @define-color theme_text {}; @define-color theme_accent {}; @define-color theme_danger {}; @define-color theme_muted {}; @define-color theme_highlight {}; @define-color theme_border {}; @define-color theme_dim_text {};\n{}",
        tokens.background, tokens.surface, tokens.text, tokens.accent, tokens.danger,
        tokens.muted, tokens.highlight, tokens.border, tokens.dim_text,
        include_str!("../../style.css")
    ));
    let display = gdk::Display::default().unwrap();
    gtk::style_context_add_provider_for_display(
        &display,
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let events = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    let opened = events.clone();
    let dismissed = events.clone();
    let dialog = SavedSearchDialog::new(
        store.clone(),
        Rc::new(move |uri| opened.borrow_mut().push(("open".into(), uri))),
        Rc::new(move || {
            dismissed
                .borrow_mut()
                .push(("dismiss".into(), String::new()))
        }),
    );
    let copied_dialog = dialog.clone();
    assert_eq!(copied_dialog.widget(), dialog.widget());
    drop(copied_dialog);
    dialog.show_manage();
    assert!(dialog.state.ready.get());
    assert!(dialog.state.create.is_sensitive());
    assert!(!dialog.state.save.is_sensitive());
    assert!(!dialog.state.open.is_sensitive());
    assert!(!dialog.state.delete.is_sensitive());
    let window = gtk::Window::builder()
        .default_width(1100)
        .default_height(900)
        .child(&dialog.widget())
        .build();
    window.present();
    settle();
    let frame = dialog
        .widget()
        .first_child()
        .unwrap()
        .next_sibling()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    assert_eq!(frame.min_content_width(), WIDTH);
    assert_eq!(frame.max_content_width(), WIDTH);
    assert_eq!(frame.min_content_height(), HEIGHT);
    assert_eq!(frame.max_content_height(), HEIGHT);
    assert!(frame.has_css_class("search-dialog"));
    assert_eq!(
        dialog.state.confirmation.transition_type(),
        gtk::StackTransitionType::None
    );
    assert_eq!(
        dialog.state.list_stack.transition_type(),
        gtk::StackTransitionType::None
    );
    let bounds = || {
        (
            (frame.width(), frame.height()),
            dialog.state.status.compute_bounds(&frame).unwrap(),
            dialog
                .state
                .save
                .parent()
                .unwrap()
                .compute_bounds(&frame)
                .unwrap(),
        )
    };
    let initial_bounds = bounds();

    let entity = SearchEntityKey::Album {
        artist: "synthetic artist".into(),
        album: "synthetic album".into(),
    };
    let search = ScopedSearch::new(
        SearchScope::ThisFolder,
        std::path::Path::new("/synthetic/music"),
        "title:needle type:audio",
    )
    .with_group(SearchGroup::Songs, Some(entity.clone()));
    dialog.show_create(&search.to_uri());
    assert_eq!(dialog.state.name.text(), search.query);
    assert_eq!(dialog.state.query.text(), search.query);
    assert_eq!(dialog.state.root.text(), "/synthetic/music");
    assert_eq!(dialog.state.scope.selected(), 0);
    assert_eq!(dialog.state.group.selected(), 2);
    assert_eq!(
        ScopedSearch::from_uri(&edited_uri(&dialog.state).unwrap())
            .unwrap()
            .entity,
        Some(entity.clone())
    );
    dialog.state.name.set_text("My songs");
    dialog.state.save.emit_clicked();
    assert!(dialog.state.pending.borrow().is_some());
    assert!(dialog.state.entries.borrow().is_empty());
    assert!(!dialog.state.save.is_sensitive());
    ack(&dialog, &store, true);
    assert_eq!(dialog.state.entries.borrow().len(), 1);
    assert!(dialog.state.status.text().contains("Saved to storage"));
    let original = dialog.state.previous.borrow().clone().unwrap();
    if let Some(path) = std::env::var_os("HERMES_SAVED_SEARCH_SCREENSHOT") {
        settle();
        let paintable = gtk::WidgetPaintable::new(Some(&dialog.widget()));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            f64::from(dialog.widget().width()),
            f64::from(dialog.widget().height()),
        );
        if let Some(node) = snapshot.to_node() {
            let renderer = gtk::gsk::CairoRenderer::new();
            renderer.realize(None::<&gdk::Surface>).unwrap();
            let texture = renderer.render_texture(&node, None);
            texture.save_to_png(path).unwrap();
            renderer.unrealize();
        }
    }

    dialog.state.name.set_text("Renamed songs");
    dialog.state.query.set_text("album:synthetic");
    dialog.state.root.set_text("/synthetic/other-root");
    let edited = ScopedSearch::from_uri(&edited_uri(&dialog.state).unwrap()).unwrap();
    assert_eq!(edited.entity, Some(entity.clone()));
    assert_eq!(edited.root, PathBuf::from("/synthetic/other-root"));
    dialog.state.save.emit_clicked();
    assert_eq!(dialog.state.entries.borrow()[0], original);
    ack(&dialog, &store, true);
    assert_eq!(
        dialog.state.previous.borrow().as_ref().unwrap().name,
        "Renamed songs"
    );
    assert_eq!(
        ScopedSearch::from_uri(&dialog.state.previous.borrow().as_ref().unwrap().uri)
            .unwrap()
            .entity,
        Some(entity)
    );

    dialog.state.group.set_selected(1);
    assert!(matches!(
        dialog.state.question.get(),
        Some(Question::ClearMembership(1))
    ));
    assert_eq!(dialog.state.group.selected(), 2);
    assert!(!dialog.state.save.is_sensitive());
    dialog.state.cancel.emit_clicked();
    assert!(
        dialog
            .state
            .context
            .borrow()
            .as_ref()
            .unwrap()
            .entity
            .is_some()
    );
    dialog.state.group.set_selected(1);
    dialog.state.confirm.emit_clicked();
    assert_eq!(dialog.state.group.selected(), 1);
    assert!(
        dialog
            .state
            .context
            .borrow()
            .as_ref()
            .unwrap()
            .entity
            .is_none()
    );
    assert_eq!(dialog.state.query.text(), "album:synthetic");
    dialog.state.save.emit_clicked();
    ack(&dialog, &store, true);
    let physical =
        ScopedSearch::from_uri(&dialog.state.previous.borrow().as_ref().unwrap().uri).unwrap();
    assert_eq!(physical.group, SearchGroup::Files);
    assert!(physical.entity.is_none());
    dialog.state.save.emit_clicked();
    ack(&dialog, &store, true);
    dialog.state.reload.emit_clicked();
    ack(&dialog, &store, true);

    let opened_uri = dialog.state.previous.borrow().as_ref().unwrap().uri.clone();
    dialog.state.open.emit_clicked();
    assert!(!dialog.widget().is_visible());
    assert!(dialog.state.poll_source.borrow().is_none());
    assert_eq!(
        &*events.borrow(),
        &[
            ("open".into(), opened_uri),
            ("dismiss".into(), String::new())
        ]
    );
    dialog.show_manage();
    dialog.state.name.set_text("My unsaved name");
    dialog.state.query.set_text("my-unsaved-query");
    let previous = dialog.state.previous.borrow().clone().unwrap();
    store
        .send(Command::Update {
            previous: previous.clone(),
            name: "Changed elsewhere".into(),
            uri: previous.uri.clone(),
        })
        .unwrap();
    assert!(store.flush(Duration::from_secs(5)));
    poll(&dialog.state);
    assert_eq!(dialog.state.name.text(), "My unsaved name");
    assert_eq!(dialog.state.previous.borrow().as_ref(), Some(&previous));
    assert!(dialog.state.status.text().contains("changed elsewhere"));
    dialog.state.save.emit_clicked();
    ack(&dialog, &store, false);
    assert_eq!(dialog.state.name.text(), "My unsaved name");
    assert_eq!(dialog.state.query.text(), "my-unsaved-query");
    assert_eq!(dialog.state.previous.borrow().as_ref(), Some(&previous));
    assert!(dialog.state.status.text().contains("Not saved"));
    assert!(dialog.state.save.is_sensitive());
    dialog.state.reload.emit_clicked();
    ack(&dialog, &store, true);
    assert_eq!(dialog.state.name.text(), "My unsaved name");
    dialog.state.load.emit_clicked();
    assert_eq!(dialog.state.name.text(), "Changed elsewhere");

    dialog.state.delete.emit_clicked();
    assert!(matches!(
        dialog.state.question.get(),
        Some(Question::Delete)
    ));
    assert_eq!(store.snapshot().entries.len(), 1);
    assert!(dialog.state.confirmation_text.text().contains("No files"));
    dialog.state.cancel.emit_clicked();
    assert_eq!(store.snapshot().entries.len(), 1);
    dialog.state.delete.emit_clicked();
    dialog.state.confirm.emit_clicked();
    assert!(dialog.state.pending.borrow().is_some());
    assert_eq!(dialog.state.entries.borrow().len(), 1);
    ack(&dialog, &store, true);
    assert!(dialog.state.entries.borrow().is_empty());
    assert!(dialog.state.status.text().contains("No files were deleted"));
    message(&dialog.state, &"A long storage error ".repeat(300));
    settle();
    assert_eq!(bounds(), initial_bounds);

    dialog.show_create("search://?scope=folder&root=%2Fsynthetic&q=test&group=albums");
    assert!(!dialog.state.save.is_sensitive());
    assert!(dialog.state.status.text().contains("Cannot save"));
    dialog.show_create(&search.to_uri());
    dialog.state.name.set_text("");
    assert!(!dialog.state.save.is_sensitive());
    dialog.state.name.set_text("Valid name");
    dialog.state.root.set_text("relative/location");
    assert!(!dialog.state.save.is_sensitive());
    dialog.state.root.set_text("/synthetic/music");
    dialog.state.scope.set_selected(1);
    assert!(!dialog.state.root.is_sensitive());
    assert_eq!(
        ScopedSearch::from_uri(&edited_uri(&dialog.state).unwrap())
            .unwrap()
            .scope,
        SearchScope::Home
    );
    assert_eq!(dialog.state.name.max_length(), 80);

    let revision = dialog.state.revision.get().unwrap();
    apply_snapshot(
        &dialog.state,
        Snapshot {
            revision: revision + 1,
            ready: false,
            writable: false,
            entries: Vec::new(),
            error: None,
        },
    );
    assert!(!dialog.state.save.is_sensitive());
    assert!(!dialog.state.create.is_sensitive());
    assert!(dialog.state.status.text().contains("Loading"));
    apply_snapshot(
        &dialog.state,
        Snapshot {
            revision: revision + 2,
            ready: true,
            writable: false,
            entries: Vec::new(),
            error: Some("Synthetic storage failure".into()),
        },
    );
    assert!(!dialog.state.save.is_sensitive());
    assert!(dialog.state.reload.is_sensitive());
    assert!(dialog.state.status.text().contains("storage unavailable"));
    let rendered_revision = dialog.state.revision.get();
    dialog.hide();
    poll(&dialog.state);
    assert_eq!(dialog.state.revision.get(), rendered_revision);
    assert!(dialog.state.poll_source.borrow().is_none());
    dialog.show_manage();
    assert!(dialog.state.poll_source.borrow().is_some());
    let weak = Rc::downgrade(&dialog.state);
    drop(dialog);
    assert!(weak.upgrade().is_none());
    settle();
    window.close();
    gtk::style_context_remove_provider_for_display(&display, &styles);
    drop(store);
}
