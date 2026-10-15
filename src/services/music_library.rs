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
mod catalog_disk;
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
    catalog_path: Option<PathBuf>,
    music_source: Option<PathBuf>,
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
            catalog_path: None,
            music_source: None,
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
    music_snapshot: Mutex<Option<Arc<Snapshot>>>,
    saved_catalog: Mutex<Option<Arc<Snapshot>>>,
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

pub(crate) fn subscribe_catalog(root: PathBuf) -> Subscription {
    let mut configuration = Configuration::new(root.clone());
    configuration.catalog_path = Some(catalog_disk::path(&root));
    let music = super::search::SearchScope::MusicLibrary.root(&gio::glib::home_dir());
    let visible_music = music.strip_prefix(&root).is_ok_and(|relative| {
        relative.components().all(|component| {
            matches!(component,
            std::path::Component::Normal(name) if !name.to_string_lossy().starts_with('.'))
        })
    });
    if root == gio::glib::home_dir() && music != root && visible_music {
        configuration.music_source = Some(music);
    }
    subscribe_with(configuration)
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
            && library.configuration.catalog_path.is_some() == configuration.catalog_path.is_some()
        {
            let mut shared = library
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if library.cancellation.is_cancelled() {
                continue;
            }
            let exact = library.configuration.root == configuration.root;
            let covered_folder = configuration.catalog_path.is_some()
                && !shared.snapshot.indexing
                && shared.snapshot.notice.is_none()
                && configuration.root.starts_with(&library.configuration.root)
                && shared
                    .snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.is_directory && entry.path == configuration.root);
            if !exact && !covered_folder {
                continue;
            }
            shared.subscribers += 1;
            if !shared.snapshot.indexing && configuration.catalog_path.is_none() {
                shared.requested = Some(Instant::now());
                shared.immediate = true;
                let mut snapshot = (*shared.snapshot).clone();
                snapshot.indexing = true;
                shared.snapshot = Arc::new(snapshot);
                shared.revision = shared.revision.wrapping_add(1);
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
        music_snapshot: Mutex::new(None),
        saved_catalog: Mutex::new(None),
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
        if !shared.snapshot.indexing {
            let mut snapshot = (*shared.snapshot).clone();
            snapshot.indexing = true;
            shared.snapshot = Arc::new(snapshot);
            shared.revision = shared.revision.wrapping_add(1);
        }
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
        let snapshot = self.compose(snapshot);
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
        let mut music = self
            .configuration
            .music_source
            .as_ref()
            .map(|root| subscribe(root.clone()));
        let mut previous = Arc::new(Vec::new());
        let mut cached = if self.configuration.catalog_path.is_none() {
            cache::read(&self.configuration.cache_path, &self.configuration.root)
        } else {
            HashMap::new()
        };
        if let Some(path) = &self.configuration.catalog_path {
            match catalog_disk::read(
                path,
                &self.configuration.root,
                self.configuration.entries,
                self.configuration.bytes,
                &self.cancellation,
            ) {
                Ok((entries, notice)) => {
                    previous = Arc::new(
                        entries
                            .into_iter()
                            .filter(|entry| {
                                !self
                                    .configuration
                                    .music_source
                                    .as_ref()
                                    .is_some_and(|music| entry.path.starts_with(music))
                            })
                            .collect(),
                    );
                    if !previous.is_empty() {
                        let mut notice = notice;
                        append_notice(&mut notice, "Verifying saved search index");
                        self.publish(Snapshot {
                            entries: previous.clone(),
                            indexing: true,
                            notice,
                        });
                    }
                }
                Err(error) => tracing::debug!(%error, "Saved search index unavailable; scanning"),
            }
        }
        let mut base_notice = None;
        let mut completed = Instant::now();
        while !self.cancellation.is_cancelled() {
            pump(context, &self.cancellation);
            if let Some(snapshot) = music.as_mut().and_then(Subscription::poll) {
                self.music_snapshot
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .replace(snapshot);
                let snapshot = Snapshot {
                    entries: previous.clone(),
                    indexing: self.requested(),
                    notice: base_notice.clone(),
                };
                self.publish(snapshot.clone());
                if !self.requested() {
                    self.save_catalog(snapshot);
                }
            }
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
                    && self.configuration.catalog_path.is_none()
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
                base_notice = notice.clone();
                let snapshot = Snapshot {
                    entries: previous.clone(),
                    indexing: pending,
                    notice,
                };
                self.publish(snapshot.clone());
                if !pending {
                    self.save_catalog(snapshot);
                }
                completed = Instant::now();
            }
            let shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let _wait = self.ready.wait_timeout(shared, TICK);
        }
    }

    fn compose(&self, mut snapshot: Snapshot) -> Snapshot {
        let music = self
            .music_snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(music) = music else {
            if self.configuration.music_source.is_some() {
                snapshot.indexing = true;
            }
            return snapshot;
        };
        let mut entries = Vec::new();
        let mut bytes = 0;
        if let Some(root) = &self.configuration.music_source
            && let Ok(metadata) = fs::symlink_metadata(root)
            && metadata.is_dir()
        {
            let identity = FileIdentity::from_metadata(&metadata);
            let entry = Entry {
                path: root.clone(),
                is_directory: true,
                modified_unix_seconds: Some(identity.modified.0),
                identity: Some(identity),
                music_tags: None,
            };
            if self.configuration.entries > 0 && entry_bytes(&entry) <= self.configuration.bytes {
                entries.reserve_exact(1);
                entries.push(entry);
                bytes = entry_bytes(&entries[0]);
            }
        }
        for entry in music.entries.iter().chain(snapshot.entries.iter()) {
            let cost = entry_bytes(entry) - size_of::<Entry>();
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
                    &mut snapshot.notice,
                    "Search catalog limit reached; some files were not indexed",
                );
                break;
            }
            if allocation > 0 {
                entries.reserve_exact(capacity - entries.len());
            }
            bytes += cost + allocation;
            entries.push(entry.clone());
        }
        if let Some(notice) = &music.notice {
            append_notice(&mut snapshot.notice, notice);
        }
        snapshot.entries = Arc::new(entries);
        snapshot.indexing |= music.indexing;
        snapshot
    }

    fn save_catalog(&self, snapshot: Snapshot) {
        let Some(path) = &self.configuration.catalog_path else {
            return;
        };
        let snapshot = self.compose(snapshot);
        if snapshot.indexing || self.cancellation.is_cancelled() {
            return;
        }
        if self
            .saved_catalog
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_deref()
            == Some(&snapshot)
        {
            return;
        }
        if let Err(error) = catalog_disk::write(
            path,
            &self.configuration.root,
            &snapshot.entries,
            snapshot.notice.as_deref(),
            &self.cancellation,
        ) {
            let mut snapshot = snapshot;
            append_notice(
                &mut snapshot.notice,
                &format!("Search index could not be saved: {error}"),
            );
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            shared.snapshot = Arc::new(snapshot);
            shared.revision = shared.revision.wrapping_add(1);
            self.ready.notify_all();
        } else {
            self.saved_catalog
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .replace(Arc::new(snapshot));
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
                Some(format!("Folder unavailable: {}: {error}", root.display())),
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
        let excluded_music = self.configuration.music_source.clone();
        let mut track_slots = 0;
        let walker = ignore::WalkBuilder::new(root)
            .filter_entry(move |entry| {
                entry.depth() == 0
                    || !excluded_music
                        .as_ref()
                        .is_some_and(|music| entry.path().starts_with(music))
            })
            .follow_links(false)
            .standard_filters(false)
            .hidden(true)
            .max_depth(Some(128))
            .build();
        let mut last_names_publish = Instant::now();
        let names_interval = if self.configuration.catalog_path.is_some() {
            Duration::from_millis(50)
        } else {
            PUBLISH_INTERVAL
        };
        for (count, entry) in walker.enumerate() {
            pump(context, &self.cancellation);
            if self.cancellation.is_cancelled() {
                break;
            }
            if count.is_multiple_of(256) {
                std::thread::sleep(Duration::from_millis(1));
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
            if record.is_audio() && track_slots < self.configuration.tracks {
                track_slots += 1;
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
            } else if record.is_audio() {
                append_notice(
                    &mut notice,
                    "Song metadata limit reached; other tracks remain searchable by filename",
                );
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
            if last_names_publish.elapsed() >= names_interval {
                self.publish(Snapshot {
                    entries: Arc::new(entries.clone()),
                    indexing: true,
                    notice: notice.clone(),
                });
                last_names_publish = Instant::now();
            }
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
        let mut metadata_reads = 0;
        for index in 0..entries.len() {
            pump(context, &self.cancellation);
            if self.cancellation.is_cancelled() || self.requested() {
                break;
            }
            if !entries[index].is_audio() {
                continue;
            }
            if metadata_reads >= self.configuration.tracks {
                append_notice(
                    &mut notice,
                    "Song metadata limit reached; other tracks remain searchable by filename",
                );
                break;
            }
            metadata_reads += 1;
            if entries[index].music_tags.is_some() {
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

#[cfg(test)]
mod catalog_tests;
