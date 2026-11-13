// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[cfg(all(target_os = "macos", feature = "macos-dev"))]
#[test]
fn finder_launched_development_bundle_uses_its_own_profile() {
    assert!(is_development());
    assert_eq!(app_dir_name(), "hermes-dev");
    assert_eq!(
        crate::identity::application_id(is_development()),
        "io.github.battlemountain2.Hermes.Dev"
    );
}

#[test]
fn test_app_dir_name_is_non_empty() {
    let name = app_dir_name();
    assert!(name == "hermes" || name == "hermes-dev");
}

#[test]
fn test_path_construction_consistency() {
    assert!(settings_file().ends_with("settings.toml"));
    assert!(trails_file().ends_with("trails.toml"));
    assert!(folder_views_file().ends_with("folder_views.json"));
    assert!(sidebar_order_file().ends_with("sidebar-order"));
    assert!(recent_folders_file().ends_with("recent-folders.toml"));
    assert!(actions_file().ends_with("actions.toml"));
    assert!(folder_integration_file().ends_with("folder-integration.json"));
    assert!(picker_integration_file().ends_with("picker-integration.json"));
    assert!(themes_dir().ends_with("themes"));
    assert!(music_collections_file().ends_with("music-collections-v1.json"));
    assert!(ratings_file().ends_with("ratings-v1.json"));
    assert!(saved_searches_file().ends_with("saved-searches/queries-v1.json"));
    assert!(music_queue_file().ends_with("music-queue-v1.json"));
    assert!(music_library_cache_dir().ends_with("music-library"));
    assert!(search_catalog_cache_dir().ends_with("search-catalog"));
    assert!(lyrics_cache_dir().ends_with("lyrics-lrclib"));
    assert!(waveform_cache_dir().ends_with("music-waveforms"));
    assert!(thumbnails_cache_dir().ends_with("thumbnails-rgba-v1"));
    assert!(office_pdf_cache_dir().ends_with("office-pdf-v1"));
    assert!(source_styles_cache_dir().ends_with("source-styles"));
    assert!(fonts_cache_dir().ends_with("fonts"));
}
