// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use super::*;
use crate::model::{SortDirection, SortKey, TrailBrowserDensity, TrailBrowserMode};

#[test]
fn test_folder_view_config_serialization_roundtrip() {
    let mut config = FolderViewConfig::default();
    assert!(config.is_empty());

    config.mode = Some(TrailBrowserMode::Grid);
    config.sort_key = Some(SortKey::Modified);
    config.sort_direction = Some(SortDirection::Descending);
    assert!(!config.is_empty());

    let json = serde_json::to_string(&config).expect("serialize config");
    assert!(json.contains("\"mode\":\"grid\""));
    assert!(json.contains("\"sort_key\":\"modified\""));
    assert!(json.contains("\"sort_direction\":\"descending\""));
    assert!(!json.contains("density"));
    assert!(!json.contains("show_hidden"));

    let decoded: FolderViewConfig = serde_json::from_str(&json).expect("deserialize config");
    assert_eq!(decoded, config);
}

#[test]
fn test_folder_views_data_relocate() {
    let mut data = FolderViewsData::default();
    data.views.insert(
        "/home/user/Pictures".to_string(),
        FolderViewConfig {
            mode: Some(TrailBrowserMode::Grid),
            ..Default::default()
        },
    );
    data.views.insert(
        "/home/user/Pictures/Vacation".to_string(),
        FolderViewConfig {
            mode: Some(TrailBrowserMode::Grid),
            sort_key: Some(SortKey::Modified),
            ..Default::default()
        },
    );
    data.views.insert(
        "/home/user/Music".to_string(),
        FolderViewConfig {
            mode: Some(TrailBrowserMode::Explorer),
            sort_key: Some(SortKey::Track),
            ..Default::default()
        },
    );

    // Relocate Pictures to Media/Photos
    assert!(data.relocate("/home/user/Pictures", "/home/user/Media/Photos"));

    assert!(data.views.get("/home/user/Pictures").is_none());
    assert!(data.views.get("/home/user/Pictures/Vacation").is_none());

    let moved_root = data.views.get("/home/user/Media/Photos").expect("moved root");
    assert_eq!(moved_root.mode, Some(TrailBrowserMode::Grid));

    let moved_child = data.views.get("/home/user/Media/Photos/Vacation").expect("moved child");
    assert_eq!(moved_child.mode, Some(TrailBrowserMode::Grid));
    assert_eq!(moved_child.sort_key, Some(SortKey::Modified));

    // Unrelated path should remain untouched
    let music = data.views.get("/home/user/Music").expect("music untouched");
    assert_eq!(music.mode, Some(TrailBrowserMode::Explorer));
    assert_eq!(music.sort_key, Some(SortKey::Track));
}

#[test]
fn test_folder_view_store_lifecycle_and_persistence() {
    let temp_dir = std::env::temp_dir().join(format!(
        "hermes-folder-views-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::create_dir_all(&temp_dir);
    let store_path = temp_dir.join("folder_views.json");

    let store = FolderViewStore::open(store_path.clone());

    assert_eq!(store.get("/home/user/Downloads"), None);

    store.set_mode("/home/user/Downloads", TrailBrowserMode::Explorer);
    store.set_sort(
        "/home/user/Downloads",
        SortKey::Modified,
        SortDirection::Descending,
    );
    store.set_density("/home/user/Downloads", TrailBrowserDensity::Compact);

    let config = store.get("/home/user/Downloads").expect("saved config");
    assert_eq!(config.mode, Some(TrailBrowserMode::Explorer));
    assert_eq!(config.sort_key, Some(SortKey::Modified));
    assert_eq!(config.sort_direction, Some(SortDirection::Descending));
    assert_eq!(config.density, Some(TrailBrowserDensity::Compact));

    // Flush to disk
    store.flush(Duration::from_secs(2)).expect("flush to disk");
    assert!(store_path.exists());

    // Reopen store from disk
    let reopened = FolderViewStore::open(store_path);
    let loaded = reopened.get("/home/user/Downloads").expect("reloaded config");
    assert_eq!(loaded, config);

    // Test set_show_hidden
    reopened.set_show_hidden("/home/user/Downloads", true);
    assert_eq!(reopened.get("/home/user/Downloads").and_then(|c| c.show_hidden), Some(true));

    // Test remove
    reopened.remove("/home/user/Downloads");
    assert_eq!(reopened.get("/home/user/Downloads"), None);

    // Test clear
    reopened.set_mode("/home/user/Documents", TrailBrowserMode::Columns);
    assert!(reopened.get("/home/user/Documents").is_some());
    reopened.clear();
    assert_eq!(reopened.get("/home/user/Documents"), None);
}
