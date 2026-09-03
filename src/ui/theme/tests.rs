// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    Preferences, blend, is_omarchy_theme_event, slugify, title_case_slug, tokens_from_quattro,
};

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
