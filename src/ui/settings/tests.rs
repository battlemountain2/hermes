use super::*;

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
        &std::fs::read_to_string(gtk::glib::user_config_dir().join("strata/settings.toml"))
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
