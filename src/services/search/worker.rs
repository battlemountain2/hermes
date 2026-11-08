use std::{
    collections::{HashMap, VecDeque},
    mem::size_of,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use super::{
    ContentSearchContext, MAX_DOCUMENT_EXTRACTIONS_PER_QUERY, ParsedSearchQuery, RESULT_LIMIT,
    SearchEntityKey, SearchEvent, SearchGroup, SearchGroups, SearchItem, SearchPage,
    TextExtractionProvider, insert_match, score_item,
};
use crate::services::music_library::{FileIdentity, Subscription};
use crate::{
    model::MusicTags,
    sandbox::Cancellation,
    services::{FormatFamily, classify_by_name},
};

const MAX_INDEX_ENTRIES: usize = 100_000;
const MAX_INDEX_BYTES: usize = 512 * 1024 * 1024;
const MAX_METADATA_READS: usize = 100_000;
const CACHE_AGE: Duration = Duration::from_secs(600);
const TICK: Duration = Duration::from_millis(20);
const PUBLISH_INTERVAL: Duration = Duration::from_millis(50);
const MAX_QUERY_BYTES: usize = 4096;

#[derive(Clone, Copy)]
struct Limits {
    entries: usize,
    bytes: usize,
    metadata: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: MAX_INDEX_ENTRIES,
            bytes: MAX_INDEX_BYTES,
            metadata: MAX_METADATA_READS,
        }
    }
}

#[derive(Default)]
struct Mailbox<T> {
    value: Mutex<Option<T>>,
    ready: Condvar,
}

impl<T> Mailbox<T> {
    fn put(&self, value: T) {
        *self.value.lock().unwrap_or_else(|error| error.into_inner()) = Some(value);
        self.ready.notify_one();
    }

    fn take(&self) -> Option<T> {
        self.value
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
}

struct Query {
    generation: u64,
    text: String,
    cancelled: Arc<AtomicBool>,
    limit: usize,
    grouping: Option<Grouping>,
}

struct Grouping {
    group: SearchGroup,
    entity: Option<SearchEntityKey>,
    virtual_results: bool,
}

enum Command {
    Query(Query),
    More(SearchPage),
}

pub(crate) struct SearchHandle {
    next_generation: AtomicU64,
    scan_cancelled: Cancellation,
    query_cancelled: Mutex<Arc<AtomicBool>>,
    commands: Arc<Mailbox<Command>>,
    events: Arc<Mailbox<SearchEvent>>,
    startup_error: Option<String>,
    live: bool,
}

impl SearchHandle {
    pub(crate) fn is_live(&self) -> bool {
        self.live
    }
    #[cfg(test)]
    pub(crate) fn query(&self, query: &str) -> u64 {
        self.query_with_limit(query, RESULT_LIMIT, None)
    }

    pub(crate) fn query_all(&self, query: &str) -> u64 {
        self.query_with_limit(query, MAX_INDEX_ENTRIES, None)
    }

    pub(crate) fn query_group(
        &self,
        query: &str,
        group: SearchGroup,
        entity: Option<SearchEntityKey>,
    ) -> u64 {
        self.query_with_limit(
            query,
            RESULT_LIMIT,
            Some(Grouping {
                group,
                entity,
                virtual_results: true,
            }),
        )
    }

    pub(crate) fn query_all_scoped(
        &self,
        query: &str,
        group: SearchGroup,
        entity: Option<SearchEntityKey>,
    ) -> u64 {
        if group == SearchGroup::All && entity.is_none() {
            return self.query_all(query);
        }
        self.query_with_limit(
            query,
            MAX_INDEX_ENTRIES,
            Some(Grouping {
                group,
                entity,
                virtual_results: false,
            }),
        )
    }

    pub(crate) fn more(&self, page: SearchPage) {
        let current = self
            .query_cancelled
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.live
            || current.load(Ordering::Acquire)
            || page.generation
                != self
                    .next_generation
                    .load(Ordering::Relaxed)
                    .saturating_sub(1)
        {
            return;
        }
        let mut command = self
            .commands
            .value
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !matches!(*command, Some(Command::Query(_))) {
            *command = Some(Command::More(page));
            self.commands.ready.notify_one();
        }
    }

    fn query_with_limit(&self, query: &str, limit: usize, grouping: Option<Grouping>) -> u64 {
        let token = Arc::new(AtomicBool::new(false));
        let mut previous = self
            .query_cancelled
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        previous.store(true, Ordering::Release);
        *previous = token.clone();
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let mut text = query.trim().to_owned();
        if text.len() > MAX_QUERY_BYTES {
            let mut end = MAX_QUERY_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        if let Some(error) = &self.startup_error {
            self.events.put(SearchEvent::Results {
                generation,
                query: text,
                items: Vec::new(),
                indexing: false,
                notice: Some(error.clone()),
                page: None,
                groups: None,
            });
            return generation;
        }
        self.commands.put(Command::Query(Query {
            generation,
            text,
            cancelled: token,
            limit,
            grouping,
        }));
        generation
    }
}

impl Drop for SearchHandle {
    fn drop(&mut self) {
        self.scan_cancelled.cancel();
        self.query_cancelled
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .store(true, Ordering::Release);
    }
}

pub(crate) struct SearchReceiver(Arc<Mailbox<SearchEvent>>);

impl SearchReceiver {
    pub(crate) fn try_recv(&self) -> Option<SearchEvent> {
        self.0.take()
    }

    #[cfg(test)]
    pub(crate) fn recv_timeout(&self, duration: Duration) -> Result<SearchEvent, ()> {
        let guard = self
            .0
            .value
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (mut guard, _) = self
            .0
            .ready
            .wait_timeout_while(guard, duration, |value| value.is_none())
            .unwrap_or_else(|error| error.into_inner());
        guard.take().ok_or(())
    }
}

struct CachedIndex {
    root: PathBuf,
    items: Arc<Vec<SearchItem>>,
    built: Instant,
    bytes: usize,
}

fn cache() -> Arc<Mutex<VecDeque<CachedIndex>>> {
    static CACHE: OnceLock<Arc<Mutex<VecDeque<CachedIndex>>>> = OnceLock::new();
    CACHE
        .get_or_init(|| Arc::new(Mutex::new(VecDeque::new())))
        .clone()
}

type TagReader = dyn Fn(&Path, &Cancellation) -> Option<MusicTags> + Send + Sync;

pub(crate) fn index_tree(
    root: PathBuf,
    extraction: Arc<dyn TextExtractionProvider>,
) -> (SearchHandle, SearchReceiver) {
    let music = super::SearchScope::MusicLibrary.root(&gio::glib::home_dir());
    if shared_scope(&root, &music) {
        let subscription = crate::services::music_library::subscribe(music);
        return start_shared(root, extraction, subscription);
    }
    if root.is_absolute() {
        let subscription = crate::services::music_library::subscribe_catalog(root.clone());
        return start_shared(root, extraction, subscription);
    }
    start(
        root,
        extraction,
        Arc::new(crate::adapters::music_metadata::read_tags),
    )
}

fn shared_scope(root: &Path, music: &Path) -> bool {
    root.strip_prefix(music).is_ok_and(|relative| {
        !relative.components().any(|part| match part {
            std::path::Component::ParentDir => true,
            std::path::Component::Normal(name) => name.to_string_lossy().starts_with('.'),
            _ => false,
        })
    })
}

fn start(
    root: PathBuf,
    extraction: Arc<dyn TextExtractionProvider>,
    tags: Arc<TagReader>,
) -> (SearchHandle, SearchReceiver) {
    start_with_limits(root, extraction, tags, Limits::default())
}

fn start_with_limits(
    root: PathBuf,
    extraction: Arc<dyn TextExtractionProvider>,
    tags: Arc<TagReader>,
    limits: Limits,
) -> (SearchHandle, SearchReceiver) {
    start_with_cache(root, extraction, tags, limits, cache())
}

fn start_with_cache(
    root: PathBuf,
    extraction: Arc<dyn TextExtractionProvider>,
    tags: Arc<TagReader>,
    limits: Limits,
    index_cache: Arc<Mutex<VecDeque<CachedIndex>>>,
) -> (SearchHandle, SearchReceiver) {
    start_worker(root, extraction, tags, limits, index_cache, None)
}

fn start_shared(
    root: PathBuf,
    extraction: Arc<dyn TextExtractionProvider>,
    subscription: Subscription,
) -> (SearchHandle, SearchReceiver) {
    start_worker(
        root,
        extraction,
        Arc::new(crate::adapters::music_metadata::read_tags),
        Limits::default(),
        cache(),
        Some(subscription),
    )
}

fn start_worker(
    root: PathBuf,
    extraction: Arc<dyn TextExtractionProvider>,
    tags: Arc<TagReader>,
    limits: Limits,
    index_cache: Arc<Mutex<VecDeque<CachedIndex>>>,
    library: Option<Subscription>,
) -> (SearchHandle, SearchReceiver) {
    let commands = Arc::new(Mailbox {
        value: Mutex::new(None),
        ready: Condvar::new(),
    });
    let events = Arc::new(Mailbox {
        value: Mutex::new(None),
        ready: Condvar::new(),
    });
    let mut handle = SearchHandle {
        next_generation: AtomicU64::new(1),
        scan_cancelled: Cancellation::default(),
        query_cancelled: Mutex::new(Arc::new(AtomicBool::new(false))),
        commands: commands.clone(),
        events: events.clone(),
        startup_error: None,
        live: library.is_some(),
    };
    let mut worker = Worker {
        commands,
        events: events.clone(),
        cancellation: handle.scan_cancelled.clone(),
        extraction,
        query: Query {
            generation: 0,
            text: String::new(),
            cancelled: Arc::new(AtomicBool::new(false)),
            limit: RESULT_LIMIT,
            grouping: None,
        },
        matches: Vec::new(),
        source: Arc::new(super::source::Source::empty()),
        candidates: None,
        candidate_attempted: false,
        shared_indexing: true,
        ranked: None,
        rank_notice: None,
        groups: None,
        revision: 0,
        shown_limit: RESULT_LIMIT,
        scan_notice: None,
        query_notice: None,
        document_budget: MAX_DOCUMENT_EXTRACTIONS_PER_QUERY,
        content_scores: HashMap::new(),
        last_publish: Instant::now(),
        limits,
        index_cache,
    };
    let failure_events = events.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("hermes-search".into())
        .spawn(move || {
            if let Some(library) = library {
                worker.run_shared(root, library);
            } else {
                worker.run(root, tags);
            }
        })
    {
        handle.startup_error = Some(format!("Unable to start search: {error}"));
        failure_events.put(SearchEvent::Results {
            generation: 0,
            query: String::new(),
            items: Vec::new(),
            indexing: false,
            notice: Some(format!("Unable to start search: {error}")),
            page: None,
            groups: None,
        });
    }
    (handle, SearchReceiver(events))
}

struct Worker {
    commands: Arc<Mailbox<Command>>,
    events: Arc<Mailbox<SearchEvent>>,
    cancellation: Cancellation,
    extraction: Arc<dyn TextExtractionProvider>,
    query: Query,
    matches: Vec<(i64, SearchItem)>,
    source: Arc<super::source::Source>,
    candidates: Option<super::candidates::Candidates>,
    candidate_attempted: bool,
    shared_indexing: bool,
    ranked: Option<super::ranked::Ranked>,
    rank_notice: Option<String>,
    groups: Option<super::groups::GroupIndex>,
    revision: u64,
    shown_limit: usize,
    scan_notice: Option<String>,
    query_notice: Option<String>,
    document_budget: usize,
    content_scores: HashMap<PathBuf, CachedContentScore>,
    last_publish: Instant,
    limits: Limits,
    index_cache: Arc<Mutex<VecDeque<CachedIndex>>>,
}

struct CachedContentScore {
    item: SearchItem,
    score: Option<i64>,
    revision: u64,
}

impl Worker {
    fn run_shared(&mut self, root: PathBuf, mut library: Subscription) {
        let mut indexing = true;
        while !self.cancellation.is_cancelled() {
            if let Some(snapshot) = library.poll() {
                indexing = snapshot.indexing;
                self.scan_notice = snapshot.notice.clone();
                let source = match super::source::Source::new(
                    root.clone(),
                    snapshot.entries.clone(),
                    self.limits.entries,
                    self.limits.bytes,
                    &|| self.cancellation.is_cancelled(),
                ) {
                    Ok(source) => source,
                    Err(error) => {
                        if self.cancellation.is_cancelled() {
                            return;
                        }
                        self.warn(error);
                        super::source::Source::empty()
                    }
                };
                if let Some(notice) = source.notice() {
                    self.warn(notice.to_owned());
                }
                self.source = Arc::new(source);
                self.candidates = None;
                self.candidate_attempted = false;
                self.shared_indexing = indexing;
                self.rescore_shared();
                self.publish(indexing);
                self.prepare_candidates();
            }
            self.pending_shared(indexing);
            let guard = self
                .commands
                .value
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let _wait = self
                .commands
                .ready
                .wait_timeout_while(guard, TICK, |pending| pending.is_none());
        }
    }

    fn score(&mut self, item: &SearchItem, parsed: &ParsedSearchQuery) -> Option<i64> {
        if self.query.cancelled.load(Ordering::Acquire) {
            return None;
        }
        if parsed.content.is_some()
            && let Some(cached) = self.content_scores.get(&item.path)
            && cached.item == *item
            && (item.source_identity.is_some()
                || (self.revision > 0 && cached.revision == self.revision))
        {
            return cached.score;
        }
        let before = self.document_budget;
        let score = score_item(
            item,
            parsed,
            &mut ContentSearchContext {
                text_extraction: self.extraction.as_ref(),
                cancelled: &self.query.cancelled,
                document_extractions_remaining: &mut self.document_budget,
                notice: &mut self.query_notice,
            },
        );
        let extracted = self.document_budget < before;
        if parsed.content.is_some()
            && ((extracted && (item.source_identity.is_some() || self.revision > 0))
                || (item.source_identity.is_some()
                    && score.is_some()
                    && self.content_scores.len() < RESULT_LIMIT))
        {
            // Reserve capacity for every budget-consuming extraction. Plain
            // text hits cannot displace document outcomes needed to replay a
            // failed paged query without extracting again. Unknown identities
            // are reusable only within this exact snapshot revision.
            self.content_scores.insert(
                item.path.clone(),
                CachedContentScore {
                    item: item.clone(),
                    score,
                    revision: self.revision,
                },
            );
        }
        score
    }

    fn pending(&mut self, items: &[SearchItem], indexing: bool) {
        if let Some(Command::Query(query)) = self.commands.take() {
            self.query = query;
            self.document_budget = MAX_DOCUMENT_EXTRACTIONS_PER_QUERY;
            self.content_scores.clear();
            self.query_notice = None;
            self.rescore(items);
            self.publish(indexing);
        }
    }

    fn pending_shared(&mut self, indexing: bool) {
        match self.commands.take() {
            Some(Command::Query(query)) => {
                self.query = query;
                self.document_budget = MAX_DOCUMENT_EXTRACTIONS_PER_QUERY;
                self.content_scores.clear();
                self.query_notice = None;
                self.rescore_shared();
                self.publish(indexing);
                self.prepare_candidates();
            }
            Some(Command::More(page)) => {
                let total = self.result_total();
                if page.generation == self.query.generation
                    && page.revision == self.revision
                    && page.shown == self.shown_limit.min(total)
                    && page.total == total
                {
                    self.shown_limit = self.shown_limit.saturating_add(RESULT_LIMIT).min(total);
                }
                self.publish(indexing);
            }
            None => {}
        }
    }

    fn rescore_shared(&mut self) {
        self.revision = self.revision.wrapping_add(1).max(1);
        self.shown_limit = self.query.limit;
        self.ranked = None;
        self.rank_notice = None;
        self.groups = None;
        let source = self.source.clone();
        let parsed = ParsedSearchQuery::parse(&self.query.text.to_lowercase());
        match self.build_ranking(&source, &parsed) {
            Ok(ranked) => {
                if let Err(error) = self.rebuild_groups(&ranked, &parsed) {
                    self.rank_notice = Some(error);
                } else {
                    self.ranked = Some(ranked);
                }
            }
            Err(error) => self.rank_notice = Some(error),
        }
    }

    fn build_ranking(
        &mut self,
        source: &super::source::Source,
        parsed: &ParsedSearchQuery,
    ) -> Result<super::ranked::Ranked, String> {
        let cancellation = self.cancellation.clone();
        let query_cancelled = self.query.cancelled.clone();
        let cancelled = || cancellation.is_cancelled() || query_cancelled.load(Ordering::Acquire);
        let album_order =
            self.query.grouping.as_ref().is_some_and(|grouping| {
                matches!(grouping.entity, Some(SearchEntityKey::Album { .. }))
            });
        let mut ranked = super::ranked::Ranked::new(album_order, &cancelled)?;
        if parsed.is_empty()
            && self
                .query
                .grouping
                .as_ref()
                .is_none_or(|grouping| grouping.entity.is_none())
        {
            ranked.finish(&cancelled)?;
            return Ok(ranked);
        }
        let mut used_candidates = false;
        if let Some(candidates) = self.candidates.take() {
            let mut rank_error = None;
            let result = candidates.for_each_page(parsed, &cancelled, |page| {
                self.rank_positions(source, page.iter().copied(), parsed, &mut ranked)
                    .inspect_err(|error| rank_error = Some(error.clone()))
            });
            self.candidates = Some(candidates);
            if let Some(error) = rank_error {
                return Err(error);
            }
            used_candidates = result.is_ok();
            if !used_candidates {
                // Discard the entire unpublished revision before retrying the
                // scorer. Reserved content outcomes survive this replay.
                drop(ranked);
                ranked = super::ranked::Ranked::new(album_order, &cancelled)?;
            }
        }
        if !used_candidates {
            source.for_each_page(&cancelled, |page| {
                for (position, item) in page {
                    self.rank_item(*position, item, parsed, &mut ranked)?;
                }
                Ok(())
            })?;
        }
        ranked.finish(&cancelled)?;
        Ok(ranked)
    }

    fn rank_positions(
        &mut self,
        source: &super::source::Source,
        positions: impl Iterator<Item = usize>,
        parsed: &ParsedSearchQuery,
        ranked: &mut super::ranked::Ranked,
    ) -> Result<(), String> {
        for position in positions {
            let item = source
                .get(position)?
                .ok_or_else(|| "Search ranking contains an invalid source position".to_owned())?;
            self.rank_item(position, &item, parsed, ranked)?;
        }
        Ok(())
    }

    fn rank_item(
        &mut self,
        position: usize,
        item: &SearchItem,
        parsed: &ParsedSearchQuery,
        ranked: &mut super::ranked::Ranked,
    ) -> Result<(), String> {
        if self.cancellation.is_cancelled() || self.query.cancelled.load(Ordering::Acquire) {
            return Err("Search query cancelled".into());
        }
        if let Some(grouping) = &self.query.grouping {
            if grouping
                .entity
                .as_ref()
                .is_some_and(|entity| !super::groups::matches_entity(item, entity))
            {
                return Ok(());
            }
            if !grouping.virtual_results {
                let audio = !item.is_directory
                    && classify_by_name(item.path.as_os_str()) == FormatFamily::Audio;
                if (grouping.group == SearchGroup::Files && audio)
                    || (grouping.group == SearchGroup::Songs && !audio)
                {
                    return Ok(());
                }
            }
        }
        if let Some(score) = if parsed.is_empty() {
            Some(0)
        } else {
            self.score(item, parsed)
        } {
            ranked.push(score, position, item, &|| {
                self.cancellation.is_cancelled() || self.query.cancelled.load(Ordering::Acquire)
            })?;
        }
        Ok(())
    }

    fn rebuild_groups(
        &mut self,
        ranked: &super::ranked::Ranked,
        parsed: &ParsedSearchQuery,
    ) -> Result<(), String> {
        if self
            .query
            .grouping
            .as_ref()
            .is_some_and(|grouping| grouping.virtual_results)
        {
            self.groups =
                super::groups::GroupIndex::build_paged(&self.source, ranked, parsed, &|| {
                    self.cancellation.is_cancelled() || self.query.cancelled.load(Ordering::Acquire)
                })
                .map(Some)?;
        }
        Ok(())
    }

    fn result_total(&self) -> usize {
        self.query
            .grouping
            .as_ref()
            .filter(|grouping| grouping.virtual_results)
            .zip(self.groups.as_ref())
            .map_or(
                self.ranked.as_ref().map_or(0, |ranked| ranked.len()),
                |(grouping, groups)| groups.counts().total(grouping.group),
            )
    }

    fn prepare_candidates(&mut self) {
        let parsed = ParsedSearchQuery::parse(&self.query.text.to_lowercase());
        let parsed = parsed.candidate_query();
        let filtered = parsed.directory.is_some()
            || parsed.category.is_some()
            || parsed.extension.is_some()
            || parsed.artist.is_some()
            || parsed.album.is_some()
            || parsed.title.is_some();
        if filtered && !self.shared_indexing && !self.candidate_attempted {
            self.candidate_attempted = true;
            self.candidates = super::candidates::Candidates::build_source(&self.source, &|| {
                self.cancellation.is_cancelled() || self.query.cancelled.load(Ordering::Acquire)
            })
            .ok();
            if self.query.cancelled.load(Ordering::Acquire) {
                self.candidate_attempted = false;
            }
        }
    }

    fn rescore(&mut self, items: &[SearchItem]) {
        self.matches.clear();
        let parsed = ParsedSearchQuery::parse(&self.query.text.to_lowercase());
        for item in items {
            if self.cancellation.is_cancelled() || self.query.cancelled.load(Ordering::Acquire) {
                break;
            }
            if let Some(score) = self.score(item, &parsed) {
                insert_match(&mut self.matches, score, item.clone());
            }
        }
    }

    fn publish(&mut self, indexing: bool) {
        if self.cancellation.is_cancelled() || self.query.cancelled.load(Ordering::Acquire) {
            return;
        }
        // Materialize only the displayed prefix. A failed read withdraws the
        // whole revision, including group counts, before publishing any event.
        let mut ranked_items = Vec::new();
        if self.revision > 0
            && self.groups.is_none()
            && let Some(ranked) = &self.ranked
        {
            match ranked.first(self.shown_limit, &|| {
                self.cancellation.is_cancelled() || self.query.cancelled.load(Ordering::Acquire)
            }) {
                Ok(rows) => {
                    match materialize_items(&self.source, rows, &|| {
                        self.cancellation.is_cancelled()
                            || self.query.cancelled.load(Ordering::Acquire)
                    }) {
                        Ok(items) => ranked_items = items,
                        Err(error) => self.rank_notice = Some(error),
                    }
                }
                Err(error) => self.rank_notice = Some(error),
            }
        }
        let mut grouped_results = self
            .query
            .grouping
            .as_ref()
            .filter(|grouping| grouping.virtual_results)
            .zip(self.groups.as_ref())
            .map(|(grouping, groups)| {
                groups
                    .page_source(&self.source, grouping.group, self.shown_limit, &|| {
                        self.cancellation.is_cancelled()
                            || self.query.cancelled.load(Ordering::Acquire)
                    })
                    .map(|results| SearchGroups {
                        group: grouping.group,
                        results,
                        counts: groups.counts(),
                    })
            })
            .transpose()
            .unwrap_or_else(|error| {
                self.rank_notice = Some(error);
                None
            });
        if self.rank_notice.is_some() {
            if self.ranked.is_some() || self.groups.is_some() {
                // Consumers reject shrinking pages within one revision. Make
                // withdrawal visible and invalidate every earlier More cursor.
                self.revision = self.revision.wrapping_add(1).max(1);
            }
            self.ranked = None;
            self.groups = None;
            ranked_items.clear();
            grouped_results = None;
        }
        if self.cancellation.is_cancelled() || self.query.cancelled.load(Ordering::Acquire) {
            return;
        }
        let mut notices = Vec::new();
        if let Some(error) = &self.rank_notice {
            notices.push(format!("Search results unavailable: {error}"));
        }
        if let Some(notice) = &self.scan_notice {
            notices.push(notice.clone());
        }
        if let Some(notice) = &self.query_notice {
            notices.push(notice.clone());
        }
        if let Some(notice) = self.groups.as_ref().and_then(|groups| groups.notice()) {
            notices.push(notice.to_owned());
        }
        if self.revision == 0 && self.matches.len() == RESULT_LIMIT {
            notices.push(format!(
                "Showing the best {RESULT_LIMIT} matches; refine your search"
            ));
        }
        let page = (self.revision > 0).then_some(SearchPage {
            generation: self.query.generation,
            revision: self.revision,
            shown: self.shown_limit.min(self.result_total()),
            total: self.result_total(),
        });
        let groups = grouped_results;
        let items = if groups.is_some() {
            Vec::new()
        } else if let Some(page) = page {
            debug_assert_eq!(ranked_items.len(), page.shown);
            ranked_items
        } else {
            self.matches.iter().map(|(_, item)| item.clone()).collect()
        };
        if self.cancellation.is_cancelled() || self.query.cancelled.load(Ordering::Acquire) {
            return;
        }
        self.events.put(SearchEvent::Results {
            generation: self.query.generation,
            query: self.query.text.clone(),
            items,
            indexing,
            notice: (!notices.is_empty()).then(|| notices.join(" · ")),
            page,
            groups,
        });
        self.last_publish = Instant::now();
    }

    fn warn(&mut self, message: String) {
        let notice = self.scan_notice.get_or_insert_with(String::new);
        if notice.len() < 1500 && !notice.contains(&message) {
            if !notice.is_empty() {
                notice.push_str(" · ");
            }
            notice.push_str(&message.chars().take(300).collect::<String>());
        }
    }

    fn run(&mut self, root: PathBuf, tags: Arc<TagReader>) {
        if let Err(error) = std::fs::read_dir(&root) {
            self.warn(format!("Cannot search {}: {error}", root.display()));
            self.idle(&[]);
            return;
        }
        let cached = {
            let mut guard = self
                .index_cache
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let position = guard
                .iter()
                .position(|cached| cached.root == root && cached.built.elapsed() < CACHE_AGE);
            position
                .and_then(|position| guard.remove(position))
                .map(|cached| {
                    let found = cached.items.clone();
                    guard.push_back(cached);
                    found
                })
        };
        let cached_tags: HashMap<_, _> = cached
            .as_ref()
            .into_iter()
            .flat_map(|items| items.iter())
            .filter(|item| item.music_tags.is_some() && item.source_identity.is_some())
            .map(|item| (item.path.as_path(), item))
            .collect();
        let mut items = Vec::new();
        let mut bytes = 0;
        let music = if root == gio::glib::home_dir() {
            super::SearchScope::MusicLibrary.root(&root)
        } else {
            root.join("Music")
        };
        let prioritize_music = music.is_dir();
        let mut builder = ignore::WalkBuilder::new(if prioritize_music { &music } else { &root });
        if prioritize_music {
            builder.add(&root);
        }
        let music_filter = music.clone();
        builder.filter_entry(move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            if prioritize_music && entry.path() == music_filter {
                return false;
            }
            let name = entry.file_name().to_string_lossy();
            if name.starts_with('.') {
                return false;
            }
            if entry.file_type().is_some_and(|kind| kind.is_dir())
                && (name == "node_modules"
                    || name == "target"
                    || name == "__pycache__"
                    || name == ".git"
                    || name == ".cache")
            {
                return false;
            }
            true
        });
        let walker = builder
            .follow_links(false)
            .standard_filters(false)
            .hidden(true)
            .max_depth(Some(128))
            .build();
        for entry in walker {
            if self.cancellation.is_cancelled() {
                return;
            }
            self.pending(&items, true);
            let entry = match entry {
                Ok(entry) if entry.depth() > 0 || entry.path() != root => entry,
                Ok(_) => continue,
                Err(error) => {
                    self.warn(format!("Some folders could not be indexed: {error}"));
                    continue;
                }
            };
            let file_name = entry.file_name().to_string_lossy();
            if file_name.starts_with('.')
                || file_name == "node_modules"
                || file_name == "target"
                || file_name == "__pycache__"
                || file_name == ".git"
                || file_name == ".cache"
            {
                continue;
            }
            let kind = entry.file_type();
            if kind.is_none_or(|kind| !kind.is_file() && !kind.is_dir()) {
                continue;
            }
            if entry.depth() == 128 && kind.is_some_and(|kind| kind.is_dir()) {
                self.warn("Folder depth limited to 128; deeper files were not indexed".into());
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => Some(metadata),
                Err(error) => {
                    self.warn(format!("Some file information is inaccessible: {error}"));
                    None
                }
            };
            let modified = metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|duration| i64::try_from(duration.as_secs()).ok());
            let path = entry.into_path();
            let mut item = SearchItem::from_path(
                path,
                &root,
                kind.is_some_and(|kind| kind.is_dir()),
                modified,
            );
            item.source_identity = metadata.as_ref().map(FileIdentity::from_metadata);
            if let Some(cached) = cached_tags.get(item.path.as_path())
                && cached.source_identity == item.source_identity
            {
                item.music_tags = cached.music_tags.clone();
                item.search_metadata = cached.search_metadata.clone();
            }
            let cost = item_bytes(&item) - size_of::<SearchItem>();
            let capacity = if items.len() == items.capacity() {
                (items.capacity() + 256).min(self.limits.entries)
            } else {
                items.capacity()
            };
            let allocation = (capacity - items.capacity()) * size_of::<SearchItem>();
            if items.len() >= self.limits.entries || bytes + cost + allocation > self.limits.bytes {
                self.warn(format!(
                    "Index limited to {} entries / {} bytes; some files were not indexed",
                    self.limits.entries, self.limits.bytes
                ));
                break;
            }
            if allocation > 0 {
                items.reserve_exact(capacity - items.len());
                bytes += allocation;
            }
            bytes += cost;
            let parsed = ParsedSearchQuery::parse(&self.query.text.to_lowercase());
            if let Some(score) = self.score(&item, &parsed) {
                insert_match(&mut self.matches, score, item.clone());
            }
            items.push(item);
            if self.last_publish.elapsed() >= PUBLISH_INTERVAL {
                self.publish(true);
            }
        }
        drop(cached_tags);
        drop(cached);
        self.publish(true);
        let audio_paths: Vec<_> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                !item.is_directory
                    && item.music_tags.is_none()
                    && classify_by_name(item.path.as_os_str()) == FormatFamily::Audio
            })
            .map(|(index, item)| (index, item.path.clone()))
            .take(self.limits.metadata + 1)
            .collect();
        if audio_paths.len() > self.limits.metadata {
            self.warn(format!(
                "Song metadata limited to {} files; other songs remain searchable by filename",
                self.limits.metadata
            ));
        }
        let (sender, receiver) = mpsc::sync_channel(8);
        let cancellation = self.cancellation.clone();
        let metadata_limit = self.limits.metadata;
        let metadata_job = std::thread::Builder::new()
            .name("hermes-search-tags".into())
            .spawn(move || {
                for (index, path) in audio_paths.into_iter().take(metadata_limit) {
                    if cancellation.is_cancelled() {
                        break;
                    }
                    let result = tags(&path, &cancellation);
                    if cancellation.is_cancelled() || sender.send((index, result)).is_err() {
                        break;
                    }
                }
            });
        if let Err(error) = metadata_job {
            self.warn(format!("Unable to start song metadata indexing: {error}"));
        }
        let mut failures = 0;
        loop {
            if self.cancellation.is_cancelled() {
                return;
            }
            self.pending(&items, true);
            match receiver.recv_timeout(TICK) {
                Ok((index, Some(tags))) => {
                    let item = &mut items[index];
                    if std::fs::metadata(&item.path)
                        .ok()
                        .as_ref()
                        .map(FileIdentity::from_metadata)
                        != item.source_identity
                    {
                        self.warn(
                            "Some songs changed during indexing; refresh to update their tags"
                                .into(),
                        );
                        continue;
                    }
                    let old_cost = item_bytes(item);
                    item.search_metadata =
                        [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
                            .into_iter()
                            .filter_map(|field| field.as_deref())
                            .collect::<Vec<_>>()
                            .join(" ")
                            .to_lowercase();
                    item.music_tags = Some(tags);
                    let cost = item_bytes(item) - old_cost;
                    if bytes + cost > self.limits.bytes {
                        item.search_metadata.clear();
                        item.music_tags = None;
                        self.warn(
                            "Song metadata exceeds the 32 MiB index budget; some tags were skipped"
                                .into(),
                        );
                    } else {
                        bytes += cost;
                        self.matches.retain(|(_, result)| result.path != item.path);
                        let parsed = ParsedSearchQuery::parse(&self.query.text.to_lowercase());
                        if let Some(score) = self.score(item, &parsed) {
                            insert_match(&mut self.matches, score, item.clone());
                        }
                    }
                }
                Ok((_, None)) => {
                    failures += 1;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if failures > 0 {
                self.warn(
                    "Some song tags could not be read; filename search remains available".into(),
                );
            }
            if self.last_publish.elapsed() >= PUBLISH_INTERVAL {
                self.publish(true);
            }
        }
        self.pending(&items, false);
        let items = Arc::new(items);
        {
            let mut guard = self
                .index_cache
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            guard.retain(|cached| cached.root != root && cached.built.elapsed() < CACHE_AGE);
            while guard.len() >= 4
                || guard.iter().map(|cached| cached.bytes).sum::<usize>() + bytes > MAX_INDEX_BYTES
            {
                if guard.pop_front().is_none() {
                    break;
                }
            }
            guard.push_back(CachedIndex {
                root,
                items: items.clone(),
                built: Instant::now(),
                bytes,
            });
        }
        self.idle(&items);
    }

    fn idle(&mut self, items: &[SearchItem]) {
        self.publish(false);
        while !self.cancellation.is_cancelled() {
            self.pending(items, false);
            let guard = self
                .commands
                .value
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let _wait = self
                .commands
                .ready
                .wait_timeout_while(guard, TICK, |pending| pending.is_none());
        }
    }
}

fn materialize_items(
    source: &super::source::Source,
    rows: Vec<(i64, usize)>,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<SearchItem>, String> {
    let mut items = Vec::new();
    for (_, position) in rows {
        if cancelled() {
            return Err("Search query cancelled".into());
        }
        items.push(
            source
                .get(position)?
                .ok_or_else(|| "Invalid search result position".to_owned())?,
        );
    }
    if cancelled() {
        return Err("Search query cancelled".into());
    }
    Ok(items)
}

fn item_bytes(item: &SearchItem) -> usize {
    size_of::<SearchItem>()
        + item.path.capacity()
        + item.name.capacity()
        + item.search_name.capacity()
        + item.search_path.capacity()
        + item.search_metadata.capacity()
        + item.music_tags.as_ref().map_or(0, |tags| {
            [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
                .into_iter()
                .filter_map(|field| field.as_ref())
                .map(String::capacity)
                .sum()
        })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod pagination_tests;

#[cfg(test)]
mod group_tests;

#[cfg(test)]
mod ranking_tests;

#[cfg(test)]
mod source_tests;
