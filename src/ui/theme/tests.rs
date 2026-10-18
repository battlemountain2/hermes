// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    Preferences, blend, is_omarchy_theme_event, slugify, title_case_slug, tokens_from_quattro,
};

#[test]
fn basic_palettes_replace_omarchy_light_without_losing_saved_light_selection() {
    let themes = super::builtins();
    assert!(!themes.iter().any(|theme| theme.id == "omarchy-light"));
    for id in ["graphite", "paper", "midnight"] {
        assert!(themes.iter().any(|theme| theme.id == id));
    }
    assert_eq!(super::migrate_theme_id("omarchy-light"), "paper");
    assert_eq!(super::migrate_theme_id("custom-light"), "custom-light");
}

#[test]
fn font_choices_are_safe_and_legacy_sizes_default_to_thirteen() {
    assert_eq!(super::normalized_font("sans"), "sans");
    assert_eq!(super::normalized_font("mono"), "mono");
    assert_eq!(super::normalized_font("malformed; CSS"), "maple");
    let preferences: Preferences = toml::from_str("mode = 'theme'\ntheme = 'paper'").unwrap();
    assert_eq!(preferences.interface_font_size, 13);
    let css = super::tokens_css(&super::azure_tokens(), "sans");
    assert!(css.contains("font-family: sans-serif"));
}

#[test]
fn names_become_safe_config_file_slugs() {
    assert_eq!(slugify("  Rosé / Pine!  "), "ros-pine");
    assert_eq!(slugify("Ocean  Blue"), "ocean-blue");
}

#[test]
fn omarchy_slugs_become_display_names() {
    assert_eq!(title_case_slug("tokyo-night"), "Tokyo Night");
}

#[test]
fn colors_can_be_blended_into_semantic_tokens() {
    assert_eq!(blend("#000000", "#ffffff", 0.5), "#808080");
}

#[test]
fn quattro_colors_map_to_strata_tokens() {
    let theme = tokens_from_quattro(
        "azure-glow",
        r##"
background = "#0a0f1a"
foreground = "#a8dfff"
accent = "#00aaff"
selection = "#a8dfff"
color8 = "#123247"
"##,
    )
    .expect("valid Quattro colors should map");

    assert_eq!(theme.name, "Azure Glow");
    assert_eq!(theme.background, "#0d1b2a");
    assert_eq!(theme.accent, "#00aaff");
    assert_eq!(theme.border, "#487089");
}

#[test]
fn legacy_palette_without_quattro_semantics_is_not_detected() {
    assert!(tokens_from_quattro("legacy", "color4 = \"#00aaff\"").is_none());
}

#[test]
fn omarchy_monitor_ignores_unrelated_state_changes() {
    assert!(is_omarchy_theme_event(&gtk::gio::File::for_path(
        "/state/current/theme"
    )));
    assert!(is_omarchy_theme_event(&gtk::gio::File::for_path(
        "/state/current/theme.name"
    )));
    assert!(!is_omarchy_theme_event(&gtk::gio::File::for_path(
        "/state/current/next-theme"
    )));
    assert!(!is_omarchy_theme_event(&gtk::gio::File::for_path(
        "/state/current/background"
    )));
}

#[test]
fn legacy_preferences_enable_single_click_previews_by_default() {
    let preferences: Preferences = toml::from_str(
        r#"
mode = "theme"
theme = "azure-glow"
"#,
    )
    .expect("legacy preferences should remain valid");

    assert!(preferences.single_click_previews);
    assert!(!preferences.search_open_files_directly);
    assert_eq!(preferences.browser_mode, "columns");
    assert_eq!(preferences.browser_density, "compact");
    assert_eq!(preferences.interface_font, "maple");
}

#[test]
fn audio_and_search_preferences_default_safely_and_roundtrip() {
    let preferences: Preferences = toml::from_str(
        r#"
mode = "theme"
theme = "azure-glow"
"#,
    )
    .expect("defaults should be applied when omitted");

    assert!(preferences.audio_continuity);
    assert!(preferences.audio_spin_animation);
    assert_eq!(preferences.audio_spectrum_bands, 32);
    assert_eq!(preferences.audio_spectrum_decay, "studio");
    assert!(preferences.search_exclude_hidden);
    assert!(preferences.search_exclude_build_artifacts);
    assert!(preferences.search_reconcile_periodically);

    let serialized = toml::to_string(&preferences).expect("Preferences must serialize cleanly");
    let roundtripped: Preferences =
        toml::from_str(&serialized).expect("Preferences must deserialize after roundtrip");
    assert_eq!(roundtripped.audio_spectrum_bands, 32);
    assert_eq!(roundtripped.audio_spectrum_decay, "studio");
}

#[test]
fn interface_font_preference_can_select_jetbrains_mono() {
    let preferences: Preferences = toml::from_str(
        r#"
mode = "theme"
theme = "azure-glow"
interface_font = "jetbrains"
"#,
    )
    .expect("font preferences should be valid");

    assert_eq!(preferences.interface_font, "jetbrains");
}

#[test]
fn single_click_previews_can_be_disabled_in_preferences() {
    let preferences: Preferences = toml::from_str(
        r#"
mode = "theme"
theme = "azure-glow"
single_click_previews = false
"#,
    )
    .expect("preferences should be valid");

    assert!(!preferences.single_click_previews);
}

#[test]
fn noctalia_follow_mode_round_trips_without_changing_selected_theme() {
    let preferences: Preferences =
        toml::from_str("mode = \"noctalia\"\ntheme = \"rose-pine\"").expect("Noctalia preference");
    let persisted = toml::to_string(&preferences).expect("serialize preference");
    let restored: Preferences = toml::from_str(&persisted).expect("restore preference");
    assert_eq!(restored.mode, "noctalia");
    assert_eq!(restored.theme, "rose-pine");
    assert_eq!(Preferences::default().mode, "theme");
}

#[test]
#[ignore = "requires an installed Noctalia GTK 4 export; reads but never modifies it"]
fn noctalia_installed_export_is_valid() {
    let path = super::noctalia::palette_path(&gtk::glib::user_config_dir());
    let mut palette = super::noctalia::Palette::new(path);
    assert!(palette.refresh(), "installed Noctalia export must validate");
    let tokens = palette.last_valid.expect("installed palette");
    assert!(super::validate_tokens(&tokens).is_ok());
}

#[test]
#[ignore = "requires a GTK display; tests live Noctalia directory monitoring"]
fn noctalia_monitor_debounces_replacement_and_recovers_without_overriding_other_themes() {
    use super::{ThemeManager, builtins, noctalia};
    use gtk::glib;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        time::{Duration, Instant},
    };

    gtk::init().expect("GTK display");
    let fixture = noctalia::tests::Fixture::new();
    let path = noctalia::palette_path(&fixture.0);
    let manager = Rc::new(ThemeManager {
        provider: gtk::CssProvider::new(),
        themes: RefCell::new(builtins()),
        preferences: RefCell::new(Preferences::default()),
        applied_tokens: RefCell::new(None),
        omarchy_available: false,
        omarchy_monitor: RefCell::new(None),
        pending_omarchy_refresh: RefCell::new(None),
        noctalia: RefCell::new(noctalia::Palette::new(path.clone())),
        noctalia_monitor: RefCell::new(None),
        pending_noctalia_refresh: RefCell::new(None),
        noctalia_poll: RefCell::new(None),
        previewing: Cell::new(false),
    });
    assert!(
        manager.waveform_colors().is_none(),
        "no colors before applying a theme"
    );
    let pump = |duration: Duration| {
        let until = Instant::now() + duration;
        while Instant::now() < until {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    manager.preferences.borrow_mut().mode = "noctalia".to_owned();
    manager.apply_selected();
    let azure = super::azure_tokens();
    assert_eq!(
        manager.waveform_colors(),
        Some((
            gtk::gdk::RGBA::parse(&azure.accent).expect("accent"),
            gtk::gdk::RGBA::parse(&azure.muted).expect("muted")
        ))
    );
    assert_eq!(
        manager.current_tokens().expect("startup fallback").name,
        "Azure Glow"
    );
    assert!(
        manager.follows_noctalia(),
        "missing export does not reset the saved follower"
    );
    manager.preferences.borrow_mut().mode = "theme".to_owned();
    manager.monitor_noctalia();
    std::fs::create_dir(path.parent().expect("export directory")).expect("create directory late");
    std::fs::write(&path, noctalia::tests::CSS).expect("first export");
    pump(Duration::from_millis(350));
    assert!(manager.is_noctalia_available());
    assert!(!manager.follows_noctalia());
    assert_eq!(
        manager.current_tokens().expect("selected theme").name,
        "Azure Glow"
    );

    manager.preferences.borrow_mut().mode = "noctalia".to_owned();
    manager.apply_selected();
    let first = manager.provider.to_string();
    let noctalia_tokens = manager.current_tokens().expect("effective Noctalia tokens");
    assert_eq!(
        manager.waveform_colors(),
        Some((
            gtk::gdk::RGBA::parse(&noctalia_tokens.accent).expect("accent"),
            gtk::gdk::RGBA::parse(&noctalia_tokens.muted).expect("muted")
        ))
    );
    std::fs::write(&path, "@define-color accent_bg_color #abcdef;").expect("partial export");
    pump(Duration::from_millis(220));
    assert_eq!(manager.provider.to_string(), first);

    let temporary = fixture.0.join("replacement.css");
    std::fs::write(
        &temporary,
        noctalia::tests::CSS.replace("#94cdf6", "#abcdef"),
    )
    .expect("replacement");
    std::fs::rename(&temporary, &path).expect("atomic rename");
    pump(Duration::from_millis(40));
    assert_eq!(
        manager.provider.to_string(),
        first,
        "not applied before debounce"
    );
    pump(Duration::from_millis(220));
    assert_eq!(
        manager.current_tokens().expect("live tokens").accent,
        "#abcdef"
    );
    assert_ne!(manager.provider.to_string(), first);

    let before_burst = manager.provider.to_string();
    std::fs::write(&path, noctalia::tests::CSS.replace("#94cdf6", "#112233"))
        .expect("first burst update");
    pump(Duration::from_millis(60));
    std::fs::write(&path, noctalia::tests::CSS.replace("#94cdf6", "#445566"))
        .expect("last burst update");
    pump(Duration::from_millis(100));
    assert_eq!(
        manager.provider.to_string(),
        before_burst,
        "debounce resets after each write"
    );
    pump(Duration::from_millis(120));
    assert_eq!(
        manager.current_tokens().expect("settled burst").accent,
        "#445566"
    );

    let mut custom_preview = super::azure_tokens();
    custom_preview.accent = "#f09020".to_owned();
    custom_preview.muted = "#273645".to_owned();
    manager.preview(&custom_preview);
    let preview_colors = Some((
        gtk::gdk::RGBA::parse("#f09020").expect("preview accent"),
        gtk::gdk::RGBA::parse("#273645").expect("preview muted"),
    ));
    assert_eq!(manager.waveform_colors(), preview_colors);
    std::fs::write(&path, noctalia::tests::CSS).expect("new export during preview");
    let preview = manager.provider.to_string();
    pump(Duration::from_millis(220));
    assert_eq!(
        manager.provider.to_string(),
        preview,
        "editor preview remains untouched"
    );
    assert_eq!(
        manager.waveform_colors(),
        preview_colors,
        "draw colors follow the applied preview, not the selected palette"
    );
    manager.cancel_preview();
    assert_eq!(manager.provider.to_string(), first);
    assert_eq!(
        manager.waveform_colors(),
        Some((
            gtk::gdk::RGBA::parse(&noctalia_tokens.accent).expect("accent"),
            gtk::gdk::RGBA::parse(&noctalia_tokens.muted).expect("muted")
        ))
    );

    std::fs::remove_dir_all(path.parent().expect("export directory")).expect("remove directory");
    pump(Duration::from_millis(220));
    assert_eq!(manager.provider.to_string(), first, "keep last valid");
    std::fs::create_dir(path.parent().expect("export directory")).expect("recreate directory");
    std::fs::write(&path, noctalia::tests::CSS.replace("#94cdf6", "#abcdef"))
        .expect("recreated export");
    pump(Duration::from_millis(350));
    assert_eq!(
        manager.current_tokens().expect("recovered tokens").accent,
        "#abcdef"
    );

    manager.preferences.borrow_mut().mode = "theme".to_owned();
    manager.apply_selected();
    let selected = manager.provider.to_string();
    std::fs::write(&path, noctalia::tests::CSS).expect("inactive update");
    pump(Duration::from_millis(220));
    assert_eq!(
        manager.provider.to_string(),
        selected,
        "explicit theme remains selected"
    );
    manager.preferences.borrow_mut().mode = "omarchy".to_owned();
    manager.set_follow_noctalia(false);
    assert!(
        manager.follows_omarchy(),
        "disabling an inactive follower must not overwrite another"
    );
    let weak = Rc::downgrade(&manager);
    drop(manager);
    assert!(
        weak.upgrade().is_none(),
        "watchers do not retain the manager"
    );
    pump(Duration::from_millis(200));
}
