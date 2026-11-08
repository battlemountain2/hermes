use std::{mem::size_of, path::PathBuf, sync::Arc};

use super::SearchItem;
use crate::services::music_library::Entry;

const PAGE_SIZE: usize = 256;

#[derive(Clone)]
enum Backing {
    Library(Arc<Vec<Entry>>),
    #[cfg(test)]
    Items(Arc<Vec<SearchItem>>),
}

// Positions always refer to the original snapshot, including gaps outside the
// root. The bound excludes the first eligible row omitted by a source limit.
#[derive(Clone)]
pub(super) struct Source {
    root: PathBuf,
    backing: Backing,
    end: usize,
    notice: Option<String>,
}

impl Source {
    pub(super) fn new(
        root: PathBuf,
        entries: Arc<Vec<Entry>>,
        entry_limit: usize,
        byte_limit: usize,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        check_cancelled(cancelled)?;
        let mut source = Self {
            root,
            end: entries.len(),
            backing: Backing::Library(entries),
            notice: None,
        };
        if !source.root.is_dir() {
            source.end = 0;
            source.notice = Some(format!(
                "Cannot search {}: folder unavailable",
                source.root.display()
            ));
            check_cancelled(cancelled)?;
            return Ok(source);
        }
        let mut count = 0;
        let mut bytes = 0usize;
        for position in 0..source.end {
            check_cancelled(cancelled)?;
            let Some(item) = source.get(position)? else {
                continue;
            };
            let Some(next_bytes) = bytes
                .checked_add(item_bytes(&item))
                .filter(|total| count < entry_limit && *total <= byte_limit)
            else {
                source.end = position;
                source.notice = Some(format!(
                    "Search index limited to {entry_limit} entries / {byte_limit} bytes; some files were not indexed"
                ));
                break;
            };
            bytes = next_bytes;
            count += 1;
        }
        check_cancelled(cancelled)?;
        Ok(source)
    }

    pub(super) fn empty() -> Self {
        Self {
            root: PathBuf::new(),
            backing: Backing::Library(Arc::new(Vec::new())),
            end: 0,
            notice: None,
        }
    }

    #[cfg(test)]
    pub(super) fn from_items(items: Vec<SearchItem>) -> Self {
        Self {
            root: PathBuf::new(),
            end: items.len(),
            backing: Backing::Items(Arc::new(items)),
            notice: None,
        }
    }

    pub(super) fn get(&self, position: usize) -> Result<Option<SearchItem>, String> {
        if position >= self.end {
            return Ok(None);
        }
        match &self.backing {
            Backing::Library(entries) => Ok(entries
                .get(position)
                .filter(|entry| entry.path != self.root && entry.path.starts_with(&self.root))
                .map(|entry| {
                    let mut item = SearchItem::from_path(
                        entry.path.clone(),
                        &self.root,
                        entry.is_directory,
                        entry.modified_unix_seconds,
                    );
                    item.source_identity = entry.identity.clone();
                    item.music_tags = entry.music_tags.clone();
                    item.search_metadata = entry
                        .music_tags
                        .as_ref()
                        .map(|tags| {
                            [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
                                .into_iter()
                                .filter_map(|field| field.as_deref())
                                .collect::<Vec<_>>()
                                .join(" ")
                                .to_lowercase()
                        })
                        .unwrap_or_default();
                    item
                })),
            #[cfg(test)]
            Backing::Items(items) => Ok(items.get(position).cloned()),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.end
    }

    pub(super) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub(super) fn for_each_page(
        &self,
        cancelled: &impl Fn() -> bool,
        mut visit: impl FnMut(&[(usize, SearchItem)]) -> Result<(), String>,
    ) -> Result<(), String> {
        check_cancelled(cancelled)?;
        let mut page = Vec::with_capacity(PAGE_SIZE);
        for position in 0..self.end {
            check_cancelled(cancelled)?;
            if let Some(item) = self.get(position)? {
                page.push((position, item));
                if page.len() == PAGE_SIZE {
                    check_cancelled(cancelled)?;
                    visit(&page)?;
                    check_cancelled(cancelled)?;
                    page.clear();
                }
            }
        }
        check_cancelled(cancelled)?;
        if !page.is_empty() {
            visit(&page)?;
        }
        check_cancelled(cancelled)
    }
}

// Preserve the shared worker's transformed-item accounting, including one
// struct per eligible row, without retaining those transformed rows.
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

fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("Search source operation cancelled".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
