use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    mem::size_of,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use gtk::glib;

use crate::{
    model::{FileEntry, FileEntryInner, Location},
    services::{
        DirectoryChange, DirectoryEvent, DirectoryRequest, FileSource, LoadHandle,
        LocationValidationError, RequestId,
    },
};

#[derive(Clone, Copy)]
struct Limits {
    directories: usize,
    bytes: usize,
    ttl: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            directories: 12,
            bytes: 32 * 1024 * 1024,
            ttl: Duration::from_secs(30),
        }
    }
}

struct Listing {
    token: u64,
    location: Location,
    include_hidden: bool,
    valid: Rc<Cell<bool>>,
    finished: bool,
    saved: Instant,
    entries: Vec<FileEntry>,
    indices: HashMap<Location, usize>,
    bytes: usize,
    _monitor: LoadHandle,
}

struct Cache {
    listings: VecDeque<Listing>,
    limits: Limits,
    sequence: u64,
}

impl Cache {
    fn remove(&mut self, position: usize) {
        if let Some(listing) = self.listings.remove(position) {
            listing.valid.set(false);
        }
    }

    fn prune(&mut self, now: Instant) {
        for position in (0..self.listings.len()).rev() {
            let listing = &self.listings[position];
            if !listing.valid.get() || now.duration_since(listing.saved) >= self.limits.ttl {
                self.remove(position);
            }
        }
    }

    fn invalidate(&mut self, location: &Location) {
        for position in (0..self.listings.len()).rev() {
            if &self.listings[position].location == location {
                self.remove(position);
            }
        }
    }

    fn position(&self, token: u64) -> Option<usize> {
        self.listings
            .iter()
            .position(|listing| listing.token == token && listing.valid.get())
    }

    fn room(&mut self, token: u64, additional: usize) -> bool {
        while self
            .listings
            .iter()
            .map(|listing| listing.bytes)
            .sum::<usize>()
            .saturating_add(additional)
            > self.limits.bytes
        {
            let Some(position) = self
                .listings
                .iter()
                .position(|listing| listing.token != token)
            else {
                if let Some(position) = self.position(token) {
                    self.remove(position);
                }
                return false;
            };
            self.remove(position);
        }
        self.position(token).is_some()
    }

    fn append(&mut self, token: u64, entries: &[FileEntry]) {
        for entry in entries {
            let Some(position) = self.position(token) else {
                return;
            };
            if self.listings[position]
                .indices
                .contains_key(&entry.location)
            {
                self.remove(position);
                return;
            }
            let bytes = entry_bytes(entry);
            if !self.room(token, bytes) {
                return;
            }
            let Some(position) = self.position(token) else {
                return;
            };
            let listing = &mut self.listings[position];
            listing
                .indices
                .insert(entry.location.clone(), listing.entries.len());
            listing.entries.push(entry.clone());
            listing.bytes += bytes;
        }
    }

    fn metadata(&mut self, token: u64, entries: &[FileEntry]) {
        for entry in entries {
            let Some(position) = self.position(token) else {
                return;
            };
            let Some(&index) = self.listings[position].indices.get(&entry.location) else {
                continue;
            };
            let old_bytes = entry_bytes(&self.listings[position].entries[index]);
            let new_bytes = entry_bytes(entry);
            if !self.room(token, new_bytes.saturating_sub(old_bytes)) {
                return;
            }
            let Some(position) = self.position(token) else {
                return;
            };
            let listing = &mut self.listings[position];
            listing.entries[index] = entry.clone();
            listing.bytes = listing.bytes - old_bytes + new_bytes;
        }
    }

    fn observe(&mut self, token: u64, request_id: RequestId, event: &DirectoryEvent, now: Instant) {
        let Some(position) = self.position(token) else {
            return;
        };
        let id = match event {
            DirectoryEvent::Batch { request_id, .. }
            | DirectoryEvent::Snapshot { request_id, .. }
            | DirectoryEvent::MusicMetadata { request_id, .. }
            | DirectoryEvent::Restarted { request_id }
            | DirectoryEvent::Finished { request_id }
            | DirectoryEvent::Failed { request_id, .. } => *request_id,
        };
        if id != request_id {
            self.remove(position);
            return;
        }
        match event {
            DirectoryEvent::Restarted { .. } => self.remove(position),
            DirectoryEvent::Batch {
                request_id: id,
                entries,
            }
            | DirectoryEvent::Snapshot {
                request_id: id,
                entries,
            } if *id == request_id => {
                if self.listings[position].finished {
                    self.remove(position);
                    return;
                }
                if matches!(event, DirectoryEvent::Snapshot { .. }) {
                    let listing = &mut self.listings[position];
                    listing.entries = Vec::new();
                    listing.indices = HashMap::new();
                    listing.bytes = listing_bytes(&listing.location);
                }
                self.append(token, entries);
            }
            DirectoryEvent::MusicMetadata {
                request_id: id,
                entries,
            } if *id == request_id => {
                self.metadata(token, entries);
            }
            DirectoryEvent::Finished { request_id: id } if *id == request_id => {
                self.listings[position].finished = true;
                self.listings[position].saved = now;
            }
            DirectoryEvent::Failed { request_id: id, .. } if *id == request_id => {
                self.remove(position)
            }
            _ => {}
        }
    }
}

fn location_bytes(location: &Location) -> usize {
    location
        .native_path()
        .map(|path| path.as_os_str().len().saturating_mul(2))
        .or_else(|| location.uri_value().map(|uri| uri.len().saturating_mul(2)))
        .unwrap_or(0)
        .saturating_add(128)
}

fn listing_bytes(location: &Location) -> usize {
    size_of::<Listing>() + location_bytes(location) + 512
}

pub(super) fn entry_bytes(entry: &FileEntry) -> usize {
    let tags = entry.music_tags.as_ref().map_or(0, |tags| {
        [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
            .into_iter()
            .flatten()
            .map(String::capacity)
            .sum::<usize>()
    });
    size_of::<FileEntryInner>()
        + 32
        + 2 * size_of::<FileEntry>()
        + 256
        + 2 * location_bytes(&entry.location)
        + entry.native_name.capacity()
        + entry.display_name.capacity()
        + tags
}

pub(super) struct CachedFileSource {
    provider: Rc<dyn FileSource>,
    cache: Rc<RefCell<Cache>>,
    now: Rc<dyn Fn() -> Instant>,
    _sweeper: Option<LoadHandle>,
}

impl CachedFileSource {
    pub(super) fn new(provider: Rc<dyn FileSource>) -> Self {
        let mut source = Self::with_limits(provider, Limits::default(), Rc::new(Instant::now));
        let cache = Rc::downgrade(&source.cache);
        let now = source.now.clone();
        let task = glib::MainContext::ref_thread_default().spawn_local(async move {
            loop {
                glib::timeout_future(Duration::from_secs(1)).await;
                let Some(cache) = cache.upgrade() else {
                    break;
                };
                cache.borrow_mut().prune(now());
            }
        });
        source._sweeper = Some(LoadHandle::new(move || task.abort()));
        source
    }

    fn with_limits(
        provider: Rc<dyn FileSource>,
        limits: Limits,
        now: Rc<dyn Fn() -> Instant>,
    ) -> Self {
        Self {
            provider,
            cache: Rc::new(RefCell::new(Cache {
                listings: VecDeque::new(),
                limits,
                sequence: 0,
            })),
            now,
            _sweeper: None,
        }
    }

    fn replay(
        &self,
        request: DirectoryRequest,
        token: u64,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        let cache = Rc::downgrade(&self.cache);
        let cancellation = crate::sandbox::Cancellation::default();
        let worker_cancellation = cancellation.clone();
        let provider = self.provider.clone();
        let fallback = Rc::new(RefCell::new(None));
        let retained = fallback.clone();
        let task = glib::MainContext::ref_thread_default().spawn_local_with_priority(
            glib::Priority::DEFAULT,
            async move {
                let mut offset = 0;
                loop {
                    let count = if offset == 0 {
                        request.batch_size.clamp(1, 32)
                    } else {
                        request.batch_size.clamp(1, 128)
                    };
                    let Some(entries) = replay_chunk(&cache, token, offset, count) else {
                        emit(DirectoryEvent::Restarted {
                            request_id: request.id,
                        });
                        if worker_cancellation.is_cancelled() {
                            return;
                        }
                        let active = worker_cancellation.clone();
                        let load = provider.enumerate(
                            request,
                            Rc::new(move |event| {
                                if !active.is_cancelled() {
                                    emit(event);
                                }
                            }),
                        );
                        if !worker_cancellation.is_cancelled() {
                            retained.replace(Some(load));
                        }
                        return;
                    };
                    if entries.is_empty() {
                        break;
                    }
                    offset += entries.len();
                    emit(DirectoryEvent::Batch {
                        request_id: request.id,
                        entries,
                    });
                    if worker_cancellation.is_cancelled() {
                        return;
                    }
                    glib::timeout_future_with_priority(glib::Priority::DEFAULT, Duration::ZERO)
                        .await;
                }
                emit(DirectoryEvent::Finished {
                    request_id: request.id,
                });
                if worker_cancellation.is_cancelled() {
                    return;
                }
                let mut offset = 0;
                while let Some(entries) = replay_chunk(&cache, token, offset, 32) {
                    if entries.is_empty() {
                        break;
                    }
                    offset += entries.len();
                    let mut known = Vec::new();
                    let mut missing = Vec::new();
                    for entry in entries {
                        if entry.music_tags.is_some() {
                            known.push(entry);
                        } else if crate::app::music_order::is_audio(&entry) {
                            missing.push(entry);
                        }
                    }
                    if !missing.is_empty() {
                        let entries =
                            super::music_metadata::enrich(missing, worker_cancellation.clone())
                                .await;
                        if worker_cancellation.is_cancelled()
                            || replay_chunk(&cache, token, 0, 0).is_none()
                        {
                            return;
                        }
                        if let Some(cache) = cache.upgrade() {
                            cache.borrow_mut().metadata(token, &entries);
                        }
                        known.extend(entries);
                    }
                    if !known.is_empty() {
                        emit(DirectoryEvent::MusicMetadata {
                            request_id: request.id,
                            entries: known,
                        });
                        if worker_cancellation.is_cancelled() {
                            return;
                        }
                    }
                    glib::timeout_future_with_priority(glib::Priority::DEFAULT, Duration::ZERO)
                        .await;
                }
            },
        );
        LoadHandle::new(move || {
            cancellation.cancel();
            task.abort();
            fallback.take();
        })
    }
}

fn replay_chunk(
    cache: &Weak<RefCell<Cache>>,
    token: u64,
    offset: usize,
    count: usize,
) -> Option<Vec<FileEntry>> {
    let cache = cache.upgrade()?;
    let cache = cache.borrow();
    let listing = &cache.listings[cache.position(token)?];
    Some(
        listing
            .entries
            .iter()
            .skip(offset)
            .take(count)
            .cloned()
            .collect(),
    )
}

impl FileSource for CachedFileSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        self.provider.validate_location(location)
    }

    fn invalidate(&self, location: &Location) {
        self.cache.borrow_mut().invalidate(location);
        self.provider.invalidate(location);
    }

    fn watch_invalidation(&self, location: &Location, notify: Rc<dyn Fn()>) -> Option<LoadHandle> {
        self.provider.watch_invalidation(location, notify)
    }

    fn watch(
        &self,
        location: Location,
        include_hidden: bool,
        notify: Rc<dyn Fn(DirectoryChange)>,
    ) -> Option<LoadHandle> {
        self.provider.watch(location, include_hidden, notify)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if !request.location.is_absolute_native() {
            return self.provider.enumerate(request, emit);
        }
        {
            let mut cache = self.cache.borrow_mut();
            cache.prune((self.now)());
            if let Some(position) = cache.listings.iter().position(|listing| {
                listing.finished
                    && listing.location == request.location
                    && listing.include_hidden == request.include_hidden
            }) {
                let listing = cache
                    .listings
                    .remove(position)
                    .expect("located cache entry");
                let token = listing.token;
                cache.listings.push_back(listing);
                drop(cache);
                tracing::debug!(request_id = request.id.0, "warm directory enumeration hit");
                return self.replay(request, token, emit);
            }
        }

        let valid = Rc::new(Cell::new(true));
        let changed = valid.clone();
        let Some(monitor) = self
            .provider
            .watch_invalidation(&request.location, Rc::new(move || changed.set(false)))
        else {
            return self.provider.enumerate(request, emit);
        };
        let token = {
            let mut cache = self.cache.borrow_mut();
            for position in (0..cache.listings.len()).rev() {
                let listing = &cache.listings[position];
                if listing.location == request.location
                    && listing.include_hidden == request.include_hidden
                {
                    cache.remove(position);
                }
            }
            while !cache.listings.is_empty() && cache.listings.len() >= cache.limits.directories {
                cache.remove(0);
            }
            cache.sequence = cache.sequence.wrapping_add(1);
            let token = cache.sequence;
            let bytes = listing_bytes(&request.location);
            cache.listings.push_back(Listing {
                token,
                location: request.location.clone(),
                include_hidden: request.include_hidden,
                valid,
                finished: false,
                saved: (self.now)(),
                entries: Vec::new(),
                indices: HashMap::new(),
                bytes,
                _monitor: monitor,
            });
            cache.room(token, 0);
            token
        };
        let active = Rc::new(Cell::new(true));
        let subscribed = active.clone();
        let weak = Rc::downgrade(&self.cache);
        let observe = weak.clone();
        let now = self.now.clone();
        let request_id = request.id;
        let load = self.provider.enumerate(
            request,
            Rc::new(move |event| {
                if !subscribed.get() {
                    return;
                }
                if let Some(cache) = observe.upgrade() {
                    cache.borrow_mut().observe(token, request_id, &event, now());
                }
                emit(event);
            }),
        );
        LoadHandle::new(move || {
            active.set(false);
            if let Some(cache) = weak.upgrade() {
                let mut cache = cache.borrow_mut();
                if let Some(position) = cache.position(token) {
                    if !cache.listings[position].finished {
                        cache.remove(position);
                    }
                }
            }
            drop(load);
        })
    }
}

#[cfg(test)]
mod tests;
