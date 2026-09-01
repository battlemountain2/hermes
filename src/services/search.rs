// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs::File,
    io::{Read, Take},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use gio::glib;

use super::classify_by_name;

const RESULT_LIMIT: usize = 100;
const PUBLISH_INTERVAL: Duration = Duration::from_millis(50);
const MAX_CONTENT_FILE_SIZE: u64 = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchItem {
    pub path: PathBuf,
    pub name: String,
    pub is_directory: bool,
    pub modified_unix_seconds: Option<i64>,
    search_name: String,
    search_path: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchFilterValues {
    pub directory: Option<bool>,
    pub extension: Option<String>,
    pub within: Option<String>,
    pub modified: Option<String>,
    pub content: Option<String>,
}

pub fn search_filter_values(query: &str) -> (String, SearchFilterValues) {
    let parsed = ParsedSearchQuery::parse(&query.trim().to_lowercase());
    (
        parsed.terms,
        SearchFilterValues {
            directory: parsed.directory,
            extension: parsed.extension,
            within: parsed.within,
            modified: parsed.modified_name,
            content: parsed.content,
        },
    )
}

pub enum SearchEvent {
    Results {
        query: String,
        items: Vec<SearchItem>,
        indexing: bool,
    },
}

enum SearchCommand {
    Query(String),
}

pub struct SearchHandle {
    cancelled: Arc<AtomicBool>,
    commands: Sender<SearchCommand>,
}

impl SearchHandle {
    pub fn query(&self, query: &str) {
        let _sent = self
            .commands
            .send(SearchCommand::Query(query.trim().to_owned()));
    }
}

impl Drop for SearchHandle {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// Builds and searches the index entirely off the GTK thread. The UI receives only the best
/// bounded result set, so typing remains responsive even while very large trees are being walked.
pub fn index_tree(root: PathBuf) -> (SearchHandle, Receiver<SearchEvent>) {
    let (command_sender, command_receiver) = mpsc::channel();
    let (event_sender, event_receiver) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let _worker = std::thread::Builder::new()
        .name("strata-search-index".into())
        .spawn(move || {
            let mut index = Vec::new();
            let mut query = String::new();
            let mut matches = Vec::<(i64, SearchItem)>::new();
            let mut last_publish = Instant::now();
            let walker = ignore::WalkBuilder::new(&root)
                .hidden(true)
                .follow_links(false)
                .standard_filters(true)
                .require_git(false)
                .build();

            for entry in walker
                .filter_map(Result::ok)
                .filter(|entry| entry.depth() > 0)
            {
                if worker_cancelled.load(Ordering::Relaxed) {
                    return;
                }
                apply_pending_queries(
                    &command_receiver,
                    &event_sender,
                    &index,
                    &mut query,
                    &mut matches,
                    true,
                    &worker_cancelled,
                );
                let is_directory = entry.file_type().is_some_and(|kind| kind.is_dir());
                let modified_unix_seconds = entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|duration| i64::try_from(duration.as_secs()).ok());
                let path = entry.into_path();
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let search_path = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_lowercase();
                let item = SearchItem {
                    search_name: name.to_lowercase(),
                    name,
                    is_directory,
                    modified_unix_seconds,
                    path,
                    search_path,
                };
                if let Some(score) = score_item(&item, &ParsedSearchQuery::parse(&query)) {
                    insert_match(&mut matches, score, item.clone());
                }
                index.push(item);

                if !query.is_empty() && last_publish.elapsed() >= PUBLISH_INTERVAL {
                    publish(&event_sender, &query, &matches, true);
                    last_publish = Instant::now();
                }
            }

            publish(&event_sender, &query, &matches, false);
            while !worker_cancelled.load(Ordering::Relaxed) {
                match command_receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(SearchCommand::Query(next)) => {
                        query = command_receiver
                            .try_iter()
                            .map(|SearchCommand::Query(query)| query)
                            .last()
                            .unwrap_or(next);
                        matches = score_index(&index, &query, &worker_cancelled);
                        publish(&event_sender, &query, &matches, false);
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
    (
        SearchHandle {
            cancelled,
            commands: command_sender,
        },
        event_receiver,
    )
}

fn apply_pending_queries(
    receiver: &Receiver<SearchCommand>,
    sender: &Sender<SearchEvent>,
    index: &[SearchItem],
    query: &mut String,
    matches: &mut Vec<(i64, SearchItem)>,
    indexing: bool,
    cancelled: &AtomicBool,
) {
    let Some(next) = receiver
        .try_iter()
        .map(|SearchCommand::Query(query)| query)
        .last()
    else {
        return;
    };
    *query = next;
    *matches = score_index(index, query, cancelled);
    publish(sender, query, matches, indexing);
}

fn score_index(
    index: &[SearchItem],
    query: &str,
    cancelled: &AtomicBool,
) -> Vec<(i64, SearchItem)> {
    let mut matches = Vec::with_capacity(RESULT_LIMIT);
    let normalized_query = query.trim().to_lowercase();
    let parsed = ParsedSearchQuery::parse(&normalized_query);
    for item in index {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        if let Some(score) = score_item(item, &parsed) {
            insert_match(&mut matches, score, item.clone());
        }
    }
    matches
}

fn insert_match(matches: &mut Vec<(i64, SearchItem)>, score: i64, item: SearchItem) {
    let position = matches
        .binary_search_by(|candidate| candidate.0.cmp(&score).reverse())
        .unwrap_or_else(|position| position);
    if position < RESULT_LIMIT {
        matches.insert(position, (score, item));
        matches.truncate(RESULT_LIMIT);
    }
}

fn publish(
    sender: &Sender<SearchEvent>,
    query: &str,
    matches: &[(i64, SearchItem)],
    indexing: bool,
) {
    if query.is_empty() {
        return;
    }
    let _sent = sender.send(SearchEvent::Results {
        query: query.to_owned(),
        items: matches.iter().map(|(_, item)| item.clone()).collect(),
        indexing,
    });
}

/// Scores ordered character matches, strongly preferring names, contiguous runs and word/path
/// boundaries. Exact substrings rank ahead of looser fuzzy matches.
#[cfg(test)]
pub fn fuzzy_score(item: &SearchItem, query: &str, _root: &std::path::Path) -> Option<i64> {
    score_item(
        item,
        &ParsedSearchQuery::parse(&query.trim().to_lowercase()),
    )
}

fn score_item(item: &SearchItem, query: &ParsedSearchQuery) -> Option<i64> {
    if query.is_empty() {
        return None;
    }
    if let Some(directory) = query.directory
        && item.is_directory != directory
    {
        return None;
    }
    if let Some(extension) = query.extension.as_deref()
        && (item.is_directory
            || item
                .path
                .extension()
                .is_none_or(|candidate| candidate.to_string_lossy().to_lowercase() != extension))
    {
        return None;
    }
    if let Some(within) = query.within.as_deref()
        && !item.search_path.contains(within)
    {
        return None;
    }
    if let Some((start, end)) = query.modified {
        let modified = item.modified_unix_seconds?;
        if modified < start || modified >= end {
            return None;
        }
        if query.terms.is_empty() && query.content.is_none() {
            return (!item.is_directory).then_some(modified);
        }
    }
    let content_score = if let Some(needle) = query.content.as_deref() {
        Some(search_file_content(item, needle)?)
    } else {
        None
    };
    if query.terms.is_empty() {
        return content_score.or(Some(if item.is_directory { 20 } else { 0 }));
    }
    let query = query.terms.as_str();
    let mut score = if let Some(position) = item.search_name.find(query) {
        10_000 - position as i64 * 12 - item.search_name.len() as i64
    } else if let Some(position) = item.search_path.find(query) {
        7_000 - position as i64 * 4 - item.search_path.len() as i64
    } else {
        fuzzy_subsequence_score(&item.search_path, query)?
    };
    if item.search_name == query {
        score += 20_000;
    }
    if item.is_directory {
        score += 20;
    }
    Some(score + content_score.unwrap_or_default())
}

#[derive(Default)]
struct ParsedSearchQuery {
    terms: String,
    directory: Option<bool>,
    extension: Option<String>,
    within: Option<String>,
    modified: Option<(i64, i64)>,
    modified_name: Option<String>,
    content: Option<String>,
}

impl ParsedSearchQuery {
    fn parse(query: &str) -> Self {
        let mut parsed = Self::default();
        let mut terms = Vec::new();
        for token in query_tokens(query) {
            if let Some(value) = token.strip_prefix("type:") {
                match value {
                    "file" => parsed.directory = Some(false),
                    "folder" | "directory" => parsed.directory = Some(true),
                    _ => terms.push(token),
                }
            } else if let Some(value) = token.strip_prefix("ext:") {
                let extension = value.trim_start_matches('.');
                if extension.is_empty() {
                    terms.push(token);
                } else {
                    parsed.extension = Some(extension.to_owned());
                }
            } else if let Some(value) = token.strip_prefix("in:") {
                if value.is_empty() {
                    terms.push(token);
                } else {
                    parsed.within = Some(value.to_owned());
                }
            } else if token.starts_with("modified:") {
                if let Some(bounds) = recent_bounds(&token) {
                    parsed.modified = Some(bounds);
                    parsed.modified_name = token.strip_prefix("modified:").map(str::to_owned);
                } else {
                    terms.push(token);
                }
            } else if let Some(value) = token.strip_prefix("content:") {
                if value.is_empty() {
                    terms.push(token);
                } else {
                    parsed.content = Some(value.to_owned());
                    parsed.directory = Some(false);
                }
            } else {
                terms.push(token);
            }
        }
        parsed.terms = terms.join(" ");
        parsed
    }

    fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.directory.is_none()
            && self.extension.is_none()
            && self.within.is_none()
            && self.modified.is_none()
            && self.content.is_none()
    }
}

fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in query.chars() {
        if escaped {
            token.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if quote == Some(character) {
            quote = None;
        } else if quote.is_none() && matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if quote.is_none() && character.is_whitespace() {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else {
            token.push(character);
        }
    }
    if escaped {
        token.push('\\');
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

fn search_file_content(item: &SearchItem, needle: &str) -> Option<i64> {
    if item.is_directory
        || needle.is_empty()
        || !classify_by_name(item.path.as_os_str()).is_searchable_text()
    {
        return None;
    }
    let metadata = item.path.symlink_metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_CONTENT_FILE_SIZE {
        return None;
    }
    let file = File::open(&item.path).ok()?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    let mut limited: Take<File> = file.take(MAX_CONTENT_FILE_SIZE + 1);
    limited.read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_CONTENT_FILE_SIZE || bytes.contains(&0) {
        return None;
    }
    let content = String::from_utf8_lossy(&bytes).to_lowercase();
    content
        .find(needle)
        .map(|position| 8_000i64.saturating_sub(i64::try_from(position).unwrap_or(i64::MAX)))
}

fn recent_bounds(query: &str) -> Option<(i64, i64)> {
    if !matches!(query, "modified:today" | "modified:yesterday") {
        return None;
    }
    let now = glib::DateTime::now_local().ok()?;
    let today =
        glib::DateTime::from_local(now.year(), now.month(), now.day_of_month(), 0, 0, 0.0).ok()?;
    let today_start = today.to_unix();
    if query == "modified:today" {
        Some((today_start, today.add_days(1).ok()?.to_unix()))
    } else {
        Some((today.add_days(-1).ok()?.to_unix(), today_start))
    }
}

fn fuzzy_subsequence_score(haystack: &str, needle: &str) -> Option<i64> {
    let mut chars = haystack.char_indices();
    let mut previous = None;
    let mut score = 1_000i64;
    for wanted in needle.chars() {
        let (position, _) = chars.find(|(_, candidate)| *candidate == wanted)?;
        score -= position as i64;
        if previous.is_some_and(|previous| previous + wanted.len_utf8() == position) {
            score += 80;
        }
        if position == 0
            || haystack[..position]
                .chars()
                .next_back()
                .is_some_and(|character| matches!(character, '/' | '-' | '_' | ' ' | '.'))
        {
            score += 45;
        }
        previous = Some(position);
    }
    Some(score)
}

#[cfg(test)]
mod tests;
