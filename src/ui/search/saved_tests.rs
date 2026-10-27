use super::*;

#[test]
#[ignore = "Private GTK display; verifies saved-query scope, filters, entity guards and stable layout"]
fn desktop_save_search_preserves_physical_queries_without_resizing() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let theme = crate::ui::theme::ThemeManager::shared();
    theme.set_show_smart_folders(true);
    let styles = gtk::CssProvider::new();
    styles.load_from_string(include_str!("../../style.css"));
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let saved = Rc::new(RefCell::new(Vec::new()));
    let callbacks = saved.clone();
    let dismissed = Rc::new(Cell::new(0));
    let dismissals = dismissed.clone();
    let dialog = SearchDialog::new(
        Rc::new(|_| {}),
        Rc::new(move || dismissals.set(dismissals.get() + 1)),
    );
    dialog.set_open_results(Rc::new(|_| {}));
    dialog.set_save_search(Rc::new(move |uri| callbacks.borrow_mut().push(uri)));
    dialog.state.scope.set_selected(0);
    dialog
        .state
        .current_folder
        .replace(PathBuf::from("/synthetic/photos"));
    dialog
        .state
        .root
        .replace(PathBuf::from("/synthetic/photos"));
    let query = r#"type:image ext:heic modified:yesterday in:Trips"#;
    dialog.state.field.set_text(query);
    dialog.state.layer.set_visible(true);
    let window = gtk::Window::builder()
        .default_width(1000)
        .default_height(900)
        .child(&dialog.widget())
        .build();
    window.present();
    let settle = || {
        let deadline = std::time::Instant::now() + Duration::from_millis(80);
        while std::time::Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    settle();
    let panel = dialog.state.field.parent().unwrap().parent().unwrap();
    let bounds = (panel.width(), panel.height());
    for (position, group) in SEARCH_GROUPS.iter().enumerate() {
        dialog.state.categories[position].emit_clicked();
        settle();
        assert_eq!((panel.width(), panel.height()), bounds);
        let exportable = !matches!(group, SearchGroup::Albums | SearchGroup::Artists);
        assert_eq!(dialog.state.save_button.is_sensitive(), exportable);
        assert!(dialog.state.save_button.is_visible());
        let previous = saved.borrow().len();
        dialog.state.save_button.emit_clicked();
        assert_eq!(saved.borrow().len(), previous + usize::from(exportable));
        if exportable {
            assert!(!dialog.is_visible());
            let request = ScopedSearch::from_uri(saved.borrow().last().unwrap()).unwrap();
            assert_eq!(request.scope, SearchScope::ThisFolder);
            assert_eq!(request.root, PathBuf::from("/synthetic/photos"));
            assert_eq!(request.group, *group);
            assert_eq!(request.query, query);
            dialog.state.layer.set_visible(true);
        }
    }
    let key = SearchEntityKey::Album {
        artist: "various artists".into(),
        album: "compilation".into(),
    };
    dialog.state.group.set(SearchGroup::Songs);
    dialog.state.entity_scope.replace(Some(EntityScope {
        key: key.clone(),
        previous_group: SearchGroup::Albums,
        previous_selection: None,
    }));
    update_group_controls(&dialog.state);
    dialog.state.save_button.emit_clicked();
    let request = ScopedSearch::from_uri(saved.borrow().last().unwrap()).unwrap();
    assert_eq!(request.entity, Some(key));
    assert_eq!(request.group, SearchGroup::Songs);
    assert_eq!(dismissed.get(), 4);
    window.close();
}
