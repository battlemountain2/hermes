use super::{Queue, Repeat, Track};
use crate::{
    model::{Location, MusicTags},
    services::{FormatFamily, classify_by_name},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TRACKS: usize = 20_000;
const MAX_POSITION: i64 = 604_800_000_000;

#[derive(Clone, Deserialize, Serialize)]
struct SavedTrack {
    path: Location,
    tags: MusicTags,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Session {
    version: u32,
    tracks: Vec<SavedTrack>,
    order: Vec<usize>,
    history: Vec<usize>,
    current: Option<usize>,
    repeat: Repeat,
    shuffle: bool,
    pub position: i64,
    pub volume: f64,
}

impl Session {
    pub fn capture(queue: &Queue, position: i64, volume: f64) -> Self {
        Self {
            version: 1,
            tracks: queue
                .tracks
                .iter()
                .map(|track| SavedTrack {
                    path: Location::local(&track.path),
                    tags: track.tags.clone(),
                })
                .collect(),
            order: queue.order.clone(),
            history: queue.history.clone(),
            current: queue.current,
            repeat: queue.repeat,
            shuffle: queue.shuffle,
            position: position.clamp(0, MAX_POSITION),
            volume: if volume.is_finite() {
                volume.clamp(0.0, 1.0)
            } else {
                0.75
            },
        }
    }

    pub fn into_queue(self) -> Queue {
        Queue {
            tracks: self
                .tracks
                .into_iter()
                .filter_map(|track| {
                    Some(Track {
                        path: track.path.native_path()?.to_owned(),
                        tags: track.tags,
                    })
                })
                .collect(),
            order: self.order,
            history: self.history,
            current: self.current,
            repeat: self.repeat,
            shuffle: self.shuffle,
            seed: 1,
        }
    }
}

pub(crate) fn path() -> PathBuf {
    crate::paths::music_queue_file()
}

pub(crate) fn load(path: &Path) -> Result<Option<Session>, String> {
    let read = || -> Result<Option<Session>, String> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.is_file() => {
                return Err("Saved queue is not a regular file".into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
            _ => {}
        }
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_BYTES {
            return Err("Saved queue exceeds its 16 MiB limit".into());
        }
        let mut data = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|error| error.to_string())?;
        if data.len() as u64 > MAX_BYTES {
            return Err("Saved queue exceeds its 16 MiB limit".into());
        }
        let session: Session = serde_json::from_slice(&data).map_err(|error| error.to_string())?;
        validate(&session)?;
        let position = session.position;
        let volume = session.volume;
        let mut queue = session.into_queue();
        let mut current_missing = false;
        for index in (0..queue.tracks.len()).rev() {
            if !fs::metadata(&queue.tracks[index].path).is_ok_and(|metadata| metadata.is_file()) {
                current_missing |= queue.current == Some(index);
                queue.remove(index);
            }
        }
        Ok(Some(Session::capture(
            &queue,
            if current_missing { 0 } else { position },
            volume,
        )))
    };
    read()
}

fn validate(session: &Session) -> Result<(), String> {
    let count = session.tracks.len();
    if session.version != 1
        || count > MAX_TRACKS
        || session.history.len() > 1024
        || session.current.is_some_and(|current| current >= count)
        || !(0..=MAX_POSITION).contains(&session.position)
        || !session.volume.is_finite()
        || !(0.0..=1.0).contains(&session.volume)
        || session.order.len() != count
        || session.history.iter().any(|index| *index >= count)
    {
        return Err("Saved queue is invalid or exceeds its limits".into());
    }
    let mut seen = vec![false; count];
    for index in &session.order {
        let Some(seen) = seen.get_mut(*index) else {
            return Err("Invalid saved queue order".into());
        };
        if *seen {
            return Err("Duplicate saved queue entry".into());
        }
        *seen = true;
    }
    for track in &session.tracks {
        let Some(path) = track.path.native_path() else {
            return Err("Queue contains a non-local song".into());
        };
        if !path.is_absolute()
            || path.as_os_str().len() > 4096
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
            || classify_by_name(path.as_os_str()) != FormatFamily::Audio
            || [
                &track.tags.title,
                &track.tags.artist,
                &track.tags.album_artist,
                &track.tags.album,
            ]
            .into_iter()
            .flatten()
            .any(|value| value.chars().count() > 256 || value.chars().any(char::is_control))
        {
            return Err("Queue contains an invalid song".into());
        }
    }
    Ok(())
}

fn write(path: &Path, session: &Session) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
    validate(session)?;
    let data = serde_json::to_vec(session).map_err(|error| error.to_string())?;
    if data.len() as u64 > MAX_BYTES {
        return Err("Saved queue exceeds its 16 MiB limit".into());
    }
    fs::create_dir_all(path.parent().ok_or("Invalid queue location")?)
        .map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
    ));
    let save = || -> std::io::Result<()> {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        output.write_all(&data)?;
        output.sync_all()?;
        fs::rename(&temporary, path)
    };
    let result = save();
    if result.is_err() {
        let _removed = fs::remove_file(&temporary);
    }
    result.map_err(|error| error.to_string())
}

#[derive(Default)]
struct Pending {
    latest: Option<Session>,
    closing: bool,
    error: Option<String>,
}

pub(crate) struct Store {
    pending: Arc<(Mutex<Pending>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        let pending = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let shared = pending.clone();
        let worker = std::thread::Builder::new()
            .name("hermes-queue-save".into())
            .spawn(move || {
                loop {
                    let session = {
                        let (lock, wake) = &*shared;
                        let mut pending = lock.lock().unwrap_or_else(|error| error.into_inner());
                        while pending.latest.is_none() && !pending.closing {
                            pending = wake
                                .wait(pending)
                                .unwrap_or_else(|error| error.into_inner());
                        }
                        match pending.latest.take() {
                            Some(session) => session,
                            None => break,
                        }
                    };
                    let error = write(&path, &session).err();
                    shared
                        .0
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .error = error;
                }
            });
        let worker = match worker {
            Ok(worker) => Some(worker),
            Err(error) => {
                pending
                    .0
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .error = Some(error.to_string());
                None
            }
        };
        Self { pending, worker }
    }

    pub fn save(&self, session: Session) {
        let mut pending = self
            .pending
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !pending.closing && self.worker.is_some() {
            pending.latest = Some(session);
            self.pending.1.notify_one();
        }
    }

    pub fn take_error(&self) -> Option<String> {
        self.pending
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .error
            .take()
    }

    pub fn finish(&mut self) {
        self.pending
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closing = true;
        self.pending.1.notify_one();
        if let Some(worker) = self.worker.take() {
            let _joined = worker.join();
        }
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests;
