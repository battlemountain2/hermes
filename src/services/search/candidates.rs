use std::mem::size_of;

use rusqlite::{
    Connection, config::DbConfig, limits::Limit, params, params_from_iter, types::Value,
};

use super::{ParsedSearchQuery, SearchItem, matches_category};

const MAX_ITEMS: usize = 40_000;
const MAX_DATABASE_BYTES: usize = 32 * 1024 * 1024;
const PAGE_SIZE: usize = 4096;
const MAX_ROW_BYTES: usize = 16 * 1024;
const CATEGORIES: [&str; 6] = [
    "audio",
    "video",
    "image",
    "document",
    "application",
    "archive",
];

pub(super) struct Candidates {
    connection: Connection,
    item_count: usize,
}

impl Candidates {
    pub(super) fn build(
        items: &[SearchItem],
        cancelled: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        check_cancelled(cancelled)?;
        if items.len() > MAX_ITEMS {
            return Err("Search candidates exceed the item limit".into());
        }
        let mut input_bytes = 0usize;
        for item in items {
            check_cancelled(cancelled)?;
            let row_bytes = item_bytes(item)?;
            if row_bytes > MAX_ROW_BYTES {
                return Err("Search candidate exceeds the row limit".into());
            }
            input_bytes = input_bytes.checked_add(row_bytes).ok_or_else(size_error)?;
            if input_bytes > MAX_DATABASE_BYTES {
                return Err("Search candidates exceed the input byte limit".into());
            }
        }
        let mut connection = Connection::open_in_memory().map_err(error)?;
        configure(&connection)?;
        check_cancelled(cancelled)?;
        connection
            .execute_batch(
                "CREATE TABLE candidates (
                sequence INTEGER PRIMARY KEY,
                directory INTEGER NOT NULL CHECK(directory IN (0, 1)),
                extension TEXT, artist TEXT, album TEXT, title TEXT
            ) STRICT;
            CREATE INDEX candidate_directory ON candidates(directory, sequence);
            CREATE INDEX candidate_extension ON candidates(extension, sequence);
            CREATE INDEX candidate_artist ON candidates(artist, sequence);
            CREATE INDEX candidate_album ON candidates(album, sequence);
            CREATE INDEX candidate_title ON candidates(title, sequence);
            CREATE TABLE categories (
                category TEXT NOT NULL, sequence INTEGER NOT NULL,
                PRIMARY KEY(category, sequence)
            ) WITHOUT ROWID, STRICT;",
            )
            .map_err(error)?;
        {
            let transaction = connection.transaction().map_err(error)?;
            let mut insert = transaction
                .prepare(
                    "INSERT INTO candidates(sequence, directory, extension, artist, album, title)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(error)?;
            let mut insert_category = transaction
                .prepare("INSERT INTO categories(category, sequence) VALUES (?1, ?2)")
                .map_err(error)?;
            for (sequence, item) in items.iter().enumerate() {
                check_cancelled(cancelled)?;
                let extension = if item.is_directory {
                    None
                } else {
                    item.path
                        .extension()
                        .map(|value| value.to_string_lossy().to_lowercase())
                };
                let tags = item.music_tags.as_ref();
                let artist = tags
                    .and_then(|tags| tags.artist.as_deref())
                    .map(str::to_lowercase);
                let album = tags
                    .and_then(|tags| tags.album.as_deref())
                    .map(str::to_lowercase);
                let title = tags
                    .and_then(|tags| tags.title.as_deref())
                    .map(str::to_lowercase);
                let normalized_bytes = [&extension, &artist, &album, &title]
                    .into_iter()
                    .filter_map(|field| field.as_ref())
                    .try_fold(128usize, |total, field| total.checked_add(field.len()))
                    .ok_or_else(size_error)?;
                if normalized_bytes > MAX_ROW_BYTES {
                    return Err("Normalized search candidate exceeds the row limit".into());
                }
                insert
                    .execute(params![
                        sequence as i64,
                        item.is_directory,
                        extension,
                        artist,
                        album,
                        title
                    ])
                    .map_err(error)?;
                if !item.is_directory {
                    for category in CATEGORIES {
                        check_cancelled(cancelled)?;
                        if matches_category(&item.path, category) {
                            insert_category
                                .execute(params![category, sequence as i64])
                                .map_err(error)?;
                        }
                    }
                }
            }
            drop(insert_category);
            drop(insert);
            check_cancelled(cancelled)?;
            transaction.commit().map_err(error)?;
        }
        connection
            .execute_batch("PRAGMA query_only=ON;")
            .map_err(error)?;
        let pages: i64 = connection
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .map_err(error)?;
        if pages < 0 || pages > (MAX_DATABASE_BYTES / PAGE_SIZE) as i64 {
            return Err("Search candidates exceed the database page limit".into());
        }
        check_cancelled(cancelled)?;
        Ok(Self {
            connection,
            item_count: items.len(),
        })
    }

    pub(super) fn matching(
        &self,
        query: &ParsedSearchQuery,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Vec<usize>, String> {
        check_cancelled(cancelled)?;
        let mut selects = Vec::with_capacity(6);
        let mut bindings = Vec::with_capacity(6);
        if let Some(directory) = query.directory {
            selects.push("SELECT sequence FROM candidates WHERE directory = ?");
            bindings.push(Value::Integer(i64::from(directory)));
        }
        for (constraint, select) in [
            (
                &query.extension,
                "SELECT sequence FROM candidates WHERE extension = ?",
            ),
            (
                &query.category,
                "SELECT sequence FROM categories WHERE category = ?",
            ),
            (
                &query.artist,
                "SELECT sequence FROM candidates WHERE artist = ?",
            ),
            (
                &query.album,
                "SELECT sequence FROM candidates WHERE album = ?",
            ),
            (
                &query.title,
                "SELECT sequence FROM candidates WHERE title = ?",
            ),
        ] {
            if let Some(value) = constraint {
                if value.len() > MAX_ROW_BYTES {
                    return Err("Search candidate constraint exceeds the value limit".into());
                }
                selects.push(select);
                bindings.push(Value::Text(value.clone()));
            }
        }
        let mut matches = Vec::with_capacity(self.item_count);
        if selects.is_empty() {
            for sequence in 0..self.item_count {
                check_cancelled(cancelled)?;
                matches.push(sequence);
            }
        } else {
            let sql = format!("{} ORDER BY sequence", selects.join(" INTERSECT "));
            let mut statement = self.connection.prepare(&sql).map_err(error)?;
            check_cancelled(cancelled)?;
            let mut rows = statement
                .query(params_from_iter(bindings.iter()))
                .map_err(error)?;
            loop {
                check_cancelled(cancelled)?;
                let Some(row) = rows.next().map_err(error)? else {
                    break;
                };
                let sequence: i64 = row.get(0).map_err(error)?;
                let sequence = usize::try_from(sequence).map_err(error)?;
                if sequence >= self.item_count
                    || matches.last().is_some_and(|previous| *previous >= sequence)
                {
                    return Err("Search candidates returned an invalid sequence".into());
                }
                matches.push(sequence);
            }
        }
        check_cancelled(cancelled)?;
        Ok(matches)
    }
}

fn configure(connection: &Connection) -> Result<(), String> {
    for (limit, value) in [
        (Limit::SQLITE_LIMIT_LENGTH, MAX_ROW_BYTES as i32),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, 4096),
        (Limit::SQLITE_LIMIT_COLUMN, 8),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 16),
        (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 6),
        (Limit::SQLITE_LIMIT_VDBE_OP, 10_000),
        (Limit::SQLITE_LIMIT_FUNCTION_ARG, 8),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
        (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 8),
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
    connection
        .execute_batch(
            "PRAGMA page_size=4096; PRAGMA max_page_count=8192;
         PRAGMA cache_size=-2048; PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY;
         PRAGMA journal_mode=MEMORY; PRAGMA trusted_schema=OFF; PRAGMA cell_size_check=ON;",
        )
        .map_err(error)?;
    Ok(())
}

fn item_bytes(item: &SearchItem) -> Result<usize, String> {
    let mut total = [
        item.path.capacity(),
        item.name.capacity(),
        item.search_name.capacity(),
        item.search_path.capacity(),
        item.search_metadata.capacity(),
    ]
    .into_iter()
    .try_fold(size_of::<SearchItem>(), usize::checked_add)
    .ok_or_else(size_error)?;
    if let Some(tags) = &item.music_tags {
        for field in [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
            .into_iter()
            .flatten()
        {
            total = total.checked_add(field.capacity()).ok_or_else(size_error)?;
        }
    }
    Ok(total)
}

fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("Search candidate operation cancelled".into())
    } else {
        Ok(())
    }
}

fn size_error() -> String {
    "Search candidate size overflow".into()
}

fn error(error: impl std::fmt::Display) -> String {
    format!("Search candidates: {error}")
}

#[cfg(test)]
mod tests;
