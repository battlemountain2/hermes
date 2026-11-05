// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;

pub(crate) mod legacy_import;

#[cfg(test)]
mod tests;

pub(crate) fn is_development() -> bool {
    std::env::var_os("HERMES_DEV").is_some() || std::env::var_os("STRATA_DEV").is_some()
}

pub(crate) fn app_dir_name() -> &'static str {
    if is_development() {
        "hermes-dev"
    } else {
        "hermes"
    }
}

pub(crate) fn config_dir() -> PathBuf {
    gtk::glib::user_config_dir().join(app_dir_name())
}

pub(crate) fn data_dir() -> PathBuf {
    gtk::gio::glib::user_data_dir().join(app_dir_name())
}

pub(crate) fn state_dir() -> PathBuf {
    gtk::gio::glib::user_state_dir().join(app_dir_name())
}

pub(crate) fn cache_dir() -> PathBuf {
    gtk::glib::user_cache_dir().join(app_dir_name())
}

// Config files & directories
pub(crate) fn settings_file() -> PathBuf {
    config_dir().join("settings.toml")
}

pub(crate) fn trails_file() -> PathBuf {
    config_dir().join("trails.toml")
}

pub(crate) fn folder_views_file() -> PathBuf {
    config_dir().join("folder_views.json")
}

pub(crate) fn sidebar_order_file() -> PathBuf {
    config_dir().join("sidebar-order")
}

pub(crate) fn recent_folders_file() -> PathBuf {
    config_dir().join("recent-folders.toml")
}

pub(crate) fn actions_file() -> PathBuf {
    config_dir().join("actions.toml")
}

pub(crate) fn folder_integration_file() -> PathBuf {
    config_dir().join("folder-integration.json")
}

pub(crate) fn picker_integration_file() -> PathBuf {
    config_dir().join("picker-integration.json")
}

pub(crate) fn themes_dir() -> PathBuf {
    config_dir().join("themes")
}

// Data files
pub(crate) fn music_collections_file() -> PathBuf {
    data_dir().join("music-collections-v1.json")
}

pub(crate) fn ratings_file() -> PathBuf {
    data_dir().join("ratings-v1.json")
}

pub(crate) fn saved_searches_file() -> PathBuf {
    data_dir().join("saved-searches").join("queries-v1.json")
}

// State files
pub(crate) fn music_queue_file() -> PathBuf {
    state_dir().join("music-queue-v1.json")
}

// Cache directories
pub(crate) fn music_library_cache_dir() -> PathBuf {
    cache_dir().join("music-library")
}

pub(crate) fn search_catalog_cache_dir() -> PathBuf {
    cache_dir().join("search-catalog")
}

pub(crate) fn lyrics_cache_dir() -> PathBuf {
    cache_dir().join("lyrics-lrclib")
}

pub(crate) fn waveform_cache_dir() -> PathBuf {
    cache_dir().join("music-waveforms")
}

pub(crate) fn thumbnails_cache_dir() -> PathBuf {
    cache_dir().join("thumbnails-rgba-v1")
}

pub(crate) fn office_pdf_cache_dir() -> PathBuf {
    cache_dir().join("office-pdf-v1")
}

pub(crate) fn source_styles_cache_dir() -> PathBuf {
    cache_dir().join("source-styles")
}

pub(crate) fn fonts_cache_dir() -> PathBuf {
    cache_dir().join("fonts")
}

// Legacy Strata roots (for migration only)
pub(crate) fn legacy_config_dir() -> PathBuf {
    gtk::glib::user_config_dir().join("strata")
}

pub(crate) fn legacy_data_dir() -> PathBuf {
    gtk::gio::glib::user_data_dir().join("strata")
}

pub(crate) fn legacy_state_dir() -> PathBuf {
    gtk::gio::glib::user_state_dir().join("strata")
}
