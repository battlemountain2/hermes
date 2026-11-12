//! Persistent normalized rows, with positions belonging to one published snapshot.

use std::{
    ffi::OsStr,
    mem::size_of,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OptionalExtension, Row, config::DbConfig, limits::Limit, params};

use super::super::SearchItem;
use crate::{
    model::MusicTags,
    sandbox::Cancellation,
    services::music_library::{
        Entry,
        query_storage::{Database, Staged, decode_identity, encode_identity},
    },
};

const VERSION: i64 = 2;
const APPLICATION_ID: i64 = 0x48515331;
const PAGE_SIZE: usize = 256;
const MAX_DATABASE_BYTES: i64 = 512 * 1024 * 1024;
const MAX_ROW_BYTES: usize = 64 * 1024;
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_FIELD_BYTES: usize = 16 * 1024;
const MAX_TAG_BYTES: usize = 8 * 1024;
const MAX_ITEM_BYTES: usize = 4 * MAX_ROW_BYTES;

const META_SCHEMA: &str = "CREATE TABLE metadata (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), root BLOB NOT NULL, root_key BLOB NOT NULL, source_end INTEGER NOT NULL CHECK(source_end >= 0), eligible INTEGER NOT NULL CHECK(eligible >= 0), item_bytes INTEGER NOT NULL CHECK(item_bytes >= 0), notice TEXT) STRICT";
const RECORD_SCHEMA: &str = "CREATE TABLE records (path BLOB PRIMARY KEY NOT NULL, signature BLOB NOT NULL, cost INTEGER NOT NULL CHECK(cost >= 0), name TEXT NOT NULL, search_name TEXT NOT NULL, search_path TEXT NOT NULL, search_metadata TEXT NOT NULL, directory INTEGER NOT NULL CHECK(directory IN (0, 1)), modified INTEGER, identity BLOB, title TEXT, artist TEXT, album_artist TEXT, album TEXT, disc INTEGER, track INTEGER, tagged INTEGER NOT NULL CHECK(tagged IN (0, 1))) STRICT, WITHOUT ROWID";
const PATH_INDEX_SCHEMA: &str = "CREATE INDEX positions_path ON positions(path)";
const POSITION_SCHEMA: &str = "CREATE TABLE positions (position INTEGER PRIMARY KEY CHECK(position >= 0), ordinal INTEGER NOT NULL UNIQUE CHECK(ordinal >= 0), path BLOB NOT NULL REFERENCES records(path), name TEXT NOT NULL, search_name TEXT NOT NULL, search_path TEXT NOT NULL, search_metadata TEXT NOT NULL, directory INTEGER NOT NULL CHECK(directory IN (0, 1)), cost INTEGER NOT NULL CHECK(cost >= 0)) STRICT";

const FULL_PAGE_SQL: &str = "SELECT p.position, p.ordinal, r.* FROM positions p
    JOIN records r ON r.path = p.path WHERE p.position > ?1
    ORDER BY p.position LIMIT 256";
const SCORE_PAGE_SQL: &str = "SELECT p.position, p.ordinal, p.path, p.name, p.search_name,
    p.search_path, p.search_metadata, p.directory, p.cost FROM positions p
    WHERE p.position > ?1 ORDER BY p.position LIMIT 256";

pub(super) struct Store {
    database: Database,
    root: PathBuf,
    end: usize,
    count: usize,
    notice: Option<String>,
}

struct Metadata {
    end: usize,
    count: usize,
    notice: Option<String>,
}

impl Store {
    pub(super) fn build(
        path: &Path,
        root: &Path,
        entries: &[Entry],
        entry_limit: usize,
        byte_limit: usize,
        cancel: &Cancellation,
        convert: impl Fn(&Entry) -> SearchItem,
    ) -> Result<Self, String> {
        check_cancelled(&|| cancel.is_cancelled())?;
        validate_path(root)?;
        let mut staged = Staged::open(path, root, cancel)?;
        cancellable(staged.connection_mut(), cancel)?;
        let root_key = staged.root_key();
        if root_key.len() != 16 {
            return Err(invalid());
        }
        if staged.reused() {
            // A future version belongs to its newer writer. Never replace its
            // public file, even when its schema differs from this reader's.
            let version = staged
                .connection()
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0));
            if version.is_ok_and(|version| version > VERSION) {
                return Err("Search query database uses a newer schema".into());
            }
            if configure(staged.connection())
                .and_then(|()| validate(staged.connection(), root, &root_key).map(|_| ()))
                .is_err()
            {
                check_cancelled(&|| cancel.is_cancelled())?;
                staged.reset(cancel)?;
                cancellable(staged.connection_mut(), cancel)?;
            }
        }
        configure(staged.connection())?;
        if !staged.reused() {
            initialize(staged.connection())?;
        }
        let metadata = {
            let transaction = staged.connection_mut().transaction().map_err(error)?;
            transaction
                .execute("DELETE FROM positions", [])
                .map_err(error)?;
            let mut count = 0usize;
            let mut bytes = 0usize;
            let mut end = entries.len();
            let mut notice = None;
            // Statements are bounded and reused for this build only. Nothing
            // enters Connection's statement cache or survives publication.
            {
                let mut lookup = transaction
                    .prepare("SELECT * FROM records WHERE path = ?1")
                    .map_err(error)?;
                let mut insert = transaction.prepare(
                    "INSERT INTO records VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                     ON CONFLICT(path) DO UPDATE SET signature=excluded.signature, cost=excluded.cost,
                     name=excluded.name, search_name=excluded.search_name, search_path=excluded.search_path,
                     search_metadata=excluded.search_metadata, directory=excluded.directory,
                     modified=excluded.modified, identity=excluded.identity, title=excluded.title,
                     artist=excluded.artist, album_artist=excluded.album_artist, album=excluded.album,
                     disc=excluded.disc, track=excluded.track, tagged=excluded.tagged",
                ).map_err(error)?;
                let mut position_insert = transaction.prepare(
                    "INSERT INTO positions(position, ordinal, path, name, search_name, search_path,
                     search_metadata, directory, cost) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                ).map_err(error)?;
                let mut duplicate = transaction
                    .prepare("SELECT 1 FROM positions WHERE path = ?1 LIMIT 1")
                    .map_err(error)?;
                for (position, entry) in entries.iter().enumerate() {
                    check_cancelled(&|| cancel.is_cancelled())?;
                    if entry.path == root || !entry.path.starts_with(root) {
                        continue;
                    }
                    let signature = signature(entry)?;
                    let raw_path = entry.path.as_os_str().as_bytes();
                    let mut item = empty_item();
                    let cached = {
                        let mut rows = lookup.query([raw_path]).map_err(error)?;
                        match rows.next().map_err(error)? {
                            Some(row)
                                if row.get_ref(1).map_err(error)?.as_blob().map_err(error)?
                                    == signature.as_slice() =>
                            {
                                // This is still a private, unpublished copy.
                                // Repair an invalid cached row from its raw input.
                                decode_into(row, 0, root, &mut item)
                                    .ok()
                                    .filter(|_| matches_entry(&item, entry))
                            }
                            _ => None,
                        }
                    };
                    let cost = match cached {
                        Some(cost) => cost,
                        None => {
                            item = convert(entry);
                            check_cancelled(&|| cancel.is_cancelled())?;
                            validate_item(&item, root)?;
                            let cost = super::item_bytes(&item);
                            validate_record(&item, &signature, cost)?;
                            cost
                        }
                    };
                    if !matches_entry(&item, entry) {
                        return Err("Search conversion changed raw source fields".into());
                    }
                    let Some(next_bytes) = bytes
                        .checked_add(cost)
                        .filter(|total| count < entry_limit && *total <= byte_limit)
                    else {
                        end = position;
                        notice = Some(format!(
                            "Search index limited to {entry_limit} entries / {byte_limit} bytes; some files were not indexed"
                        ));
                        break;
                    };
                    if cached.is_none() {
                        // Reject contradictory duplicates before overwriting a
                        // row already admitted at an earlier source position.
                        if duplicate.exists([raw_path]).map_err(error)? {
                            return Err(
                                "Conflicting search source records for the same native path".into(),
                            );
                        }
                        let tags = item.music_tags.as_ref();
                        let identity = item.source_identity.as_ref().map(encode_identity);
                        if let Some(identity) = identity.as_deref() {
                            decode_identity(identity)?;
                        }
                        insert
                            .execute(params![
                                raw_path,
                                signature,
                                integer(cost)?,
                                item.name,
                                item.search_name,
                                item.search_path,
                                item.search_metadata,
                                i64::from(item.is_directory),
                                item.modified_unix_seconds,
                                identity,
                                tags.and_then(|tags| tags.title.as_deref()),
                                tags.and_then(|tags| tags.artist.as_deref()),
                                tags.and_then(|tags| tags.album_artist.as_deref()),
                                tags.and_then(|tags| tags.album.as_deref()),
                                tags.and_then(|tags| tags.disc),
                                tags.and_then(|tags| tags.track),
                                i64::from(tags.is_some()),
                            ])
                            .map_err(error)?;
                    }
                    // Copy the normalized values already decoded or converted
                    // above into sequential rows for this snapshot's scoring.
                    position_insert
                        .execute(params![
                            integer(position)?,
                            integer(count)?,
                            raw_path,
                            item.name,
                            item.search_name,
                            item.search_path,
                            item.search_metadata,
                            i64::from(item.is_directory),
                            integer(cost)?,
                        ])
                        .map_err(error)?;
                    bytes = next_bytes;
                    count += 1;
                }
            }
            check_cancelled(&|| cancel.is_cancelled())?;
            transaction.execute(
                "DELETE FROM records WHERE NOT EXISTS (SELECT 1 FROM positions WHERE positions.path = records.path)", [],
            ).map_err(error)?;
            transaction
                .execute("DELETE FROM metadata", [])
                .map_err(error)?;
            transaction
                .execute(
                    "INSERT INTO metadata VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        root.as_os_str().as_bytes(),
                        root_key,
                        integer(end)?,
                        integer(count)?,
                        integer(bytes)?,
                        notice
                    ],
                )
                .map_err(error)?;
            check_cancelled(&|| cancel.is_cancelled())?;
            transaction.commit().map_err(error)?;
            validate(staged.connection(), root, &root_key)?
        };
        check_cancelled(&|| cancel.is_cancelled())?;
        let database = staged.publish(cancel)?;
        Ok(Self {
            database,
            root: root.to_owned(),
            end: metadata.end,
            count: metadata.count,
            notice: metadata.notice,
        })
    }

    pub(super) fn end(&self) -> usize {
        self.end
    }

    pub(super) fn count(&self) -> usize {
        self.count
    }

    pub(super) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub(super) fn get(&mut self, position: usize) -> Result<Option<SearchItem>, String> {
        self.database.check_root()?;
        if position >= self.end {
            return Ok(None);
        }
        let mut statement = self.database.connection().prepare(
            "SELECT r.* FROM positions p JOIN records r ON r.path = p.path WHERE p.position = ?1",
        ).map_err(error)?;
        let mut rows = statement.query([integer(position)?]).map_err(error)?;
        let item = match rows.next().map_err(error)? {
            Some(row) => {
                let mut item = empty_item();
                decode_into(row, 0, &self.root, &mut item)?;
                Some(item)
            }
            None => {
                // A broken record reference must not masquerade as a scope hole.
                if self
                    .database
                    .connection()
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM positions WHERE position = ?1)",
                        [integer(position)?],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(error)?
                {
                    return Err(invalid());
                }
                None
            }
        };
        self.database.check_root()?;
        Ok(item)
    }

    /// Returns the used prefix length. The caller may keep up to 256 decoded
    /// slots in `page` between calls, including after EOF, to reuse allocations.
    pub(super) fn read_page(
        &mut self,
        after: Option<usize>,
        page: &mut Vec<(usize, SearchItem)>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<usize, String> {
        let result =
            self.read_page_into(after, page, cancelled, FULL_PAGE_SQL, |row, root, item| {
                decode_into(row, 2, root, item).map(|_| ())
            });
        if result.is_err() {
            page.clear();
        }
        result
    }

    /// Text-only scoring projection. Raw tags, identity and modification time
    /// are omitted. Results and grouping must resolve the full source rows.
    /// Like `read_page`, only the returned prefix belongs to this page.
    pub(super) fn read_score_page(
        &mut self,
        after: Option<usize>,
        page: &mut Vec<(usize, SearchItem)>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<usize, String> {
        let result = self.read_page_into(after, page, cancelled, SCORE_PAGE_SQL, decode_score_into);
        if let Ok(used) = result {
            // Unused reusable slots may still contain full rows from an earlier
            // read. Release their raw payloads too, including at successful EOF.
            for (_, item) in &mut page[used..] {
                item.music_tags = None;
                item.source_identity = None;
                item.modified_unix_seconds = None;
            }
        } else {
            page.clear();
        }
        result
    }

    fn read_page_into(
        &self,
        after: Option<usize>,
        page: &mut Vec<(usize, SearchItem)>,
        cancelled: &impl Fn() -> bool,
        sql: &str,
        decode: impl Fn(&Row<'_>, &Path, &mut SearchItem) -> Result<(), String>,
    ) -> Result<usize, String> {
        check_cancelled(cancelled)?;
        self.database.check_root()?;
        page.truncate(PAGE_SIZE);
        if after.is_some_and(|position| position >= self.end) {
            return Ok(0);
        }
        let after = after.map(integer).transpose()?.unwrap_or(-1);
        let connection = self.database.connection();
        let ordinal: Option<i64> = connection
            .query_row(
                "SELECT ordinal FROM positions WHERE position <= ?1 ORDER BY position DESC LIMIT 1",
                [after],
                |row| row.get(0),
            )
            .optional()
            .map_err(error)?;
        let start = match ordinal {
            Some(ordinal) => natural(ordinal)?.checked_add(1).ok_or_else(invalid)?,
            None => 0,
        };
        let expected = self
            .count
            .checked_sub(start)
            .ok_or_else(invalid)?
            .min(PAGE_SIZE);
        let mut statement = connection.prepare(sql).map_err(error)?;
        let mut rows = statement.query([after]).map_err(error)?;
        let mut used = 0;
        let mut previous = after;
        loop {
            check_cancelled(cancelled)?;
            let Some(row) = rows.next().map_err(error)? else {
                break;
            };
            let position: i64 = row.get(0).map_err(error)?;
            let ordinal = natural(row.get(1).map_err(error)?)?;
            if position <= previous
                || natural(position)? >= self.end
                || ordinal != start + used
                || used >= expected
            {
                return Err(invalid());
            }
            if used == page.len() {
                page.push((0, empty_item()));
            }
            decode(row, &self.root, &mut page[used].1)?;
            page[used].0 = natural(position)?;
            previous = position;
            used += 1;
        }
        if used != expected {
            return Err("Search query source ended before its committed count".into());
        }
        check_cancelled(cancelled)?;
        self.database.check_root()?;
        Ok(used)
    }
}

fn initialize(connection: &Connection) -> Result<(), String> {
    connection.execute_batch("BEGIN IMMEDIATE").map_err(error)?;
    for schema in [
        META_SCHEMA,
        RECORD_SCHEMA,
        POSITION_SCHEMA,
        PATH_INDEX_SCHEMA,
    ] {
        connection.execute_batch(schema).map_err(error)?;
    }
    connection
        .pragma_update(None, "user_version", VERSION)
        .map_err(error)?;
    connection
        .pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(error)?;
    connection.execute_batch("COMMIT").map_err(error)
}

fn configure(connection: &Connection) -> Result<(), String> {
    for (limit, value) in [
        (Limit::SQLITE_LIMIT_LENGTH, MAX_ROW_BYTES as i32),
        (Limit::SQLITE_LIMIT_SQL_LENGTH, MAX_ROW_BYTES as i32),
        (Limit::SQLITE_LIMIT_COLUMN, 32),
        (Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 32),
        (Limit::SQLITE_LIMIT_EXPR_DEPTH, 32),
        (Limit::SQLITE_LIMIT_COMPOUND_SELECT, 2),
        (Limit::SQLITE_LIMIT_VDBE_OP, 2_000_000),
        (Limit::SQLITE_LIMIT_ATTACHED, 0),
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
        (DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true),
    ] {
        connection.set_db_config(setting, enabled).map_err(error)?;
    }
    connection
        .execute_batch(
            "PRAGMA cache_size=-2048; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;
         PRAGMA trusted_schema=OFF; PRAGMA cell_size_check=ON;",
        )
        .map_err(error)?;
    let page_size: i64 = connection
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .map_err(error)?;
    if !(512..=65536).contains(&page_size) || !(page_size as u64).is_power_of_two() {
        return Err(invalid());
    }
    connection
        .pragma_update(None, "max_page_count", MAX_DATABASE_BYTES / page_size)
        .map_err(error)?;
    connection
        .execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")
        .map_err(error)?;
    let pages: i64 = connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .map_err(error)?;
    if !(0..=MAX_DATABASE_BYTES / page_size).contains(&pages) {
        return Err("Search query database exceeds 512 MiB".into());
    }
    Ok(())
}

fn cancellable(connection: &mut Connection, cancellation: &Cancellation) -> Result<(), String> {
    let cancellation = cancellation.clone();
    connection
        .progress_handler(1000, Some(move || cancellation.is_cancelled()))
        .map_err(error)
}

fn validate(connection: &Connection, root: &Path, root_key: &[u8]) -> Result<Metadata, String> {
    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(error)?;
    let application: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(error)?;
    if version != VERSION || application != APPLICATION_ID {
        return Err(invalid());
    }
    let mut statement = connection.prepare(
        "SELECT name, type, tbl_name, sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name LIMIT 5",
    ).map_err(error)?;
    let mut rows = statement.query([]).map_err(error)?;
    for (name, kind, table, schema) in [
        ("metadata", "table", "metadata", META_SCHEMA),
        ("positions", "table", "positions", POSITION_SCHEMA),
        ("positions_path", "index", "positions", PATH_INDEX_SCHEMA),
        ("records", "table", "records", RECORD_SCHEMA),
    ] {
        let row = rows.next().map_err(error)?.ok_or_else(invalid)?;
        if row.get::<_, String>(0).map_err(error)? != name
            || row.get::<_, String>(1).map_err(error)? != kind
            || row.get::<_, String>(2).map_err(error)? != table
            || row.get::<_, String>(3).map_err(error)? != schema
        {
            return Err(invalid());
        }
    }
    if rows.next().map_err(error)?.is_some() {
        return Err(invalid());
    }
    let mut statement = connection
        .prepare("SELECT * FROM metadata LIMIT 2")
        .map_err(error)?;
    let mut rows = statement.query([]).map_err(error)?;
    let row = rows.next().map_err(error)?.ok_or_else(invalid)?;
    if row.get::<_, i64>(0).map_err(error)? != 1
        || row.get_ref(1).map_err(error)?.as_blob().map_err(error)? != root.as_os_str().as_bytes()
        || row.get_ref(2).map_err(error)?.as_blob().map_err(error)? != root_key
    {
        return Err(invalid());
    }
    let end = natural(row.get(3).map_err(error)?)?;
    let count = natural(row.get(4).map_err(error)?)?;
    let bytes = natural(row.get(5).map_err(error)?)?;
    let notice: Option<String> = row.get(6).map_err(error)?;
    if count > end
        || notice.as_ref().is_some_and(|notice| notice.len() > 4096)
        || rows.next().map_err(error)?.is_some()
    {
        return Err(invalid());
    }
    let (actual_count, first, last, maximum, actual_bytes): (i64, Option<i64>, Option<i64>, Option<i64>, i64) = connection.query_row(
        "SELECT COUNT(*), MIN(p.ordinal), MAX(p.ordinal), MAX(p.position), COALESCE(SUM(r.cost), 0)
         FROM positions p JOIN records r ON r.path = p.path", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    ).map_err(error)?;
    let positions: i64 = connection
        .query_row("SELECT COUNT(*) FROM positions", [], |row| row.get(0))
        .map_err(error)?;
    let orphan: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM records r WHERE NOT EXISTS (SELECT 1 FROM positions p WHERE p.path = r.path))", [], |row| row.get(0),
    ).map_err(error)?;
    if natural(actual_count)? != count
        || positions != actual_count
        || natural(actual_bytes)? != bytes
        || orphan
    {
        return Err(invalid());
    }
    if count == 0 {
        if first.is_some() || last.is_some() || maximum.is_some() {
            return Err(invalid());
        }
    } else if first != Some(0)
        || last != Some(integer(count - 1)?)
        || maximum.is_none_or(|position| position < 0 || position as u64 >= end as u64)
    {
        return Err(invalid());
    }
    Ok(Metadata { end, count, notice })
}

fn empty_item() -> SearchItem {
    SearchItem {
        path: PathBuf::new(),
        name: String::new(),
        is_directory: false,
        modified_unix_seconds: None,
        music_tags: None,
        search_name: String::new(),
        search_path: String::new(),
        search_metadata: String::new(),
        source_identity: None,
    }
}

fn validate_path(path: &Path) -> Result<(), String> {
    let path = path.as_os_str().as_bytes();
    if path.is_empty() || path.len() > MAX_PATH_BYTES || path.contains(&0) {
        return Err("Search query source has an invalid or oversized native path".into());
    }
    Ok(())
}

fn validate_tags(tags: Option<&MusicTags>) -> Result<(), String> {
    if let Some(tags) = tags {
        let bytes: usize = [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
            .into_iter()
            .flatten()
            .map(String::len)
            .sum();
        if bytes > MAX_TAG_BYTES {
            return Err("Search query source tags exceed the row limit".into());
        }
    }
    Ok(())
}

fn matches_entry(item: &SearchItem, entry: &Entry) -> bool {
    item.path.as_os_str().as_bytes() == entry.path.as_os_str().as_bytes()
        && item.is_directory == entry.is_directory
        && item.modified_unix_seconds == entry.modified_unix_seconds
        && item.source_identity == entry.identity
        && item.music_tags == entry.music_tags
}

fn signature(entry: &Entry) -> Result<Vec<u8>, String> {
    validate_path(&entry.path)?;
    validate_tags(entry.music_tags.as_ref())?;
    let bytes = serde_json::to_vec(&(
        entry.is_directory,
        entry.modified_unix_seconds,
        &entry.identity,
        &entry.music_tags,
    ))
    .map_err(error)?;
    if bytes.len() > MAX_ROW_BYTES {
        return Err("Search query input signature exceeds the row limit".into());
    }
    Ok(bytes)
}

fn validate_item(item: &SearchItem, root: &Path) -> Result<(), String> {
    validate_path(&item.path)?;
    if item.path == root || !item.path.starts_with(root) {
        return Err(invalid());
    }
    validate_tags(item.music_tags.as_ref())?;
    for text in [
        &item.name,
        &item.search_name,
        &item.search_path,
        &item.search_metadata,
    ] {
        if text.len() > MAX_FIELD_BYTES {
            return Err("Search query normalized field exceeds the row limit".into());
        }
    }
    Ok(())
}

fn validate_record(item: &SearchItem, signature: &[u8], cost: usize) -> Result<(), String> {
    let strings: usize = [
        &item.name,
        &item.search_name,
        &item.search_path,
        &item.search_metadata,
    ]
    .into_iter()
    .map(|text| text.len())
    .sum();
    let tags: usize = item.music_tags.as_ref().map_or(0, |tags| {
        [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
            .into_iter()
            .flatten()
            .map(String::len)
            .sum()
    });
    let payload = item.path.as_os_str().len() + strings + tags;
    let minimum_cost = size_of::<SearchItem>() + payload;
    // Leave ample space for SQLite's record header and numeric values. The
    // engine's LENGTH limit independently caps its complete encoded record.
    let row_bytes = payload + signature.len() + 56 + 512;
    if row_bytes > MAX_ROW_BYTES || !(minimum_cost..=MAX_ITEM_BYTES).contains(&cost) {
        return Err("Search query source has an invalid or oversized row cost".into());
    }
    Ok(())
}

fn decode_into(
    row: &Row<'_>,
    offset: usize,
    root: &Path,
    item: &mut SearchItem,
) -> Result<usize, String> {
    let path = row
        .get_ref(offset)
        .map_err(error)?
        .as_blob()
        .map_err(error)?;
    item.path.clear();
    item.path.push(OsStr::from_bytes(path));
    for (index, target) in [
        &mut item.name,
        &mut item.search_name,
        &mut item.search_path,
        &mut item.search_metadata,
    ]
    .into_iter()
    .enumerate()
    {
        row.get_ref(offset + 3 + index)
            .map_err(error)?
            .as_str()
            .map_err(error)?
            .clone_into(target);
    }
    item.is_directory = boolean(row.get(offset + 7).map_err(error)?)?;
    item.modified_unix_seconds = row.get(offset + 8).map_err(error)?;
    let identity = row.get_ref(offset + 9).map_err(error)?;
    item.source_identity = match identity {
        rusqlite::types::ValueRef::Null => None,
        _ => Some(decode_identity(identity.as_blob().map_err(error)?)?),
    };
    let tagged = boolean(row.get(offset + 16).map_err(error)?)?;
    let tags = item.music_tags.get_or_insert_with(MusicTags::default);
    for (index, target) in [
        &mut tags.title,
        &mut tags.artist,
        &mut tags.album_artist,
        &mut tags.album,
    ]
    .into_iter()
    .enumerate()
    {
        match row.get_ref(offset + 10 + index).map_err(error)? {
            rusqlite::types::ValueRef::Null => *target = None,
            value => value
                .as_str()
                .map_err(error)?
                .clone_into(target.get_or_insert_with(String::new)),
        }
    }
    tags.disc = row.get(offset + 14).map_err(error)?;
    tags.track = row.get(offset + 15).map_err(error)?;
    if !tagged {
        if *tags != MusicTags::default() {
            return Err(invalid());
        }
        item.music_tags = None;
    }
    validate_item(item, root)?;
    let signature = row
        .get_ref(offset + 1)
        .map_err(error)?
        .as_blob()
        .map_err(error)?;
    let cost = natural(row.get(offset + 2).map_err(error)?)?;
    validate_record(item, signature, cost)?;
    Ok(cost)
}

fn decode_score_into(row: &Row<'_>, root: &Path, item: &mut SearchItem) -> Result<(), String> {
    let path = row.get_ref(2).map_err(error)?.as_blob().map_err(error)?;
    item.path.clear();
    item.path.push(OsStr::from_bytes(path));
    for (index, target) in [
        &mut item.name,
        &mut item.search_name,
        &mut item.search_path,
        &mut item.search_metadata,
    ]
    .into_iter()
    .enumerate()
    {
        row.get_ref(3 + index)
            .map_err(error)?
            .as_str()
            .map_err(error)?
            .clone_into(target);
    }
    item.is_directory = boolean(row.get(7).map_err(error)?)?;
    item.music_tags = None;
    item.source_identity = None;
    item.modified_unix_seconds = None;
    validate_item(item, root)?;
    let cost = natural(row.get(8).map_err(error)?)?;
    // The persisted cost includes the omitted raw fields. Its lower bound can
    // only use this projection's payload, while the complete cost cap remains.
    validate_record(item, &[], cost)
}

fn boolean(value: i64) -> Result<bool, String> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid()),
    }
}

fn integer(value: usize) -> Result<i64, String> {
    i64::try_from(value).map_err(error)
}

fn natural(value: i64) -> Result<usize, String> {
    usize::try_from(value).map_err(error)
}

fn check_cancelled(cancelled: &impl Fn() -> bool) -> Result<(), String> {
    if cancelled() {
        Err("Search query source operation cancelled".into())
    } else {
        Ok(())
    }
}

fn invalid() -> String {
    "Invalid normalized search query database".into()
}

fn error(error: impl std::fmt::Display) -> String {
    format!("Search query source: {error}")
}

#[cfg(test)]
mod tests;
