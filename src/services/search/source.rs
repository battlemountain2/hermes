use std::{mem::size_of, path::PathBuf, sync::Arc};

use crate::sandbox::Cancellation;
#[cfg(target_os = "linux")]
use std::sync::Mutex;

#[cfg(target_os = "linux")]
mod sqlite;

use super::SearchItem;
use crate::services::music_library::{Entry, FileIdentity, RootBinding};

const PAGE_SIZE: usize = 256;

#[derive(Clone)]
enum Backing {
    Library(Arc<Vec<Entry>>),
    #[cfg(target_os = "linux")]
    Sqlite(Arc<Mutex<sqlite::Store>>),
    #[cfg(test)]
    Items(Arc<Vec<SearchItem>>),
}

// Positions always refer to the original snapshot, including gaps outside the
// root. The bound excludes the first eligible row omitted by a source limit.
#[derive(Clone)]
pub(super) struct Source {
    root: PathBuf,
    root_identity: Option<FileIdentity>,
    root_bindings: Vec<RootBinding>,
    backing: Backing,
    end: usize,
    notice: Option<String>,
}

impl Source {
    pub(super) fn persistent(
        root: PathBuf,
        entries: Arc<Vec<Entry>>,
        entry_limit: usize,
        byte_limit: usize,
        cancellation: &Cancellation,
    ) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        {
            let path = persistent_path(&root)?;
            Self::persistent_at(root, entries, entry_limit, byte_limit, cancellation, &path)
        }
        #[cfg(not(target_os = "linux"))]
        Self::new(root, entries, entry_limit, byte_limit, &|| {
            cancellation.is_cancelled()
        })
    }

    #[cfg(target_os = "linux")]
    fn persistent_at(
        root: PathBuf,
        entries: Arc<Vec<Entry>>,
        entry_limit: usize,
        byte_limit: usize,
        cancellation: &Cancellation,
        path: &std::path::Path,
    ) -> Result<Self, String> {
        let root_identity = FileIdentity::directory(&root);
        let store = sqlite::Store::build(
            path,
            &root,
            &entries,
            entry_limit,
            byte_limit,
            cancellation,
            |entry| item_from_entry(entry, &root),
        )?;
        let end = store.end();
        let notice = store.notice().map(str::to_owned);
        Ok(Self {
            root,
            root_identity,
            root_bindings: Vec::new(),
            backing: Backing::Sqlite(Arc::new(Mutex::new(store))),
            end,
            notice,
        })
    }

    pub(super) fn new(
        root: PathBuf,
        entries: Arc<Vec<Entry>>,
        entry_limit: usize,
        byte_limit: usize,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        check_cancelled(cancelled)?;
        let mut source = Self {
            root_identity: FileIdentity::directory(&root),
            root_bindings: Vec::new(),
            root,
            end: entries.len(),
            backing: Backing::Library(entries),
            notice: None,
        };
        if source.root_identity.is_none() {
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
            root_identity: None,
            root_bindings: Vec::new(),
            backing: Backing::Library(Arc::new(Vec::new())),
            end: 0,
            notice: None,
        }
    }

    #[cfg(test)]
    pub(super) fn from_items(items: Vec<SearchItem>) -> Self {
        Self {
            root: PathBuf::new(),
            root_identity: None,
            root_bindings: Vec::new(),
            end: items.len(),
            backing: Backing::Items(Arc::new(items)),
            notice: None,
        }
    }

    pub(super) fn get(&self, position: usize) -> Result<Option<SearchItem>, String> {
        self.check_root()?;
        if position >= self.end {
            return Ok(None);
        }
        match &self.backing {
            Backing::Library(entries) => Ok(entries
                .get(position)
                .filter(|entry| entry.path != self.root && entry.path.starts_with(&self.root))
                .map(|entry| item_from_entry(entry, &self.root))),
            #[cfg(target_os = "linux")]
            Backing::Sqlite(store) => store
                .lock()
                .map_err(|_| "Persistent query source lock failed".to_owned())?
                .get(position),
            #[cfg(test)]
            Backing::Items(items) => Ok(items.get(position).cloned()),
        }
    }

    fn read_into(&self, position: usize, item: &mut SearchItem) -> Result<bool, String> {
        if position >= self.end {
            return Ok(false);
        }
        match &self.backing {
            #[cfg(target_os = "linux")]
            Backing::Sqlite(store) => {
                let Some(original) = store
                    .lock()
                    .map_err(|_| "Persistent query source lock failed".to_owned())?
                    .get(position)?
                else {
                    return Ok(false);
                };
                item.clone_from(&original);
            }
            Backing::Library(entries) => {
                let Some(entry) = entries.get(position) else {
                    return Ok(false);
                };
                let Ok(relative) = entry.path.strip_prefix(&self.root) else {
                    return Ok(false);
                };
                if relative.as_os_str().is_empty() {
                    return Ok(false);
                }
                item.path.clone_from(&entry.path);
                entry
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref()
                    .clone_into(&mut item.name);
                item.search_name.clone_from(&item.name);
                lowercase(&mut item.search_name);
                relative
                    .to_string_lossy()
                    .as_ref()
                    .clone_into(&mut item.search_path);
                lowercase(&mut item.search_path);
                item.is_directory = entry.is_directory;
                item.modified_unix_seconds = entry.modified_unix_seconds;
                item.source_identity.clone_from(&entry.identity);
                item.music_tags.clone_from(&entry.music_tags);
                item.search_metadata.clear();
                if let Some(tags) = &entry.music_tags {
                    for (index, field) in
                        [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
                            .into_iter()
                            .flatten()
                            .enumerate()
                    {
                        if index > 0 {
                            item.search_metadata.push(' ');
                        }
                        item.search_metadata.push_str(field);
                    }
                    lowercase(&mut item.search_metadata);
                }
            }
            #[cfg(test)]
            Backing::Items(items) => {
                let Some(original) = items.get(position) else {
                    return Ok(false);
                };
                item.clone_from(original);
            }
        }
        Ok(true)
    }

    pub(super) fn len(&self) -> usize {
        self.end
    }

    pub(super) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub(super) fn bind_roots(mut self, roots: Vec<RootBinding>) -> Self {
        self.root_bindings = roots
            .into_iter()
            .filter(|binding| {
                binding.path.starts_with(&self.root) || self.root.starts_with(&binding.path)
            })
            .collect();
        self
    }

    pub(super) fn check_root(&self) -> Result<(), String> {
        if self.root.as_os_str().is_empty() || self.end == 0 {
            return Ok(());
        }
        if self
            .root_identity
            .as_ref()
            .zip(FileIdentity::directory(&self.root).as_ref())
            .is_some_and(|(expected, current)| expected.same_object(current))
            && self.root_bindings.iter().all(|binding| {
                FileIdentity::directory(&binding.path)
                    .is_some_and(|current| binding.identity.same_object(&current))
            })
        {
            Ok(())
        } else {
            Err("Search source root changed or is unavailable".into())
        }
    }

    pub(super) fn for_each_page(
        &self,
        cancelled: &impl Fn() -> bool,
        visit: impl FnMut(&[(usize, SearchItem)]) -> Result<(), String>,
    ) -> Result<(), String> {
        self.pages(false, cancelled, visit)
    }

    pub(super) fn for_scoring_page(
        &self,
        query: &super::ParsedSearchQuery,
        entity_filter: bool,
        cancelled: &impl Fn() -> bool,
        visit: impl FnMut(&[(usize, SearchItem)]) -> Result<(), String>,
    ) -> Result<(), String> {
        self.pages(
            !entity_filter && query.can_project_score(),
            cancelled,
            visit,
        )
    }

    fn pages(
        &self,
        projection: bool,
        cancelled: &impl Fn() -> bool,
        mut visit: impl FnMut(&[(usize, SearchItem)]) -> Result<(), String>,
    ) -> Result<(), String> {
        #[cfg(not(target_os = "linux"))]
        let _ = projection;
        check_cancelled(cancelled)?;
        self.check_root()?;
        #[cfg(target_os = "linux")]
        if let Backing::Sqlite(store) = &self.backing {
            let expected = store
                .lock()
                .map_err(|_| "Persistent query source lock failed".to_owned())?
                .count();
            let mut page = Vec::with_capacity(PAGE_SIZE);
            let mut seen = 0usize;
            let mut after = None;
            loop {
                check_cancelled(cancelled)?;
                self.check_root()?;
                let count = {
                    let mut store = store
                        .lock()
                        .map_err(|_| "Persistent query source lock failed".to_owned())?;
                    if projection {
                        store.read_score_page(after, &mut page, cancelled)?
                    } else {
                        store.read_page(after, &mut page, cancelled)?
                    }
                };
                if count > PAGE_SIZE || count > page.len() || count > expected.saturating_sub(seen)
                {
                    return Err("Persistent query source returned an invalid page".into());
                }
                if count == 0 {
                    return if seen == expected {
                        Ok(())
                    } else {
                        Err(
                            "Persistent query source ended before all rows were accounted for"
                                .into(),
                        )
                    };
                }
                for (position, _) in &page[..count] {
                    if *position >= self.end || after.is_some_and(|previous| *position <= previous)
                    {
                        return Err("Persistent query source returned an invalid position".into());
                    }
                    after = Some(*position);
                }
                check_cancelled(cancelled)?;
                // A visitor can resolve representatives through this Source.
                // Release the database lock before calling it.
                visit(&page[..count])?;
                check_cancelled(cancelled)?;
                seen += count;
            }
        }
        let mut page = Vec::with_capacity(PAGE_SIZE);
        let mut used = 0;
        for position in 0..self.end {
            check_cancelled(cancelled)?;
            if used == page.len() {
                page.push((
                    0,
                    SearchItem::from_path(PathBuf::new(), &self.root, false, None),
                ));
            }
            if self.read_into(position, &mut page[used].1)? {
                page[used].0 = position;
                used += 1;
                if used == PAGE_SIZE {
                    check_cancelled(cancelled)?;
                    self.check_root()?;
                    visit(&page)?;
                    check_cancelled(cancelled)?;
                    used = 0;
                }
            }
        }
        check_cancelled(cancelled)?;
        if used > 0 {
            self.check_root()?;
            visit(&page[..used])?;
        }
        self.check_root()?;
        check_cancelled(cancelled)
    }
}

fn item_from_entry(entry: &Entry, root: &std::path::Path) -> SearchItem {
    let mut item = SearchItem::from_path(
        entry.path.clone(),
        root,
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
}

#[cfg(target_os = "linux")]
fn persistent_path(root: &std::path::Path) -> Result<PathBuf, String> {
    let digest = gio::glib::compute_checksum_for_data(
        gio::glib::ChecksumType::Sha256,
        root.as_os_str().as_encoded_bytes(),
    )
    .ok_or("SHA-256 is unavailable")?;
    Ok(crate::paths::search_catalog_cache_dir()
        .join(format!("normalized-{digest}"))
        .join("source.sqlite"))
}

// ASCII keeps its reusable buffer. Unicode uses Rust's contextual lowercase
// rules, including final sigma and expanding case mappings.
fn lowercase(value: &mut String) {
    if value.is_ascii() {
        value.make_ascii_lowercase();
    } else {
        *value = value.to_lowercase();
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

#[cfg(all(test, target_os = "linux"))]
mod persistent_tests;
