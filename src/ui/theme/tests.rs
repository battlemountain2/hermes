// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    Preferences, blend, is_omarchy_theme_event, slugify, title_case_slug, tokens_from_quattro,
};

#[test]
fn hover_previews_default_on_and_round_trip_without_changing_older_preferences() {
    let defaults = Preferences::default();
    assert!(defaults.hover_previews);
    let mut serialized = serde_json::to_value(defaults).unwrap();
    serialized.as_object_mut().unwrap().remove("hover_previews");
    let legacy: Preferences = serde_json::from_value(serialized.clone()).unwrap();
    assert!(legacy.hover_previews);
    serialized["hover_previews"] = false.into();
    let disabled: Preferences = serde_json::from_value(serialized).unwrap();
    assert!(!disabled.hover_previews);
}

#[test]
#[ignore = "Requires isolated GTK display and private temporary preferences; no media"]
fn desktop_audio_preferences_reload_and_normalize_legacy_values() {
    gtk::init().unwrap();
    assert!(super::settings_path().starts_with(std::env::temp_dir()));
    let manager = super::ThemeManager::load();
    manager.set_audio_spin_animation(false);
    manager.set_audio_spectrum_bands(16);
    manager.set_audio_spectrum_decay("snappy");
    let restored = super::ThemeManager::load();
    assert!(!restored.audio_spin_animation());
    assert_eq!(restored.audio_spectrum_bands(), 16);
    assert_eq!(restored.audio_spectrum_decay(), "snappy");
    assert_eq!(
        crate::ui::audio_preferences::current(),
        crate::ui::audio_preferences::Options {
            spin: false,
            columns: 16,
            decay: crate::ui::audio_preferences::Decay::Snappy,
        }
    );
    restored.preferences.borrow_mut().audio_spectrum_bands = 64;
    restored.preferences.borrow_mut().audio_spectrum_decay = "retired".into();
    restored.preferences.borrow_mut().audio_continuity = false;
    restored.save_preferences();
    let legacy = super::ThemeManager::load();
    assert_eq!(legacy.audio_spectrum_bands(), 32);
    assert_eq!(legacy.audio_spectrum_decay(), "studio");
    assert!(!crate::ui::audio_preferences::current().spin);
    assert_eq!(crate::ui::audio_preferences::current().columns, 32);
    assert_eq!(crate::ui::audio_preferences::current().decay.id(), "studio");
    legacy.set_audio_spin_animation(true);
    legacy.set_audio_spectrum_bands(32);
    legacy.set_audio_spectrum_decay("studio");
}

#[test]
#[ignore = "Requires isolated GTK display and private temporary preferences"]
fn desktop_date_and_size_preferences_reload_into_the_cached_formatting_policy() {
    gtk::init().unwrap();
    assert!(super::settings_path().starts_with(std::env::temp_dir()));
    let manager = super::ThemeManager::load();
    manager.set_date_format("iso");
    manager.set_size_unit("binary");
    assert_eq!(crate::ui::formatting::format_size(1_048_576), "1 MiB");
    let restored = super::ThemeManager::load();
    assert_eq!(restored.date_format(), "iso");
    assert_eq!(restored.size_unit(), "binary");
    assert_eq!(crate::ui::formatting::format_size(1_048_576), "1 MiB");
    restored.set_date_format("relative");
    restored.set_size_unit("decimal");
    assert_eq!(crate::ui::formatting::format_size(1_048_576), "1 MB");
}

#[test]
#[ignore = "Requires isolated GTK display and private temporary settings"]
fn desktop_general_preferences_reload_and_validate_startup_choices() {
    gtk::init().unwrap();
    assert!(super::settings_path().starts_with(std::env::temp_dir()));
    let manager = super::ThemeManager::load();
    manager.set_startup_location("music");
    manager.set_startup_location("invalid-choice");
    assert_eq!(manager.startup_location(), "music");
    manager.set_confirm_permanent_delete(false);
    manager.set_show_hidden_default(true);
    manager.set_reduce_motion(true);
    assert!(!crate::ui::motion::animations_enabled());
    let reloaded = super::ThemeManager::load();
    assert_eq!(reloaded.startup_location(), "music");
    assert!(!reloaded.confirm_permanent_delete());
    assert!(reloaded.show_hidden_default());
    assert!(reloaded.reduce_motion());
    assert!(!crate::ui::motion::animations_enabled());
    reloaded.set_startup_location("last");
    reloaded.set_reduce_motion(false);
    let restored = super::ThemeManager::load();
    assert_eq!(restored.startup_location(), "last");
    assert!(!restored.reduce_motion());
    restored.preferences.borrow_mut().startup_location = "obsolete-mode".into();
    assert_eq!(restored.startup_location(), "home");
}

#[test]
fn filled_controls_choose_the_more_legible_semantic_foreground() {
    let mut tokens = super::azure_tokens();
    tokens.background = "#101010".into();
    tokens.text = "#f0f0f0".into();
    tokens.accent = "#aaddff".into();
    assert_eq!(super::on_accent_color(&tokens), tokens.background);
    assert!(super::tokens_css(&tokens, "mono").contains("@define-color theme_on_accent #101010;"));
    tokens.accent = "#203040".into();
    assert_eq!(super::on_accent_color(&tokens), tokens.text);
    tokens.accent = "rgb(210, 230, 255)".into();
    assert_eq!(super::on_accent_color(&tokens), tokens.background);
}

#[test]
#[ignore = "Requires an isolated GTK display; renders filled player controls in light and dark themes"]
fn desktop_filled_player_controls_render_with_live_theme_contrast() {
    use gtk::{gdk::prelude::*, prelude::*};
    gtk::init().unwrap();
    gtk::Settings::default().unwrap().set_gtk_enable_animations(false);
    crate::assets::prepare().unwrap();
    let manager = super::ThemeManager::shared();
    let styles = gtk::CssProvider::new();
    styles.load_from_string(include_str!("../../style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().unwrap(),
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);
    let mut buttons = Vec::new();
    for class in ["immersive-play-btn", "preview-audio-play-button", "mini-play-btn"] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        row.append(&gtk::Label::new(Some(class)));
        for state in [
            gtk::StateFlags::NORMAL,
            gtk::StateFlags::PRELIGHT,
            gtk::StateFlags::ACTIVE,
        ] {
            let button = gtk::Button::new();
            button.add_css_class(class);
            button.set_child(Some(&crate::assets::on_accent_icon(
                crate::assets::icons::PLAY,
                20,
            )));
            button.set_state_flags(state, false);
            row.append(&button);
            buttons.push(button);
        }
        content.append(&row);
    }
    let window = gtk::Window::builder()
        .default_width(700)
        .default_height(260)
        .child(&content)
        .build();
    let paintable = gtk::WidgetPaintable::new(Some(&content));
    window.present();
    for theme in ["graphite", "paper", "azure-glow"] {
        manager.select_theme(theme);
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
        while std::time::Instant::now() < deadline {
            gtk::glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let tokens = manager.applied_tokens().unwrap();
        let expected_color = gtk::gdk::RGBA::parse(super::on_accent_color(&tokens)).unwrap();
        let expected_icon = crate::assets::on_accent_icon(crate::assets::icons::PLAY, 20);
        for button in &buttons {
            assert_eq!(button.color(), expected_color);
            let image = button.child().and_downcast::<gtk::Image>().unwrap();
            assert_eq!(image.paintable(), expected_icon.paintable());
        }
        if let Ok(directory) = std::env::var("HERMES_PLAYER_CONTRAST_SHOTS") {
            content.queue_draw();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            let node = loop {
                for _ in 0..128 {
                    if !gtk::glib::MainContext::default().iteration(false) {
                        break;
                    }
                }
                let snapshot = gtk::Snapshot::new();
                window.allocate(700, 260, -1, None);
                paintable.snapshot(&snapshot, content.width() as f64, content.height() as f64);
                if let Some(node) = snapshot.to_node() {
                    break Some(node);
                }
                if std::time::Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            let Some(node) = node else {
                eprintln!("Optional screenshot unavailable for {theme}; no frame from isolated display");
                continue;
            };
            let renderer = gtk::gsk::CairoRenderer::new();
            renderer.realize(window.surface().as_ref()).unwrap();
            let texture = renderer.render_texture(&node, None);
            texture
                .save_to_png(
                    std::path::Path::new(&directory).join(format!("player-controls-{theme}.png")),
                )
                .unwrap();
            renderer.unrealize();
        }
    }
    window.close();
}

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
    assert!(!preferences.online_lyrics);
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
fn dolphin_preferences_default_safely_and_roundtrip() {
    let preferences: Preferences = toml::from_str(
        r#"
mode = "theme"
theme = "azure-glow"
"#,
    )
    .expect("defaults should be applied when omitted");

    assert_eq!(preferences.startup_location, "home");
    assert!(preferences.confirm_permanent_delete);
    assert!(!preferences.reduce_motion);
    assert_eq!(preferences.date_format, "relative");
    assert_eq!(preferences.size_unit, "decimal");
    assert!(!preferences.show_hidden_default);
    assert_eq!(preferences.max_preview_size_mb, 50);
    assert!(preferences.preview_audio);
    assert!(preferences.preview_images);
    assert!(preferences.preview_videos);
    assert!(preferences.preview_pdf);
    assert!(preferences.preview_spreadsheets);
    assert!(preferences.preview_code);
    assert_eq!(preferences.music_player_layout, "top");

    let serialized = toml::to_string(&preferences).expect("Preferences must serialize cleanly");
    let roundtripped: Preferences =
        toml::from_str(&serialized).expect("Preferences must deserialize after roundtrip");
    assert_eq!(roundtripped.startup_location, "home");
    assert_eq!(roundtripped.max_preview_size_mb, 50);
    assert!(roundtripped.confirm_permanent_delete);
    assert!(!roundtripped.reduce_motion);
    assert_eq!(roundtripped.date_format, "relative");
    assert_eq!(roundtripped.size_unit, "decimal");
    assert!(!roundtripped.show_hidden_default);
    assert_eq!(roundtripped.music_player_layout, "top");
}

#[test]
#[ignore = "requires a GTK display; tests theme manager layout observer"]
fn layout_observer_notified_on_music_player_layout_change() {
    gtk::init().unwrap();
    use super::ThemeManager;
    use std::cell::Cell;
    use std::rc::Rc;

    let manager = ThemeManager::shared();
    let notified = Rc::new(Cell::new(false));
    let flag = notified.clone();
    manager.observe_layout(move || {
        flag.set(true);
    });
    let original = manager.music_player_layout();
    let new_layout = if original == "top" { "bottom" } else { "top" };
    manager.set_music_player_layout(new_layout);
    assert!(notified.get());
    manager.set_music_player_layout(&original);
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
        layout_observers: RefCell::new(Vec::new()),
        lyrics_observers: RefCell::new(Vec::new()),
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
