use std::{
    collections::HashMap,
    fs,
    mem::size_of,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{FormatFamily, classify_by_name};
use crate::{model::MusicTags, sandbox::Cancellation};

mod cache;
mod monitoring;

const MAX_ENTRIES: usize = 40_000;
const MAX_TRACKS: usize = 20_000;
const MAX_INDEX_BYTES: usize = 32 * 1024 * 1024;
const MAX_WALK_ENTRIES: usize = 100_000;
const PUBLISH_INTERVAL: Duration = Duration::from_millis(500);
const TICK: Duration = Duration::from_millis(20);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct FileIdentity {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl FileIdentity {
    pub(crate) fn size(&self) -> u64 {
        self.size
    }
    pub(crate) fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }

    fn read(path: &Path) -> Option<Self> {
        fs::symlink_metadata(path)
            .ok()
            .filter(|metadata| !metadata.file_type().is_symlink())
            .as_ref()
            .map(Self::from_metadata)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    pub path: PathBuf,
    pub is_directory: bool,
    pub modified_unix_seconds: Option<i64>,
    pub identity: Option<FileIdentity>,
    pub music_tags: Option<MusicTags>,
}

impl Entry {
    pub(crate) fn is_audio(&self) -> bool {
        !self.is_directory && classify_by_name(self.path.as_os_str()) == FormatFamily::Audio
    }
}

#[derive(Clone, Default, Eq, PartialEq)]
pub(crate) struct Snapshot {
    pub entries: Arc<Vec<Entry>>,
    pub indexing: bool,
    pub notice: Option<String>,
}

type TagReader = dyn Fn(&Path, &Cancellation) -> Option<MusicTags> + Send + Sync;

struct Configuration {
    root: PathBuf,
    cache_path: PathBuf,
    tags: Arc<TagReader>,
    monitor_limit: usize,
    refresh_interval: Duration,
    debounce: Duration,
    entries: usize,
    tracks: usize,
    bytes: usize,
}

impl Configuration {
    fn new(root: PathBuf) -> Self {
        Self {
            cache_path: cache::path(&root),
            root,
            tags: Arc::new(crate::adapters::music_metadata::read_tags),
            monitor_limit: 4096,
            refresh_interval: Duration::from_secs(60),
            debounce: Duration::from_millis(250),
            entries: MAX_ENTRIES,
            tracks: MAX_TRACKS,
            bytes: MAX_INDEX_BYTES,
        }
    }
}

struct Shared {
    snapshot: Arc<Snapshot>,
    revision: u64,
    subscribers: usize,
    requested: Option<Instant>,
    immediate: bool,
}

struct Library {
    configuration: Configuration,
    shared: Mutex<Shared>,
    ready: Condvar,
    cancellation: Cancellation,
}

pub(crate) struct Subscription {
    library: Arc<Library>,
    seen: Option<u64>,
}

impl Subscription {
    pub(crate) fn poll(&mut self) -> Option<Arc<Snapshot>> {
        let shared = self
            .library
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.seen == Some(shared.revision) {
            return None;
        }
        self.seen = Some(shared.revision);
        Some(shared.snapshot.clone())
    }

    pub(crate) fn refresh(&self) {
        self.library.request(true);
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut shared = self
            .library
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        shared.subscribers = shared.subscribers.saturating_sub(1);
        if shared.subscribers == 0 {
            self.library.cancellation.cancel();
            self.library.ready.notify_all();
        }
    }
}

pub(crate) fn subscribe(root: PathBuf) -> Subscription {
    subscribe_with(Configuration::new(root))
}

fn registry() -> &'static Mutex<Vec<Weak<Library>>> {
    static REGISTRY: OnceLock<Mutex<Vec<Weak<Library>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn cached_tags(path: &Path) -> Option<MusicTags> {
    let identity = FileIdentity::read(path)?;
    let libraries: Vec<_> = registry()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .filter_map(Weak::upgrade)
        .filter(|library| path.starts_with(&library.configuration.root))
        .collect();
    for library in libraries {
        let snapshot = library
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot
            .clone();
        if let Some(entry) = snapshot
            .entries
            .iter()
            .find(|entry| entry.path == path && entry.identity.as_ref() == Some(&identity))
            && let Some(tags) = &entry.music_tags
        {
            return Some(tags.clone());
        }
    }
    None
}

fn subscribe_with(configuration: Configuration) -> Subscription {
    let mut registry = registry().lock().unwrap_or_else(|error| error.into_inner());
    registry.retain(|weak| weak.strong_count() > 0);
    for weak in registry.iter() {
        if let Some(library) = weak.upgrade()
            && library.configuration.root == configuration.root
        {
            let mut shared = library
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if library.cancellation.is_cancelled() {
                continue;
            }
            shared.subscribers += 1;
            if !shared.snapshot.indexing {
                shared.requested = Some(Instant::now());
                shared.immediate = true;
                library.ready.notify_one();
            }
            drop(shared);
            return Subscription {
                library,
                seen: None,
            };
        }
    }
    let library = Arc::new(Library {
        configuration,
        shared: Mutex::new(Shared {
            snapshot: Arc::new(Snapshot {
                indexing: true,
                ..Snapshot::default()
            }),
            revision: 0,
            subscribers: 1,
            requested: Some(Instant::now()),
            immediate: true,
        }),
        ready: Condvar::new(),
        cancellation: Cancellation::default(),
    });
    registry.push(Arc::downgrade(&library));
    drop(registry);
    let worker = library.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("hermes-library".into())
        .spawn(move || {
            let context = gio::glib::MainContext::new();
            if let Err(error) = context.with_thread_default(|| worker.run(&context)) {
                worker.publish(Snapshot {
                    notice: Some(format!("Library monitoring unavailable: {error}")),
                    ..Snapshot::default()
                });
            }
        })
    {
        library.publish(Snapshot {
            notice: Some(format!("Unable to start music library: {error}")),
            ..Snapshot::default()
        });
    }
    Subscription {
        library,
        seen: None,
    }
}

impl Library {
    fn request(&self, immediate: bool) {
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        shared.requested.get_or_insert_with(Instant::now);
        shared.immediate |= immediate;
        self.ready.notify_one();
    }

    fn requested(&self) -> bool {
        self.shared
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requested
            .is_some()
    }

    fn publish(&self, snapshot: Snapshot) {
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *shared.snapshot == snapshot {
            return;
        }
        shared.snapshot = Arc::new(snapshot);
        shared.revision = shared.revision.wrapping_add(1);
        self.ready.notify_all();
    }

    fn run(self: &Arc<Self>, context: &gio::glib::MainContext) {
        let mut monitors = monitoring::Monitors::new(self);
        let mut previous = Arc::new(Vec::new());
        let mut cached = cache::read(&self.configuration.cache_path, &self.configuration.root);
        let mut completed = Instant::now();
        while !self.cancellation.is_cancelled() {
            pump(context, &self.cancellation);
            let scan = {
                let mut shared = self
                    .shared
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let due = shared.requested.is_some_and(|time| {
                    shared.immediate || time.elapsed() >= self.configuration.debounce
                }) || completed.elapsed() >= self.configuration.refresh_interval;
                if due {
                    shared.requested = None;
                    shared.immediate = false;
                }
                due
            };
            if scan {
                let (entries, notice) = self.scan(context, &mut monitors, &previous, &mut cached);
                previous = Arc::new(entries);
                let pending = self.requested();
                let mut notice = notice;
                if !pending
                    && let Err(error) = cache::write(&self.configuration.cache_path, &previous)
                {
                    append_notice(
                        &mut notice,
                        &format!("Library cache could not be saved: {error}"),
                    );
                }
                if self.cancellation.is_cancelled() {
                    break;
                }
                self.publish(Snapshot {
                    entries: previous.clone(),
                    indexing: pending,
                    notice,
                });
                completed = Instant::now();
            }
            let shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let _wait = self.ready.wait_timeout(shared, TICK);
        }
    }

    fn scan(
        self: &Arc<Self>,
        context: &gio::glib::MainContext,
        monitors: &mut monitoring::Monitors,
        previous: &[Entry],
        cached: &mut HashMap<PathBuf, cache::CachedTags>,
    ) -> (Vec<Entry>, Option<String>) {
        let root = &self.configuration.root;
        let mut notice = None;
        monitors.begin();
        monitors.parent(root);
        if let Err(error) = fs::read_dir(root) {
            monitors.finish(&mut notice);
            return (
                Vec::new(),
                Some(format!(
                    "Music folder unavailable: {}: {error}",
                    root.display()
                )),
            );
        }
        monitors.add(root);
        let remembered: HashMap<_, _> = previous
            .iter()
            .filter(|entry| entry.music_tags.is_some())
            .map(|entry| (entry.path.as_path(), entry))
            .collect();
        let mut entries = Vec::new();
        let mut bytes = 0;
        let mut tracks = 0;
        let walker = ignore::WalkBuilder::new(root)
            .follow_links(false)
            .standard_filters(false)
            .hidden(true)
            .max_depth(Some(128))
            .build();
        for (count, entry) in walker.enumerate() {
            pump(context, &self.cancellation);
            if self.cancellation.is_cancelled() {
                break;
            }
            if count >= MAX_WALK_ENTRIES {
                append_notice(&mut notice, "Library walk limited to 100,000 entries");
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    append_notice(&mut notice, "Some library folders could not be read");
                    continue;
                }
            };
            if entry.depth() == 0 || entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let kind = entry.file_type();
            if kind.is_none_or(|kind| !kind.is_dir() && !kind.is_file()) {
                continue;
            }
            let directory = kind.is_some_and(|kind| kind.is_dir());
            if directory {
                monitors.add(entry.path());
                if entry.depth() == 128 {
                    append_notice(&mut notice, "Folders deeper than 128 levels were skipped");
                }
            }
            let identity = entry
                .metadata()
                .ok()
                .as_ref()
                .map(FileIdentity::from_metadata);
            let modified_unix_seconds = identity.as_ref().map(|identity| identity.modified.0);
            let path = entry.into_path();
            let mut record = Entry {
                path,
                is_directory: directory,
                modified_unix_seconds,
                identity,
                music_tags: None,
            };
            if record.is_audio() {
                if tracks >= self.configuration.tracks {
                    append_notice(
                        &mut notice,
                        "Library track limit reached; some songs were skipped",
                    );
                    continue;
                }
                tracks += 1;
                record.music_tags = remembered
                    .get(record.path.as_path())
                    .filter(|entry| entry.identity.is_some() && entry.identity == record.identity)
                    .and_then(|entry| entry.music_tags.clone())
                    .or_else(|| {
                        cached
                            .remove(&record.path)
                            .filter(|entry| Some(&entry.identity.file) == record.identity.as_ref())
                            .map(|entry| entry.tags)
                    });
            }
            let cost = entry_bytes(&record) - size_of::<Entry>();
            let capacity = if entries.len() == entries.capacity() {
                (entries.capacity() + 256).min(self.configuration.entries)
            } else {
                entries.capacity()
            };
            let allocation = (capacity - entries.capacity()) * size_of::<Entry>();
            if entries.len() >= self.configuration.entries
                || bytes + cost + allocation > self.configuration.bytes
            {
                append_notice(
                    &mut notice,
                    "Library index limited to 40,000 entries / 32 MiB",
                );
                break;
            }
            if allocation > 0 {
                entries.reserve_exact(capacity - entries.len());
            }
            bytes += cost + allocation;
            entries.push(record);
        }
        monitors.finish(&mut notice);
        cached.clear();
        self.publish(Snapshot {
            entries: Arc::new(entries.clone()),
            indexing: true,
            notice: notice.clone(),
        });
        let mut last_publish = Instant::now();
        let mut failures = 0;
        for index in 0..entries.len() {
            pump(context, &self.cancellation);
            if self.cancellation.is_cancelled() || self.requested() {
                break;
            }
            if !entries[index].is_audio() || entries[index].music_tags.is_some() {
                continue;
            }
            let old_cost = entry_bytes(&entries[index]);
            let tags = (self.configuration.tags)(&entries[index].path, &self.cancellation);
            if let Some(tags) = tags.filter(cache::valid_tags) {
                if entries[index].identity.is_none()
                    || FileIdentity::read(&entries[index].path) != entries[index].identity
                {
                    self.request(false);
                    append_notice(
                        &mut notice,
                        "Some songs changed while indexing; updating again",
                    );
                    continue;
                }
                entries[index].music_tags = Some(tags);
                let cost = entry_bytes(&entries[index]) - old_cost;
                if bytes + cost > self.configuration.bytes {
                    entries[index].music_tags = None;
                    append_notice(
                        &mut notice,
                        "Library metadata exceeds the 32 MiB index budget",
                    );
                } else {
                    bytes += cost;
                }
            } else {
                failures += 1;
            }
            if last_publish.elapsed() >= PUBLISH_INTERVAL {
                self.publish(Snapshot {
                    entries: Arc::new(entries.clone()),
                    indexing: true,
                    notice: notice.clone(),
                });
                last_publish = Instant::now();
            }
        }
        if failures > 0 {
            append_notice(
                &mut notice,
                &format!("{failures} songs have unavailable tags; filenames remain usable"),
            );
        }
        (entries, notice)
    }
}

fn pump(context: &gio::glib::MainContext, cancellation: &Cancellation) {
    for _ in 0..64 {
        if cancellation.is_cancelled() || !context.iteration(false) {
            break;
        }
    }
}

fn entry_bytes(entry: &Entry) -> usize {
    size_of::<Entry>()
        + entry.path.capacity()
        + entry.music_tags.as_ref().map_or(0, |tags| {
            [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
                .into_iter()
                .flatten()
                .map(String::capacity)
                .sum()
        })
}

fn append_notice(notice: &mut Option<String>, message: &str) {
    let notice = notice.get_or_insert_with(String::new);
    if !notice.contains(message) && notice.len() < 1500 {
        if !notice.is_empty() {
            notice.push_str(" · ");
        }
        notice.push_str(&message.chars().take(300).collect::<String>());
    }
}

#[cfg(test)]
pub(crate) mod tests;
