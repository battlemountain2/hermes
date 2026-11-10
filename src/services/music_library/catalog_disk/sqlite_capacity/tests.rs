use super::enforce;
use rusqlite::{Connection, Error, ErrorCode};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const PAGE_SIZES: [u64; 3] = [512, 4096, 65536];

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "hermes-sqlite-capacity-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .expect("private fixture directory");
        Self(directory)
    }

    fn path(&self) -> PathBuf {
        self.0.join("snapshot.sqlite")
    }

    fn create(&self, page_size: u64) -> Connection {
        let connection = Connection::open(self.path()).expect("create fixture database");
        connection
            .pragma_update(None, "page_size", page_size)
            .expect("fixture page size");
        connection
            .execute_batch(
                "CREATE TABLE marker(value BLOB NOT NULL);
                 INSERT INTO marker VALUES(x'07');",
            )
            .expect("allocate fixture database");
        connection.close().expect("close fixture database");
        self.open()
    }

    fn open(&self) -> Connection {
        let connection = Connection::open(self.path()).expect("reopen fixture database");
        // The requested page size leaves existing databases at their actual size.
        connection
            .execute_batch(
                "PRAGMA page_size=4096; PRAGMA journal_mode=DELETE;
                 PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF;
                 PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY;",
            )
            .expect("standard writable PRAGMAs");
        connection
    }

    fn bytes(&self) -> Vec<u8> {
        fs::read(self.path()).expect("fixture main-file bytes")
    }

    fn length(&self) -> u64 {
        fs::metadata(self.path()).expect("fixture metadata").len()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

fn pragma(connection: &Connection, name: &str) -> u64 {
    connection
        .query_row(&format!("PRAGMA main.{name}"), [], |row| row.get(0))
        .expect("read fixture PRAGMA")
}

fn assert_full(error: Error) {
    assert!(
        matches!(&error, Error::SqliteFailure(failure, _) if failure.code == ErrorCode::DiskFull),
        "expected SQLITE_FULL, got {error}"
    );
}

#[test]
fn empty_database_installs_capacity_before_the_first_schema_write() {
    for page_size in PAGE_SIZES {
        let fixture = Fixture::new();
        let connection = fixture.open();
        connection
            .pragma_update(None, "page_size", page_size)
            .expect("unallocated fixture page size");
        assert_eq!(pragma(&connection, "page_count"), 0);

        enforce(&connection, page_size).expect("one page is a valid empty-file budget");
        assert_eq!(pragma(&connection, "max_page_count"), 1);
        assert_full(
            connection
                .execute("CREATE TABLE marker(value BLOB NOT NULL)", [])
                .expect_err("schema needs more than one page"),
        );
        assert!(fixture.length() <= page_size);

        enforce(&connection, 2 * page_size).expect("two pages allow the initial schema");
        connection
            .execute_batch(
                "CREATE TABLE marker(value BLOB NOT NULL);
                 INSERT INTO marker VALUES(x'07');",
            )
            .expect("initial schema fits the enforced capacity");
        assert_eq!(pragma(&connection, "page_count"), 2);
        assert_eq!(fixture.length(), 2 * page_size);
    }
}

#[test]
fn exact_allocation_and_fractional_page_budgets_use_actual_page_size() {
    for page_size in PAGE_SIZES {
        let fixture = Fixture::new();
        let connection = fixture.create(page_size);
        let pages = pragma(&connection, "page_count");
        let allocated = fixture.length();
        let original = fixture.bytes();
        assert_eq!(pragma(&connection, "page_size"), page_size);
        assert_eq!(allocated, pages * page_size);

        for bytes in [allocated, allocated + page_size - 1] {
            enforce(&connection, bytes).expect("existing allocation fits byte cap");
            assert_eq!(pragma(&connection, "max_page_count"), pages);
            assert_full(
                connection
                    .execute("INSERT INTO marker VALUES(zeroblob(?1))", [page_size])
                    .expect_err("an additional page exceeds the floored byte cap"),
            );
            assert_eq!(fixture.bytes(), original);
        }
    }
}

#[test]
fn oversized_existing_allocation_is_refused_without_changing_bytes_or_limit() {
    for page_size in PAGE_SIZES {
        let fixture = Fixture::new();
        let connection = fixture.create(page_size);
        connection
            .execute("INSERT INTO marker VALUES(zeroblob(?1))", [8 * page_size])
            .expect("grow legacy database");
        connection
            .execute("DELETE FROM marker WHERE rowid > 1", [])
            .expect("leave allocated free pages");
        assert!(pragma(&connection, "freelist_count") > 0);
        let original = fixture.bytes();
        let limit = pragma(&connection, "max_page_count");
        let allocated = fixture.length();

        for bytes in [allocated - 1, 2 * page_size] {
            let error = enforce(&connection, bytes).expect_err("allocated pages exceed cap");
            assert!(error.contains("exceeds its byte budget"), "{error}");
            assert_eq!(pragma(&connection, "max_page_count"), limit);
            assert_eq!(fixture.bytes(), original);
        }
        connection.close().expect("close refused legacy database");
        assert_eq!(fixture.bytes(), original);
        let reopened = fixture.open();
        let marker: Vec<u8> = reopened
            .query_row("SELECT value FROM marker", [], |row| row.get(0))
            .expect("preserved legacy marker");
        assert_eq!(marker, vec![7]);
    }
}

#[test]
fn bounded_writes_reach_sqlite_full_and_preserve_committed_rows() {
    for page_size in PAGE_SIZES {
        let fixture = Fixture::new();
        let connection = fixture.create(page_size);
        let bytes = 8 * page_size + page_size - 1;
        enforce(&connection, bytes).expect("small writable capacity");
        assert_eq!(pragma(&connection, "max_page_count"), 8);
        let mut inserted = 0u64;
        let mut reached_full = false;
        for _ in 0..16 {
            match connection.execute("INSERT INTO marker VALUES(zeroblob(?1))", [page_size]) {
                Ok(_) => inserted += 1,
                Err(error) => {
                    assert_full(error);
                    reached_full = true;
                    break;
                }
            }
            assert!(pragma(&connection, "page_count") <= 8);
            assert!(fixture.length() <= bytes);
        }
        assert!(inserted > 0, "fixture must allow growth before SQLITE_FULL");
        assert!(reached_full, "bounded writes must reach SQLITE_FULL");
        assert!(fixture.length() <= 8 * page_size);
        assert!(pragma(&connection, "page_count") <= 8);
        connection.close().expect("close full database");

        let reopened = fixture.open();
        enforce(&reopened, bytes).expect("reapply cap after reopening full database");
        let rows: u64 = reopened
            .query_row("SELECT count(*) FROM marker", [], |row| row.get(0))
            .expect("read committed rows after FULL");
        assert_eq!(rows, inserted + 1);
        let integrity: String = reopened
            .query_row("PRAGMA main.integrity_check", [], |row| row.get(0))
            .expect("check full database integrity");
        assert_eq!(integrity, "ok");
    }
}

#[test]
fn reopening_different_page_sizes_applies_the_same_byte_budget() {
    let bytes = 8 * 65536;
    for page_size in PAGE_SIZES {
        let fixture = Fixture::new();
        fixture
            .create(page_size)
            .close()
            .expect("close original page-size fixture");
        let connection = fixture.open();
        connection
            .pragma_update(None, "max_page_count", 131072)
            .expect("legacy fixed page-count ceiling");

        enforce(&connection, bytes).expect("actual page size determines reopened byte cap");
        assert_eq!(pragma(&connection, "page_size"), page_size);
        assert_eq!(pragma(&connection, "max_page_count"), bytes / page_size);
        assert!(fixture.length() <= bytes);
    }
}

#[test]
fn zero_and_subpage_budgets_are_refused_without_changes() {
    for page_size in PAGE_SIZES {
        let fixture = Fixture::new();
        let connection = fixture.create(page_size);
        let original = fixture.bytes();
        let limit = pragma(&connection, "max_page_count");
        for bytes in [0, 1, 511, page_size - 1] {
            let error = enforce(&connection, bytes).expect_err("budget cannot fit one page");
            assert!(error.contains("smaller than one SQLite page"), "{error}");
            assert_eq!(pragma(&connection, "max_page_count"), limit);
            assert_eq!(fixture.bytes(), original);
        }
    }
}

#[test]
fn huge_byte_budgets_do_not_wrap_native_page_counts_or_signed_integers() {
    let native_pages = u64::from(u32::MAX) - 1;
    for page_size in PAGE_SIZES {
        let fixture = Fixture::new();
        let connection = fixture.create(page_size);
        let original = fixture.bytes();
        for bytes in [
            native_pages * page_size,
            (native_pages + 1) * page_size,
            (native_pages + 2) * page_size,
            i64::MAX as u64,
            i64::MAX as u64 + 1,
            u64::MAX,
        ] {
            enforce(&connection, bytes).expect("large byte budget has a safe native ceiling");
            assert_eq!(pragma(&connection, "max_page_count"), native_pages);
            assert_eq!(fixture.bytes(), original);
        }
        enforce(&connection, fixture.length()).expect("large limit can be reduced again");
        assert_eq!(
            pragma(&connection, "max_page_count"),
            pragma(&connection, "page_count")
        );
    }
}
