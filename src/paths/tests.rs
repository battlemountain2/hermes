// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::fs;

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(prefix: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("hermes-paths-test-{prefix}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
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
    assert!(migration_journal_file().ends_with("migration-v1.json"));
    assert!(music_library_cache_dir().ends_with("music-library"));
    assert!(search_catalog_cache_dir().ends_with("search-catalog"));
    assert!(lyrics_cache_dir().ends_with("lyrics-lrclib"));
    assert!(waveform_cache_dir().ends_with("music-waveforms"));
    assert!(thumbnails_cache_dir().ends_with("thumbnails-rgba-v1"));
    assert!(office_pdf_cache_dir().ends_with("office-pdf-v1"));
    assert!(source_styles_cache_dir().ends_with("source-styles"));
    assert!(fonts_cache_dir().ends_with("fonts"));
}

#[test]
fn test_migration_from_roots_copies_files_and_writes_journal() {
    let root = TempRoot::new("migration-copy");
    let legacy_config = root.path.join("strata-config");
    let legacy_data = root.path.join("strata-data");
    let legacy_state = root.path.join("strata-state");

    let target_config = root.path.join("hermes-config");
    let target_data = root.path.join("hermes-data");
    let target_state = root.path.join("hermes-state");

    fs::create_dir_all(&legacy_config).unwrap();
    fs::create_dir_all(&legacy_data).unwrap();
    fs::create_dir_all(&legacy_state).unwrap();

    let settings_content = b"[general]\nstartup = \"Home\"\n";
    let collections_content = b"{\"favorites\": [\"/music/song.flac\"]}";
    let ratings_content = b"{\"ratings\": {}}";
    let queue_content = b"{\"tracks\": []}";
    let recent_folders_content = b"locations = [\"file:///home/user/Music\"]\n";

    fs::write(legacy_config.join("settings.toml"), settings_content).unwrap();
    fs::write(legacy_config.join("recent-folders.toml"), recent_folders_content).unwrap();
    fs::write(legacy_data.join("music-collections-v1.json"), collections_content).unwrap();
    fs::write(legacy_data.join("ratings-v1.json"), ratings_content).unwrap();
    fs::write(legacy_state.join("music-queue-v1.json"), queue_content).unwrap();

    // Custom theme
    let legacy_themes = legacy_config.join("themes");
    fs::create_dir_all(&legacy_themes).unwrap();
    fs::write(legacy_themes.join("nord.toml"), b"name = \"Nord\"\n").unwrap();

    // Perform migration
    let result = migrate_from_roots(
        &legacy_config,
        &legacy_data,
        &legacy_state,
        &target_config,
        &target_data,
        &target_state,
    );

    assert!(result.is_some());
    let journal = result.unwrap();
    assert!(journal.completed);
    assert_eq!(journal.version, 1);
    assert_eq!(journal.entries.len(), 6);

    // Verify target files exist with identical content
    assert_eq!(fs::read(target_config.join("settings.toml")).unwrap(), settings_content);
    assert_eq!(fs::read(target_config.join("recent-folders.toml")).unwrap(), recent_folders_content);
    assert_eq!(fs::read(target_data.join("music-collections-v1.json")).unwrap(), collections_content);
    assert_eq!(fs::read(target_data.join("ratings-v1.json")).unwrap(), ratings_content);
    assert_eq!(fs::read(target_state.join("music-queue-v1.json")).unwrap(), queue_content);
    assert_eq!(fs::read(target_config.join("themes/nord.toml")).unwrap(), b"name = \"Nord\"\n");

    // Verify legacy files STILL exist (never deleted)
    assert!(legacy_config.join("settings.toml").exists());
    assert!(legacy_config.join("recent-folders.toml").exists());
    assert!(legacy_data.join("music-collections-v1.json").exists());
    assert!(legacy_data.join("ratings-v1.json").exists());
    assert!(legacy_state.join("music-queue-v1.json").exists());
    assert!(legacy_themes.join("nord.toml").exists());

    // Verify journal file was written to target state
    let journal_file = target_state.join("migration-v1.json");
    assert!(journal_file.exists());

    // Re-running migration must be a complete no-op (idempotent)
    let rerun = migrate_from_roots(
        &legacy_config,
        &legacy_data,
        &legacy_state,
        &target_config,
        &target_data,
        &target_state,
    );
    assert!(rerun.is_none());
}

#[test]
fn test_migration_does_not_overwrite_existing_destination() {
    let root = TempRoot::new("no-overwrite");
    let legacy_config = root.path.join("strata-config");
    let legacy_data = root.path.join("strata-data");
    let legacy_state = root.path.join("strata-state");

    let target_config = root.path.join("hermes-config");
    let target_data = root.path.join("hermes-data");
    let target_state = root.path.join("hermes-state");

    fs::create_dir_all(&legacy_config).unwrap();
    fs::create_dir_all(&target_config).unwrap();

    let legacy_settings = b"legacy settings";
    let modern_settings = b"hermes existing settings";

    fs::write(legacy_config.join("settings.toml"), legacy_settings).unwrap();
    fs::write(target_config.join("settings.toml"), modern_settings).unwrap();

    let result = migrate_from_roots(
        &legacy_config,
        &legacy_data,
        &legacy_state,
        &target_config,
        &target_data,
        &target_state,
    );

    // No files needed migration since destination already existed
    assert!(result.is_none());
    assert_eq!(fs::read(target_config.join("settings.toml")).unwrap(), modern_settings);
    assert_eq!(fs::read(legacy_config.join("settings.toml")).unwrap(), legacy_settings);
}
