// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    model::{Location, Trail, TrailCollection, TrailId},
    services::{StoredTrails, TRAIL_SCHEMA_VERSION, TrailStore},
};

use super::LocalTrailStore;

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "strata-trails-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ))
}

#[test]
fn missing_store_loads_defaults() {
    let path = fixture_path("missing").join("trails.toml");
    let stored = LocalTrailStore::at(path).load().expect("load defaults");
    assert_eq!(stored, StoredTrails::default());
}

#[test]
fn store_round_trips_atomically() {
    let directory = fixture_path("round-trip");
    let path = directory.join("trails.toml");
    let store = LocalTrailStore::at(&path);
    let trail = Trail::new(
        TrailId::new("work").expect("valid id"),
        "Work",
        vec![Location::local("/tmp")],
    )
    .expect("valid trail");
    let stored = StoredTrails {
        schema_version: TRAIL_SCHEMA_VERSION,
        collection: TrailCollection {
            active: Some(trail.id.clone()),
            trails: vec![trail],
        },
    };

    store.save(&stored).expect("save trails");

    assert_eq!(store.load().expect("load trails"), stored);
    assert!(!path.with_extension("toml.tmp").exists());
    fs::remove_dir_all(directory).expect("remove fixture");
}

#[test]
fn future_schema_is_rejected() {
    let directory = fixture_path("future");
    fs::create_dir_all(&directory).expect("create fixture");
    let path = directory.join("trails.toml");
    fs::write(&path, "schema_version = 999\ntrails = []\n").expect("write fixture");

    let error = LocalTrailStore::at(path)
        .load()
        .expect_err("reject future schema");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    fs::remove_dir_all(directory).expect("remove fixture");
}
