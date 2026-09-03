// SPDX-License-Identifier: GPL-3.0-or-later

use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

use super::*;

#[test]
fn trail_requires_a_name_and_location() {
    let id = TrailId::new("trail-1").expect("valid id");
    assert!(Trail::new(id.clone(), "", vec![Location::local("/tmp")]).is_none());
    assert!(Trail::new(id.clone(), "Work", Vec::new()).is_none());
    assert!(Trail::new(id, "Work", vec![Location::local("/tmp")]).is_some());
}

#[test]
fn collection_repairs_a_missing_active_trail() {
    let trail = Trail::new(
        TrailId::new("available").expect("valid id"),
        "Available",
        vec![Location::local("/tmp")],
    )
    .expect("valid trail");
    let mut collection = TrailCollection {
        trails: vec![trail],
        active: TrailId::new("missing"),
    };

    collection.normalize();

    assert_eq!(collection.active_trail(), collection.trails.first());
}

#[test]
fn serialization_preserves_invalid_utf8_paths() {
    let path = PathBuf::from(OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]));
    let trail = Trail::new(
        TrailId::new("native-bytes").expect("valid id"),
        "Native bytes",
        vec![Location::local(path.clone())],
    )
    .expect("valid trail");

    let encoded = toml::to_string(&trail).expect("serialize trail");
    let decoded: Trail = toml::from_str(&encoded).expect("deserialize trail");

    assert_eq!(
        decoded.active_location().and_then(Location::native_path),
        Some(path.as_path())
    );
}
