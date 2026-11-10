use rusqlite::Connection;

/// Bound the main file on a writable connection after its standard PRAGMAs.
pub(super) fn enforce(connection: &Connection, bytes: u64) -> Result<(), String> {
    let page_size: i64 = connection
        .query_row("PRAGMA main.page_size", [], |row| row.get(0))
        .map_err(super::error)?;
    if !(512..=65536).contains(&page_size) || !(page_size as u64).is_power_of_two() {
        return Err("Search catalog has an invalid SQLite page size".into());
    }

    // SQLite page numbers stop at 2^32 - 2. Clamp before passing the value
    // through its PRAGMA parser, including for budgets near u64::MAX.
    let max_pages = (bytes / page_size as u64).min(u64::from(u32::MAX) - 1) as i64;
    if max_pages == 0 {
        return Err("Search catalog byte budget is smaller than one SQLite page".into());
    }
    let pages: i64 = connection
        .query_row("PRAGMA main.page_count", [], |row| row.get(0))
        .map_err(super::error)?;
    if !(0..=max_pages).contains(&pages) {
        return Err("Search catalog main file exceeds its byte budget".into());
    }

    // max_page_count cannot shrink an existing database. Its returned value
    // must establish the bound, even if SQLite accepts a different limit.
    let applied: i64 = connection
        .query_row(
            &format!("PRAGMA main.max_page_count={max_pages}"),
            [],
            |row| row.get(0),
        )
        .map_err(super::error)?;
    if !(1..=max_pages).contains(&applied) {
        return Err("Search catalog could not enforce its SQLite page limit".into());
    }
    let pages: i64 = connection
        .query_row("PRAGMA main.page_count", [], |row| row.get(0))
        .map_err(super::error)?;
    if !(0..=applied).contains(&pages) {
        return Err("Search catalog main file exceeds its byte budget".into());
    }
    Ok(())
}
