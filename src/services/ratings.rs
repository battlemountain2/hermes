// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak, mpsc},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{FormatFamily, classify_by_name};

#[cfg(test)]
mod tests;

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RATINGS: usize = 50_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct RatingsData {
    pub version: u32,
    pub ratings: BTreeMap<PathBuf, u8>,
}

impl Default for RatingsData {
    fn default() -> Self {
        Self {
            version: 1,
            ratings: BTreeMap::new(),
        }
    }
}

impl RatingsData {
    pub fn relocate(&mut self, from: &Path, to: &Path) -> bool {
        if from == to {
            return false;
        }
        let mut changed = false;
        let mut updates = Vec::new();
        let mut removals = Vec::new();

        for (path, &rating) in &self.ratings {
            if path == from {
                removals.push(path.clone());
                updates.push((to.to_path_buf(), rating));
                changed = true;
            } else if let Ok(suffix) = path.strip_prefix(from) {
                removals.push(path.clone());
                updates.push((to.join(suffix), rating));
                changed = true;
            }
        }

        for path in removals {
            self.ratings.remove(&path);
        }
        for (new_path, rating) in updates {
            self.ratings.insert(new_path, rating);
        }

        changed
    }

    fn apply(&mut self, command: Command) -> Result<bool, String> {
        match command {
            Command::Set { path, rating } => {
                validate_target_path(&path)?;
                match rating {
                    Some(star) => {
                        validate_rating_value(star)?;
                        if self.ratings.get(&path) == Some(&star) {
                            return Ok(false);
                        }
                        if self.ratings.len() >= MAX_RATINGS && !self.ratings.contains_key(&path) {
                            return Err("Rating limit reached (50,000 items)".into());
                        }
                        self.ratings.insert(path, star);
                    }
                    None => {
                        if self.ratings.remove(&path).is_none() {
                            return Ok(false);
                        }
                    }
                }
            }
            Command::SetMany { paths, rating } => {
                if paths.len() > MAX_RATINGS {
                    return Err("Too many paths in rating request".into());
                }
                for path in &paths {
                    validate_target_path(path)?;
                }
                let mut changed = false;
                match rating {
                    Some(star) => {
                        validate_rating_value(star)?;
                        let new_count = paths
                            .iter()
                            .filter(|p| !self.ratings.contains_key(*p))
                            .count();
                        if self.ratings.len() + new_count > MAX_RATINGS {
                            return Err("Rating limit reached (50,000 items)".into());
                        }
                        for path in paths {
                            if self.ratings.insert(path, star) != Some(star) {
                                changed = true;
                            }
                        }
                    }
                    None => {
                        for path in paths {
                            if self.ratings.remove(&path).is_some() {
                                changed = true;
                            }
                        }
                    }
                }
                return Ok(changed);
            }
            Command::Relocate { from, to } => {
                validate_relocate_path(&from)?;
                validate_relocate_path(&to)?;
                return Ok(self.relocate(&from, &to));
            }
            Command::Flush(_) => return Ok(false),
        }
        Ok(true)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Command {
    Set {
        path: PathBuf,
        rating: Option<u8>,
    },
    SetMany {
        paths: Vec<PathBuf>,
        rating: Option<u8>,
    },
    Relocate {
        from: PathBuf,
        to: PathBuf,
    },
    Flush(mpsc::SyncSender<Result<(), String>>),
}

#[allow(dead_code)]
#[derive(Default, Clone)]
pub(crate) struct Snapshot {
    pub revision: u64,
    pub ready: bool,
    pub error: Option<String>,
    pub data: RatingsData,
}

struct Shared {
    sender: mpsc::SyncSender<Command>,
    snapshot: Arc<Mutex<Arc<Snapshot>>>,
}

#[derive(Clone)]
pub(crate) struct Store(Arc<Shared>);

impl Store {
    pub fn shared() -> Self {
        static STORE: OnceLock<Mutex<Weak<Shared>>> = OnceLock::new();
        let mut shared = STORE
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(store) = shared.upgrade() {
            return Self(store);
        }
        let store = Self::start(Some(
            gtk::gio::glib::user_data_dir().join("strata/ratings-v1.json"),
        ));
        *shared = Arc::downgrade(&store.0);
        store
    }

    pub fn start(path: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::sync_channel(16);
        let snapshot = Arc::new(Mutex::new(Arc::new(Snapshot::default())));
        let output = snapshot.clone();
        let worker = std::thread::Builder::new()
            .name("hermes-ratings".into())
            .spawn(move || run(path, receiver, output));
        if let Err(error) = worker {
            *snapshot.lock().unwrap_or_else(|error| error.into_inner()) = Arc::new(Snapshot {
                ready: true,
                error: Some(error.to_string()),
                ..Default::default()
            });
        }
        Self(Arc::new(Shared { sender, snapshot }))
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.0
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn get(&self, path: &Path) -> Option<u8> {
        self.snapshot().data.ratings.get(path).copied()
    }

    #[allow(dead_code)]
    pub fn all(&self) -> BTreeMap<PathBuf, u8> {
        self.snapshot().data.ratings.clone()
    }

    pub fn set(&self, path: PathBuf, rating: Option<u8>) -> Result<(), String> {
        validate_target_path(&path)?;
        if let Some(r) = rating {
            validate_rating_value(r)?;
        }
        self.send(Command::Set { path, rating })
    }

    pub fn set_many(&self, paths: Vec<PathBuf>, rating: Option<u8>) -> Result<(), String> {
        for path in &paths {
            validate_target_path(path)?;
        }
        if let Some(r) = rating {
            validate_rating_value(r)?;
        }
        self.send(Command::SetMany { paths, rating })
    }

    pub fn relocate(&self, from: PathBuf, to: PathBuf) -> Result<(), String> {
        if from == to {
            return Ok(());
        }
        validate_relocate_path(&from)?;
        validate_relocate_path(&to)?;
        self.send(Command::Relocate { from, to })
    }

    pub fn flush(&self, timeout: Duration) -> Result<(), String> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let deadline = Instant::now() + timeout;
        let mut command = Command::Flush(sender);
        loop {
            match self.0.sender.try_send(command) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(pending)) if Instant::now() < deadline => {
                    command = pending;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return Err("Rating flush could not be queued before shutdown".into()),
            }
        }
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "Rating flush timed out".to_owned())?
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.0
            .sender
            .try_send(command)
            .map_err(|_| "Ratings worker is busy or unavailable".into())
    }
}

/// Returns true if the file format is eligible for rating (photo, video, document).
pub fn is_ratable_format(path: &Path) -> bool {
    let family = classify_by_name(path.as_os_str());
    matches!(
        family,
        FormatFamily::Image
            | FormatFamily::Svg
            | FormatFamily::Heif
            | FormatFamily::RawImage
            | FormatFamily::GeoTiff
            | FormatFamily::Video
            | FormatFamily::Pdf
            | FormatFamily::OfficeDocument
            | FormatFamily::Spreadsheet
            | FormatFamily::PlainText
    )
}

fn validate_target_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path.as_os_str().len() > 4096
        || path.components().any(|part| matches!(part, Component::ParentDir))
    {
        return Err("Invalid file path".into());
    }
    let family = classify_by_name(path.as_os_str());
    if family == FormatFamily::Audio {
        return Err("Music cannot be rated; favorites are used instead".into());
    }
    if !is_ratable_format(path) {
        return Err("Only photos, videos, and documents can be rated".into());
    }
    Ok(())
}

fn validate_relocate_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path.as_os_str().len() > 4096
        || path.components().any(|part| matches!(part, Component::ParentDir))
    {
        return Err("Invalid relocate path".into());
    }
    Ok(())
}

fn validate_rating_value(star: u8) -> Result<(), String> {
    if !(1..=5).contains(&star) {
        return Err("Rating must be between 1 and 5 stars".into());
    }
    Ok(())
}

fn validate(data: &RatingsData) -> Result<(), String> {
    if data.version != 1 || data.ratings.len() > MAX_RATINGS {
        return Err("Saved ratings data exceeds limits or has unsupported version".into());
    }
    for (path, &star) in &data.ratings {
        validate_target_path(path)?;
        validate_rating_value(star)?;
    }
    Ok(())
}

fn load(path: &Path) -> Result<RatingsData, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RatingsData::default());
        }
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_BYTES => {}
        Ok(_) => return Err("Ratings file is not a regular file or exceeds 16 MiB".into()),
        Err(error) => return Err(error.to_string()),
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Ratings exceed 16 MiB".into());
    }
    let data = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    validate(&data)?;
    Ok(data)
}

fn save(path: &Path, data: &RatingsData) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(data).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Ratings exceed 16 MiB".into());
    }
    let parent = path.parent().ok_or("Invalid storage path")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(
        ".ratings-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    fs::rename(&temporary, path).map_err(|error| error.to_string())?;
    Ok(())
}

fn run(
    path: Option<PathBuf>,
    receiver: mpsc::Receiver<Command>,
    output: Arc<Mutex<Arc<Snapshot>>>,
) {
    let loaded = path.as_deref().map(load).unwrap_or_else(|| Ok(RatingsData::default()));
    let read_only = loaded.is_err();
    let mut error = loaded.as_ref().err().cloned();
    let mut data = loaded.unwrap_or_default();
    let mut revision = 1;
    let mut dirty = false;
    let mut last_save = Instant::now();

    let publish = |data: &RatingsData, revision, error: &Option<String>| {
        let snapshot = Arc::new(Snapshot {
            revision,
            ready: true,
            data: data.clone(),
            error: error.clone(),
        });
        *output.lock().unwrap_or_else(|error| error.into_inner()) = snapshot;
    };
    publish(&data, revision, &error);

    loop {
        let command = receiver.recv_timeout(Duration::from_millis(200));
        let disconnected = matches!(command, Err(mpsc::RecvTimeoutError::Disconnected));
        let timed_out = matches!(command, Err(mpsc::RecvTimeoutError::Timeout));
        let mut flush = None;
        let command = match command {
            Ok(Command::Flush(reply)) => {
                flush = Some(reply);
                Err(mpsc::RecvTimeoutError::Timeout)
            }
            value => value,
        };
        if let Ok(command) = command {
            if read_only {
                if error.is_none() {
                    error = Some("Saved ratings are read-only; restart Hermes to retry".into());
                }
            } else {
                let mut candidate = data.clone();
                match candidate.apply(command) {
                    Ok(changed) => {
                        if changed {
                            data = candidate;
                            dirty = true;
                        }
                    }
                    Err(message) => error = Some(message),
                }
            }
            revision += 1;
            publish(&data, revision, &error);
        }

        if dirty
            && (disconnected
                || flush.is_some()
                || (last_save.elapsed() >= Duration::from_millis(500) || timed_out))
        {
            if let Some(path) = path.as_deref() {
                match save(path, &data) {
                    Ok(()) => {
                        dirty = false;
                        last_save = Instant::now();
                        if let Some(reply) = flush {
                            let _ = reply.send(Ok(()));
                        }
                    }
                    Err(save_err) => {
                        error = Some(save_err.clone());
                        revision += 1;
                        publish(&data, revision, &error);
                        if let Some(reply) = flush {
                            let _ = reply.send(Err(save_err));
                        }
                    }
                }
            } else {
                dirty = false;
                if let Some(reply) = flush {
                    let _ = reply.send(Ok(()));
                }
            }
        } else if let Some(reply) = flush {
            let _ = reply.send(Ok(()));
        }

        if disconnected {
            break;
        }
    }
}
