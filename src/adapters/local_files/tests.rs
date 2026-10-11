// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::ErrorKind,
    os::unix::ffi::{OsStrExt, OsStringExt},
    time::SystemTime,
};

use super::*;
use crate::model::Location;

#[test]
fn unsupported_trash_reports_the_missing_backend() {
    let error = glib::Error::new(gio::IOErrorEnum::NotSupported, "Operation not supported");
    let message = directory_error_message(&Location::uri("trash:///"), &error);
    assert!(message.contains("GVfs"));
    assert!(message.contains("restart Hermes"));
    assert_eq!(
        directory_error_message(&Location::local("/tmp"), &error),
        error.to_string()
    );
    let denied = glib::Error::new(gio::IOErrorEnum::PermissionDenied, "Permission denied");
    assert_eq!(
        directory_error_message(&Location::uri("trash:///"), &denied),
        denied.to_string()
    );
}

#[test]
#[ignore = "Uses the exclusive default GLib context; generated native enumeration fixture"]
fn streaming_enumeration_starts_small_and_keeps_all_entries() {
    use std::{cell::{Cell, RefCell}, rc::Rc, time::{Duration, Instant}};
    let folder = std::env::temp_dir().join(format!("hermes-first-batch-{}", std::process::id()));
    fs::create_dir(&folder).unwrap();
    for index in 0..300 {
        fs::write(folder.join(format!("entry-{index}")), b"fixture").unwrap();
    }
    let context = glib::MainContext::default();
    let _owned = context.acquire().unwrap();
    let batches = Rc::new(RefCell::new(Vec::new()));
    let completed = Rc::new(Cell::new(false));
    let received = batches.clone();
    let finished = completed.clone();
    let load = LocalFileSource.enumerate(DirectoryRequest {
        id: crate::services::RequestId(1), location: Location::local(&folder), batch_size: 128, include_hidden: false,
    }, Rc::new(move |event| match event {
        DirectoryEvent::Batch { entries, .. } => received.borrow_mut().push(entries),
        DirectoryEvent::Finished { .. } => finished.set(true),
        DirectoryEvent::Failed { message, .. } => panic!("{message}"),
        _ => {}
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !completed.get() && Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(completed.get());
    assert_eq!(batches.borrow()[0].len(), 32);
    assert!(batches.borrow().iter().skip(1).all(|entries| entries.len() <= 128));
    let names = batches.borrow().iter().flat_map(|entries| entries.iter().map(|entry| entry.location.clone())).collect::<std::collections::HashSet<_>>();
    assert_eq!(names.len(), 300);
    drop(load);
    while context.iteration(false) {}
    fs::remove_dir_all(folder).unwrap();
}



#[test]
fn invalid_utf8_names_keep_their_native_bytes() -> Result<(), Box<dyn Error>> {
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-native-name-test-{unique}"));
    fs::create_dir(&directory)?;
    let native_name = OsString::from_vec(b"invalid-\xff".to_vec());
    let path = directory.join(&native_name);
    fs::write(&path, b"fixture")?;

    let info = gio::File::for_path(&path).query_info(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NONE,
        None::<&gio::Cancellable>,
    )?;
    let entry = entry_from_info(Location::local(path.clone()), info);

    assert_eq!(entry.native_name.as_bytes(), native_name.as_bytes());
    assert_eq!(entry.location.native_path(), Some(path.as_path()));
    assert!(!entry.display_name.is_empty());

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn symlink_targets_and_broken_links_are_distinguished() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("strata-symlink-test-{unique}"));
    fs::create_dir(&directory)?;
    fs::create_dir(directory.join("directory"))?;
    fs::write(directory.join("file"), b"fixture")?;
    symlink("directory", directory.join("directory-link"))?;
    symlink("file", directory.join("file-link"))?;
    symlink("missing", directory.join("broken-link"))?;

    let kind = |name: &str| -> Result<EntryKind, glib::Error> {
        let path = directory.join(name);
        let info = gio::File::for_path(&path).query_info(
            ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )?;
        Ok(entry_from_info(Location::local(path), info).kind)
    };

    assert_eq!(kind("directory-link")?, EntryKind::DirectorySymbolicLink);
    assert_eq!(kind("file-link")?, EntryKind::FileSymbolicLink);
    assert_eq!(kind("broken-link")?, EntryKind::SymbolicLink);

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn coalescing_preserves_a_move_when_metadata_follows_it() {
    let change = merge_pending_change(
        PendingMonitorChange::Move {
            from: "/fixture/old".into(),
            to: "/fixture/new".into(),
        },
        PendingMonitorChange::Upsert("/fixture/new".into()),
    );

    assert!(matches!(change, PendingMonitorChange::Move { .. }));
}

#[test]
fn conflicting_move_events_fall_back_to_a_rescan() {
    let change = merge_pending_change(
        PendingMonitorChange::Move {
            from: "/fixture/old".into(),
            to: "/fixture/new".into(),
        },
        PendingMonitorChange::Remove("/fixture/new".into()),
    );

    assert!(matches!(change, PendingMonitorChange::Rescan));
}

#[test]
fn permission_errors_are_reported_as_inaccessible() {
    let error = std::io::Error::from(ErrorKind::PermissionDenied);
    assert_eq!(
        map_validation_error(error),
        LocationValidationError::Inaccessible
    );
}

fn music_entry() -> FileEntry {
    FileEntry(Rc::new(crate::model::FileEntryInner {
        location: Location::local("/generated/song.wav"),
        native_name: "song.wav".into(),
        display_name: "Song".into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(100),
        modified_unix_seconds: MetadataValue::Known(10),
        music_tags: None,
    }))
}

#[test]
fn pending_music_count_limit_switches_to_bounded_rescan_without_losing_small_loads() {
    let entry = music_entry();
    let mut pending = PendingMusic::default();
    for _ in 0..4096 {
        pending.push(&entry);
    }
    assert_eq!(pending.entries.len(), 4096);
    assert!(!pending.rescan);
    assert!(pending.bytes <= 8 * 1024 * 1024);
    pending.push(&entry);
    assert!(pending.entries.is_empty());
    assert!(pending.rescan);
    assert_eq!(pending.bytes, 0);
    pending.push(&entry);
    assert!(pending.entries.is_empty());
}

#[test]
fn pending_music_byte_limit_releases_oversized_payloads() {
    let mut entry = music_entry();
    Rc::make_mut(&mut entry.0).display_name = "x".repeat(8 * 1024 * 1024);
    let mut pending = PendingMusic::default();
    pending.push(&entry);
    assert!(pending.rescan);
    assert!(pending.entries.is_empty());
    assert_eq!(pending.bytes, 0);
}
