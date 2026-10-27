use super::*;

#[test]
fn library_refresh_waits_for_finished_snapshots_and_preserves_service_errors() {
    use crate::services::music_library::Snapshot;
    let mut snapshot = Snapshot { indexing: true, ..Snapshot::default() };
    assert_eq!(library_refresh_result(&snapshot), None);
    snapshot.notice = Some("Folder unavailable: generated fixture".into());
    assert_eq!(library_refresh_result(&snapshot), None);
    snapshot.indexing = false;
    assert_eq!(library_refresh_result(&snapshot), Some(Err("Folder unavailable: generated fixture".into())));
    snapshot.notice = None;
    assert_eq!(library_refresh_result(&snapshot), Some(Ok(())));
    snapshot.indexing = true;
    assert_eq!(library_refresh_result(&snapshot), None);
}

#[test]
fn refreshed_settings_keep_the_current_release_version() {
    assert_eq!(crate::build_info::VERSION, env!("CARGO_PKG_VERSION"));
    assert!(!crate::build_info::VERSION.starts_with("1.0"));
}

#[test]
#[ignore = "Requires isolated GTK display and settings directory"]
fn desktop_context_menu_options_apply_persist_and_restore_defaults() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = ThemeManager::shared();
    manager.set_context_menu_options(super::super::context_menu::Options::default());
    let page = context_menu_page(manager.clone());
    let mut row = page.first_child();
    let mut toggles = Vec::new();
    while let Some(widget) = row {
        row = widget.next_sibling();
        if let Some(toggle) = widget.last_child().and_downcast::<gtk::Switch>() {
            toggles.push(toggle);
        }
    }
    assert_eq!(toggles.len(), 8);
    toggles[3].set_active(true);
    toggles[6].set_active(false);
    assert!(manager.context_menu_options().transfer_destinations);
    assert!(!manager.context_menu_options().music);
    let saved: toml::Value = toml::from_str(
        &std::fs::read_to_string(crate::paths::settings_file())
            .unwrap(),
    )
    .unwrap();
    let saved: super::super::context_menu::Options =
        saved["context_menu"].clone().try_into().unwrap();
    assert_eq!(saved, manager.context_menu_options());
    page.last_child()
        .and_downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    assert_eq!(
        manager.context_menu_options(),
        super::super::context_menu::Options::default()
    );
    assert!(!toggles[3].is_active() && toggles[6].is_active());
}

#[test]
#[ignore = "Requires an isolated GTK display; settings and font control fixture"]
fn desktop_fonts_and_shortcuts_have_live_controls_and_scrollable_pages() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = ThemeManager::shared();
    let fonts = fonts_page(manager.clone());
    let row = fonts.first_child().unwrap().next_sibling().unwrap();
    let family = row
        .last_child()
        .unwrap()
        .downcast::<gtk::DropDown>()
        .unwrap();
    family.set_selected(2);
    assert_eq!(manager.interface_font(), "sans");
    let size_row = row.next_sibling().unwrap();
    let size = size_row
        .last_child()
        .unwrap()
        .downcast::<gtk::SpinButton>()
        .unwrap();
    size.set_value(17.0);
    assert_eq!(manager.interface_font_size(), 17);
    fonts
        .last_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap()
        .emit_clicked();
    assert_eq!(manager.interface_font(), "maple");
    assert_eq!(manager.interface_font_size(), 13);
    assert_eq!(family.selected(), 0);
    assert_eq!(size.value_as_int(), 13);
    let window = gtk::Window::builder()
        .default_width(700)
        .default_height(500)
        .child(&scroll_page(&fonts))
        .build();
    window.present();
    let until = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < until {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(fonts.width() > 0);
    window.close();
    let shortcuts = keybindings_page()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    assert_eq!(shortcuts.hscrollbar_policy(), gtk::PolicyType::Never);
    assert_eq!(shortcuts.vscrollbar_policy(), gtk::PolicyType::Automatic);
}

#[test]
#[ignore = "Requires isolated GTK display"]
fn audio_and_search_preferences_default_values_and_roundtrip() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let toml_str = r#"
mode = "theme"
theme = "azure-glow"
"#;
    let saved: toml::Value = toml::from_str(toml_str).unwrap();
    assert!(saved.get("mode").is_some());

    // Verify Preferences defaults
    let manager = ThemeManager::shared();
    assert!(manager.audio_spin_animation());
    assert_eq!(manager.audio_spectrum_bands(), 32);
    assert_eq!(manager.audio_spectrum_decay(), "studio");
    assert!(manager.search_exclude_hidden());
    assert!(manager.search_exclude_build_artifacts());
    assert!(manager.search_reconcile_periodically());
    assert_eq!(manager.startup_location(), "home");
    assert!(manager.confirm_permanent_delete());
    assert_eq!(manager.date_format(), "relative");
    assert_eq!(manager.size_unit(), "decimal");
    assert!(!manager.show_hidden_default());
    assert_eq!(manager.max_preview_size_mb(), 50);
    assert!(manager.preview_audio());
    assert!(manager.preview_images());
    assert!(manager.preview_videos());
    assert!(manager.preview_pdf());
    assert!(manager.preview_spreadsheets());
    assert!(manager.preview_code());
}

#[test]
#[ignore = "Requires isolated GTK display"]
fn audio_and_diagnostics_pages_build_cleanly() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = ThemeManager::shared();
    let audio = audio_page(manager.clone());
    assert!(audio.is::<gtk::Box>());
    let search = search_page(manager.clone());
    assert!(search.is::<gtk::Box>());
    let diag = diagnostics_page();
    assert!(diag.is::<gtk::Box>());
    let browser = BrowserView::new(
        std::rc::Rc::new(crate::adapters::RoutedFileSource::new()),
        crate::ui::browser::PeekBehavior::default(),
    );
    let view_formats = view_formats_page(&browser, manager.clone(), std::rc::Rc::new(|| {}));
    assert!(view_formats.is::<gtk::Box>());
    let previews = previews_page(&browser, manager.clone());
    assert!(previews.is::<gtk::Box>());
    let general = general_page(&browser, manager, std::rc::Rc::new(|| {}));
    assert!(general.is::<gtk::Box>());
}
