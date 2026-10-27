// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

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

#[allow(dead_code)]
pub(crate) fn migration_journal_file() -> PathBuf {
    state_dir().join("migration-v1.json")
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MigratedFileEntry {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub bytes: u64,
    pub success: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MigrationJournal {
    pub version: u32,
    pub completed: bool,
    pub entries: Vec<MigratedFileEntry>,
}

pub(crate) fn migrate_legacy_data_if_needed() -> Option<MigrationJournal> {
    migrate_from_roots(
        &legacy_config_dir(),
        &legacy_data_dir(),
        &legacy_state_dir(),
        &config_dir(),
        &data_dir(),
        &state_dir(),
    )
}

fn copy_file_atomic(source: &Path, destination: &Path) -> std::io::Result<u64> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = destination.with_extension("hermes-migrating-tmp");
    let bytes = fs::copy(source, &temp_path)?;
    fs::rename(&temp_path, destination)?;
    Ok(bytes)
}

pub(crate) fn migrate_from_roots(
    legacy_config: &Path,
    legacy_data: &Path,
    legacy_state: &Path,
    target_config: &Path,
    target_data: &Path,
    target_state: &Path,
) -> Option<MigrationJournal> {
    let journal_path = target_state.join("migration-v1.json");
    if journal_path.exists() {
        return None;
    }

    let pairs = [
        // Config files
        (legacy_config.join("settings.toml"), target_config.join("settings.toml")),
        (legacy_config.join("trails.toml"), target_config.join("trails.toml")),
        (legacy_config.join("folder_views.json"), target_config.join("folder_views.json")),
        (legacy_config.join("sidebar-order"), target_config.join("sidebar-order")),
        (legacy_config.join("recent-folders.toml"), target_config.join("recent-folders.toml")),
        // Data files
        (legacy_data.join("music-collections-v1.json"), target_data.join("music-collections-v1.json")),
        (legacy_data.join("ratings-v1.json"), target_data.join("ratings-v1.json")),
        (legacy_data.join("saved-searches/queries-v1.json"), target_data.join("saved-searches/queries-v1.json")),
        // State files
        (legacy_state.join("music-queue-v1.json"), target_state.join("music-queue-v1.json")),
    ];

    let mut entries = Vec::new();
    let mut any_migrated = false;

    for (src, dst) in pairs {
        if src.is_file() && !dst.exists() {
            match copy_file_atomic(&src, &dst) {
                Ok(bytes) => {
                    entries.push(MigratedFileEntry {
                        source: src,
                        destination: dst,
                        bytes,
                        success: true,
                    });
                    any_migrated = true;
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to migrate file: {}", src.display());
                    entries.push(MigratedFileEntry {
                        source: src,
                        destination: dst,
                        bytes: 0,
                        success: false,
                    });
                }
            }
        }
    }

    // Migrate custom themes if any exist
    let legacy_themes = legacy_config.join("themes");
    if legacy_themes.is_dir() {
        if let Ok(theme_entries) = fs::read_dir(&legacy_themes) {
            for entry in theme_entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|ext| ext == "toml") {
                    if let Some(file_name) = path.file_name() {
                        let dst = target_config.join("themes").join(file_name);
                        if !dst.exists() {
                            match copy_file_atomic(&path, &dst) {
                                Ok(bytes) => {
                                    entries.push(MigratedFileEntry {
                                        source: path,
                                        destination: dst,
                                        bytes,
                                        success: true,
                                    });
                                    any_migrated = true;
                                }
                                Err(error) => {
                                    tracing::warn!(%error, "failed to migrate theme: {}", path.display());
                                    entries.push(MigratedFileEntry {
                                        source: path,
                                        destination: dst,
                                        bytes: 0,
                                        success: false,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if !any_migrated && entries.is_empty() {
        // No legacy strata files existed to migrate.
        return None;
    }

    let journal = MigrationJournal {
        version: 1,
        completed: any_migrated && entries.iter().all(|entry| entry.success),
        entries,
    };

    if let Ok(serialized) = serde_json::to_string_pretty(&journal) {
        if let Some(parent) = journal_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let temp_journal = journal_path.with_extension("hermes-migrating-tmp");
        if fs::write(&temp_journal, serialized.as_bytes()).is_ok() {
            let _ = fs::rename(&temp_journal, &journal_path);
        }
    }

    Some(journal)
}
