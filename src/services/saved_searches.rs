use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak, mpsc},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::services::ScopedSearch;

mod disk;

const MAX_ENTRIES: usize = 128;
const MAX_NAME_BYTES: usize = 1024;
const MAX_URI_BYTES: usize = 64 * 1024;
const MAX_QUERY_BYTES: usize = 4096;
const MAX_ROOT_BYTES: usize = 4096;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const QUEUE_CAPACITY: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedSearch {
    pub id: u64,
    pub name: String,
    pub uri: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Snapshot {
    pub revision: u64,
    pub ready: bool,
    pub writable: bool,
    pub entries: Vec<SavedSearch>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) enum Command {
    Create {
        name: String,
        uri: String,
    },
    Update {
        previous: SavedSearch,
        name: String,
        uri: String,
    },
    Delete(SavedSearch),
    Reload,
}

enum Request {
    Command(Command),
    Barrier(mpsc::SyncSender<bool>),
}

struct Shared {
    sender: mpsc::SyncSender<Request>,
    snapshot: Arc<Mutex<Snapshot>>,
}

#[derive(Clone)]
pub(crate) struct Store(Arc<Shared>);

impl Store {
    pub(crate) fn shared() -> Self {
        static REGISTRY: OnceLock<Mutex<Weak<Shared>>> = OnceLock::new();
        let mut registry = REGISTRY
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(shared) = registry.upgrade() {
            return Self(shared);
        }
        let store = Self::spawn(None);
        *registry = Arc::downgrade(&store.0);
        store
    }

    #[cfg(test)]
    pub(crate) fn start(path: PathBuf) -> Self {
        Self::spawn(Some(path))
    }

    fn spawn(path: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let output = snapshot.clone();
        let worker = std::thread::Builder::new()
            .name("strata-saved-searches".into())
            .spawn(move || {
                let path = path.unwrap_or_else(|| {
                    gio::glib::user_data_dir().join("strata/saved-searches/queries-v1.json")
                });
                run(path, receiver, output);
            });
        if let Err(error) = worker {
            publish(&snapshot, &[], Some(error.to_string()), false);
        }
        Self(Arc::new(Shared { sender, snapshot }))
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        self.0
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.0
            .snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .revision
    }

    pub(crate) fn send(&self, command: Command) -> Result<(), String> {
        let bounded = match &command {
            Command::Create { name, uri } => bounded_fields(name, uri),
            Command::Update {
                previous,
                name,
                uri,
            } => bounded_fields(name, uri) && bounded_fields(&previous.name, &previous.uri),
            Command::Delete(previous) => bounded_fields(&previous.name, &previous.uri),
            Command::Reload => true,
        };
        if !bounded {
            return Err("Saved search request exceeds the name or location byte limit".into());
        }
        self.0
            .sender
            .try_send(Request::Command(command))
            .map_err(|_| "Saved search worker is busy or unavailable; please try again".into())
    }

    pub(crate) fn flush(&self, timeout: Duration) -> bool {
        let Some(deadline) = Instant::now().checked_add(timeout) else {
            return false;
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut request = Request::Barrier(sender);
        loop {
            match self.0.sender.try_send(request) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(pending)) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return false;
                    }
                    request = pending;
                    std::thread::sleep(remaining.min(Duration::from_millis(1)));
                }
                Err(mpsc::TrySendError::Disconnected(_)) => return false,
            }
        }
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    next_id: u64,
    entries: Vec<SavedSearch>,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            next_id: 1,
            entries: Vec::new(),
        }
    }
}

impl Document {
    fn validate(&mut self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported saved search version; the existing file is preserved".into());
        }
        if self.entries.len() > MAX_ENTRIES || self.next_id == 0 {
            return Err("Invalid saved search entry count or next ID".into());
        }
        let mut identifiers = BTreeSet::new();
        let mut names = BTreeSet::new();
        for entry in &mut self.entries {
            if entry.id == 0 || entry.id >= self.next_id || !identifiers.insert(entry.id) {
                return Err("Invalid or duplicate saved search ID".into());
            }
            entry.name = valid_name(&entry.name)?;
            entry.uri = canonical_uri(&entry.uri)?;
            if !names.insert(gio::glib::casefold(&entry.name).to_string()) {
                return Err("Saved search names must be unique".into());
            }
        }
        Ok(())
    }

    fn apply(&mut self, command: Command) -> Result<bool, String> {
        match command {
            Command::Create { name, uri } => {
                let name = self.unique_name(&name, None)?;
                let uri = canonical_uri(&uri)?;
                if self.entries.len() >= MAX_ENTRIES {
                    return Err("Saved search limit reached (128)".into());
                }
                let next_id = self
                    .next_id
                    .checked_add(1)
                    .ok_or("Saved search IDs exhausted")?;
                self.entries.push(SavedSearch {
                    id: self.next_id,
                    name,
                    uri,
                });
                self.next_id = next_id;
                Ok(true)
            }
            Command::Update {
                previous,
                name,
                uri,
            } => {
                let position = self.position(&previous)?;
                let name = self.unique_name(&name, Some(previous.id))?;
                let uri = canonical_uri(&uri)?;
                let updated = SavedSearch {
                    id: previous.id,
                    name,
                    uri,
                };
                if self.entries[position] == updated {
                    return Ok(false);
                }
                self.entries[position] = updated;
                Ok(true)
            }
            Command::Delete(previous) => {
                let position = self.position(&previous)?;
                self.entries.remove(position);
                Ok(true)
            }
            Command::Reload => Err("Reload must be handled by the storage worker".into()),
        }
    }

    fn position(&self, previous: &SavedSearch) -> Result<usize, String> {
        self.entries
            .iter()
            .position(|entry| entry == previous)
            .ok_or_else(|| "Saved search changed or no longer exists; reload before editing".into())
    }

    fn unique_name(&self, name: &str, except: Option<u64>) -> Result<String, String> {
        let name = valid_name(name)?;
        let folded = gio::glib::casefold(&name);
        if self
            .entries
            .iter()
            .any(|entry| Some(entry.id) != except && gio::glib::casefold(&entry.name) == folded)
        {
            return Err("A saved search already uses that name".into());
        }
        Ok(name)
    }
}

fn bounded_fields(name: &str, uri: &str) -> bool {
    name.len() <= MAX_NAME_BYTES && uri.len() <= MAX_URI_BYTES
}

fn valid_name(name: &str) -> Result<String, String> {
    if name.len() > MAX_NAME_BYTES || name.chars().any(char::is_control) {
        return Err("Use a saved search name of 1–80 characters without control characters".into());
    }
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err("Use a saved search name of 1–80 characters without control characters".into());
    }
    Ok(name.to_owned())
}

fn canonical_uri(uri: &str) -> Result<String, String> {
    if uri.len() > MAX_URI_BYTES || uri.chars().any(char::is_control) {
        return Err("Saved search location is invalid or too large".into());
    }
    let search = ScopedSearch::from_uri(uri)?;
    if search.query.len() > MAX_QUERY_BYTES
        || search.query.chars().any(char::is_control)
        || search.root.as_os_str().len() > MAX_ROOT_BYTES
        || search
            .root
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .any(|byte| *byte < 32 || *byte == 127)
    {
        return Err("Saved search query or root is invalid or too large".into());
    }
    let canonical = search.to_uri();
    if canonical.len() > MAX_URI_BYTES {
        return Err("Saved search location is too large".into());
    }
    Ok(canonical)
}

fn publish(
    output: &Mutex<Snapshot>,
    entries: &[SavedSearch],
    error: Option<String>,
    writable: bool,
) {
    let mut snapshot = output.lock().unwrap_or_else(|error| error.into_inner());
    snapshot.revision = snapshot.revision.saturating_add(1);
    snapshot.ready = true;
    snapshot.writable = writable;
    if snapshot.entries != entries {
        snapshot.entries = entries.to_vec();
    }
    snapshot.error = error;
}

fn reload(storage: &mut Option<disk::Storage>, path: &std::path::Path) -> Result<Document, String> {
    *storage = None;
    *storage = Some(disk::Storage::open(path)?);
    storage
        .as_mut()
        .ok_or("Saved search storage unavailable")?
        .load()
}

fn run(path: PathBuf, receiver: mpsc::Receiver<Request>, output: Arc<Mutex<Snapshot>>) {
    let mut storage = None;
    let loaded = reload(&mut storage, &path);
    let mut blocked = loaded.as_ref().err().cloned();
    let mut error = blocked.clone();
    let mut data = loaded.unwrap_or_default();
    publish(&output, &data.entries, error.clone(), blocked.is_none());
    while let Ok(request) = receiver.recv() {
        let command = match request {
            Request::Barrier(reply) => {
                let _sent = reply.try_send(blocked.is_none() && error.is_none());
                continue;
            }
            Request::Command(command) => command,
        };
        if matches!(command, Command::Reload) {
            match reload(&mut storage, &path) {
                Ok(loaded) => {
                    data = loaded;
                    blocked = None;
                    error = None;
                }
                Err(message) => {
                    blocked = Some(message.clone());
                    error = Some(message);
                }
            }
        } else if let Some(message) = &blocked {
            error = Some(message.clone());
        } else {
            let mut candidate = data.clone();
            match candidate.apply(command) {
                Ok(changed) => {
                    let saved = storage
                        .as_mut()
                        .ok_or_else(|| "Saved search storage unavailable".to_owned())
                        .and_then(|storage| storage.save(&candidate, changed));
                    match saved {
                        Ok(()) => {
                            data = candidate;
                            error = None;
                        }
                        Err(message) => {
                            blocked = Some(message.clone());
                            error = Some(message);
                        }
                    }
                }
                Err(message) => error = Some(message),
            }
        }
        publish(&output, &data.entries, error.clone(), blocked.is_none());
    }
}

#[cfg(test)]
mod tests;
