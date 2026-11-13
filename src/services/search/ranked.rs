use std::{
    cell::RefCell,
    path::{Component, Path},
};

use rusqlite::{Connection, config::DbConfig, limits::Limit, params};

use super::SearchItem;

const MAX_DATABASE_BYTES: usize = 512 * 1024 * 1024;
const PAGE_SIZE: usize = 4096;
const RANK_PAGE_SIZE: usize = 256;
// Allow the existing auxiliary row budget for the raw path, including paths
// longer than the saved catalog's 4096-byte limit. Escaping, component tags,
// terminators and the numeric keys need their own worst-case allowance.
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_KEY_BYTES: usize = 4 * MAX_PATH_BYTES + 32;
const MAX_ROW_BYTES: usize = MAX_KEY_BYTES + 128;

/// One query revision. SQLite owns and removes the anonymous temporary file.
/// Any failed operation closes it, including failures after pages were visited.
pub(super) struct Ranked {
    connection: RefCell<Option<Connection>>,
    failure: RefCell<Option<String>>,
    album_order: bool,
    item_count: usize,
    finished: bool,
    #[cfg(test)]
    fail_after_page: std::cell::Cell<Option<usize>>,
}

impl Ranked {
    pub(super) fn new(album_order: bool, cancelled: &impl Fn() -> bool) -> Result<Self, String> {
        check_cancelled(cancelled)?;
        let connection = Connection::open("").map_err(error)?;
        configure(&connection)?;
        // Build only this ordering's index, before inserting rows. No later
        // index build or read needs to globally sort the query's results.
        connection
            .execute_batch(
                "CREATE TABLE ranked (
                    position INTEGER PRIMARY KEY CHECK(position >= 0),
                    score INTEGER NOT NULL,
                    sort_key BLOB NOT NULL
                ) STRICT;
                CREATE UNIQUE INDEX ranked_order ON ranked(sort_key);
                BEGIN IMMEDIATE;",
            )
            .map_err(error)?;
        check_cancelled(cancelled)?;
        Ok(Self {
            connection: RefCell::new(Some(connection)),
            failure: RefCell::new(None),
            album_order,
            item_count: 0,
            finished: false,
            #[cfg(test)]
            fail_after_page: std::cell::Cell::new(None),
        })
    }

    pub(super) fn push(
        &mut self,
        score: i64,
        position: usize,
        item: &SearchItem,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(), String> {
        let result = (|| {
            self.check(cancelled)?;
            if self.finished {
                return Err("Search ranking is already finished".into());
            }
            let position = i64::try_from(position).map_err(error)?;
            let sequence = u64::try_from(position).map_err(error)?;
            let next_count = self.item_count.checked_add(1).ok_or_else(size_error)?;
            let key = sort_key(self.album_order, score, sequence, item, cancelled)?;
            {
                let connection = self.connection.borrow();
                let connection = connection.as_ref().ok_or_else(unavailable)?;
                connection
                    .prepare("INSERT INTO ranked(position, score, sort_key) VALUES (?1, ?2, ?3)")
                    .map_err(error)?
                    .execute(params![position, score, key])
                    .map_err(error)?;
            }
            self.check(cancelled)?;
            self.item_count = next_count;
            Ok(())
        })();
        self.latch(result)
    }

    pub(super) fn finish(&mut self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        let result = (|| {
            self.check(cancelled)?;
            if !self.finished {
                {
                    let connection = self.connection.borrow();
                    let connection = connection.as_ref().ok_or_else(unavailable)?;
                    connection
                        .execute_batch("COMMIT; PRAGMA query_only=ON;")
                        .map_err(error)?;
                    let pages: i64 = connection
                        .query_row("PRAGMA page_count", [], |row| row.get(0))
                        .map_err(error)?;
                    if !(0..=(MAX_DATABASE_BYTES / PAGE_SIZE) as i64).contains(&pages) {
                        return Err("Search ranking exceeds the database page limit".into());
                    }
                }
                self.check(cancelled)?;
                self.finished = true;
            }
            Ok(())
        })();
        self.latch(result)
    }

    pub(super) fn len(&self) -> usize {
        if self.finished && self.failure.borrow().is_none() {
            self.item_count
        } else {
            0
        }
    }

    pub(super) fn for_each_page(
        &self,
        cancelled: &impl Fn() -> bool,
        visit: impl FnMut(&[(i64, usize)]) -> Result<(), String>,
    ) -> Result<(), String> {
        self.latch(self.read(cancelled, self.item_count, visit))
    }

    pub(super) fn first(
        &self,
        limit: usize,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<(i64, usize)>, String> {
        let mut prefix = Vec::new();
        let result = self.read(cancelled, limit.min(self.item_count), |page| {
            prefix.extend_from_slice(page);
            Ok(())
        });
        self.latch(result)?;
        Ok(prefix)
    }

    #[cfg(test)]
    pub(super) fn fail_after_page_for_test(&self, page: usize) {
        self.fail_after_page.set(Some(page));
    }

    fn read(
        &self,
        cancelled: &impl Fn() -> bool,
        limit: usize,
        mut visit: impl FnMut(&[(i64, usize)]) -> Result<(), String>,
    ) -> Result<(), String> {
        self.check(cancelled)?;
        if !self.finished {
            return Err("Search ranking is unfinished".into());
        }
        let mut after = Vec::new();
        let mut remaining = limit;
        #[cfg(test)]
        let mut page_number = 0;
        while remaining > 0 {
            self.check(cancelled)?;
            let requested = remaining.min(RANK_PAGE_SIZE);
            let mut page = Vec::with_capacity(requested);
            {
                let connection = self.connection.borrow();
                let connection = connection.as_ref().ok_or_else(unavailable)?;
                let mut statement = connection
                    .prepare(
                        "SELECT score, position, sort_key FROM ranked INDEXED BY ranked_order
                         WHERE sort_key > ?1 ORDER BY sort_key LIMIT ?2",
                    )
                    .map_err(error)?;
                let mut rows = statement
                    .query(params![&after, requested as i64])
                    .map_err(error)?;
                loop {
                    self.check(cancelled)?;
                    let Some(row) = rows.next().map_err(error)? else {
                        break;
                    };
                    self.check(cancelled)?;
                    let score = row.get(0).map_err(error)?;
                    let position =
                        usize::try_from(row.get::<_, i64>(1).map_err(error)?).map_err(error)?;
                    let key: Vec<u8> = row.get(2).map_err(error)?;
                    if key <= after || key.len() > MAX_KEY_BYTES || page.len() >= requested {
                        return Err("Search ranking returned an invalid row".into());
                    }
                    after = key;
                    page.push((score, position));
                }
            }
            // The committed count is authoritative, including the last page.
            if page.len() != requested {
                return Err("Search ranking ended before its committed count".into());
            }
            self.check(cancelled)?;
            visit(&page)?;
            self.check(cancelled)?;
            #[cfg(test)]
            {
                page_number += 1;
                if self.fail_after_page.get() == Some(page_number) {
                    self.fail_after_page.set(None);
                    return Err("Generated ranking read failure after a page".into());
                }
            }
            remaining -= page.len();
        }
        self.check(cancelled)
    }

    fn check(&self, cancelled: &impl Fn() -> bool) -> Result<(), String> {
        if let Some(failure) = self.failure.borrow().as_ref() {
            return Err(failure.clone());
        }
        check_cancelled(cancelled)
    }

    fn latch<T>(&self, result: Result<T, String>) -> Result<T, String> {
        if let Err(failure) = &result {
            if self.failure.borrow().is_none() {
                *self.failure.borrow_mut() = Some(failure.clone());
            }
            // Statements and visitors have released their borrows before this
            // point. Closing also discards an unfinished transaction on error.
            drop(self.connection.borrow_mut().take());
        }
        result
    }
}

fn configure(connection: &Connection) -> Result<(), String> {
    for (limit, value) in [
        (Limit::SQLITE_LIMIT_LENGTH, MAX_ROW_BYTES as i32),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, 4096),
        (Limit::SQLITE_LIMIT_COLUMN, 8),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 16),
        (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 1),
        (Limit::SQLITE_LIMIT_VDBE_OP, 10_000),
        (Limit::SQLITE_LIMIT_FUNCTION_ARG, 8),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
        (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 3),
        (Limit::SQLITE_LIMIT_TRIGGER_DEPTH, 0),
        (Limit::SQLITE_LIMIT_WORKER_THREADS, 0),
    ] {
        connection.set_limit(limit, value).map_err(error)?;
    }
    for (setting, enabled) in [
        (DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true),
        (DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_CREATE, false),
        (DbConfig::SQLITE_DBCONFIG_ENABLE_ATTACH_WRITE, false),
    ] {
        connection.set_db_config(setting, enabled).map_err(error)?;
    }
    // DELETE keeps any rollback journal on disk, including during cache spill.
    // This transaction appends to a fresh database, so its original pages are
    // only the small schema. Avoid an unbounded in-memory rollback journal.
    connection
        .execute_batch(
            "PRAGMA page_size=4096; PRAGMA max_page_count=131072;
             PRAGMA cache_size=-2048; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;
             PRAGMA journal_mode=DELETE; PRAGMA synchronous=OFF;
             PRAGMA trusted_schema=OFF; PRAGMA cell_size_check=ON;",
        )
        .map_err(error)?;
    Ok(())
}

fn sort_key(
    album_order: bool,
    score: i64,
    sequence: u64,
    item: &SearchItem,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<u8>, String> {
    let path_bytes = item.path.as_os_str().len();
    if path_bytes > MAX_PATH_BYTES {
        return Err("Search ranking path exceeds the row limit".into());
    }
    let capacity = path_bytes
        .checked_mul(4)
        .and_then(|bytes| bytes.checked_add(32))
        .ok_or_else(size_error)?;
    let mut key = Vec::with_capacity(capacity);
    if album_order {
        let (disc, track) = album_numbers(item);
        key.extend_from_slice(&disc.to_be_bytes());
        key.extend_from_slice(&track.to_be_bytes());
    } else {
        // Complement reverses signed score order even at MIN. Flipping the
        // sign bit then gives unsigned big-endian bytes the same signed order.
        key.extend_from_slice(&((!score as u64) ^ (1u64 << 63)).to_be_bytes());
    }
    encode_path(&item.path, &mut key, cancelled)?;
    key.extend_from_slice(&sequence.to_be_bytes());
    if key.len() > MAX_KEY_BYTES {
        return Err("Search ranking key exceeds the row limit".into());
    }
    Ok(key)
}

// Component discriminants follow Rust's Component ordering. Iterating
// components preserves its normalization of separators and '.', without
// resolving '..'. Unix OsStr encoded bytes preserve non-UTF8 ordering.
fn encode_path(
    path: &Path,
    key: &mut Vec<u8>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    for component in path.components() {
        check_cancelled(cancelled)?;
        let tag = match component {
            Component::Prefix(_) => 1,
            Component::RootDir => 2,
            Component::CurDir => 3,
            Component::ParentDir => 4,
            Component::Normal(_) => 5,
        };
        key.push(tag);
        for &byte in component.as_os_str().as_encoded_bytes() {
            if byte == 0 {
                key.extend_from_slice(&[0, 255]);
            } else {
                key.push(byte);
            }
        }
        key.extend_from_slice(&[0, 0]);
    }
    // A shorter component sequence precedes its descendants, independently
    // of the sequence number appended after this explicit path terminator.
    key.push(0);
    Ok(())
}

// Keep the worker's album ordering defaults and filename fallback identical.
fn album_numbers(item: &SearchItem) -> (u32, u32) {
    let tags = item.music_tags.as_ref();
    let disc = tags
        .and_then(|tags| tags.disc)
        .filter(|number| *number > 0)
        .unwrap_or(1);
    let track = tags
        .and_then(|tags| tags.track)
        .filter(|number| *number > 0)
        .or_else(|| {
            let name = item.name.trim_start();
            let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
            let suffix = name.get(digits.len()..)?;
            if !suffix.starts_with(['.', '-', '_', ' ']) {
                return None;
            }
            digits
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0 && *number <= 1_000_000)
        })
        .unwrap_or(u32::MAX);
    (disc, track)
}

fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("Search ranking operation cancelled".into())
    } else {
        Ok(())
    }
}

fn size_error() -> String {
    "Search ranking size overflow".into()
}

fn unavailable() -> String {
    "Search ranking is unavailable".into()
}

fn error(error: impl std::fmt::Display) -> String {
    format!("Search ranking: {error}")
}

#[cfg(test)]
mod tests;
