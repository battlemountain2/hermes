// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use gtk::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{
    model::TrailCollection,
    services::{StoredTrails, TRAIL_SCHEMA_VERSION, TrailStore},
};

#[derive(Deserialize, Serialize)]
struct TrailFile {
    schema_version: u32,
    #[serde(flatten)]
    collection: TrailCollection,
}

pub struct LocalTrailStore {
    path: PathBuf,
    writer: Option<Arc<Writer>>,
    lifecycle: Arc<Mutex<Weak<Writer>>>,
}

impl LocalTrailStore {
    pub fn xdg() -> Self {
        let store = Self::at(gtk::glib::user_config_dir().join("strata/trails.toml"));
        if let Some(application) = gtk::gio::Application::default() {
            store.connect_shutdown(&application);
        }
        store
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let key = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()
                .map(|directory| directory.join(&path))
                .unwrap_or_else(|_| path.clone())
        };
        let lifecycle = {
            let mut writers = writers().lock().unwrap_or_else(|error| error.into_inner());
            writers.retain(|_, lifecycle| lifecycle.strong_count() > 0);
            writers
                .get(&key)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| {
                    let lifecycle = Arc::new(Mutex::new(Weak::new()));
                    writers.insert(key.clone(), Arc::downgrade(&lifecycle));
                    lifecycle
                })
        };
        let writer = {
            let mut previous = lifecycle.lock().unwrap_or_else(|error| error.into_inner());
            previous.upgrade().unwrap_or_else(|| {
                let writer = Arc::new(Writer::new(key));
                *previous = Arc::downgrade(&writer);
                writer
            })
        };
        Self {
            path,
            writer: Some(writer),
            lifecycle,
        }
    }

    pub fn flush(&self) -> io::Result<()> {
        flush(&self.writer().shared)
    }

    fn writer(&self) -> &Writer {
        self.writer
            .as_deref()
            .expect("live Trail store has a writer")
    }

    fn connect_shutdown(&self, application: &gtk::gio::Application) {
        let shared = &self.writer().shared;
        let mut pending = shared
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pending.shutdown_connected {
            return;
        }
        pending.shutdown_connected = true;
        let shared = Arc::downgrade(shared);
        application.connect_shutdown(move |_| {
            if let Some(shared) = shared.upgrade()
                && let Err(error) = flush(&shared)
            {
                tracing::warn!(%error, "unable to flush saved Trails at shutdown");
            }
        });
    }
}

impl Drop for LocalTrailStore {
    fn drop(&mut self) {
        let _lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(writer) = &self.writer
            && Arc::strong_count(writer) == 1
        {
            writer.close();
            if let Err(error) = self.flush() {
                tracing::warn!(%error, "unable to flush saved Trails on final drop");
            }
        }
        drop(self.writer.take());
    }
}

impl TrailStore for LocalTrailStore {
    fn load(&self) -> io::Result<StoredTrails> {
        drop(wait_for_saves(&self.writer().shared));
        let source = match fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(StoredTrails::default());
            }
            Err(error) => return Err(error),
        };
        let mut stored: TrailFile = toml::from_str(&source).map_err(io::Error::other)?;
        if stored.schema_version > TRAIL_SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported Trail schema version {}", stored.schema_version),
            ));
        }
        stored.collection.normalize();
        Ok(StoredTrails {
            schema_version: stored.schema_version,
            collection: stored.collection,
        })
    }

    fn save(&self, trails: &StoredTrails) -> io::Result<()> {
        if trails.schema_version != TRAIL_SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "only the current Trail schema can be saved",
            ));
        }
        let shared = &self.writer().shared;
        let mut pending = shared
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pending.stopped || pending.closing {
            return Err(pending.error.as_ref().map(copy_error).unwrap_or_else(|| {
                io::Error::new(io::ErrorKind::BrokenPipe, "Trail writer has stopped")
            }));
        }
        pending.submitted += 1;
        pending.latest = Some((pending.submitted, trails.clone()));
        shared.wake.notify_all();
        Ok(())
    }
}

type WriterRegistry = HashMap<PathBuf, Weak<Mutex<Weak<Writer>>>>;

fn writers() -> &'static Mutex<WriterRegistry> {
    static WRITERS: OnceLock<Mutex<WriterRegistry>> = OnceLock::new();
    WRITERS.get_or_init(Mutex::default)
}

#[derive(Default)]
struct Pending {
    latest: Option<(u64, StoredTrails)>,
    submitted: u64,
    completed: u64,
    flush_target: u64,
    closing: bool,
    stopped: bool,
    shutdown_connected: bool,
    error: Option<io::Error>,
}

#[derive(Default)]
struct Shared {
    pending: Mutex<Pending>,
    wake: Condvar,
}

struct Writer {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl Writer {
    fn new(path: PathBuf) -> Self {
        Self::start(path, write_snapshot)
    }

    fn start(
        path: PathBuf,
        save: impl Fn(&Path, &StoredTrails) -> io::Result<()> + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Shared::default());
        let worker_shared = shared.clone();
        let worker = thread::Builder::new()
            .name("hermes-trail-save".into())
            .spawn(move || {
                let _exit = WorkerExit(worker_shared.clone());
                run_writer(&path, &worker_shared, save);
            });
        let worker = match worker {
            Ok(worker) => Some(worker),
            Err(error) => {
                let mut pending = shared
                    .pending
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                pending.stopped = true;
                pending.error = Some(error);
                None
            }
        };
        Self { shared, worker }
    }

    fn close(&self) {
        self.shared
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closing = true;
        self.shared.wake.notify_all();
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.close();
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            tracing::warn!("Trail writer panicked during final flush");
        }
    }
}

struct WorkerExit(Arc<Shared>);

impl Drop for WorkerExit {
    fn drop(&mut self) {
        let mut pending = self
            .0
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.stopped = true;
        self.0.wake.notify_all();
    }
}

fn run_writer(path: &Path, shared: &Shared, save: impl Fn(&Path, &StoredTrails) -> io::Result<()>) {
    loop {
        let (revision, trails) = {
            let mut pending = shared
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while pending.latest.is_none() && !pending.closing {
                pending = shared
                    .wake
                    .wait(pending)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if pending.latest.is_none() {
                break;
            }
            let deadline = Instant::now() + Duration::from_millis(30);
            while !pending.closing && pending.flush_target <= pending.completed {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                pending = shared
                    .wake
                    .wait_timeout(pending, remaining)
                    .unwrap_or_else(|error| error.into_inner())
                    .0;
            }
            pending.latest.take().expect("pending Trail snapshot")
        };
        let result = save(path, &trails);
        if let Err(error) = &result {
            tracing::warn!(%error, path = %path.display(), "unable to save Trails");
        }
        let mut pending = shared
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.completed = revision;
        pending.error = result.err();
        shared.wake.notify_all();
    }
}

fn wait_for_saves(shared: &Shared) -> (MutexGuard<'_, Pending>, u64) {
    let mut pending = shared
        .pending
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let target = pending.submitted;
    pending.flush_target = pending.flush_target.max(target);
    shared.wake.notify_all();
    while pending.completed < target && !pending.stopped {
        pending = shared
            .wake
            .wait(pending)
            .unwrap_or_else(|error| error.into_inner());
    }
    (pending, target)
}

fn flush(shared: &Shared) -> io::Result<()> {
    let (pending, target) = wait_for_saves(shared);
    if let Some(error) = &pending.error {
        return Err(copy_error(error));
    }
    if pending.completed < target {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "Trail writer stopped before flushing",
        ));
    }
    Ok(())
}

fn copy_error(error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), error.to_string())
}

fn write_snapshot(destination: &Path, trails: &StoredTrails) -> io::Result<()> {
    let encoded = toml::to_string_pretty(&TrailFile {
        schema_version: trails.schema_version,
        collection: trails.collection.clone(),
    })
    .map_err(io::Error::other)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);
    loop {
        let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
        let temporary =
            destination.with_extension(format!("toml.{}.{sequence}.tmp", std::process::id()));
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary);
        let mut file = match file {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = write_and_replace(&mut file, &temporary, destination, encoded.as_bytes());
        if result.is_err() {
            let _cleanup = fs::remove_file(&temporary);
        }
        return result;
    }
}

fn write_and_replace(
    file: &mut fs::File,
    temporary: &Path,
    destination: &Path,
    contents: &[u8],
) -> io::Result<()> {
    file.write_all(contents)?;
    file.sync_all()?;
    fs::rename(temporary, destination)?;
    if let Some(parent) = destination.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
