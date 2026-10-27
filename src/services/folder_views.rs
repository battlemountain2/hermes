// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::model::{SortDirection, SortKey, TrailBrowserDensity, TrailBrowserMode};

#[cfg(test)]
mod tests;

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_VIEWS: usize = 10_000;
const DEBOUNCE_DELAY: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FolderViewConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<TrailBrowserMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<TrailBrowserDensity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_key: Option<SortKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_direction: Option<SortDirection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_hidden: Option<bool>,
}

impl FolderViewConfig {
    #[cfg_attr(not(test), expect(dead_code, reason = "FolderViewConfig API"))]
    pub fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.density.is_none()
            && self.sort_key.is_none()
            && self.sort_direction.is_none()
            && self.show_hidden.is_none()
    }

    pub fn apply_update(&mut self, update: FolderViewUpdate) {
        if let Some(mode) = update.mode {
            self.mode = Some(mode);
        }
        if let Some(density) = update.density {
            self.density = Some(density);
        }
        if let Some(sort_key) = update.sort_key {
            self.sort_key = Some(sort_key);
        }
        if let Some(sort_direction) = update.sort_direction {
            self.sort_direction = Some(sort_direction);
        }
        if let Some(show_hidden) = update.show_hidden {
            self.show_hidden = Some(show_hidden);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FolderViewUpdate {
    pub mode: Option<TrailBrowserMode>,
    pub density: Option<TrailBrowserDensity>,
    pub sort_key: Option<SortKey>,
    pub sort_direction: Option<SortDirection>,
    pub show_hidden: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FolderViewsData {
    pub version: u32,
    pub views: BTreeMap<String, FolderViewConfig>,
}

impl Default for FolderViewsData {
    fn default() -> Self {
        Self {
            version: 1,
            views: BTreeMap::new(),
        }
    }
}

impl FolderViewsData {
    pub fn relocate(&mut self, from: &str, to: &str) -> bool {
        if from == to || from.is_empty() {
            return false;
        }
        let mut changed = false;
        let mut updates = Vec::new();
        let mut removals = Vec::new();

        for (path, &config) in &self.views {
            if path == from {
                removals.push(path.clone());
                updates.push((to.to_string(), config));
                changed = true;
            } else if path.starts_with(from) && path[from.len()..].starts_with('/') {
                let suffix = &path[from.len()..];
                removals.push(path.clone());
                updates.push((format!("{to}{suffix}"), config));
                changed = true;
            }
        }

        for path in removals {
            self.views.remove(&path);
        }
        for (path, config) in updates {
            self.views.insert(path, config);
        }
        changed
    }

    pub fn prune_if_needed(&mut self) {
        while self.views.len() > MAX_VIEWS {
            if let Some(first_key) = self.views.keys().next().cloned() {
                self.views.remove(&first_key);
            } else {
                break;
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub data: FolderViewsData,
}

enum Command {
    Update {
        path: String,
        update: FolderViewUpdate,
    },
    #[cfg_attr(not(test), expect(dead_code, reason = "FolderViewStore API"))]
    Remove(String),
    Relocate {
        from: String,
        to: String,
    },
    #[cfg_attr(not(test), expect(dead_code, reason = "FolderViewStore API"))]
    Clear,
    #[cfg_attr(not(test), expect(dead_code, reason = "FolderViewStore API"))]
    Flush(mpsc::SyncSender<Result<(), String>>),
}

struct Shared {
    sender: mpsc::SyncSender<Command>,
    snapshot: Mutex<Arc<Snapshot>>,
}

#[derive(Clone)]
pub struct FolderViewStore(Arc<Shared>);

impl FolderViewStore {
    pub fn shared() -> Self {
        static INSTANCE: OnceLock<FolderViewStore> = OnceLock::new();
        INSTANCE
            .get_or_init(|| {
                let path = default_storage_path();
                Self::open(path)
            })
            .clone()
    }

    pub fn open(path: PathBuf) -> Self {
        let (initial_data, _) = read_views(&path);
        let snapshot = Mutex::new(Arc::new(Snapshot { data: initial_data }));
        let (sender, receiver) = mpsc::sync_channel::<Command>(256);

        // Spawn background writer
        let writer_path = path.clone();
        std::thread::Builder::new()
            .name("hermes-folder-views".into())
            .spawn(move || {
                run_worker(receiver, writer_path);
            })
            .expect("Failed to spawn folder views writer thread");

        Self(Arc::new(Shared {
            sender,
            snapshot,
        }))
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.0
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn get(&self, path: &str) -> Option<FolderViewConfig> {
        self.snapshot().data.views.get(path).copied()
    }

    pub fn update(&self, path: &str, update: FolderViewUpdate) {
        if path.is_empty() {
            return;
        }
        {
            let mut guard = self.0.snapshot.lock().unwrap_or_else(|error| error.into_inner());
            let mut next = (**guard).clone();
            let config = next.data.views.entry(path.to_string()).or_default();
            config.apply_update(update);
            next.data.prune_if_needed();
            *guard = Arc::new(next);
        }
        let _ = self.0.sender.try_send(Command::Update {
            path: path.to_string(),
            update,
        });
    }

    pub fn set_mode(&self, path: &str, mode: TrailBrowserMode) {
        self.update(
            path,
            FolderViewUpdate {
                mode: Some(mode),
                ..Default::default()
            },
        );
    }

    pub fn set_density(&self, path: &str, density: TrailBrowserDensity) {
        self.update(
            path,
            FolderViewUpdate {
                density: Some(density),
                ..Default::default()
            },
        );
    }

    pub fn set_sort(&self, path: &str, sort_key: SortKey, sort_direction: SortDirection) {
        self.update(
            path,
            FolderViewUpdate {
                sort_key: Some(sort_key),
                sort_direction: Some(sort_direction),
                ..Default::default()
            },
        );
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "FolderViewStore API"))]
    pub fn set_show_hidden(&self, path: &str, show_hidden: bool) {
        self.update(
            path,
            FolderViewUpdate {
                show_hidden: Some(show_hidden),
                ..Default::default()
            },
        );
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "FolderViewStore API"))]
    pub fn remove(&self, path: &str) {
        {
            let mut guard = self.0.snapshot.lock().unwrap_or_else(|error| error.into_inner());
            let mut next = (**guard).clone();
            next.data.views.remove(path);
            *guard = Arc::new(next);
        }
        let _ = self.0.sender.try_send(Command::Remove(path.to_string()));
    }

    pub fn relocate(&self, from: &str, to: &str) {
        {
            let mut guard = self.0.snapshot.lock().unwrap_or_else(|error| error.into_inner());
            let mut next = (**guard).clone();
            if next.data.relocate(from, to) {
                *guard = Arc::new(next);
            }
        }
        let _ = self.0.sender.try_send(Command::Relocate {
            from: from.to_string(),
            to: to.to_string(),
        });
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "FolderViewStore API"))]
    pub fn clear(&self) {
        {
            let mut guard = self.0.snapshot.lock().unwrap_or_else(|error| error.into_inner());
            let mut next = (**guard).clone();
            next.data.views.clear();
            *guard = Arc::new(next);
        }
        let _ = self.0.sender.try_send(Command::Clear);
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "FolderViewStore API"))]
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
                Err(_) => return Err("Folder views flush could not be queued before shutdown".into()),
            }
        }
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "Folder views flush timed out".to_owned())?
    }
}

pub fn folder_heuristics(path: &Path) -> Option<FolderViewConfig> {
    if let Some(pictures_dir) = gtk::glib::user_special_dir(gtk::glib::UserDirectory::Pictures) {
        if path.starts_with(&pictures_dir) {
            return Some(FolderViewConfig {
                mode: Some(TrailBrowserMode::Grid),
                density: None,
                sort_key: None,
                sort_direction: None,
                show_hidden: None,
            });
        }
    }
    if let Some(music_dir) = gtk::glib::user_special_dir(gtk::glib::UserDirectory::Music) {
        if path.starts_with(&music_dir) {
            return Some(FolderViewConfig {
                mode: None,
                density: None,
                sort_key: Some(SortKey::Track),
                sort_direction: Some(SortDirection::Ascending),
                show_hidden: None,
            });
        }
    }
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        if name.eq_ignore_ascii_case("Pictures")
            || name.eq_ignore_ascii_case("Photos")
            || name.eq_ignore_ascii_case("Wallpapers")
            || name.eq_ignore_ascii_case("DCIM")
            || name.eq_ignore_ascii_case("Camera")
            || name.eq_ignore_ascii_case("Screenshots")
        {
            return Some(FolderViewConfig {
                mode: Some(TrailBrowserMode::Grid),
                density: None,
                sort_key: None,
                sort_direction: None,
                show_hidden: None,
            });
        }
        if name.eq_ignore_ascii_case("Music")
            || name.eq_ignore_ascii_case("Audio")
            || name.eq_ignore_ascii_case("Albums")
            || name.eq_ignore_ascii_case("Soundtracks")
        {
            return Some(FolderViewConfig {
                mode: None,
                density: None,
                sort_key: Some(SortKey::Track),
                sort_direction: Some(SortDirection::Ascending),
                show_hidden: None,
            });
        }
    }
    None
}

fn default_storage_path() -> PathBuf {
    crate::paths::folder_views_file()
}

fn read_views(path: &Path) -> (FolderViewsData, bool) {
    if !path.exists() {
        return (FolderViewsData::default(), false);
    }
    let Ok(metadata) = fs::metadata(path) else {
        return (FolderViewsData::default(), false);
    };
    if metadata.len() > MAX_BYTES {
        tracing::warn!("folder_views.json exceeds size limit, resetting to default");
        return (FolderViewsData::default(), false);
    }
    let Ok(mut file) = OpenOptions::new().read(true).open(path) else {
        return (FolderViewsData::default(), false);
    };
    let mut contents = String::new();
    if file.read_to_string(&mut contents).is_err() {
        return (FolderViewsData::default(), false);
    }
    match serde_json::from_str::<FolderViewsData>(&contents) {
        Ok(mut data) => {
            data.prune_if_needed();
            (data, true)
        }
        Err(error) => {
            tracing::warn!(%error, "Unable to parse folder_views.json");
            (FolderViewsData::default(), false)
        }
    }
}

fn write_views(path: &Path, data: &FolderViewsData) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let serialized = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    let temp_path = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        let mut temp_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)
            .map_err(|e| e.to_string())?;
        temp_file
            .write_all(serialized.as_bytes())
            .map_err(|e| e.to_string())?;
        temp_file.flush().map_err(|e| e.to_string())?;
    }
    fs::rename(&temp_path, path).map_err(|e| e.to_string())?;
    Ok(())
}

fn run_worker(receiver: mpsc::Receiver<Command>, path: PathBuf) {
    let (mut data, _) = read_views(&path);
    let mut dirty = false;
    let mut last_save = Instant::now();

    loop {
        let timeout = if dirty {
            let elapsed = last_save.elapsed();
            if elapsed >= DEBOUNCE_DELAY {
                Duration::ZERO
            } else {
                DEBOUNCE_DELAY - elapsed
            }
        } else {
            Duration::from_secs(3600)
        };

        let msg = if timeout.is_zero() {
            receiver.try_recv().ok()
        } else {
            receiver.recv_timeout(timeout).ok()
        };

        if let Some(cmd) = msg {
            match cmd {
                Command::Update { path, update } => {
                    let config = data.views.entry(path).or_default();
                    config.apply_update(update);
                    data.prune_if_needed();
                    dirty = true;
                }
                Command::Remove(path) => {
                    if data.views.remove(&path).is_some() {
                        dirty = true;
                    }
                }
                Command::Relocate { from, to } => {
                    if data.relocate(&from, &to) {
                        dirty = true;
                    }
                }
                Command::Clear => {
                    if !data.views.is_empty() {
                        data.views.clear();
                        dirty = true;
                    }
                }
                Command::Flush(responder) => {
                    if dirty {
                        let res = write_views(&path, &data);
                        dirty = false;
                        last_save = Instant::now();
                        let _ = responder.send(res);
                    } else {
                        let _ = responder.send(Ok(()));
                    }
                }
            }
        } else if dirty {
            if let Err(error) = write_views(&path, &data) {
                tracing::warn!(%error, "Failed to persist folder_views.json");
            }
            dirty = false;
            last_save = Instant::now();
        }
    }
}
