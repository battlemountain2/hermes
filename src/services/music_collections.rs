use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::model::Location;

mod disk;

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TRACKS: usize = 20_000;
const MAX_PLAYLISTS: usize = 128;
const MAX_MEMBERSHIPS: usize = 100_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Playlist {
    pub id: u64,
    pub name: String,
    pub tracks: Vec<Location>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Indexed {
    path: Location,
    added: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PlayCount {
    path: Location,
    count: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Collections {
    version: u32,
    next_id: u64,
    favorites: Vec<Location>,
    pub playlists: Vec<Playlist>,
    indexed: Vec<Indexed>,
    #[serde(default)]
    played: Vec<PlayCount>,
}

impl Default for Collections {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            favorites: Vec::new(),
            playlists: Vec::new(),
            indexed: Vec::new(),
            played: Vec::new(),
        }
    }
}

impl Collections {
    pub fn favorite_paths(&self) -> BTreeSet<PathBuf> {
        self.favorites
            .iter()
            .filter_map(|path| path.native_path().map(Path::to_owned))
            .collect()
    }

    pub fn playlist(&self, id: u64) -> Option<&Playlist> {
        self.playlists.iter().find(|playlist| playlist.id == id)
    }

    pub fn added_dates(&self) -> BTreeMap<PathBuf, u64> {
        self.indexed
            .iter()
            .filter_map(|entry| Some((entry.path.native_path()?.to_owned(), entry.added)))
            .collect()
    }

    pub fn play_counts(&self) -> BTreeMap<PathBuf, u64> {
        self.played
            .iter()
            .filter_map(|entry| Some((entry.path.native_path()?.to_owned(), entry.count)))
            .collect()
    }

    fn relocate_path(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
        if path == from {
            Some(to.to_path_buf())
        } else if let Ok(suffix) = path.strip_prefix(from) {
            Some(to.join(suffix))
        } else {
            None
        }
    }

    fn relocate(&mut self, from: &Path, to: &Path) -> bool {
        if from == to {
            return false;
        }
        let mut changed = false;

        let mut seen_favorites = BTreeSet::new();
        let mut new_favorites = Vec::new();
        for loc in self.favorites.drain(..) {
            let native = loc.native_path().map(Path::to_owned);
            let updated = if let Some(p) = native.as_deref() {
                if let Some(target) = Self::relocate_path(p, from, to) {
                    if valid_path(&target) {
                        changed = true;
                        Location::local(target)
                    } else {
                        loc
                    }
                } else {
                    loc
                }
            } else {
                loc
            };
            if let Some(p) = updated.native_path() {
                if seen_favorites.insert(p.to_owned()) {
                    new_favorites.push(updated);
                } else {
                    changed = true;
                }
            } else {
                new_favorites.push(updated);
            }
        }
        self.favorites = new_favorites;

        for playlist in &mut self.playlists {
            for track in &mut playlist.tracks {
                if let Some(p) = track.native_path() {
                    if let Some(target) = Self::relocate_path(p, from, to) {
                        if valid_path(&target) {
                            *track = Location::local(target);
                            changed = true;
                        }
                    }
                }
            }
        }

        let mut seen_indexed = BTreeSet::new();
        let mut new_indexed = Vec::new();
        for entry in self.indexed.drain(..) {
            let native = entry.path.native_path().map(Path::to_owned);
            let updated_path = if let Some(p) = native.as_deref() {
                if let Some(target) = Self::relocate_path(p, from, to) {
                    if valid_path(&target) {
                        changed = true;
                        Location::local(target)
                    } else {
                        entry.path
                    }
                } else {
                    entry.path
                }
            } else {
                entry.path
            };
            if let Some(p) = updated_path.native_path() {
                if seen_indexed.insert(p.to_owned()) {
                    new_indexed.push(Indexed {
                        path: updated_path,
                        added: entry.added,
                    });
                } else {
                    changed = true;
                }
            } else {
                new_indexed.push(Indexed {
                    path: updated_path,
                    added: entry.added,
                });
            }
        }
        self.indexed = new_indexed;

        let mut new_played: Vec<PlayCount> = Vec::new();
        for entry in self.played.drain(..) {
            let native = entry.path.native_path().map(Path::to_owned);
            let updated_path = if let Some(p) = native.as_deref() {
                if let Some(target) = Self::relocate_path(p, from, to) {
                    if valid_path(&target) {
                        changed = true;
                        Location::local(target)
                    } else {
                        entry.path
                    }
                } else {
                    entry.path
                }
            } else {
                entry.path
            };
            if let Some(existing) = new_played.iter_mut().find(|e| e.path == updated_path) {
                existing.count = existing.count.saturating_add(entry.count);
                changed = true;
            } else {
                new_played.push(PlayCount {
                    path: updated_path,
                    count: entry.count,
                });
            }
        }
        self.played = new_played;

        changed
    }

    fn apply(&mut self, command: Command) -> Result<bool, String> {
        match command {
            Command::Create(name) => {
                let name = valid_name(name)?;
                if self.playlists.len() >= MAX_PLAYLISTS || self.next_id == u64::MAX {
                    return Err("Playlist limit reached (128)".into());
                }
                self.playlists.push(Playlist {
                    id: self.next_id,
                    name,
                    tracks: Vec::new(),
                });
                self.next_id += 1;
            }
            Command::Rename(id, name) => {
                let name = valid_name(name)?;
                let playlist = self
                    .playlists
                    .iter_mut()
                    .find(|playlist| playlist.id == id)
                    .ok_or("Playlist no longer exists")?;
                if playlist.name == name {
                    return Ok(false);
                }
                playlist.name = name;
            }
            Command::Delete(id) => {
                let before = self.playlists.len();
                self.playlists.retain(|playlist| playlist.id != id);
                return Ok(before != self.playlists.len());
            }
            Command::Favorite(paths, favorite) => {
                let paths = checked_paths(paths)?;
                let before = self.favorite_paths();
                let mut after = before.clone();
                for path in paths {
                    if favorite {
                        after.insert(path);
                    } else {
                        after.remove(&path);
                    }
                }
                if after.len() > MAX_TRACKS {
                    return Err("Favorite limit reached (20,000)".into());
                }
                if before == after {
                    return Ok(false);
                }
                self.favorites = after.into_iter().map(Location::local).collect();
            }
            Command::Add(id, paths) => {
                let paths = checked_paths(paths)?;
                let memberships: usize = self
                    .playlists
                    .iter()
                    .map(|playlist| playlist.tracks.len())
                    .sum();
                let playlist = self
                    .playlists
                    .iter_mut()
                    .find(|playlist| playlist.id == id)
                    .ok_or("Playlist no longer exists")?;
                let mut seen: BTreeSet<_> = playlist
                    .tracks
                    .iter()
                    .filter_map(Location::native_path)
                    .map(Path::to_owned)
                    .collect();
                let additions: Vec<_> = paths
                    .into_iter()
                    .filter(|path| seen.insert(path.clone()))
                    .map(Location::local)
                    .collect();
                if playlist.tracks.len() + additions.len() > MAX_TRACKS
                    || memberships + additions.len() > MAX_MEMBERSHIPS
                {
                    return Err("Playlist track limit reached".into());
                }
                if additions.is_empty() {
                    return Ok(false);
                }
                playlist.tracks.extend(additions);
            }
            Command::Remove(id, paths) => {
                let paths: BTreeSet<_> = checked_paths(paths)?.into_iter().collect();
                let playlist = self
                    .playlists
                    .iter_mut()
                    .find(|playlist| playlist.id == id)
                    .ok_or("Playlist no longer exists")?;
                let before = playlist.tracks.len();
                playlist
                    .tracks
                    .retain(|path| path.native_path().is_none_or(|path| !paths.contains(path)));
                return Ok(before != playlist.tracks.len());
            }
            Command::Move(id, path, offset) => {
                let playlist = self
                    .playlists
                    .iter_mut()
                    .find(|playlist| playlist.id == id)
                    .ok_or("Playlist no longer exists")?;
                let Some(source) = playlist
                    .tracks
                    .iter()
                    .position(|entry| entry.native_path() == Some(path.as_path()))
                else {
                    return Ok(false);
                };
                let destination = source
                    .saturating_add_signed(offset)
                    .min(playlist.tracks.len().saturating_sub(1));
                if source == destination {
                    return Ok(false);
                }
                let track = playlist.tracks.remove(source);
                playlist.tracks.insert(destination, track);
            }
            Command::MoveBefore(id, path, anchor, after) => {
                let playlist = self
                    .playlists
                    .iter_mut()
                    .find(|playlist| playlist.id == id)
                    .ok_or("Playlist no longer exists")?;
                let Some(source) = playlist
                    .tracks
                    .iter()
                    .position(|entry| entry.native_path() == Some(path.as_path()))
                else {
                    return Ok(false);
                };
                let Some(destination) = playlist
                    .tracks
                    .iter()
                    .position(|entry| entry.native_path() == Some(anchor.as_path()))
                else {
                    return Ok(false);
                };
                let destination = destination + usize::from(after);
                let destination = destination - usize::from(source < destination);
                if source == destination {
                    return Ok(false);
                }
                let track = playlist.tracks.remove(source);
                playlist.tracks.insert(destination, track);
            }
            Command::Observe(paths, now) => {
                let paths = checked_paths(paths)?;
                let mut seen: BTreeSet<_> = self
                    .indexed
                    .iter()
                    .filter_map(|entry| entry.path.native_path().map(Path::to_owned))
                    .collect();
                let additions: Vec<_> = paths
                    .into_iter()
                    .filter(|path| seen.insert(path.clone()))
                    .map(|path| Indexed {
                        path: Location::local(path),
                        added: now,
                    })
                    .collect();
                if self.indexed.len() + additions.len() > MAX_TRACKS {
                    return Err("Recently added history limit reached (20,000)".into());
                }
                if additions.is_empty() {
                    return Ok(false);
                }
                self.indexed.extend(additions);
            }
            Command::Played(path) => {
                if !valid_path(&path) {
                    return Err("Listening history requires a local audio path".into());
                }
                if let Some(entry) = self
                    .played
                    .iter_mut()
                    .find(|entry| entry.path.native_path() == Some(path.as_path()))
                {
                    entry.count = entry.count.saturating_add(1);
                } else {
                    if self.played.len() >= MAX_TRACKS {
                        return Err("Listening history limit reached (20,000)".into());
                    }
                    self.played.push(PlayCount {
                        path: Location::local(path),
                        count: 1,
                    });
                }
            }
            Command::Relocate { from, to } => {
                if !valid_relocate_path(&from) || !valid_relocate_path(&to) {
                    return Err("Invalid relocate path".into());
                }
                return Ok(self.relocate(&from, &to));
            }
            Command::Flush(_) => return Ok(false),
        }
        Ok(true)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Command {
    Create(String),
    Rename(u64, String),
    Delete(u64),
    Favorite(Vec<PathBuf>, bool),
    Add(u64, Vec<PathBuf>),
    Remove(u64, Vec<PathBuf>),
    Move(u64, PathBuf, isize),
    MoveBefore(u64, PathBuf, PathBuf, bool),
    Observe(Vec<PathBuf>, u64),
    Played(PathBuf),
    Relocate { from: PathBuf, to: PathBuf },
    Flush(mpsc::SyncSender<Result<(), String>>),
}

#[derive(Default)]
pub(crate) struct Snapshot {
    pub revision: u64,
    pub ready: bool,
    pub error: Option<String>,
    pub data: Collections,
}

struct Shared {
    sender: mpsc::SyncSender<Command>,
    snapshot: Arc<Mutex<Arc<Snapshot>>>,
}

#[derive(Clone)]
pub(crate) struct Store(Arc<Shared>);

impl Store {
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
                Err(_) => return Err("Collection flush could not be queued before shutdown".into()),
            }
        }
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "Collection flush timed out during shutdown".to_owned())?
    }

    pub fn record_listen(
        &self,
        path: PathBuf,
        listened_us: i64,
        duration_us: i64,
    ) -> Result<bool, String> {
        let threshold = if duration_us > 0 {
            (duration_us / 2).clamp(1, 30_000_000)
        } else {
            30_000_000
        };
        if listened_us < threshold {
            return Ok(false);
        }
        self.send(Command::Played(path))?;
        Ok(true)
    }

    pub fn relocate(&self, from: PathBuf, to: PathBuf) -> Result<(), String> {
        if from == to {
            return Ok(());
        }
        self.send(Command::Relocate { from, to })
    }

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
            gio::glib::user_data_dir().join("strata/music-collections-v1.json"),
        ));
        *shared = Arc::downgrade(&store.0);
        store
    }

    pub fn start(path: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::sync_channel(8);
        let snapshot = Arc::new(Mutex::new(Arc::new(Snapshot::default())));
        let output = snapshot.clone();
        let worker = std::thread::Builder::new()
            .name("hermes-collections".into())
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

    pub fn send(&self, command: Command) -> Result<(), String> {
        match &command {
            Command::Favorite(paths, _)
            | Command::Add(_, paths)
            | Command::Remove(_, paths)
            | Command::Observe(paths, _)
                if paths.len() > MAX_TRACKS
                    || paths
                        .iter()
                        .map(|path| path.as_os_str().len())
                        .sum::<usize>()
                        > 4 * 1024 * 1024 =>
            {
                return Err("Collection request exceeds 20,000 tracks / 4 MiB of paths".into());
            }
            Command::Create(name) | Command::Rename(_, name) if name.len() > 512 => {
                return Err("Playlist name is too long".into());
            }
            Command::Played(path) | Command::Move(_, path, _) if !valid_path(path) => {
                return Err("Invalid collection audio path".into());
            }
            Command::MoveBefore(_, path, anchor, _) if !valid_path(path) || !valid_path(anchor) => {
                return Err("Invalid playlist reorder path".into());
            }
            Command::Relocate { from, to }
                if !valid_relocate_path(from) || !valid_relocate_path(to) =>
            {
                return Err("Invalid relocate path".into());
            }
            _ => {}
        }
        self.0
            .sender
            .try_send(command)
            .map_err(|_| "Collection worker is busy or unavailable; please try again".into())
    }
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn run(
    path: Option<PathBuf>,
    receiver: mpsc::Receiver<Command>,
    output: Arc<Mutex<Arc<Snapshot>>>,
) {
    let mut storage = path.as_deref().map(disk::Storage::open);
    let loaded = match storage.as_ref() {
        Some(Ok(storage)) => storage.load(),
        Some(Err(_)) => path
            .as_deref()
            .map_or_else(|| Ok(Collections::default()), load),
        None => Ok(Collections::default()),
    };
    let storage_error = storage
        .as_ref()
        .and_then(|storage| storage.as_ref().err().cloned());
    let read_only = loaded.is_err() || storage_error.is_some();
    let mut error = storage_error.or_else(|| loaded.as_ref().err().cloned());
    let mut data = loaded.unwrap_or_default();
    let mut revision = 1;
    let mut dirty = false;
    let mut last_save = Instant::now();
    let mut retry_at = None;
    let mut backoff = Duration::from_secs(1);
    let publish = |data: &Collections, revision, error: &Option<String>| {
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
                    error = Some("Saved collections are read-only; existing data is preserved. Restart Hermes after resolving the storage issue.".into());
                }
            } else {
                let mut candidate = data.clone();
                match candidate.apply(command).and_then(|changed| {
                    if changed {
                        let bytes =
                            serde_json::to_vec(&candidate).map_err(|error| error.to_string())?;
                        if bytes.len() as u64 > MAX_BYTES {
                            return Err("Collections exceed 16 MiB; change was not applied".into());
                        }
                    }
                    Ok(changed)
                }) {
                    Ok(changed) => {
                        if changed {
                            data = candidate;
                        }
                        dirty |= changed;
                    }
                    Err(message) => error = Some(message),
                }
            }
            revision += 1;
            publish(&data, revision, &error);
        }
        let retry_due = retry_at.is_some_and(|deadline| Instant::now() >= deadline);
        if dirty
            && (disconnected
                || flush.is_some()
                || retry_due
                || (retry_at.is_none()
                    && (last_save.elapsed() >= Duration::from_secs(1) || timed_out)))
        {
            let saved = match storage.as_mut() {
                Some(Ok(storage)) => storage.save(&data),
                Some(Err(error)) => Err(error.clone()),
                None => Ok(()),
            };
            if let Err(message) = saved {
                error = Some(message);
                retry_at = Some(Instant::now() + backoff);
                backoff = (backoff * 2).min(Duration::from_secs(30));
            } else {
                dirty = false;
                retry_at = None;
                backoff = Duration::from_secs(1);
                error = None;
            }
            last_save = Instant::now();
            revision += 1;
            publish(&data, revision, &error);
        }
        if let Some(reply) = flush {
            let _sent = reply.try_send(if dirty || read_only {
                Err(error
                    .clone()
                    .unwrap_or_else(|| "Collections could not be flushed".into()))
            } else {
                Ok(())
            });
        }
        if disconnected {
            break;
        }
    }
}

fn valid_name(name: String) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("Use a playlist name of 1–80 characters without control characters".into());
    }
    Ok(name.to_owned())
}

fn checked_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    if paths.len() > MAX_TRACKS || paths.iter().any(|path| !valid_path(path)) {
        return Err("Collection contains an invalid or non-local audio path".into());
    }
    Ok(paths)
}

fn valid_path(path: &Path) -> bool {
    path.is_absolute()
        && path.as_os_str().len() <= 4096
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        && super::classify_by_name(path.as_os_str()) == super::FormatFamily::Audio
}

fn valid_relocate_path(path: &Path) -> bool {
    path.is_absolute()
        && path.as_os_str().len() <= 4096
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
}

fn validate(data: &Collections) -> Result<(), String> {
    let valid_locations = |locations: &[Location], limit| {
        let paths: Option<BTreeSet<_>> = locations.iter().map(Location::native_path).collect();
        paths.is_some_and(|paths| {
            locations.len() <= limit
                && paths.len() == locations.len()
                && paths.iter().all(|path| valid_path(path))
        })
    };
    let ids: BTreeSet<_> = data.playlists.iter().map(|playlist| playlist.id).collect();
    let indexed: Vec<_> = data
        .indexed
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    let played: Vec<_> = data.played.iter().map(|entry| entry.path.clone()).collect();
    if data.version != 1
        || data.next_id == 0
        || data.playlists.len() > MAX_PLAYLISTS
        || ids.len() != data.playlists.len()
        || ids.iter().any(|id| *id == 0 || *id >= data.next_id)
        || !valid_locations(&data.favorites, MAX_TRACKS)
        || !valid_locations(&indexed, MAX_TRACKS)
        || !valid_locations(&played, MAX_TRACKS)
        || data.played.iter().any(|entry| entry.count == 0)
        || data
            .playlists
            .iter()
            .map(|playlist| playlist.tracks.len())
            .sum::<usize>()
            > MAX_MEMBERSHIPS
        || data.playlists.iter().any(|playlist| {
            valid_name(playlist.name.clone()).ok().as_ref() != Some(&playlist.name)
                || !valid_locations(&playlist.tracks, MAX_TRACKS)
        })
    {
        return Err("Saved collections are invalid or exceed their limits".into());
    }
    Ok(())
}

fn load(path: &Path) -> Result<Collections, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Collections::default());
        }
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_BYTES => {}
        Ok(_) => return Err("Collection file is not a regular file or exceeds 16 MiB".into()),
        Err(error) => return Err(error.to_string()),
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Collections exceed 16 MiB".into());
    }
    let data = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    validate(&data)?;
    Ok(data)
}

#[cfg(test)]
fn write(path: &Path, data: &Collections) -> Result<(), String> {
    write_guarded(path, data, || Ok(()))
}

fn write_guarded(
    path: &Path,
    data: &Collections,
    unchanged: impl Fn() -> Result<(), String>,
) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    validate(data)?;
    let bytes = serde_json::to_vec(data).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Collections exceed 16 MiB".into());
    }
    fs::create_dir_all(path.parent().ok_or("Invalid collections path")?)
        .map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let save = || -> Result<(), String> {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        output
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        output.sync_all().map_err(|error| error.to_string())?;
        unchanged()?;
        fs::rename(&temporary, path).map_err(|error| error.to_string())
    };
    let result = save();
    if result.is_err() {
        let _removed = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests;
