// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, gio, prelude::*};

#[cfg(test)]
mod tests;

use crate::{
    assets::icons,
    services::{actions_path, create_custom_actions_template},
};

use super::{
    blur::BlurBin,
    browser::BrowserView,
    motion::set_reduce_motion,
    theme::{INTERFACE_FONTS, Theme, ThemeManager, ThemeTokens},
};

type ThemeCards = Rc<RefCell<Vec<(String, gtk::Button, gtk::Image)>>>;
type ThemeFollows = [gtk::Switch; 2];

pub fn build_layer(
    browser: &BrowserView,
    settings_button: &gtk::Button,
    root: &BlurBin,
    themes: Rc<ThemeManager>,
    refresh_sidebar: Rc<dyn Fn()>,
) -> gtk::Box {
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("settings-backdrop");
    layer.set_halign(gtk::Align::Fill);
    layer.set_valign(gtk::Align::Fill);
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    layer.set_focusable(true);
    layer.set_visible(false);

    let panel = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    panel.add_css_class("settings-dialog");
    panel.set_halign(gtk::Align::Center);
    panel.set_valign(gtk::Align::Center);
    panel.set_size_request(940, 640);

    let navigation = gtk::Box::new(gtk::Orientation::Vertical, 4);
    navigation.add_css_class("settings-navigation");
    append_heading(&navigation, "SETTINGS");

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.add_css_class("settings-page");
    page.set_hexpand(true);
    let titlebar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    titlebar.add_css_class("settings-titlebar");
    let title = gtk::Label::new(Some("General"));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("settings-title");
    let close = gtk::Button::builder()
        .tooltip_text("Close settings")
        .build();
    close.set_child(Some(&crate::assets::primary_icon(icons::X, 18)));
    close.add_css_class("settings-close");
    close.set_valign(gtk::Align::Center);
    titlebar.append(&title);
    titlebar.append(&close);
    page.append(&titlebar);

    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(120)
        .hexpand(true)
        .vexpand(true)
        .build();
    stack.add_named(
        &scroll_page(&general_page(browser, themes.clone(), refresh_sidebar)),
        Some("general"),
    );
    stack.add_named(
        &scroll_page(&audio_page(themes.clone())),
        Some("audio"),
    );
    stack.add_named(
        &scroll_page(&search_page(themes.clone())),
        Some("search"),
    );
    stack.add_named(
        &scroll_page(&context_menu_page(themes.clone())),
        Some("context-menu"),
    );
    stack.add_named(&scroll_page(&actions_page()), Some("actions"));
    stack.add_named(&theme_page(themes.clone()), Some("theme"));
    stack.add_named(&scroll_page(&fonts_page(themes)), Some("fonts"));
    stack.add_named(&keybindings_page(), Some("keybindings"));
    stack.add_named(&scroll_page(&diagnostics_page()), Some("diagnostics"));
    stack.add_named(&scroll_page(&about_page()), Some("about"));
    page.append(&stack);

    let nav_buttons: Rc<RefCell<Vec<gtk::Button>>> = Rc::new(RefCell::new(Vec::new()));
    for (label, icon, name) in [
        ("General", icons::SLIDERS, "general"),
        ("Audio & DSP", icons::MUSIC, "audio"),
        ("Search & index", icons::SEARCH, "search"),
        ("Context menu", icons::LIST, "context-menu"),
        ("Custom actions", icons::SETTINGS_2, "actions"),
        ("Themes", icons::PALETTE, "theme"),
        ("Fonts", icons::FILE_CODE, "fonts"),
        ("Keyboard shortcuts", icons::KEYBOARD, "keybindings"),
        ("Diagnostics", icons::TERMINAL, "diagnostics"),
        ("About", icons::INFO, "about"),
    ] {
        let button = navigation_button(icon, label);
        if name == "general" {
            button.add_css_class("settings-nav-active");
        }
        nav_buttons.borrow_mut().push(button.clone());
        let buttons = nav_buttons.clone();
        let stack = stack.clone();
        let title = title.clone();
        let page_title = label.to_owned();
        button.connect_clicked(move |clicked| {
            for candidate in buttons.borrow().iter() {
                candidate.remove_css_class("settings-nav-active");
            }
            clicked.add_css_class("settings-nav-active");
            stack.set_visible_child_name(name);
            title.set_text(&page_title);
        });
        navigation.append(&button);
    }

    panel.append(&navigation);
    panel.append(&page);
    let top_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    top_spacer.set_vexpand(true);
    let bottom_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom_spacer.set_vexpand(true);
    layer.append(&top_spacer);
    layer.append(&panel);
    layer.append(&bottom_spacer);

    let hidden_layer = layer.clone();
    let inactive_settings = settings_button.clone();
    let unblurred_root = root.clone();
    close.connect_clicked(move |_| hide(&hidden_layer, &inactive_settings, &unblurred_root));
    let hidden_layer = layer.clone();
    let inactive_settings = settings_button.clone();
    let unblurred_root = root.clone();
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key != gdk::Key::Escape {
            return gtk::glib::Propagation::Proceed;
        }
        hide(&hidden_layer, &inactive_settings, &unblurred_root);
        gtk::glib::Propagation::Stop
    });
    layer.add_controller(keys);
    layer
}

fn hide(layer: &gtk::Box, button: &gtk::Button, root: &BlurBin) {
    layer.set_visible(false);
    root.set_blurred(false);
    button.remove_css_class("active");
}

fn context_menu_page(manager: Rc<ThemeManager>) -> gtk::Widget {
    let content = page_content();
    append_heading(&content, "RIGHT-CLICK ACTIONS");
    let help = gtk::Label::new(Some(
        "Cut, Copy, Paste, Copy location, Rename, Trash and Properties stay available where applicable. Optional actions appear only for supported selections; keyboard shortcuts are unchanged.",
    ));
    help.set_wrap(true);
    help.set_xalign(0.0);
    help.add_css_class("settings-option-description");
    content.append(&help);
    let options = manager.context_menu_options();
    let controls = Rc::new(
        vec![
            (
                "Open With",
                "Choose another application for a file.",
                options.open_with,
            ),
            (
                "Quick preview",
                "Preview a supported file without opening it externally.",
                options.preview,
            ),
            ("Pin to sidebar", "Available on folders.", options.pin),
            (
                "Copy to / Move to",
                "Optional destination dialogs, separate from clipboard Copy and Cut.",
                options.transfer_destinations,
            ),
            (
                "Compress / Extract",
                "Compress folders and extract supported archives.",
                options.archives,
            ),
            (
                "Compress individual files",
                "Also offer compression on files and mixed selections.",
                options.compress_files,
            ),
            (
                "Music actions",
                "Play and queue selected tracks.",
                options.music,
            ),
            (
                "Custom commands",
                "Show matching commands configured on the Custom actions page.",
                options.custom_actions,
            ),
        ]
        .into_iter()
        .map(|(title, detail, enabled)| {
            let (row, toggle) = settings_option(title, detail, enabled);
            content.append(&row);
            toggle
        })
        .collect::<Vec<_>>(),
    );
    for control in controls.iter() {
        let weak_controls = Rc::downgrade(&controls);
        let manager = manager.clone();
        control.connect_active_notify(move |_| {
            if let Some(controls) = weak_controls.upgrade() {
                manager.set_context_menu_options(super::context_menu::Options {
                    open_with: controls[0].is_active(),
                    preview: controls[1].is_active(),
                    pin: controls[2].is_active(),
                    transfer_destinations: controls[3].is_active(),
                    archives: controls[4].is_active(),
                    compress_files: controls[5].is_active(),
                    music: controls[6].is_active(),
                    custom_actions: controls[7].is_active(),
                });
            }
        });
    }
    let defaults = gtk::Button::with_label("Restore context menu defaults");
    defaults.add_css_class("settings-font-reset");
    defaults.set_halign(gtk::Align::Start);
    defaults.connect_clicked(move |_| {
        let options = super::context_menu::Options::default();
        for (toggle, enabled) in controls.iter().zip([
            options.open_with,
            options.preview,
            options.pin,
            options.transfer_destinations,
            options.archives,
            options.compress_files,
            options.music,
            options.custom_actions,
        ]) {
            toggle.set_active(enabled);
        }
        manager.set_context_menu_options(options);
    });
    content.append(&defaults);
    content.upcast()
}

fn general_page(
    browser: &BrowserView,
    manager: Rc<ThemeManager>,
    refresh_sidebar: Rc<dyn Fn()>,
) -> gtk::Widget {
    let preferences = page_content();

    append_heading(&preferences, "BROWSING");
    let browsing_card = settings_card();

    let (peeking_row, peeking) = settings_option(
        "Folder peeking",
        "Preview folders automatically while moving through a pane.",
        true,
    );
    let browser_for_peeking = browser.clone();
    peeking.connect_active_notify(move |toggle| {
        browser_for_peeking.set_peek_enabled(toggle.is_active())
    });
    browsing_card.append(&peeking_row);

    let single_click_enabled = manager.single_click_previews();
    browser.set_single_click_previews(single_click_enabled);
    let (preview_row, single_click_previews) = settings_option(
        "Single-click file previews",
        "Show a quick preview when selecting a supported file.",
        single_click_enabled,
    );
    let browser_for_previews = browser.clone();
    let manager_for_previews = manager.clone();
    single_click_previews.connect_active_notify(move |toggle| {
        let enabled = toggle.is_active();
        browser_for_previews.set_single_click_previews(enabled);
        manager_for_previews.set_single_click_previews(enabled);
    });
    browsing_card.append(&preview_row);

    let direct_open_enabled = manager.search_open_files_directly();
    let (search_open_row, search_open_files) = settings_option(
        "Open search results directly",
        "Launch files from search instead of opening Hermes' quick preview.",
        direct_open_enabled,
    );
    let manager_for_search = manager.clone();
    search_open_files.connect_active_notify(move |toggle| {
        manager_for_search.set_search_open_files_directly(toggle.is_active());
    });
    browsing_card.append(&search_open_row);
    preferences.append(&browsing_card);

    append_heading(&preferences, "SIDEBAR & HISTORY");
    let sidebar_card = settings_card();

    let recent_enabled = manager.show_recent_files();
    let (recent_row, recent_files) = settings_option(
        "Recent files in sidebar",
        "Show optional Today and Yesterday views backed by the local search index.",
        recent_enabled,
    );
    let manager_for_recent = manager.clone();
    let refresh_sidebar_recent = refresh_sidebar.clone();
    recent_files.connect_active_notify(move |toggle| {
        manager_for_recent.set_show_recent_files(toggle.is_active());
        refresh_sidebar_recent();
    });
    sidebar_card.append(&recent_row);

    let recent_folders_enabled = manager.show_recent_folders();
    let (recent_folders_row, recent_folders) = settings_option(
        "Recent folders in sidebar",
        "Show recently visited locations in the sidebar for quick navigation.",
        recent_folders_enabled,
    );
    let manager_for_recent_folders = manager.clone();
    let refresh_sidebar_recent_folders = refresh_sidebar.clone();
    recent_folders.connect_active_notify(move |toggle| {
        manager_for_recent_folders.set_show_recent_folders(toggle.is_active());
        refresh_sidebar_recent_folders();
    });
    sidebar_card.append(&recent_folders_row);

    let recent_folders_limit = manager.recent_folders_limit();
    let (recent_folders_limit_row, recent_folders_limit_spin) = settings_spin(
        "Recent folders limit",
        "Maximum number of recent folders to display in the sidebar (1-12).",
        recent_folders_limit as f64,
        1.0,
        12.0,
        1.0,
    );
    let manager_for_recent_limit = manager.clone();
    let refresh_sidebar_recent_limit = refresh_sidebar.clone();
    recent_folders_limit_spin.connect_value_changed(move |spin| {
        manager_for_recent_limit.set_recent_folders_limit(spin.value() as u32);
        refresh_sidebar_recent_limit();
    });
    sidebar_card.append(&recent_folders_limit_row);
    preferences.append(&sidebar_card);

    append_heading(&preferences, "LAYOUT & MOTION");
    let layout_card = settings_card();

    let folder_tree_enabled = manager.show_folder_tree();
    let (folder_tree_row, folder_tree) = settings_option(
        "Show folder tree panel",
        "Display a classic folder tree hierarchy next to the browser.",
        folder_tree_enabled,
    );
    let manager_for_tree = manager.clone();
    let refresh_sidebar_tree = refresh_sidebar.clone();
    folder_tree.connect_active_notify(move |toggle| {
        manager_for_tree.set_show_folder_tree(toggle.is_active());
        refresh_sidebar_tree();
    });
    layout_card.append(&folder_tree_row);

    let (motion_row, reduce_motion) = settings_option(
        "Reduce motion",
        "Disable nonessential interface animations.",
        false,
    );
    reduce_motion.connect_active_notify(|toggle| set_reduce_motion(toggle.is_active()));
    layout_card.append(&motion_row);
    preferences.append(&layout_card);

    preferences.upcast()
}

fn audio_page(manager: Rc<ThemeManager>) -> gtk::Widget {
    let content = page_content();

    append_heading(&content, "PLAYBACK CONTINUITY");
    let card1 = settings_card();
    let (continuity_row, continuity) = settings_option(
        "Background playback continuity",
        "Keep audio playing smoothly when switching between files, folders, and tabs.",
        manager.audio_continuity(),
    );
    let manager_for_continuity = manager.clone();
    continuity.connect_active_notify(move |toggle| {
        manager_for_continuity.set_audio_continuity(toggle.is_active());
    });
    card1.append(&continuity_row);

    let (spin_row, spin) = settings_option(
        "Mechanical hero animation",
        "Subtle vinyl disc and tape reel rotation in the player during playback.",
        manager.audio_spin_animation(),
    );
    let manager_for_spin = manager.clone();
    spin.connect_active_notify(move |toggle| {
        manager_for_spin.set_audio_spin_animation(toggle.is_active());
    });
    card1.append(&spin_row);
    content.append(&card1);

    append_heading(&content, "DSP & SPECTRUM ENGINE");
    let card2 = settings_card();
    let bands_choices = [
        "16 bands · Fast low-latency",
        "32 bands · Studio default",
        "64 bands · High definition",
    ];
    let selected_bands_idx = match manager.audio_spectrum_bands() {
        16 => 0,
        64 => 2,
        _ => 1,
    };
    let (bands_row, bands_choice) = settings_choice(
        "Spectrum frequency resolution",
        "Discrete log-spaced frequency bands analyzed and rendered in real-time.",
        &bands_choices,
        selected_bands_idx,
    );
    let manager_for_bands = manager.clone();
    bands_choice.connect_selected_notify(move |dropdown| {
        let bands = match dropdown.selected() {
            0 => 16,
            2 => 64,
            _ => 32,
        };
        manager_for_bands.set_audio_spectrum_bands(bands);
    });
    card2.append(&bands_row);

    let decay_choices = [
        "Studio ballistics · Standard hold & decay",
        "Fast response · Snappy peak return",
        "Smooth decay · Cinematic glide",
    ];
    let selected_decay_idx = match manager.audio_spectrum_decay().as_str() {
        "snappy" => 1,
        "smooth" => 2,
        _ => 0,
    };
    let (decay_row, decay_choice) = settings_choice(
        "Ballistics & meter falloff",
        "Peak-hold timing and decay rate for visualizer segment meters.",
        &decay_choices,
        selected_decay_idx,
    );
    let manager_for_decay = manager.clone();
    decay_choice.connect_selected_notify(move |dropdown| {
        let decay = match dropdown.selected() {
            1 => "snappy",
            2 => "smooth",
            _ => "studio",
        };
        manager_for_decay.set_audio_spectrum_decay(decay);
    });
    card2.append(&decay_row);
    content.append(&card2);

    append_heading(&content, "DSP PIPELINE TELEMETRY");
    let telemetry_card = settings_card();
    append_telemetry_row(&telemetry_card, "Decoder core", "Symphonia v0.5.4 · Pure Rust media framework");
    append_telemetry_row(&telemetry_card, "Audio output", "Rodio ALSA/PulseAudio/PipeWire backend");
    append_telemetry_row(&telemetry_card, "Internal format", "32-bit float PCM (f32) · Dual-channel stereo");
    append_telemetry_row(&telemetry_card, "Filterbank", "Log-spaced 20 Hz – 20,000 Hz · 60 fps GTK frame-clock");
    append_telemetry_row(&telemetry_card, "Lossless codecs", "FLAC (16/24-bit), WAV, ALAC");
    append_telemetry_row(&telemetry_card, "Lossy codecs", "MP3, Vorbis, AAC, Opus, M4A");
    content.append(&telemetry_card);

    content.upcast()
}

fn search_page(manager: Rc<ThemeManager>) -> gtk::Widget {
    let content = page_content();

    append_heading(&content, "CATALOG INDEXES");
    let card1 = settings_card();
    let music_dir = gtk::glib::user_special_dir(gtk::glib::UserDirectory::Music)
        .unwrap_or_else(|| gtk::glib::home_dir().join("Music"));
    append_catalog_status_row(
        &card1,
        "Music library catalog",
        &music_dir.display().to_string(),
        "Active · SQLite snapshot",
    );
    let home_dir = gtk::glib::home_dir();
    append_catalog_status_row(
        &card1,
        "Home fast-search index",
        &home_dir.display().to_string(),
        "Instant candidate cache",
    );
    content.append(&card1);

    append_heading(&content, "INDEX EXCLUSIONS & FILTERS");
    let card2 = settings_card();
    let (hidden_row, hidden_toggle) = settings_option(
        "Exclude hidden files and folders",
        "Skip dotfiles and hidden trees (e.g. .git, .cache, .config) during index walks.",
        manager.search_exclude_hidden(),
    );
    let manager_for_hidden = manager.clone();
    hidden_toggle.connect_active_notify(move |toggle| {
        manager_for_hidden.set_search_exclude_hidden(toggle.is_active());
    });
    card2.append(&hidden_row);

    let (artifacts_row, artifacts_toggle) = settings_option(
        "Exclude development build artifacts",
        "Ignore high-churn directories like node_modules, target, dist, build, and .venv.",
        manager.search_exclude_build_artifacts(),
    );
    let manager_for_artifacts = manager.clone();
    artifacts_toggle.connect_active_notify(move |toggle| {
        manager_for_artifacts.set_search_exclude_build_artifacts(toggle.is_active());
    });
    card2.append(&artifacts_row);

    let (reconcile_row, reconcile_toggle) = settings_option(
        "Periodic index reconciliation",
        "Silently synchronize index with disk changes when idle.",
        manager.search_reconcile_periodically(),
    );
    let manager_for_reconcile = manager.clone();
    reconcile_toggle.connect_active_notify(move |toggle| {
        manager_for_reconcile.set_search_reconcile_periodically(toggle.is_active());
    });
    card2.append(&reconcile_row);
    content.append(&card2);

    append_heading(&content, "MAINTENANCE");
    let card3 = settings_card();
    let maintenance_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    maintenance_row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some("Re-index Music Library"));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let desc = gtk::Label::new(Some(
        "Force a rescan of your Music directory and regenerate the SQLite tag cache.",
    ));
    desc.set_xalign(0.0);
    desc.set_wrap(true);
    desc.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&desc);

    let reindex_button = gtk::Button::with_label("Rebuild index");
    reindex_button.add_css_class("settings-action-button");
    reindex_button.set_valign(gtk::Align::Center);
    let status_badge = gtk::Label::new(Some("Ready"));
    status_badge.add_css_class("settings-telemetry-badge");
    status_badge.set_valign(gtk::Align::Center);

    let weak_badge = status_badge.downgrade();
    reindex_button.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        if let Some(badge) = weak_badge.upgrade() {
            badge.set_text("Re-indexing...");
        }
        let weak_badge = weak_badge.clone();
        let weak_btn = btn.downgrade();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
            if let Some(badge) = weak_badge.upgrade() {
                badge.set_text("Up to date");
            }
            if let Some(btn) = weak_btn.upgrade() {
                btn.set_sensitive(true);
            }
        });
    });

    maintenance_row.append(&copy);
    maintenance_row.append(&status_badge);
    maintenance_row.append(&reindex_button);
    card3.append(&maintenance_row);
    content.append(&card3);

    content.upcast()
}

fn diagnostics_page() -> gtk::Widget {
    let content = page_content();

    append_heading(&content, "DISPLAY & COMPOSITOR");
    let card1 = settings_card();
    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| {
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            "wayland".to_owned()
        } else {
            "x11".to_owned()
        }
    });
    let compositor = std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("DESKTOP_SESSION"))
        .unwrap_or_else(|_| "Unknown".to_owned());
    append_telemetry_row(&card1, "Display protocol", &session_type.to_uppercase());
    append_telemetry_row(&card1, "Compositor / Desktop", &compositor);
    append_telemetry_row(
        &card1,
        "GTK version",
        &format!("{}.{}.{}", gtk::major_version(), gtk::minor_version(), gtk::micro_version()),
    );
    append_telemetry_row(&card1, "Graphics renderer", "Cairo 2D vector pipeline");
    content.append(&card1);

    append_heading(&content, "STORAGE & PATHS");
    let card2 = settings_card();
    let config_dir = gtk::glib::user_config_dir().join("strata");
    let cache_dir = gtk::glib::user_cache_dir().join("strata");
    let data_dir = gtk::glib::user_data_dir().join("strata");
    append_telemetry_row(&card2, "Configuration directory", &config_dir.display().to_string());
    append_telemetry_row(&card2, "Cache directory", &cache_dir.display().to_string());
    append_telemetry_row(&card2, "Data directory", &data_dir.display().to_string());
    content.append(&card2);

    append_heading(&content, "CACHE MANAGEMENT");
    let card3 = settings_card();
    let thumb_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    thumb_row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some("Thumbnail cache"));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let thumb_dir = gtk::glib::user_cache_dir().join("strata/thumbnails");
    let desc = gtk::Label::new(Some(&format!(
        "Cached album art and image previews in {}",
        thumb_dir.display()
    )));
    desc.set_xalign(0.0);
    desc.set_wrap(true);
    desc.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&desc);

    let clear_btn = gtk::Button::with_label("Clear thumbnails");
    clear_btn.add_css_class("settings-action-button");
    clear_btn.set_valign(gtk::Align::Center);
    let badge = gtk::Label::new(Some("Cached"));
    badge.add_css_class("settings-telemetry-badge");
    badge.set_valign(gtk::Align::Center);

    let weak_badge = badge.downgrade();
    clear_btn.connect_clicked(move |btn| {
        let thumb_path = gtk::glib::user_cache_dir().join("strata/thumbnails");
        let _ = std::fs::remove_dir_all(&thumb_path);
        let _ = std::fs::create_dir_all(&thumb_path);
        if let Some(b) = weak_badge.upgrade() {
            b.set_text("Cleared");
        }
        btn.set_sensitive(false);
    });

    thumb_row.append(&copy);
    thumb_row.append(&badge);
    thumb_row.append(&clear_btn);
    card3.append(&thumb_row);
    content.append(&card3);

    content.upcast()
}

fn scroll_page(content: &gtk::Widget) -> gtk::Widget {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .child(content)
        .build()
        .upcast()
}

fn fonts_page(manager: Rc<ThemeManager>) -> gtk::Widget {
    let content = page_content();
    append_heading(&content, "TYPOGRAPHY");
    let names: Vec<_> = INTERFACE_FONTS.iter().map(|(_, name)| *name).collect();
    let selected = INTERFACE_FONTS
        .iter()
        .position(|(id, _)| *id == manager.interface_font())
        .unwrap_or(0);
    let (row, family) = settings_choice(
        "Interface font",
        "Bundled monospace fonts or your system's default font. Updates immediately.",
        &names,
        selected as u32,
    );
    content.append(&row);
    let manager_for_font = manager.clone();
    family.connect_selected_notify(move |font| {
        if let Some((id, _)) = INTERFACE_FONTS.get(font.selected() as usize) {
            manager_for_font.set_interface_font(id);
        }
    });
    let size_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    size_row.add_css_class("settings-option");
    let label = gtk::Label::new(Some("Interface size (px)"));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.add_css_class("settings-option-title");
    let size = gtk::SpinButton::with_range(10.0, 22.0, 1.0);
    size.set_value(f64::from(manager.interface_font_size()));
    size.set_valign(gtk::Align::Center);
    let manager_for_size = manager.clone();
    size.connect_value_changed(move |size| {
        manager_for_size.set_interface_font_size(size.value_as_int() as u32)
    });
    size_row.append(&label);
    size_row.append(&size);
    content.append(&size_row);
    append_heading(&content, "PREVIEW");
    let sample = gtk::Label::new(Some(
        "Hermes · Files, media & music\nThe quick brown fox jumps over the lazy dog.\n0123456789 — Aa Bb Cc",
    ));
    sample.set_xalign(0.0);
    sample.set_wrap(true);
    sample.set_selectable(true);
    sample.add_css_class("settings-font-preview");
    content.append(&sample);
    let reset = gtk::Button::with_label("Restore default font and size");
    reset.add_css_class("settings-font-reset");
    reset.set_halign(gtk::Align::Start);
    let weak_family = family.downgrade();
    let weak_size = size.downgrade();
    reset.connect_clicked(move |_| {
        manager.set_interface_font("maple");
        manager.set_interface_font_size(13);
        if let Some(family) = weak_family.upgrade() {
            family.set_selected(0);
        }
        if let Some(size) = weak_size.upgrade() {
            size.set_value(13.0);
        }
    });
    content.append(&reset);
    content.upcast()
}

fn keybindings_page() -> gtk::Widget {
    let content = page_content();
    let help = gtk::Label::new(Some(
        "Current shortcuts · Navigation keys apply in file views, not while typing. F opens supported previews fullscreen.",
    ));
    help.set_wrap(true);
    help.set_xalign(0.0);
    help.add_css_class("settings-option-description");
    content.append(&help);

    append_heading(&content, "NAVIGATION");
    let card1 = settings_card();
    for (label, keys) in [
        ("Move through items", "J / K  or  ↑ / ↓"),
        ("Open folder", "L / → / Enter"),
        ("Go to parent", "H / ←"),
        ("Back / forward", "Alt + ← / →"),
        ("Parent folder", "Alt + ↑ / Backspace"),
        ("Home", "Alt + Home"),
        ("Edit location", "Ctrl + L"),
        ("Filter items", "Ctrl + F"),
        ("Toggle sidebar", "Ctrl + B"),
        ("Show hidden files", "Ctrl + H"),
    ] {
        append_keybinding(&card1, label, keys);
    }
    content.append(&card1);

    append_heading(&content, "FILE OPERATIONS");
    let card2 = settings_card();
    for (label, keys) in [
        ("Create new folder", "Ctrl + Shift + N"),
        ("Cut", "Ctrl + X"),
        ("Copy", "Ctrl + C"),
        ("Paste", "Ctrl + V"),
        ("Select all", "Ctrl + A"),
        ("Rename", "F2"),
        ("Move to Trash", "Delete"),
        ("Delete permanently (confirmation)", "Shift + Delete"),
        ("Properties", "Alt + Enter"),
    ] {
        append_keybinding(&card2, label, keys);
    }
    content.append(&card2);

    append_heading(&content, "APPLICATION");
    let card3 = settings_card();
    for (label, keys) in [
        ("Search Home", "Ctrl + K"),
        ("Search Music library", "Ctrl + Shift + K"),
        ("Refresh search index", "Ctrl + R (in search)"),
        ("Open settings", "Ctrl + ,"),
    ] {
        append_keybinding(&card3, label, keys);
    }
    content.append(&card3);

    append_heading(&content, "MEDIA & MUSIC");
    let card4 = settings_card();
    for (label, keys) in [
        ("Preview selected file", "Space"),
        ("Fullscreen image, video or PDF", "F"),
        ("Leave fullscreen / close preview", "Escape"),
        ("Toggle Music screen", "Ctrl + M"),
        ("Play selected music in main player", "Ctrl + Enter"),
        ("Track or album actions", "Menu / Shift + F10"),
    ] {
        append_keybinding(&card4, label, keys);
    }
    content.append(&card4);

    append_heading(&content, "TABS");
    let card5 = settings_card();
    for (label, keys) in [
        ("New tab", "Ctrl + T"),
        ("Close tab", "Ctrl + W"),
        ("Next tab", "Ctrl + Tab"),
        ("Previous tab", "Ctrl + Shift + Tab"),
    ] {
        append_keybinding(&card5, label, keys);
    }
    content.append(&card5);

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .child(&content)
        .build();
    scroller.add_css_class("settings-keybindings-scroll");
    scroller.upcast()
}

fn actions_page() -> gtk::Widget {
    let content = page_content();
    append_heading(&content, "CONTEXT MENU ACTIONS");
    let card = settings_card();

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 3);
    copy.set_hexpand(true);
    let title = gtk::Label::new(Some("Custom actions file"));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(
        "Add safe, argument-based commands to the file and folder context menu. Restart Hermes after editing the file.",
    ));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-option-description");
    let path = gtk::Label::new(Some(&actions_path().display().to_string()));
    path.set_xalign(0.0);
    path.set_selectable(true);
    path.add_css_class("keybinding-keys");
    copy.append(&title);
    copy.append(&description);
    copy.append(&path);
    let open = gtk::Button::with_label(if actions_path().is_file() {
        "Open file"
    } else {
        "Create file"
    });
    open.add_css_class("theme-editor-save");
    open.set_valign(gtk::Align::Center);
    row.append(&copy);
    row.append(&open);
    card.append(&row);
    content.append(&card);

    let help = gtk::Label::new(Some(
        "Placeholders: {path} for the first item, {paths} for every selected item, {parent} for its folder, and {name} for its filename. Commands are never run through a shell.",
    ));
    help.set_xalign(0.0);
    help.set_wrap(true);
    help.add_css_class("settings-option-description");
    content.append(&help);

    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.add_css_class("theme-editor-error");
    error.set_visible(false);
    content.append(&error);
    open.connect_clicked(move |button| match create_custom_actions_template() {
        Ok(path) => {
            error.set_visible(false);
            button.set_label("Open file");
            let file = gio::File::for_path(path);
            if let Err(launch_error) =
                gio::AppInfo::launch_default_for_uri(&file.uri(), None::<&gio::AppLaunchContext>)
            {
                error.set_text(&format!("Unable to open the actions file: {launch_error}"));
                error.set_visible(true);
            }
        }
        Err(message) => {
            error.set_text(&message);
            error.set_visible(true);
        }
    });

    content.upcast()
}

fn about_page() -> gtk::Widget {
    let content = page_content();
    content.add_css_class("about-page");

    let identity = gtk::Box::new(gtk::Orientation::Vertical, 7);
    identity.add_css_class("about-identity");
    identity.set_halign(gtk::Align::Center);

    let name = gtk::Label::new(Some("Hermes"));
    name.add_css_class("about-name");
    let description = gtk::Label::new(Some(
        "Your files, media and music in one place.\nBuilt for Linux, with a focused interface and native playback.",
    ));
    description.add_css_class("about-description");
    description.set_tooltip_text(Some(crate::build_info::DESCRIPTION));
    description.set_justify(gtk::Justification::Center);
    description.set_wrap(true);
    identity.append(&name);
    identity.append(&description);
    content.append(&identity);

    append_heading(&content, "BUILD INFORMATION");
    let build = settings_card();
    append_about_detail(&build, "Version", crate::build_info::VERSION, false);
    append_about_detail(&build, "Status", "In development · Pre-1.0", false);
    append_about_detail(&build, "Commit", crate::build_info::COMMIT, true);
    append_about_detail(&build, "License", "GPL-3.0-or-later", false);
    content.append(&build);

    append_heading(&content, "HERMES PROJECT");
    let project = settings_card();
    append_about_detail(&project, "Hermes author", crate::build_info::AUTHOR, false);
    project.append(&repository_button(
        crate::build_info::REPOSITORY,
        "Hermes repository",
        "Open the Hermes repository",
    ));
    content.append(&project);

    append_heading(&content, "ACKNOWLEDGMENTS");
    let original = settings_card();
    append_about_detail(&original, "Original author", "LGSE Ltd.", false);
    original.append(&repository_button(
        "https://github.com/LGSE/strata",
        "Original Strata repository",
        "Open the original Strata repository",
    ));
    content.append(&original);

    content.upcast()
}

fn repository_button(uri: &str, label: &str, tooltip: &str) -> gtk::LinkButton {
    let repository = gtk::LinkButton::builder()
        .uri(uri)
        .tooltip_text(tooltip)
        .build();
    repository.add_css_class("about-repository");
    let repository_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let repository_label = gtk::Label::new(Some(label));
    repository_label.set_xalign(0.0);
    repository_label.set_hexpand(true);
    repository_content.append(&repository_label);
    repository_content.append(&crate::assets::primary_icon(icons::EXTERNAL_LINK, 16));
    repository.set_child(Some(&repository_content));
    repository
}

fn append_about_detail(container: &gtk::Box, label: &str, value: &str, monospace: bool) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("about-detail-row");
    let label = gtk::Label::new(Some(label));
    label.add_css_class("about-detail-label");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    let value = gtk::Label::new(Some(value));
    value.add_css_class("about-detail-value");
    value.set_selectable(true);
    if monospace {
        value.add_css_class("monospace");
    }
    row.append(&label);
    row.append(&value);
    container.append(&row);
}

fn append_keybinding(content: &gtk::Box, label: &str, keys: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("keybinding-row");
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    let keys = gtk::Label::new(Some(keys));
    keys.add_css_class("keybinding-keys");
    row.append(&label);
    row.append(&keys);
    content.append(&row);
}

fn theme_page(manager: Rc<ThemeManager>) -> gtk::Widget {
    let content = page_content();
    content.add_css_class("theme-page");

    let follow = gtk::Switch::builder()
        .active(manager.follows_omarchy())
        .valign(gtk::Align::Center)
        .build();
    append_heading(&content, "SYSTEM");
    if manager.is_omarchy_available() {
        let system = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        system.add_css_class("settings-option");
        let icon = crate::assets::primary_icon(icons::MONITOR, 22);
        icon.add_css_class("system-theme-icon");
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
        copy.set_hexpand(true);
        copy.set_valign(gtk::Align::Center);
        let system_title = gtk::Label::new(Some("Follow Omarchy"));
        system_title.set_xalign(0.0);
        system_title.add_css_class("settings-option-title");
        let system_description = gtk::Label::new(Some(
            "Use the active Omarchy Quattro theme and follow system theme changes.",
        ));
        system_description.set_xalign(0.0);
        system_description.set_wrap(true);
        system_description.add_css_class("settings-option-description");
        copy.append(&system_title);
        copy.append(&system_description);
        system.append(&icon);
        system.append(&copy);
        system.append(&follow);
        content.append(&system);
    }

    let description = if manager.is_noctalia_available() {
        "Follow Noctalia's exported GTK colors live. Noctalia settings are not changed."
    } else {
        "Enable GTK 4 in Noctalia's Templates settings to export colors. Until then, keep your selected theme."
    };
    let (noctalia_row, noctalia_follow) =
        settings_option("Follow Noctalia", description, manager.follows_noctalia());
    content.append(&noctalia_row);
    let follows = [follow, noctalia_follow];

    append_heading(&content, "THEMES");
    let packaged = gtk::FlowBox::builder()
        .column_spacing(12)
        .row_spacing(12)
        .max_children_per_line(3)
        .min_children_per_line(2)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    packaged.add_css_class("theme-grid");
    content.append(&packaged);

    append_heading(&content, "YOUR THEMES");
    let custom = gtk::FlowBox::builder()
        .column_spacing(12)
        .row_spacing(12)
        .max_children_per_line(3)
        .min_children_per_line(2)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    custom.add_css_class("theme-grid");
    content.append(&custom);

    let cards: ThemeCards = Rc::new(RefCell::new(Vec::new()));
    for theme in manager.themes() {
        let flow = if theme.custom { &custom } else { &packaged };
        append_theme_card(flow, theme, &manager, &follows, &cards);
    }

    let add = gtk::Button::new();
    add.add_css_class("add-theme-card");
    add.set_has_frame(false);
    let add_content = gtk::Box::new(gtk::Orientation::Vertical, 7);
    add_content.set_halign(gtk::Align::Center);
    add_content.set_valign(gtk::Align::Center);
    let plus = crate::assets::primary_icon(icons::PLUS, 22);
    let add_label = gtk::Label::new(Some("Add a theme"));
    add_content.append(&plus);
    add_content.append(&add_label);
    add.set_child(Some(&add_content));
    custom.insert(&add, -1);

    let editor = theme_editor(manager.clone(), custom, follows.clone(), cards.clone());
    editor.set_reveal_child(false);
    content.append(&editor);
    let shown_editor = editor.clone();
    add.connect_clicked(move |_| shown_editor.set_reveal_child(true));

    let scroller = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .build();
    for (index, follow) in follows.iter().enumerate() {
        let manager = manager.clone();
        let other_follow = follows[1 - index].downgrade();
        let cards = cards.clone();
        follow.connect_active_notify(move |toggle| {
            let active = toggle.is_active();
            if index == 0 {
                manager.set_follow_omarchy(active);
            } else {
                manager.set_follow_noctalia(active);
            }
            if active && let Some(other_follow) = other_follow.upgrade() {
                other_follow.set_active(false);
            }
            refresh_theme_cards(&manager, &cards);
        });
    }
    scroller.upcast()
}

fn refresh_theme_cards(manager: &ThemeManager, cards: &ThemeCards) {
    let selected_id = manager.selected_id();
    for (id, card, check) in cards.borrow().iter() {
        let selected =
            !manager.follows_omarchy() && !manager.follows_noctalia() && id == &selected_id;
        if selected {
            card.add_css_class("selected");
        } else {
            card.remove_css_class("selected");
        }
        check.set_visible(selected);
    }
}

fn append_theme_card(
    flow: &gtk::FlowBox,
    theme: Theme,
    manager: &Rc<ThemeManager>,
    follows: &ThemeFollows,
    cards: &ThemeCards,
) {
    let card = gtk::Button::new();
    card.add_css_class("theme-card");
    card.set_has_frame(false);
    card.set_overflow(gtk::Overflow::Visible);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let preview = gtk::Overlay::new();
    preview.set_child(Some(&theme_preview(&theme.tokens)));
    let check = gtk::Image::from_icon_name(icons::CHECK_ON_PRIMARY);
    check.add_css_class("theme-card-check");
    check.set_halign(gtk::Align::End);
    check.set_valign(gtk::Align::Start);
    check.set_margin_top(8);
    check.set_margin_end(8);
    check.set_pixel_size(10);
    preview.add_overlay(&check);
    content.append(&preview);
    let label_row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    let selected = !manager.follows_omarchy()
        && !manager.follows_noctalia()
        && manager.selected_id() == theme.id;
    check.set_visible(selected);
    if selected {
        card.add_css_class("selected");
    }
    let label = gtk::Label::new(Some(&theme.tokens.name));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label_row.append(&label);
    content.append(&label_row);
    card.set_child(Some(&content));
    cards
        .borrow_mut()
        .push((theme.id.clone(), card.clone(), check));

    let theme_id = theme.id;
    let manager = manager.clone();
    let follows = follows.clone();
    let cards = cards.clone();
    card.connect_clicked(move |_| {
        manager.select_theme(&theme_id);
        for follow in &follows {
            follow.set_active(false);
        }
        refresh_theme_cards(&manager, &cards);
    });
    flow.insert(&card, -1);
}

fn theme_preview(tokens: &ThemeTokens) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class("theme-preview");
    area.set_content_width(190);
    area.set_content_height(72);
    let tokens = tokens.clone();
    area.set_draw_func(move |_, context, width, height| {
        let color = |value: &str| gdk::RGBA::parse(value).unwrap_or(gdk::RGBA::BLACK);
        let paint = |context: &gtk::cairo::Context, value: &str| {
            let value = color(value);
            context.set_source_rgba(
                f64::from(value.red()),
                f64::from(value.green()),
                f64::from(value.blue()),
                1.0,
            );
        };
        context.rounded_rectangle(0.0, 0.0, f64::from(width), f64::from(height), 6.0);
        context.clip();
        paint(context, &tokens.background);
        context.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
        let _ = context.fill();
        paint(context, &tokens.surface);
        context.rectangle(0.0, 0.0, f64::from(width) * 0.40, f64::from(height));
        let _ = context.fill();
        for (x, y, w, value) in [
            (10.0, 23.0, 45.0, &tokens.dim_text),
            (10.0, 36.0, 59.0, &tokens.accent),
            (10.0, 51.0, 39.0, &tokens.dim_text),
            (f64::from(width) * 0.45, 23.0, 47.0, &tokens.accent),
            (f64::from(width) * 0.45, 37.0, 83.0, &tokens.dim_text),
            (f64::from(width) * 0.45, 51.0, 66.0, &tokens.dim_text),
        ] {
            paint(context, value);
            context.rounded_rectangle(x, y, w, 5.0, 2.5);
            let _ = context.fill();
        }
    });
    area
}

fn theme_editor(
    manager: Rc<ThemeManager>,
    custom: gtk::FlowBox,
    follows: ThemeFollows,
    cards: ThemeCards,
) -> gtk::Revealer {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    panel.add_css_class("theme-editor");
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some("Add a theme"));
    title.add_css_class("settings-option-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    panel.append(&header);
    let name = gtk::Entry::builder().placeholder_text("Theme name").build();
    name.add_css_class("theme-name");
    panel.append(&name);

    let values = Rc::new(RefCell::new(manager.starter_tokens()));
    let grid = gtk::Grid::builder()
        .column_spacing(18)
        .row_spacing(10)
        .build();
    for (index, (label_text, field)) in [
        ("Background", ColorField::Background),
        ("Surface", ColorField::Surface),
        ("Text", ColorField::Text),
        ("Accent", ColorField::Accent),
        ("Danger", ColorField::Danger),
        ("Muted", ColorField::Muted),
        ("Highlight", ColorField::Highlight),
        ("Border", ColorField::Border),
        ("Dim text", ColorField::DimText),
    ]
    .into_iter()
    .enumerate()
    {
        let row = (index / 4) as i32;
        let column = ((index % 4) * 2) as i32;
        let label = gtk::Label::new(Some(label_text));
        label.set_xalign(0.0);
        let dialog = gtk::ColorDialog::builder()
            .title(format!("Choose {label_text}"))
            .with_alpha(false)
            .build();
        let picker = gtk::ColorDialogButton::new(Some(dialog));
        picker.add_css_class("theme-color-picker");
        if let Ok(color) = gdk::RGBA::parse(field.get(&values.borrow())) {
            picker.set_rgba(&color);
        }
        let values_for_color = values.clone();
        let manager_for_color = manager.clone();
        picker.connect_rgba_notify(move |picker| {
            field.set(
                &mut values_for_color.borrow_mut(),
                picker.rgba().to_string(),
            );
            manager_for_color.preview(&values_for_color.borrow());
        });
        grid.attach(&picker, column, row, 1, 1);
        grid.attach(&label, column + 1, row, 1, 1);
    }
    panel.append(&grid);
    let error = gtk::Label::new(None);
    error.add_css_class("theme-editor-error");
    error.set_xalign(0.0);
    error.set_visible(false);
    panel.append(&error);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("theme-editor-cancel");
    let save = gtk::Button::with_label("Add theme");
    save.add_css_class("theme-editor-save");
    actions.append(&cancel);
    actions.append(&save);
    panel.append(&actions);
    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&panel)
        .build();
    let hidden = revealer.clone();
    let manager_for_cancel = manager.clone();
    cancel.connect_clicked(move |_| {
        manager_for_cancel.cancel_preview();
        hidden.set_reveal_child(false);
    });
    let hidden = revealer.clone();
    save.connect_clicked(move |_| {
        let mut tokens = values.borrow().clone();
        tokens.name = name.text().trim().to_owned();
        match manager.save_custom_theme(tokens.clone()) {
            Ok(id) => {
                error.set_visible(false);
                append_theme_card(
                    &custom,
                    Theme {
                        id,
                        tokens,
                        custom: true,
                    },
                    &manager,
                    &follows,
                    &cards,
                );
                for follow in &follows {
                    follow.set_active(false);
                }
                refresh_theme_cards(&manager, &cards);
                hidden.set_reveal_child(false);
            }
            Err(message) => {
                error.set_text(&message.to_string());
                error.set_visible(true);
            }
        }
    });
    revealer
}

#[derive(Clone, Copy)]
enum ColorField {
    Background,
    Surface,
    Text,
    Accent,
    Danger,
    Muted,
    Highlight,
    Border,
    DimText,
}
impl ColorField {
    fn get(self, tokens: &ThemeTokens) -> &str {
        match self {
            Self::Background => &tokens.background,
            Self::Surface => &tokens.surface,
            Self::Text => &tokens.text,
            Self::Accent => &tokens.accent,
            Self::Danger => &tokens.danger,
            Self::Muted => &tokens.muted,
            Self::Highlight => &tokens.highlight,
            Self::Border => &tokens.border,
            Self::DimText => &tokens.dim_text,
        }
    }
    fn set(self, tokens: &mut ThemeTokens, value: String) {
        *match self {
            Self::Background => &mut tokens.background,
            Self::Surface => &mut tokens.surface,
            Self::Text => &mut tokens.text,
            Self::Accent => &mut tokens.accent,
            Self::Danger => &mut tokens.danger,
            Self::Muted => &mut tokens.muted,
            Self::Highlight => &mut tokens.highlight,
            Self::Border => &mut tokens.border,
            Self::DimText => &mut tokens.dim_text,
        } = value;
    }
}

fn navigation_button(icon: &str, label: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = crate::assets::primary_icon(icon, 18);
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    content.append(&icon);
    content.append(&label);
    let button = gtk::Button::builder().child(&content).build();
    button.set_has_frame(false);
    button
}

fn page_content() -> gtk::Box {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("settings-preferences");
    content
}

fn settings_option(title: &str, description: &str, active: bool) -> (gtk::Box, gtk::Switch) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(description));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&description);
    let toggle = gtk::Switch::builder()
        .active(active)
        .valign(gtk::Align::Center)
        .build();
    row.append(&copy);
    row.append(&toggle);
    (row, toggle)
}

fn settings_choice(
    title: &str,
    description: &str,
    choices: &[&str],
    selected: u32,
) -> (gtk::Box, gtk::DropDown) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(description));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&description);
    let choice = gtk::DropDown::from_strings(choices);
    choice.set_selected(selected);
    choice.set_valign(gtk::Align::Center);
    choice.add_css_class("settings-choice");
    row.append(&copy);
    row.append(&choice);
    (row, choice)
}

fn settings_spin(
    title: &str,
    description: &str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
) -> (gtk::Box, gtk::SpinButton) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(title));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let description = gtk::Label::new(Some(description));
    description.set_xalign(0.0);
    description.set_wrap(true);
    description.add_css_class("settings-option-description");
    copy.append(&title);
    copy.append(&description);
    let spin = gtk::SpinButton::with_range(min, max, step);
    spin.set_value(value);
    spin.set_valign(gtk::Align::Center);
    spin.add_css_class("settings-spin");
    row.append(&copy);
    row.append(&spin);
    (row, spin)
}

fn append_heading(container: &gtk::Box, text: &str) {
    let heading = gtk::Label::new(Some(text));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    container.append(&heading);
}

fn settings_card() -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("settings-card");
    card
}

fn append_telemetry_row(card: &gtk::Box, label: &str, value: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-option");
    let label_widget = gtk::Label::new(Some(label));
    label_widget.set_xalign(0.0);
    label_widget.set_hexpand(true);
    label_widget.add_css_class("settings-option-title");
    let badge = gtk::Label::new(Some(value));
    badge.add_css_class("settings-telemetry-badge");
    badge.set_valign(gtk::Align::Center);
    badge.set_selectable(true);
    row.append(&label_widget);
    row.append(&badge);
    card.append(&row);
}

fn append_catalog_status_row(card: &gtk::Box, label: &str, path: &str, status: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("settings-option");
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.add_css_class("settings-option-title");
    let desc = gtk::Label::new(Some(path));
    desc.set_xalign(0.0);
    desc.set_selectable(true);
    desc.add_css_class("settings-option-description");
    desc.add_css_class("monospace");
    copy.append(&title);
    copy.append(&desc);

    let badge = gtk::Label::new(Some(status));
    badge.add_css_class("settings-telemetry-badge");
    badge.set_valign(gtk::Align::Center);

    row.append(&copy);
    row.append(&badge);
    card.append(&row);
}

trait RoundedRectangle {
    fn rounded_rectangle(&self, x: f64, y: f64, width: f64, height: f64, radius: f64);
}
impl RoundedRectangle for gtk::cairo::Context {
    fn rounded_rectangle(&self, x: f64, y: f64, width: f64, height: f64, radius: f64) {
        let degrees = std::f64::consts::PI / 180.0;
        self.new_sub_path();
        self.arc(x + width - radius, y + radius, radius, -90.0 * degrees, 0.0);
        self.arc(
            x + width - radius,
            y + height - radius,
            radius,
            0.0,
            90.0 * degrees,
        );
        self.arc(
            x + radius,
            y + height - radius,
            radius,
            90.0 * degrees,
            180.0 * degrees,
        );
        self.arc(
            x + radius,
            y + radius,
            radius,
            180.0 * degrees,
            270.0 * degrees,
        );
        self.close_path();
    }
}
