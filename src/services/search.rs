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
mod candidates;
mod groups;
pub(crate) use groups::{SearchCounts, SearchEntity, SearchEntityKey, SearchGroup, SearchGroups, SearchResult, SearchResultKey};
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferredFilterChip {
    pub label: String,
    pub token_text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchFilterValues {
    pub directory: Option<bool>,
    pub category: Option<String>,
    pub extension: Option<String>,
    pub within: Option<String>,
    pub modified: Option<String>,
    pub content: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub title: Option<String>,
    pub inferred: Vec<InferredFilterChip>,
}

pub fn search_filter_values(query: &str) -> (String, SearchFilterValues) {
    let parsed = ParsedSearchQuery::parse(query.trim());
    (
        parsed.terms.clone(),
        SearchFilterValues {
            directory: parsed.directory,
            category: parsed.category.clone(),
            extension: parsed.extension.clone(),
            within: parsed.within.clone(),
            modified: parsed.modified_name.clone(),
            content: parsed.content.clone(),
            artist: parsed.artist.clone().or_else(|| parsed.inferred_artist.clone()),
            album: parsed.album.clone(),
            title: parsed.title.clone(),
            inferred: parsed.inferred.clone(),
        },
    )
}

pub enum SearchEvent {
    Results {
        generation: u64,
        query: String,
        items: Vec<SearchItem>,
        indexing: bool,
        notice: Option<String>,
        page: Option<SearchPage>,
        groups: Option<SearchGroups>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchPage {
    pub generation: u64,
    pub revision: u64,
    pub shown: usize,
    pub total: usize,
}

impl SearchPage {
    pub fn has_more(self) -> bool {
        self.shown < self.total
    }
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
        &ParsedSearchQuery::parse(query.trim()),
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
    if let Some(category) = query.category.as_deref()
        && (item.is_directory || !matches_category(&item.path, category))
    {
        return None;
    }
    if let Some(within) = query.within.as_deref()
        && !item.search_path.contains(within)
    {
        return None;
    }
    let tags = item.music_tags.as_ref();
    for (constraint, field) in [
        (&query.artist, tags.and_then(|tags| tags.artist.as_deref())),
        (&query.album, tags.and_then(|tags| tags.album.as_deref())),
        (&query.title, tags.and_then(|tags| tags.title.as_deref())),
    ] {
        if let Some(expected) = constraint
            && field.is_none_or(|value| value.to_lowercase() != *expected)
        {
            return None;
        }
    }
    let inferred_artist_bonus = if let Some(inferred_artist) = query.inferred_artist.as_deref() {
        let folded_artist = fold_diacritics(inferred_artist);
        let tag_artist = tags.and_then(|t| t.artist.as_deref()).map(fold_diacritics);
        let tag_album_artist = tags.and_then(|t| t.album_artist.as_deref()).map(fold_diacritics);
        if tag_artist.as_deref() == Some(&folded_artist) || tag_album_artist.as_deref() == Some(&folded_artist) {
            15_000
        } else if fold_diacritics(&item.search_metadata).contains(&folded_artist) || fold_diacritics(&item.search_name).contains(&folded_artist) {
            10_000
        } else {
            return None;
        }
    } else {
        0
    };
    if let Some((start, end)) = query.modified {
        let modified = item.modified_unix_seconds?;
        if modified < start || modified >= end {
            return None;
        }
        if query.terms.is_empty() && query.content.is_none() {
            return (!item.is_directory).then_some(modified + inferred_artist_bonus);
        }
    }
    let content_score = if let Some(needle) = query.content.as_deref() {
        Some(search_file_content(item, needle, content_search)?)
    } else {
        None
    };
    if query.terms.is_empty() {
        return content_score.or(Some(if item.is_directory { 20 } else { 0 } + inferred_artist_bonus));
    }

    let raw_terms = query.terms.as_str();
    let folded_terms = fold_diacritics(raw_terms);
    let folded_name = fold_diacritics(&item.search_name);
    let folded_path = fold_diacritics(&item.search_path);
    let folded_metadata = fold_diacritics(&item.search_metadata);

    let exact_name = item.search_name == raw_terms || folded_name == folded_terms;

    let mut score = if exact_name {
        25_000 - folded_name.len() as i64
    } else if let Some(position) = folded_name.find(&folded_terms) {
        12_000 - position as i64 * 12 - folded_name.len() as i64
    } else if let Some(position) = folded_metadata.find(&folded_terms) {
        10_000 - position as i64 * 8 - folded_metadata.len() as i64
    } else {
        let term_words: Vec<&str> = folded_terms.split_whitespace().collect();
        if term_words.len() > 1 && term_words.iter().all(|term| {
            folded_metadata.contains(term) || folded_name.contains(term)
        }) {
            8_500 - folded_metadata.len() as i64
        } else if let Some(position) = folded_path.find(&folded_terms) {
            5_000 - position as i64 * 4 - folded_path.len() as i64
        } else if term_words.iter().all(|term| {
            folded_metadata.contains(term) || folded_name.contains(term) || folded_path.contains(term)
        }) {
            let all_in_name_or_meta = term_words.iter().all(|t| folded_metadata.contains(t) || folded_name.contains(t));
            if all_in_name_or_meta {
                8_000 - folded_metadata.len() as i64
            } else {
                4_000 - folded_path.len() as i64
            }
        } else {
            let mut typos = 0;
            let all_match_typo = !term_words.is_empty() && term_words.iter().all(|term| {
                if folded_name.contains(term) || folded_metadata.contains(term) {
                    true
                } else if matches_word_with_typo(term, &folded_name) || matches_word_with_typo(term, &folded_metadata) {
                    typos += 1;
                    true
                } else {
                    false
                }
            });
            if all_match_typo && typos > 0 {
                7_500 - (typos as i64 * 400) - folded_metadata.len() as i64
            } else {
                fuzzy_subsequence_score(&item.search_path, raw_terms)?
            }
        }
    };

    if exact_name {
        score += 20_000;
    }
    if item.is_directory {
        score += 20;
    }
    Some(score + content_score.unwrap_or_default() + inferred_artist_bonus)
}

pub(crate) fn fold_diacritics(input: &str) -> String {
    let mut folded = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            'a' | 'A' | 'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą'
            | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' => folded.push('a'),
            'c' | 'C' | 'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' | 'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => folded.push('c'),
            'd' | 'D' | 'ď' | 'đ' | 'Ď' | 'Đ' => folded.push('d'),
            'e' | 'E' | 'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě'
            | 'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' => folded.push('e'),
            'g' | 'G' | 'ĝ' | 'ğ' | 'ġ' | 'ģ' | 'Ĝ' | 'Ğ' | 'Ġ' | 'Ģ' => folded.push('g'),
            'h' | 'H' | 'ĥ' | 'ħ' | 'Ĥ' | 'Ħ' => folded.push('h'),
            'i' | 'I' | 'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı'
            | 'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ' | 'Ī' | 'Ĭ' | 'Į' | 'İ' => folded.push('i'),
            'j' | 'J' | 'ĵ' | 'Ĵ' => folded.push('j'),
            'k' | 'K' | 'ķ' | 'ĸ' | 'Ķ' => folded.push('k'),
            'l' | 'L' | 'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' | 'Ĺ' | 'Ļ' | 'Ľ' | 'Ŀ' | 'Ł' => folded.push('l'),
            'n' | 'N' | 'ñ' | 'ń' | 'ņ' | 'ň' | 'ŉ' | 'ŋ' | 'Ñ' | 'Ń' | 'Ņ' | 'Ň' | 'Ŋ' => folded.push('n'),
            'o' | 'O' | 'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő'
            | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ō' | 'Ŏ' | 'Ő' => folded.push('o'),
            'r' | 'R' | 'ŕ' | 'ŗ' | 'ř' | 'Ŕ' | 'Ŗ' | 'Ř' => folded.push('r'),
            's' | 'S' | 'ś' | 'ŝ' | 'ş' | 'š' | 'Ś' | 'Ŝ' | 'Ş' | 'Š' => folded.push('s'),
            't' | 'T' | 'ţ' | 'ť' | 'ŧ' | 'Ţ' | 'Ť' | 'Ŧ' => folded.push('t'),
            'u' | 'U' | 'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų'
            | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' => folded.push('u'),
            'w' | 'W' | 'ŵ' | 'Ŵ' => folded.push('w'),
            'y' | 'Y' | 'ý' | 'ÿ' | 'ŷ' | 'Ý' | 'Ÿ' | 'Ŷ' => folded.push('y'),
            'z' | 'Z' | 'ź' | 'ż' | 'ž' | 'Ź' | 'Ż' | 'Ž' => folded.push('z'),
            'æ' | 'Æ' => folded.push_str("ae"),
            'œ' | 'Œ' => folded.push_str("oe"),
            'ß' => folded.push_str("ss"),
            other => {
                for lower in other.to_lowercase() {
                    folded.push(lower);
                }
            }
        }
    }
    folded
}

fn edit_distance_at_most_1(a: &str, b: &str) -> bool {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();
    if a_len.abs_diff(b_len) > 1 {
        return false;
    }
    if a_len == b_len {
        let mut diffs = 0;
        let mut diff_pos = 0;
        for i in 0..a_len {
            if a_chars[i] != b_chars[i] {
                diffs += 1;
                diff_pos = i;
                if diffs > 2 {
                    return false;
                }
            }
        }
        if diffs <= 1 {
            return true;
        }
        if diffs == 2 && diff_pos > 0 && a_chars[diff_pos - 1] == b_chars[diff_pos] && a_chars[diff_pos] == b_chars[diff_pos - 1] {
            return true;
        }
        false
    } else {
        let (shorter, longer) = if a_len < b_len { (&a_chars, &b_chars) } else { (&b_chars, &a_chars) };
        let mut i = 0;
        let mut j = 0;
        let mut skipped = false;
        while i < shorter.len() && j < longer.len() {
            if shorter[i] == longer[j] {
                i += 1;
                j += 1;
            } else if !skipped {
                skipped = true;
                j += 1;
            } else {
                return false;
            }
        }
        true
    }
}

fn matches_word_with_typo(term: &str, target: &str) -> bool {
    let term_len = term.chars().count();
    if term_len < 4 {
        return false;
    }
    for word in target.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        if word.chars().count() >= 4 && edit_distance_at_most_1(term, word) {
            return true;
        }
    }
    false
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QueryToken {
    text: String,
    quoted: bool,
}

fn tokenize_query(query: &str) -> Vec<QueryToken> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut token_starts_with_quote = false;

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
            if token.is_empty() {
                token_starts_with_quote = true;
            }
        } else if quote.is_none() && character.is_whitespace() {
            if !token.is_empty() {
                tokens.push(QueryToken {
                    text: std::mem::take(&mut token),
                    quoted: token_starts_with_quote,
                });
                token_starts_with_quote = false;
            }
        } else {
            token.push(character);
        }
    }
    if escaped {
        token.push('\\');
    }
    if !token.is_empty() {
        tokens.push(QueryToken {
            text: token,
            quoted: token_starts_with_quote,
        });
    }
    tokens
}

#[derive(Clone, Default)]
struct ParsedSearchQuery {
    terms: String,
    directory: Option<bool>,
    category: Option<String>,
    extension: Option<String>,
    within: Option<String>,
    modified: Option<(i64, i64)>,
    modified_name: Option<String>,
    content: Option<String>,
    artist: Option<String>,
    inferred_artist: Option<String>,
    album: Option<String>,
    title: Option<String>,
    inferred: Vec<InferredFilterChip>,
}

impl ParsedSearchQuery {
    fn parse(query: &str) -> Self {
        let now = glib::DateTime::now_local().unwrap_or_else(|_| {
            glib::DateTime::from_unix_local(0).expect("unix epoch")
        });
        Self::parse_at(query, &now)
    }

    pub(crate) fn parse_at(query: &str, now: &glib::DateTime) -> Self {
        let mut parsed = Self::default();
        let tokens = tokenize_query(query);
        let mut remaining = Vec::new();

        for token in tokens {
            let lower = token.text.to_lowercase();
            if let Some(value) = lower.strip_prefix("type:") {
                match value {
                    "file" => parsed.directory = Some(false),
                    "folder" | "directory" => parsed.directory = Some(true),
                    "audio" | "video" | "image" | "document" | "application" | "archive" => {
                        parsed.category = Some(value.to_owned());
                    }
                    _ => remaining.push(token),
                }
                continue;
            } else if let Some(value) = lower.strip_prefix("ext:") {
                let extension = value.trim_start_matches('.');
                if extension.is_empty() {
                    remaining.push(token);
                } else {
                    parsed.extension = Some(extension.to_owned());
                }
                continue;
            } else if let Some(value) = lower.strip_prefix("in:") {
                if value.is_empty() {
                    remaining.push(token);
                } else {
                    parsed.within = Some(value.to_owned());
                }
                continue;
            } else if lower.starts_with("modified:") {
                if let Some(bounds) = recent_bounds_at(&lower, now) {
                    parsed.modified = Some(bounds);
                    parsed.modified_name = lower.strip_prefix("modified:").map(str::to_owned);
                } else {
                    remaining.push(token);
                }
                continue;
            } else if let Some(value) = lower.strip_prefix("content:") {
                if value.is_empty() {
                    remaining.push(token);
                } else {
                    parsed.content = Some(value.to_owned());
                    parsed.directory = Some(false);
                }
                continue;
            } else if let Some((field, value)) = lower.split_once(':') {
                if matches!(field, "artist" | "album" | "title") {
                    if value.is_empty() {
                        remaining.push(token);
                    } else {
                        let constraint = match field {
                            "artist" => &mut parsed.artist,
                            "album" => &mut parsed.album,
                            _ => &mut parsed.title,
                        };
                        *constraint = Some(value.to_owned());
                    }
                    continue;
                }
            }
            remaining.push(token);
        }

        parse_date_phrases(&mut remaining, &mut parsed, now);
        parse_location_phrases(&mut remaining, &mut parsed);
        parse_artist_phrases(&mut remaining, &mut parsed);
        parse_type_and_format_phrases(&mut remaining, &mut parsed);

        let terms: Vec<String> = remaining.into_iter().map(|t| t.text).collect();
        parsed.terms = terms.join(" ");
        parsed
    }

    fn is_empty(&self) -> bool {
        self.terms.is_empty()
            && self.directory.is_none()
            && self.category.is_none()
            && self.extension.is_none()
            && self.within.is_none()
            && self.modified.is_none()
            && self.content.is_none()
            && self.artist.is_none()
            && self.inferred_artist.is_none()
            && self.album.is_none()
            && self.title.is_none()
    }
}

fn parse_date_phrases(
    tokens: &mut Vec<QueryToken>,
    parsed: &mut ParsedSearchQuery,
    now: &glib::DateTime,
) {
    if parsed.modified.is_some() || tokens.is_empty() {
        return;
    }
    let mut i = 0;
    while i < tokens.len() {
        if i + 2 < tokens.len()
            && !tokens[i].quoted
            && !tokens[i + 1].quoted
            && !tokens[i + 2].quoted
            && (tokens[i].text.eq_ignore_ascii_case("from") || tokens[i].text.eq_ignore_ascii_case("modified"))
            && tokens[i + 1].text.eq_ignore_ascii_case("last")
            && tokens[i + 2].text.eq_ignore_ascii_case("week")
        {
            if let Some(bounds) = recent_bounds_at("modified:last-week", now) {
                parsed.modified = Some(bounds);
                parsed.modified_name = Some("last-week".to_owned());
                parsed.inferred.push(InferredFilterChip {
                    label: "Modified: last week".to_owned(),
                    token_text: format!("{} last week", tokens[i].text),
                });
                tokens.drain(i..=i + 2);
                return;
            }
        }
        if i + 3 < tokens.len()
            && !tokens[i].quoted
            && !tokens[i + 1].quoted
            && !tokens[i + 2].quoted
            && !tokens[i + 3].quoted
            && (tokens[i].text.eq_ignore_ascii_case("from") || tokens[i].text.eq_ignore_ascii_case("the"))
            && (tokens[i + 1].text.eq_ignore_ascii_case("past") || tokens[i + 1].text.eq_ignore_ascii_case("last"))
            && tokens[i + 2].text == "7"
            && tokens[i + 3].text.eq_ignore_ascii_case("days")
        {
            if let Some(bounds) = recent_bounds_at("modified:past-7-days", now) {
                parsed.modified = Some(bounds);
                parsed.modified_name = Some("past-7-days".to_owned());
                parsed.inferred.push(InferredFilterChip {
                    label: "Modified: past 7 days".to_owned(),
                    token_text: "past 7 days".to_owned(),
                });
                tokens.drain(i..=i + 3);
                return;
            }
        }
        if i + 2 < tokens.len()
            && !tokens[i].quoted
            && !tokens[i + 1].quoted
            && !tokens[i + 2].quoted
            && (tokens[i].text.eq_ignore_ascii_case("past") || tokens[i].text.eq_ignore_ascii_case("last"))
            && tokens[i + 1].text == "7"
            && tokens[i + 2].text.eq_ignore_ascii_case("days")
        {
            if let Some(bounds) = recent_bounds_at("modified:past-7-days", now) {
                parsed.modified = Some(bounds);
                parsed.modified_name = Some("past-7-days".to_owned());
                parsed.inferred.push(InferredFilterChip {
                    label: "Modified: past 7 days".to_owned(),
                    token_text: "past 7 days".to_owned(),
                });
                tokens.drain(i..=i + 2);
                return;
            }
        }
        if i + 1 < tokens.len()
            && !tokens[i].quoted
            && !tokens[i + 1].quoted
            && tokens[i].text.eq_ignore_ascii_case("last")
            && tokens[i + 1].text.eq_ignore_ascii_case("week")
        {
            if let Some(bounds) = recent_bounds_at("modified:last-week", now) {
                parsed.modified = Some(bounds);
                parsed.modified_name = Some("last-week".to_owned());
                parsed.inferred.push(InferredFilterChip {
                    label: "Modified: last week".to_owned(),
                    token_text: "last week".to_owned(),
                });
                tokens.drain(i..=i + 1);
                return;
            }
        }
        if i + 1 < tokens.len()
            && !tokens[i].quoted
            && !tokens[i + 1].quoted
            && (tokens[i].text.eq_ignore_ascii_case("from") || tokens[i].text.eq_ignore_ascii_case("modified"))
            && (tokens[i + 1].text.eq_ignore_ascii_case("today") || tokens[i + 1].text.eq_ignore_ascii_case("yesterday"))
        {
            let target = tokens[i + 1].text.to_lowercase();
            if let Some(bounds) = recent_bounds_at(&format!("modified:{target}"), now) {
                parsed.modified = Some(bounds);
                parsed.modified_name = Some(target.clone());
                parsed.inferred.push(InferredFilterChip {
                    label: format!("Modified: {target}"),
                    token_text: format!("{} {}", tokens[i].text, tokens[i + 1].text),
                });
                tokens.drain(i..=i + 1);
                return;
            }
        }
        i += 1;
    }
}

fn parse_location_phrases(tokens: &mut Vec<QueryToken>, parsed: &mut ParsedSearchQuery) {
    if parsed.within.is_some() || tokens.is_empty() {
        return;
    }
    for i in 0..tokens.len() {
        if (tokens[i].text.eq_ignore_ascii_case("in") || tokens[i].text.eq_ignore_ascii_case("within"))
            && !tokens[i].quoted
            && i + 1 < tokens.len()
            && !tokens[i + 1].quoted
        {
            let folder = tokens[i + 1].text.clone();
            parsed.within = Some(folder.to_lowercase());
            parsed.inferred.push(InferredFilterChip {
                label: format!("In: {folder}"),
                token_text: format!("{} {folder}", tokens[i].text),
            });
            tokens.drain(i..=i + 1);
            return;
        }
    }
}

fn parse_artist_phrases(tokens: &mut Vec<QueryToken>, parsed: &mut ParsedSearchQuery) {
    if parsed.artist.is_some() || parsed.inferred_artist.is_some() || tokens.is_empty() {
        return;
    }
    for i in 0..tokens.len() {
        if tokens[i].text.eq_ignore_ascii_case("by") && !tokens[i].quoted && i + 1 < tokens.len() {
            let artist_tokens: Vec<String> = tokens.drain(i + 1..).map(|t| t.text).collect();
            let artist_display = artist_tokens.join(" ");
            let artist_token_text = format!("by {artist_display}");
            tokens.remove(i);
            parsed.inferred_artist = Some(artist_display.to_lowercase());
            parsed.inferred.push(InferredFilterChip {
                label: format!("Artist: {artist_display}"),
                token_text: artist_token_text,
            });
            return;
        }
    }
}

const AUDIO_EXTS: [&str; 8] = ["flac", "mp3", "wav", "ogg", "m4a", "aac", "alac", "aiff"];

fn parse_type_and_format_phrases(tokens: &mut Vec<QueryToken>, parsed: &mut ParsedSearchQuery) {
    for i in 0..tokens.len() {
        let is_audio_ext = !tokens[i].quoted
            && AUDIO_EXTS.iter().any(|ext| tokens[i].text.eq_ignore_ascii_case(ext));
        if is_audio_ext && i + 1 < tokens.len() && !tokens[i + 1].quoted {
            let next = &tokens[i + 1].text;
            if matches!(next.to_ascii_lowercase().as_str(), "songs" | "song" | "tracks" | "track" | "audio") {
                let ext = tokens[i].text.to_lowercase();
                if parsed.extension.is_none() {
                    parsed.extension = Some(ext.clone());
                    parsed.inferred.push(InferredFilterChip {
                        label: format!("Format: {}", ext.to_uppercase()),
                        token_text: tokens[i].text.clone(),
                    });
                }
                if parsed.category.is_none() {
                    parsed.category = Some("audio".to_owned());
                    parsed.inferred.push(InferredFilterChip {
                        label: "Audio".to_owned(),
                        token_text: next.clone(),
                    });
                }
                tokens.drain(i..=i + 1);
                return;
            }
        }
    }

    if tokens.len() >= 2 {
        let last_idx = tokens.len() - 1;
        let last = &tokens[last_idx];
        if !last.quoted && matches!(last.text.to_ascii_lowercase().as_str(), "songs" | "song" | "tracks" | "track") {
            let token_text = last.text.clone();
            if parsed.category.is_none() {
                parsed.category = Some("audio".to_owned());
                parsed.inferred.push(InferredFilterChip {
                    label: "Audio".to_owned(),
                    token_text,
                });
            }
            tokens.pop();
            return;
        }
    }

    for i in 0..tokens.len() {
        if !tokens[i].quoted {
            if tokens[i].text.eq_ignore_ascii_case("pdfs") {
                if parsed.extension.is_none() {
                    parsed.extension = Some("pdf".to_owned());
                    parsed.inferred.push(InferredFilterChip {
                        label: "Format: PDF".to_owned(),
                        token_text: tokens[i].text.clone(),
                    });
                }
                if parsed.category.is_none() {
                    parsed.category = Some("document".to_owned());
                }
                tokens.remove(i);
                return;
            } else if tokens[i].text.eq_ignore_ascii_case("pdf") && i + 1 < tokens.len() && !tokens[i + 1].quoted {
                let next = tokens[i + 1].text.to_ascii_lowercase();
                if matches!(next.as_str(), "files" | "file" | "documents" | "document") {
                    if parsed.extension.is_none() {
                        parsed.extension = Some("pdf".to_owned());
                        parsed.inferred.push(InferredFilterChip {
                            label: "Format: PDF".to_owned(),
                            token_text: format!("{} {}", tokens[i].text, tokens[i + 1].text),
                        });
                    }
                    if parsed.category.is_none() {
                        parsed.category = Some("document".to_owned());
                    }
                    tokens.drain(i..=i + 1);
                    return;
                }
            }
        }
    }

    for i in 0..tokens.len() {
        if !tokens[i].quoted && (tokens[i].text.eq_ignore_ascii_case("videos") || tokens[i].text.eq_ignore_ascii_case("video")) {
            if parsed.category.is_none() {
                parsed.category = Some("video".to_owned());
                parsed.inferred.push(InferredFilterChip {
                    label: "Video".to_owned(),
                    token_text: tokens[i].text.clone(),
                });
            }
            tokens.remove(i);
            return;
        }
    }

    for i in 0..tokens.len() {
        if !tokens[i].quoted && matches!(tokens[i].text.to_ascii_lowercase().as_str(), "photos" | "images") {
            if parsed.category.is_none() {
                parsed.category = Some("image".to_owned());
                parsed.inferred.push(InferredFilterChip {
                    label: "Image".to_owned(),
                    token_text: tokens[i].text.clone(),
                });
            }
            tokens.remove(i);
            return;
        }
    }

    for i in 0..tokens.len() {
        if !tokens[i].quoted && tokens[i].text.eq_ignore_ascii_case("documents") {
            if parsed.category.is_none() {
                parsed.category = Some("document".to_owned());
                parsed.inferred.push(InferredFilterChip {
                    label: "Document".to_owned(),
                    token_text: tokens[i].text.clone(),
                });
            }
            tokens.remove(i);
            return;
        }
    }

    for i in 0..tokens.len() {
        if !tokens[i].quoted && matches!(tokens[i].text.to_ascii_lowercase().as_str(), "archives" | "zips") {
            if parsed.category.is_none() {
                parsed.category = Some("archive".to_owned());
                parsed.inferred.push(InferredFilterChip {
                    label: "Archive".to_owned(),
                    token_text: tokens[i].text.clone(),
                });
            }
            tokens.remove(i);
            return;
        }
    }

    for i in 0..tokens.len() {
        if !tokens[i].quoted && matches!(tokens[i].text.to_ascii_lowercase().as_str(), "folders" | "directories") {
            if parsed.directory.is_none() {
                parsed.directory = Some(true);
                parsed.inferred.push(InferredFilterChip {
                    label: "Folders".to_owned(),
                    token_text: tokens[i].text.clone(),
                });
            }
            tokens.remove(i);
            return;
        }
    }
}

fn matches_category(path: &Path, category: &str) -> bool {
    use super::{FormatFamily, classify_by_name};
    let family = classify_by_name(path.as_os_str());
    match category {
        "audio" => family == FormatFamily::Audio,
        "video" => family == FormatFamily::Video,
        "image" => matches!(
            family,
            FormatFamily::Image | FormatFamily::Svg | FormatFamily::Heif
                | FormatFamily::RawImage | FormatFamily::GeoTiff
        ),
        "document" => matches!(
            family,
            FormatFamily::Pdf | FormatFamily::OfficeDocument
                | FormatFamily::Spreadsheet | FormatFamily::PlainText
        ),
        "application" => family == FormatFamily::DesktopEntry
            || path.extension().is_some_and(|extension| matches!(
                extension.to_string_lossy().to_ascii_lowercase().as_str(), "exe" | "appimage"
            )),
        "archive" => matches!(family, FormatFamily::Archive | FormatFamily::Comic),
        _ => false,
    }
}

#[cfg(test)]
#[expect(dead_code, reason = "Query tokenization helper for search tests")]
pub(crate) fn query_tokens(query: &str) -> Vec<String> {
    tokenize_query(query).into_iter().map(|t| t.text).collect()
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

pub(crate) fn recent_bounds_at(query: &str, now: &glib::DateTime) -> Option<(i64, i64)> {
    let name = query.strip_prefix("modified:")?;
    let today = glib::DateTime::from_local(
        now.year(),
        now.month(),
        now.day_of_month(),
        0,
        0,
        0.0,
    ).ok()?;
    let today_start = today.to_unix();
    match name {
        "today" => Some((today_start, today.add_days(1).ok()?.to_unix())),
        "yesterday" => Some((today.add_days(-1).ok()?.to_unix(), today_start)),
        "last-week" => {
            let day_of_week = today.day_of_week();
            let this_monday = today.add_days(-(day_of_week - 1)).ok()?;
            let last_monday = this_monday.add_days(-7).ok()?;
            Some((last_monday.to_unix(), this_monday.to_unix()))
        }
        "past-7-days" => {
            let start = now.add_days(-7).ok()?;
            Some((start.to_unix(), now.to_unix()))
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn recent_bounds(query: &str) -> Option<(i64, i64)> {
    let now = glib::DateTime::now_local().ok()?;
    recent_bounds_at(query, &now)
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
