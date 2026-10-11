// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs::File,
    io::{Read, Take},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::AtomicBool,
    },
};

use gio::glib;

use super::{TextExtractor, capabilities_by_name};
use crate::model::MusicTags;

mod worker;
mod scope;
pub(crate) use scope::{SearchScope, ScopedSearch};
pub(crate) use worker::{SearchHandle, index_tree};

const RESULT_LIMIT: usize = 100;
const MAX_CONTENT_FILE_SIZE: u64 = 1024 * 1024;
const MAX_DOCUMENT_FILE_SIZE: u64 = 32 * 1024 * 1024;
const MAX_DOCUMENT_EXTRACTIONS_PER_QUERY: usize = 32;

struct ContentSearchContext<'a> {
    text_extraction: &'a dyn TextExtractionProvider,
    cancelled: &'a Arc<AtomicBool>,
    document_extractions_remaining: &'a mut usize,
    notice: &'a mut Option<String>,
}

pub trait TextExtractionProvider: Send + Sync {
    fn extract_text(
        &self,
        path: &Path,
        extractor: TextExtractor,
        byte_limit: usize,
        cancelled: Arc<AtomicBool>,
    ) -> Result<String, String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchItem {
    pub path: PathBuf,
    pub name: String,
    pub is_directory: bool,
    pub modified_unix_seconds: Option<i64>,
    pub music_tags: Option<MusicTags>,
    search_name: String,
    search_path: String,
    search_metadata: String,
    source_identity: Option<super::music_library::FileIdentity>,
}

impl SearchItem {
    pub(crate) fn file_entry(&self) -> crate::model::FileEntry {
        use crate::model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue};
        FileEntry(std::rc::Rc::new(FileEntryInner {
            music_tags: self.music_tags.clone(),
            location: Location::local(&self.path),
            native_name: self.path.file_name().unwrap_or_default().to_owned(),
            display_name: self.name.clone(),
            kind: if self.is_directory { EntryKind::Directory } else { EntryKind::File },
            size: self.source_identity.as_ref().map_or(MetadataValue::Unknown, |identity| MetadataValue::Known(identity.size())),
            modified_unix_seconds: self.modified_unix_seconds.map_or(MetadataValue::Unknown, MetadataValue::Known),
        }))
    }
    pub(crate) fn from_path(path: PathBuf, root: &Path, is_directory: bool, modified_unix_seconds: Option<i64>) -> Self {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        Self { search_name: name.to_lowercase(),
            search_path: path.strip_prefix(root).unwrap_or(&path).to_string_lossy().to_lowercase(),
            name, path, is_directory, modified_unix_seconds, music_tags: None, search_metadata: String::new(), source_identity: None }
    }
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
        notice: Option<String>,
    },
}


fn insert_match(matches: &mut Vec<(i64, SearchItem)>, score: i64, item: SearchItem) {
    let position = matches
        .binary_search_by(|candidate| candidate.0.cmp(&score).reverse().then_with(|| candidate.1.path.cmp(&item.path)))
        .unwrap_or_else(|position| position);
    if position < RESULT_LIMIT {
        matches.insert(position, (score, item));
        matches.truncate(RESULT_LIMIT);
    }
}


/// Scores ordered character matches, strongly preferring names, contiguous runs and word/path
/// boundaries. Exact substrings rank ahead of looser fuzzy matches.
#[cfg(test)]
pub fn fuzzy_score(item: &SearchItem, query: &str, _root: &std::path::Path) -> Option<i64> {
    let mut document_extractions_remaining = 0;
    let mut notice = None;
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut content_search = ContentSearchContext {
        text_extraction: &UnavailableTextExtraction,
        cancelled: &cancelled,
        document_extractions_remaining: &mut document_extractions_remaining,
        notice: &mut notice,
    };
    score_item(
        item,
        &ParsedSearchQuery::parse(&query.trim().to_lowercase()),
        &mut content_search,
    )
}

fn score_item(
    item: &SearchItem,
    query: &ParsedSearchQuery,
    content_search: &mut ContentSearchContext<'_>,
) -> Option<i64> {
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
        Some(search_file_content(item, needle, content_search)?)
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
    } else if query.split_whitespace().all(|term|
        item.search_metadata.contains(term) || item.search_name.contains(term) || item.search_path.contains(term)) {
        8_000 - item.search_metadata.len() as i64
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

fn search_file_content(
    item: &SearchItem,
    needle: &str,
    content_search: &mut ContentSearchContext<'_>,
) -> Option<i64> {
    if item.is_directory || needle.is_empty() {
        return None;
    }
    let metadata = item.path.symlink_metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    let extractor = capabilities_by_name(item.path.as_os_str()).text_extractor?;
    let content = match extractor {
        TextExtractor::PlainText => read_plain_text(&item.path, metadata.len())?,
        TextExtractor::Pdf | TextExtractor::Office | TextExtractor::Spreadsheet => {
            if metadata.len() > MAX_DOCUMENT_FILE_SIZE {
                return None;
            }
            if *content_search.document_extractions_remaining == 0 {
                content_search.notice.get_or_insert_with(|| {
                    format!(
                        "Document content search is limited to {MAX_DOCUMENT_EXTRACTIONS_PER_QUERY} files per query"
                    )
                });
                return None;
            }
            *content_search.document_extractions_remaining -= 1;
            match content_search.text_extraction.extract_text(
                &item.path,
                extractor,
                MAX_CONTENT_FILE_SIZE as usize,
                content_search.cancelled.clone(),
            ) {
                Ok(text) => text,
                Err(message) => {
                    content_search.notice.get_or_insert(message);
                    return None;
                }
            }
        }
    };
    let content = content.to_lowercase();
    content
        .find(needle)
        .map(|position| 8_000i64.saturating_sub(i64::try_from(position).unwrap_or(i64::MAX)))
}

fn read_plain_text(path: &Path, file_size: u64) -> Option<String> {
    if file_size > MAX_CONTENT_FILE_SIZE {
        return None;
    }
    let file = File::open(path).ok()?;
    let mut bytes = Vec::with_capacity(file_size as usize);
    let mut limited: Take<File> = file.take(MAX_CONTENT_FILE_SIZE + 1);
    limited.read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_CONTENT_FILE_SIZE || bytes.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
struct UnavailableTextExtraction;

#[cfg(test)]
impl TextExtractionProvider for UnavailableTextExtraction {
    fn extract_text(
        &self,
        _path: &Path,
        _extractor: TextExtractor,
        _byte_limit: usize,
        _cancelled: Arc<AtomicBool>,
    ) -> Result<String, String> {
        Err("Document text extraction is unavailable".to_owned())
    }
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
