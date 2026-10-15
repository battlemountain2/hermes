use std::{collections::BTreeMap, mem::size_of, path::PathBuf};

use super::{ParsedSearchQuery, SearchItem, fold_diacritics, fuzzy_subsequence_score, matches_category};

const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const MAX_ENTITIES: usize = 40_000;
const ENTITY_OVERHEAD_BYTES: usize = 512;
const PARTIAL_NOTICE: &str = "Album and artist grouping is partial because its metadata limit was reached; all matching files and songs remain available";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SearchGroup {
    #[default]
    All,
    Files,
    Songs,
    Albums,
    Artists,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SearchEntityKey {
    Album { artist: String, album: String },
    Artist(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchEntity {
    pub key: SearchEntityKey,
    pub title: String,
    pub artist: Option<String>,
    pub songs: usize,
    pub representative: SearchItem,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchResult {
    File(SearchItem),
    Album(SearchEntity),
    Artist(SearchEntity),
}

impl SearchResult {
    pub fn key(&self) -> SearchResultKey {
        match self {
            Self::File(item) => SearchResultKey::File(item.path.clone()),
            Self::Album(entity) | Self::Artist(entity) => {
                SearchResultKey::Entity(entity.key.clone())
            }
        }
    }

    pub fn group(&self) -> SearchGroup {
        match self {
            Self::File(item) if is_song(item) => SearchGroup::Songs,
            Self::File(_) => SearchGroup::Files,
            Self::Album(_) => SearchGroup::Albums,
            Self::Artist(_) => SearchGroup::Artists,
        }
    }

    #[cfg(test)]
    pub fn file(&self) -> Option<&SearchItem> {
        match self {
            Self::File(item) => Some(item),
            Self::Album(_) | Self::Artist(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SearchResultKey {
    File(PathBuf),
    Entity(SearchEntityKey),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SearchCounts {
    pub files: usize,
    pub songs: usize,
    pub albums: usize,
    pub artists: usize,
}

impl SearchCounts {
    pub fn total(&self, group: SearchGroup) -> usize {
        match group {
            SearchGroup::All => self
                .files
                .saturating_add(self.songs)
                .saturating_add(self.albums)
                .saturating_add(self.artists),
            SearchGroup::Files => self.files,
            SearchGroup::Songs => self.songs,
            SearchGroup::Albums => self.albums,
            SearchGroup::Artists => self.artists,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchGroups {
    pub group: SearchGroup,
    pub results: Vec<SearchResult>,
    pub counts: SearchCounts,
}

struct EntityPosition {
    key: SearchEntityKey,
    summary: EntitySummary,
}

struct EntitySummary {
    songs: usize,
    representative: usize,
    score: i64,
}

pub(super) struct GroupIndex {
    files: Vec<usize>,
    songs: Vec<usize>,
    albums: Vec<EntityPosition>,
    artists: Vec<EntityPosition>,
    partial: bool,
}

impl GroupIndex {
    pub(super) fn build(
        items: &[SearchItem],
        ranked: &[(i64, usize)],
        parsed: &ParsedSearchQuery,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        Self::build_with_limits(
            items,
            ranked,
            parsed,
            cancelled,
            MAX_METADATA_BYTES,
            MAX_ENTITIES,
        )
    }

    fn build_with_limits(
        items: &[SearchItem],
        ranked: &[(i64, usize)],
        parsed: &ParsedSearchQuery,
        cancelled: &impl Fn() -> bool,
        max_bytes: usize,
        max_entities: usize,
    ) -> Result<Self, String> {
        check_cancelled(cancelled)?;
        let mut index = Self {
            files: Vec::new(),
            songs: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            partial: false,
        };
        let mut entities = BTreeMap::<SearchEntityKey, EntitySummary>::new();
        let mut metadata_bytes = size_of::<Self>();
        let mut album_count = 0;
        let terms = parsed.terms.trim().to_lowercase();
        for &(_, position) in ranked {
            check_cancelled(cancelled)?;
            let item = items
                .get(position)
                .ok_or_else(|| "Grouped search contains an invalid source position".to_owned())?;
            if !is_song(item) {
                index.files.push(position);
                continue;
            }
            index.songs.push(position);
            let Some(labels) = entity_labels(item) else {
                continue;
            };
            for key in [album_key(&labels), artist_key(&labels)]
                .into_iter()
                .flatten()
            {
                let Some(score) = entity_score(&key, &terms) else {
                    continue;
                };
                if let Some(summary) = entities.get_mut(&key) {
                    summary.songs += 1;
                    if item.path < items[summary.representative].path {
                        summary.representative = position;
                    }
                    continue;
                }
                let bytes = key_bytes(&key).saturating_add(ENTITY_OVERHEAD_BYTES);
                if entities.len() >= max_entities
                    || bytes > max_bytes.saturating_sub(metadata_bytes)
                {
                    index.partial = true;
                    continue;
                }
                metadata_bytes += bytes;
                if matches!(&key, SearchEntityKey::Album { .. }) {
                    album_count += 1;
                }
                entities.insert(
                    key,
                    EntitySummary {
                        songs: 1,
                        representative: position,
                        score,
                    },
                );
            }
        }
        index.albums = Vec::with_capacity(album_count);
        index.artists = Vec::with_capacity(entities.len() - album_count);
        for (key, summary) in entities {
            check_cancelled(cancelled)?;
            let positions = match &key {
                SearchEntityKey::Album { .. } => &mut index.albums,
                SearchEntityKey::Artist(_) => &mut index.artists,
            };
            positions.push(EntityPosition { key, summary });
        }
        for positions in [&mut index.albums, &mut index.artists] {
            check_cancelled(cancelled)?;
            positions.sort_unstable_by(|left, right| {
                right
                    .summary
                    .score
                    .cmp(&left.summary.score)
                    .then_with(|| left.key.cmp(&right.key))
            });
        }
        check_cancelled(cancelled)?;
        Ok(index)
    }

    pub(super) fn counts(&self) -> SearchCounts {
        SearchCounts {
            files: self.files.len(),
            songs: self.songs.len(),
            albums: self.albums.len(),
            artists: self.artists.len(),
        }
    }

    pub(super) fn page(
        &self,
        items: &[SearchItem],
        group: SearchGroup,
        limit: usize,
    ) -> Vec<SearchResult> {
        let lengths = [
            self.files.len(),
            self.songs.len(),
            self.albums.len(),
            self.artists.len(),
        ];
        let mut quotas = [0usize; 4];
        match group {
            SearchGroup::All => {
                let target = limit.min(self.counts().total(SearchGroup::All));
                let mut assigned = 0;
                while assigned < target {
                    for (quota, length) in quotas.iter_mut().zip(lengths) {
                        if *quota < length && assigned < target {
                            *quota += 1;
                            assigned += 1;
                        }
                    }
                }
            }
            SearchGroup::Files => quotas[0] = limit.min(lengths[0]),
            SearchGroup::Songs => quotas[1] = limit.min(lengths[1]),
            SearchGroup::Albums => quotas[2] = limit.min(lengths[2]),
            SearchGroup::Artists => quotas[3] = limit.min(lengths[3]),
        }
        let mut results = Vec::with_capacity(quotas.iter().sum());
        for (positions, quota) in [(&self.files, quotas[0]), (&self.songs, quotas[1])] {
            results.extend(
                positions
                    .iter()
                    .take(quota)
                    .map(|&position| SearchResult::File(items[position].clone())),
            );
        }
        for (positions, quota, album) in [
            (&self.albums, quotas[2], true),
            (&self.artists, quotas[3], false),
        ] {
            results.extend(positions.iter().take(quota).map(|position| {
                let representative = &items[position.summary.representative];
                let labels = entity_labels(representative).expect("Indexed entity has valid tags");
                let (title, artist) = if album {
                    (
                        labels.album.expect("Indexed album has a title"),
                        labels.album_artist,
                    )
                } else {
                    (labels.artist.expect("Indexed artist has a name"), None)
                };
                let entity = SearchEntity {
                    key: position.key.clone(),
                    title: title.to_owned(),
                    artist: artist.map(str::to_owned),
                    songs: position.summary.songs,
                    representative: representative.clone(),
                };
                if album {
                    SearchResult::Album(entity)
                } else {
                    SearchResult::Artist(entity)
                }
            }));
        }
        results
    }

    pub(super) fn notice(&self) -> Option<&str> {
        self.partial.then_some(PARTIAL_NOTICE)
    }
}

pub(super) fn matches_entity(item: &SearchItem, key: &SearchEntityKey) -> bool {
    let Some(labels) = entity_labels(item) else {
        return false;
    };
    match key {
        SearchEntityKey::Album { .. } => album_key(&labels).as_ref() == Some(key),
        SearchEntityKey::Artist(_) => artist_key(&labels).as_ref() == Some(key),
    }
}

struct EntityLabels<'a> {
    album: Option<&'a str>,
    album_artist: Option<&'a str>,
    artist: Option<&'a str>,
}

fn is_song(item: &SearchItem) -> bool {
    !item.is_directory && matches_category(&item.path, "audio")
}

fn entity_labels(item: &SearchItem) -> Option<EntityLabels<'_>> {
    if !is_song(item) {
        return None;
    }
    let tags = item.music_tags.as_ref()?;
    if [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
        .into_iter()
        .flatten()
        .any(|value| value.chars().count() > 256 || value.chars().any(char::is_control))
    {
        return None;
    }
    let artist = label(tags.artist.as_deref());
    let album_artist = label(tags.album_artist.as_deref());
    Some(EntityLabels {
        album: label(tags.album.as_deref()),
        album_artist: album_artist.or(artist),
        artist: artist.or(album_artist),
    })
}

fn label(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn album_key(labels: &EntityLabels<'_>) -> Option<SearchEntityKey> {
    Some(SearchEntityKey::Album {
        artist: labels.album_artist?.to_lowercase(),
        album: labels.album?.to_lowercase(),
    })
}

fn artist_key(labels: &EntityLabels<'_>) -> Option<SearchEntityKey> {
    Some(SearchEntityKey::Artist(labels.artist?.to_lowercase()))
}

fn entity_score(key: &SearchEntityKey, terms: &str) -> Option<i64> {
    match key {
        SearchEntityKey::Album { artist, album } => {
            let label = format!("{album} {artist}");
            label_score(&label, terms).map(|score| {
                score
                    + if album == terms || artist == terms {
                        20_000
                    } else {
                        0
                    }
            })
        }
        SearchEntityKey::Artist(artist) => label_score(artist, terms),
    }
}

fn label_score(label: &str, terms: &str) -> Option<i64> {
    if terms.is_empty() {
        return Some(0);
    }
    let folded_label = fold_diacritics(label);
    let folded_terms = fold_diacritics(terms);
    if label == terms || folded_label == folded_terms {
        return Some(30_000 - label.len() as i64);
    }
    if let Some(position) = folded_label.find(&folded_terms) {
        return Some(10_000 - position as i64 * 12 - label.len() as i64);
    }
    if folded_terms.split_whitespace().all(|term| folded_label.contains(term)) {
        return Some(8_000 - label.len() as i64);
    }
    let mut score = 0i64;
    for term in folded_terms.split_whitespace() {
        score = score.saturating_add(fuzzy_subsequence_score(&folded_label, term)?);
    }
    Some(score.min(6_999))
}

fn key_bytes(key: &SearchEntityKey) -> usize {
    match key {
        SearchEntityKey::Album { artist, album } => artist.capacity() + album.capacity(),
        SearchEntityKey::Artist(artist) => artist.capacity(),
    }
}

fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("Grouped search cancelled".to_owned())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
