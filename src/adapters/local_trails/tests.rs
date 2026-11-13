// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs, io,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    model::{Location, Trail, TrailCollection, TrailId},
    services::{StoredTrails, TRAIL_SCHEMA_VERSION, TrailStore},
};

use super::{LocalTrailStore, Writer, write_snapshot, writers};

const WAIT: Duration = Duration::from_secs(5);

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        Self(fixture_path(name))
    }

    fn path(&self) -> PathBuf {
        self.0.join("trails.toml")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).expect("remove generated fixture");
        }
    }
}

fn snapshot(name: &str) -> StoredTrails {
    let trail = Trail::new(
        TrailId::new("work").expect("valid id"),
        name,
        vec![Location::local("/generated/trails")],
    )
    .expect("valid trail");
    StoredTrails {
        schema_version: TRAIL_SCHEMA_VERSION,
        collection: TrailCollection {
            active: Some(trail.id.clone()),
            trails: vec![trail],
        },
    }
}

fn store_with_save(
    path: PathBuf,
    save: impl Fn(&Path, &StoredTrails) -> io::Result<()> + Send + 'static,
) -> LocalTrailStore {
    let writer = Arc::new(Writer::start(path.clone(), save));
    let lifecycle = Arc::new(Mutex::new(Arc::downgrade(&writer)));
    assert!(
        writers()
            .lock()
            .expect("writer registry")
            .insert(path.clone(), Arc::downgrade(&lifecycle))
            .is_none()
    );
    LocalTrailStore {
        path,
        writer: Some(writer),
        lifecycle,
    }
}

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
fn background_writer_coalesces_orders_and_does_not_block_save() {
    let fixture = Fixture::new("coalesce");
    let (entered, started) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let saved_calls = calls.clone();
    let caller = thread::current().id();
    let store = Arc::new(store_with_save(fixture.path(), move |path, trails| {
        assert_ne!(thread::current().id(), caller);
        let name = trails.collection.trails[0].name.clone();
        saved_calls.lock().expect("save log").push(name.clone());
        if name == "first" {
            entered.send(()).expect("signal worker entry");
            blocked
                .recv_timeout(WAIT)
                .expect("release slow disk fixture");
        }
        write_snapshot(path, trails)
    }));
    store.save(&snapshot("first")).expect("queue first");
    started.recv_timeout(WAIT).expect("worker started");
    let submitting_store = store.clone();
    let (submitted, enqueued) = mpsc::channel();
    let submitter = thread::spawn(move || {
        for revision in 0..100 {
            submitting_store
                .save(&snapshot(&format!("revision-{revision}")))
                .expect("queue revision");
        }
        submitted.send(()).expect("signal submissions");
    });
    let nonblocking = enqueued.recv_timeout(WAIT);
    release.send(()).expect("release writer");
    nonblocking.expect("save must not wait for disk");
    submitter.join().expect("submitter");
    store.flush().expect("flush latest");
    assert_eq!(store.load().expect("load latest"), snapshot("revision-99"));
    assert_eq!(*calls.lock().expect("save log"), ["first", "revision-99"]);
    drop(store);
}

#[test]
fn multiple_stores_share_one_ordered_writer_and_last_drop_flushes() {
    let fixture = Fixture::new("shared-writer");
    let first = LocalTrailStore::at(fixture.path());
    let second = LocalTrailStore::at(fixture.0.join(".").join("trails.toml"));
    assert!(Arc::ptr_eq(
        first.writer.as_ref().expect("first writer"),
        second.writer.as_ref().expect("second writer")
    ));
    first
        .save(&snapshot("first window"))
        .expect("first window save");
    second
        .save(&snapshot("second window"))
        .expect("second window save");
    drop(first);
    second
        .save(&snapshot("last window"))
        .expect("save after first drop");
    drop(second);
    let reopened = LocalTrailStore::at(fixture.path());
    assert_eq!(
        reopened.load().expect("load after last drop"),
        snapshot("last window")
    );
    drop(reopened);
}

#[test]
fn recreating_store_waits_for_old_writer_instead_of_overlapping() {
    let fixture = Fixture::new("recreate");
    let (entered, started) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let store = store_with_save(fixture.path(), move |path, trails| {
        entered.send(()).expect("signal old writer");
        blocked.recv_timeout(WAIT).expect("release old writer");
        write_snapshot(path, trails)
    });
    let shared = store.writer().shared.clone();
    store.save(&snapshot("old")).expect("queue old");
    started.recv_timeout(WAIT).expect("old writer running");
    let dropping = thread::spawn(move || drop(store));
    let pending = shared.pending.lock().expect("pending");
    let (pending, timeout) = shared
        .wake
        .wait_timeout_while(pending, WAIT, |pending| !pending.closing)
        .expect("wait for close");
    assert!(!timeout.timed_out());
    drop(pending);
    let path = fixture.path();
    let (opened, created) = mpsc::channel();
    let opening = thread::spawn(move || {
        let store = LocalTrailStore::at(path);
        opened.send(()).expect("signal new store");
        store.save(&snapshot("new")).expect("queue new");
        drop(store);
    });
    assert!(matches!(created.try_recv(), Err(mpsc::TryRecvError::Empty)));
    release.send(()).expect("release old writer");
    dropping.join().expect("old writer joined");
    opening.join().expect("new writer joined");
    let store = LocalTrailStore::at(fixture.path());
    assert_eq!(store.load().expect("load newest"), snapshot("new"));
    drop(store);
}

#[test]
fn failed_save_preserves_old_file_reports_error_and_recovers() {
    let fixture = Fixture::new("failure");
    let calls = Arc::new(AtomicUsize::new(0));
    let save_calls = calls.clone();
    let store = store_with_save(fixture.path(), move |path, trails| {
        if save_calls.fetch_add(1, Ordering::Relaxed) == 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "generated disk failure",
            ));
        }
        write_snapshot(path, trails)
    });
    store.save(&snapshot("original")).expect("queue original");
    store.flush().expect("flush original");
    store.save(&snapshot("failed")).expect("queue failure");
    assert_eq!(
        store.flush().expect_err("report disk failure").kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        store.load().expect("old file survives"),
        snapshot("original")
    );
    store.save(&snapshot("recovered")).expect("queue recovery");
    store.flush().expect("flush recovery");
    assert_eq!(store.load().expect("recovered file"), snapshot("recovered"));
    drop(store);
}

#[test]
fn last_drop_waits_for_inflight_save_and_flushes_the_newest_pending_snapshot() {
    let fixture = Fixture::new("inflight-drop");
    let (entered, started) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let saved_calls = calls.clone();
    let store = store_with_save(fixture.path(), move |path, trails| {
        let name = trails.collection.trails[0].name.clone();
        saved_calls.lock().expect("save log").push(name.clone());
        if name == "inflight" {
            entered.send(()).expect("signal inflight save");
            blocked.recv_timeout(WAIT).expect("release inflight save");
        }
        write_snapshot(path, trails)
    });
    let shared = store.writer().shared.clone();
    store.save(&snapshot("inflight")).expect("queue inflight");
    started.recv_timeout(WAIT).expect("writer started");
    store
        .save(&snapshot("superseded"))
        .expect("queue intermediate");
    store.save(&snapshot("final")).expect("queue final");
    let (finished, dropped) = mpsc::channel();
    let dropping = thread::spawn(move || {
        drop(store);
        finished.send(()).expect("signal drop completed");
    });
    let pending = shared.pending.lock().expect("pending");
    let (pending, timeout) = shared
        .wake
        .wait_timeout_while(pending, WAIT, |pending| !pending.closing)
        .expect("wait for close");
    assert!(!timeout.timed_out());
    drop(pending);
    assert!(matches!(dropped.try_recv(), Err(mpsc::TryRecvError::Empty)));
    release.send(()).expect("release writer");
    dropped
        .recv_timeout(WAIT)
        .expect("last drop flushed and joined");
    dropping.join().expect("drop thread");
    assert_eq!(*calls.lock().expect("save log"), ["inflight", "final"]);
    let store = LocalTrailStore::at(fixture.path());
    assert_eq!(
        store.load().expect("load final snapshot"),
        snapshot("final")
    );
    drop(store);
}

#[test]
fn legacy_schema_and_normalization_keep_existing_load_behavior() {
    let fixture = Fixture::new("legacy-load");
    fs::create_dir_all(&fixture.0).expect("create fixture");
    let mut legacy = snapshot("legacy");
    legacy.schema_version = 0;
    legacy.collection.active = TrailId::new("missing");
    legacy.collection.trails[0].pinned = true;
    legacy.collection.trails[0].custom_name = true;
    let mut invalid_trail = legacy.collection.trails[0].clone();
    invalid_trail.locations.clear();
    legacy.collection.trails.push(invalid_trail);
    let encoded = toml::to_string_pretty(&super::TrailFile {
        schema_version: legacy.schema_version,
        collection: legacy.collection.clone(),
    })
    .expect("encode legacy fixture");
    fs::write(fixture.path(), &encoded).expect("write legacy fixture");
    legacy.collection.normalize();
    let store = LocalTrailStore::at(fixture.path());
    assert_eq!(
        store.load().expect("load and normalize legacy state"),
        legacy
    );
    assert_eq!(
        fs::read_to_string(fixture.path()).expect("read unchanged source"),
        encoded
    );
    fs::write(fixture.path(), "not a valid [TOML document").expect("write malformed fixture");
    assert!(store.load().is_err());
    drop(store);
}

#[test]
fn rename_failure_cleans_private_temporary_file_and_recovers() {
    let fixture = Fixture::new("rename-failure");
    fs::create_dir_all(fixture.path()).expect("destination directory blocks rename");
    let store = LocalTrailStore::at(fixture.path());
    store
        .save(&snapshot("blocked"))
        .expect("enqueue blocked write");
    assert!(store.flush().is_err());
    assert_eq!(
        fs::read_dir(&fixture.0).expect("fixture entries").count(),
        1
    );
    fs::remove_dir(fixture.path()).expect("remove blocker");
    store.save(&snapshot("recovery")).expect("enqueue recovery");
    store.flush().expect("flush recovery");
    assert_eq!(store.load().expect("load recovery"), snapshot("recovery"));
    drop(store);
}

#[test]
fn saves_are_private_and_never_follow_legacy_temporary_symlinks() {
    let fixture = Fixture::new("private");
    fs::create_dir_all(&fixture.0).expect("create fixture");
    let unrelated = fixture.0.join("generated-unrelated");
    fs::write(&unrelated, b"untouched").expect("write unrelated fixture");
    symlink(&unrelated, fixture.path().with_extension("toml.tmp"))
        .expect("legacy temporary symlink");
    fs::write(fixture.path(), b"old state").expect("old public state");
    fs::set_permissions(fixture.path(), fs::Permissions::from_mode(0o644))
        .expect("public old permissions");
    let store = LocalTrailStore::at(fixture.path());
    store
        .save(&snapshot("private"))
        .expect("queue private save");
    store.flush().expect("flush private save");
    assert_eq!(
        fs::metadata(fixture.path())
            .expect("state metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::read(&unrelated).expect("unrelated contents"),
        b"untouched"
    );
    assert!(fixture.path().with_extension("toml.tmp").is_symlink());
    assert_eq!(
        fs::read_dir(&fixture.0).expect("fixture entries").count(),
        3
    );
    drop(store);
}

#[test]
fn invalid_save_is_rejected_without_replacing_pending_snapshot() {
    let fixture = Fixture::new("invalid-save");
    let store = LocalTrailStore::at(fixture.path());
    store.save(&snapshot("valid")).expect("queue valid");
    let mut invalid = snapshot("invalid");
    invalid.schema_version = TRAIL_SCHEMA_VERSION + 1;
    assert_eq!(
        store.save(&invalid).expect_err("reject schema").kind(),
        io::ErrorKind::InvalidInput
    );
    drop(store);
    let store = LocalTrailStore::at(fixture.path());
    assert_eq!(store.load().expect("load valid"), snapshot("valid"));
    drop(store);
}

#[test]
fn load_waits_for_the_last_save_even_from_another_store() {
    let fixture = Fixture::new("load-pending");
    let (entered, started) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let store = store_with_save(fixture.path(), move |path, trails| {
        entered.send(()).expect("signal worker");
        blocked.recv_timeout(WAIT).expect("release writer");
        write_snapshot(path, trails)
    });
    store.save(&snapshot("pending")).expect("queue snapshot");
    started.recv_timeout(WAIT).expect("worker running");
    let shared = store.writer().shared.clone();
    let reader = LocalTrailStore::at(fixture.path());
    let (loaded, result) = mpsc::channel();
    let loading = thread::spawn(move || {
        loaded.send(reader.load()).expect("signal loaded state");
    });
    let pending = shared.pending.lock().expect("pending");
    let (pending, timeout) = shared
        .wake
        .wait_timeout_while(pending, WAIT, |pending| {
            pending.flush_target < pending.submitted
        })
        .expect("wait for load barrier");
    assert!(!timeout.timed_out());
    drop(pending);
    assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
    release.send(()).expect("release writer");
    assert_eq!(
        result
            .recv_timeout(WAIT)
            .expect("load completed")
            .expect("load persisted snapshot"),
        snapshot("pending")
    );
    loading.join().expect("load thread");
    assert_eq!(
        store.load().expect("read persisted state"),
        snapshot("pending")
    );
    drop(store);
}

#[test]
fn xdg_instances_coalesce_in_order_and_load_the_last_submitted_save() {
    const CHILD_CONFIG: &str = "HERMES_TRAIL_XDG_TEST_CONFIG";
    let Some(_directory) = std::env::var_os(CHILD_CONFIG) else {
        let fixture = Fixture::new("xdg-shared");
        let output = Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "adapters::local_trails::tests::xdg_instances_coalesce_in_order_and_load_the_last_submitted_save", "--nocapture"])
            .env(CHILD_CONFIG, &fixture.0)
            .env("XDG_CONFIG_HOME", &fixture.0)
            .env("HOME", fixture.0.join("home"))
            .output().expect("run isolated XDG fixture");
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("running 1 test"),
            "isolated XDG test failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    };
    let path = crate::paths::trails_file();
    let (entered, started) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let saved_calls = calls.clone();
    let seeded = store_with_save(path.clone(), move |path, trails| {
        let name = trails.collection.trails[0].name.clone();
        saved_calls.lock().expect("save log").push(name.clone());
        if name == "first window" {
            entered.send(()).expect("signal first save");
            blocked.recv_timeout(WAIT).expect("release first save");
        }
        write_snapshot(path, trails)
    });
    let first = LocalTrailStore::xdg();
    assert_eq!(first.path, path);
    first
        .save(&snapshot("first window"))
        .expect("queue first window");
    started.recv_timeout(WAIT).expect("writer started");
    let second = LocalTrailStore::xdg();
    assert_eq!(second.path, path);
    assert!(Arc::ptr_eq(
        first.writer.as_ref().expect("first writer"),
        second.writer.as_ref().expect("second writer")
    ));
    second
        .save(&snapshot("superseded window"))
        .expect("queue superseded state");
    first
        .save(&snapshot("last window"))
        .expect("queue last submitted state");
    let shared = seeded.writer().shared.clone();
    let releasing = thread::spawn(move || {
        let pending = shared.pending.lock().expect("pending");
        let (pending, timeout) = shared
            .wake
            .wait_timeout_while(pending, WAIT, |pending| {
                pending.flush_target < pending.submitted
            })
            .expect("wait for load barrier");
        assert!(!timeout.timed_out());
        drop(pending);
        release.send(()).expect("release writer");
    });
    assert_eq!(
        second.load().expect("read last save across windows"),
        snapshot("last window")
    );
    releasing.join().expect("release thread");
    assert_eq!(
        *calls.lock().expect("save log"),
        ["first window", "last window"]
    );
    assert_eq!(
        first.load().expect("read same committed state"),
        snapshot("last window")
    );
    drop(first);
    drop(second);
    drop(seeded);
}

#[test]
fn worker_panic_does_not_leave_flush_waiting_forever() {
    let fixture = Fixture::new("worker-panic");
    let store = store_with_save(fixture.path(), |_, _| panic!("generated worker panic"));
    store.save(&snapshot("panic")).expect("queue snapshot");
    assert_eq!(
        store.flush().expect_err("worker stopped").kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        store
            .save(&snapshot("later"))
            .expect_err("writer unavailable")
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    drop(store);
}

#[test]
fn application_shutdown_flushes_even_when_store_is_retained() {
    use gtk::prelude::*;
    let fixture = Fixture::new("shutdown");
    let application = gtk::gio::Application::builder()
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let store = LocalTrailStore::at(fixture.path());
    store.connect_shutdown(&application);
    store.connect_shutdown(&application);
    store
        .save(&snapshot("shutdown"))
        .expect("queue shutdown snapshot");
    application.emit_by_name::<()>("shutdown", &[]);
    assert_eq!(
        store.load().expect("shutdown persisted state"),
        snapshot("shutdown")
    );
    store
        .save(&snapshot("after shutdown callback"))
        .expect("writer remains usable");
    drop(store);
    let store = LocalTrailStore::at(fixture.path());
    assert_eq!(
        store.load().expect("drop flush after shutdown"),
        snapshot("after shutdown callback")
    );
    drop(store);
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
