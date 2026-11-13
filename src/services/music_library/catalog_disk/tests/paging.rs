use super::*;
use crate::services::music_library::{MAX_ENTRIES, MAX_INDEX_BYTES, entry_bytes};

fn entries(fixture: &Fixture, count: usize) -> Vec<Entry> {
    (0..count)
        .map(|index| fixture.entry(format!("source-{index:03}.txt")))
        .collect()
}

fn open(fixture: &Fixture) -> PagedReader {
    PagedReader::open(
        &fixture.cache,
        &fixture.root,
        MAX_ENTRIES,
        MAX_INDEX_BYTES,
        &Cancellation::default(),
    )
    .expect("open generated catalog snapshot")
}

fn assert_preserved(fixture: &Fixture, original: &[u8]) {
    assert_eq!(
        fs::read(&fixture.cache).expect("read public catalog after paging"),
        original
    );
    fixture.assert_no_stages();
}

#[test]
fn page_boundaries_preserve_every_row_in_order_and_complete_idempotently() {
    assert_eq!(PAGE_ENTRIES, 256);
    let fixture = Fixture::new();
    let entries = entries(&fixture, 512);
    for count in [0, 1, 255, 256, 257, 512] {
        let expected = &entries[..count];
        fixture.save(expected, Some("Saved notice"));
        let original = fs::read(&fixture.cache).expect("read original boundary catalog");
        let mut reader = open(&fixture);
        assert_eq!(reader.notice(), Some("Saved notice"));
        for chunk in expected.chunks(PAGE_ENTRIES) {
            assert_eq!(
                reader
                    .next_page(&Cancellation::default())
                    .expect("read boundary page")
                    .as_deref(),
                Some(chunk),
                "catalog with {count} entries"
            );
        }
        for _ in 0..2 {
            assert_eq!(
                reader
                    .next_page(&Cancellation::default())
                    .expect("read completed boundary catalog"),
                None
            );
        }
        assert_eq!(reader.notice(), Some("Saved notice"));
        drop(reader);
        assert_preserved(&fixture, &original);
    }
}

#[test]
fn fully_stale_page_is_empty_and_later_fresh_rows_and_notices_survive() {
    let fixture = Fixture::new();
    let entries = entries(&fixture, PAGE_ENTRIES + 2);
    fixture.save(&entries, Some("Original partial scan"));
    let original = fs::read(&fixture.cache).expect("read original stale catalog");
    let mut reader = open(&fixture);
    for entry in &entries[..PAGE_ENTRIES / 2] {
        fs::write(&entry.path, b"changed source").expect("change generated source");
    }
    for entry in &entries[PAGE_ENTRIES / 2..=PAGE_ENTRIES] {
        fs::remove_file(&entry.path).expect("remove generated stale source");
    }
    assert_eq!(
        reader
            .next_page(&Cancellation::default())
            .expect("read fully stale page"),
        Some(Vec::new())
    );
    let notice = reader
        .notice()
        .expect("stale page adds a notice")
        .to_owned();
    assert!(notice.contains("Original partial scan"));
    assert!(notice.contains("Cached sources changed"));
    assert_eq!(
        reader
            .next_page(&Cancellation::default())
            .expect("read mixed fresh and stale tail")
            .as_deref(),
        Some(&entries[PAGE_ENTRIES + 1..])
    );
    assert_eq!(reader.notice(), Some(notice.as_str()));
    assert_eq!(
        reader
            .next_page(&Cancellation::default())
            .expect("finish stale catalog"),
        None
    );
    drop(reader);
    assert_preserved(&fixture, &original);
}

#[test]
fn cancellation_after_a_page_latches_until_reopen_and_cleans_private_snapshot() {
    let fixture = Fixture::new();
    let entries = entries(&fixture, PAGE_ENTRIES + 1);
    fixture.save(&entries, None);
    let original = fs::read(&fixture.cache).expect("read original cancellation catalog");
    let cancellation = Cancellation::default();
    let mut reader = open(&fixture);
    assert_eq!(
        reader
            .next_page(&cancellation)
            .expect("read page before cancellation")
            .as_deref(),
        Some(&entries[..PAGE_ENTRIES])
    );
    cancellation.cancel();
    assert!(
        reader
            .next_page(&cancellation)
            .expect_err("cancelled next page must fail")
            .contains("cancelled")
    );
    for _ in 0..2 {
        assert!(reader.next_page(&Cancellation::default()).is_err());
    }
    drop(reader);
    assert_preserved(&fixture, &original);
    let mut reopened = open(&fixture);
    for chunk in entries.chunks(PAGE_ENTRIES) {
        assert_eq!(
            reopened
                .next_page(&Cancellation::default())
                .expect("read reopened page after cancellation")
                .as_deref(),
            Some(chunk)
        );
    }
    assert_eq!(
        reopened
            .next_page(&Cancellation::default())
            .expect("finish reopened cancellation catalog"),
        None
    );
    drop(reopened);
    assert_preserved(&fixture, &original);
}

#[test]
fn public_catalog_replacement_keeps_open_and_new_readers_on_separate_snapshots() {
    let fixture = Fixture::new();
    let entries = entries(&fixture, PAGE_ENTRIES + 1);
    fixture.save(&entries, Some("Old snapshot"));
    let original = fs::read(&fixture.cache).expect("read original public catalog");
    let mut old = open(&fixture);
    assert_eq!(
        old.next_page(&Cancellation::default())
            .expect("read old first page")
            .as_deref(),
        Some(&entries[..PAGE_ENTRIES])
    );
    assert_eq!(
        fs::read(&fixture.cache).expect("read public catalog before replacement"),
        original
    );
    let replacement = vec![entries[PAGE_ENTRIES].clone(), entries[0].clone()];
    fixture.save(&replacement, Some("New snapshot"));
    let replaced = fs::read(&fixture.cache).expect("read replaced public catalog");
    assert_ne!(replaced, original);
    let mut new = open(&fixture);
    assert_eq!(old.notice(), Some("Old snapshot"));
    assert_eq!(new.notice(), Some("New snapshot"));
    assert_eq!(
        new.next_page(&Cancellation::default())
            .expect("read replacement snapshot")
            .as_deref(),
        Some(replacement.as_slice())
    );
    assert_eq!(
        old.next_page(&Cancellation::default())
            .expect("read old tail after public replacement")
            .as_deref(),
        Some(&entries[PAGE_ENTRIES..])
    );
    assert_eq!(
        old.next_page(&Cancellation::default())
            .expect("finish old snapshot"),
        None
    );
    assert_eq!(
        new.next_page(&Cancellation::default())
            .expect("finish replacement snapshot"),
        None
    );
    assert_eq!(old.notice(), Some("Old snapshot"));
    assert_eq!(new.notice(), Some("New snapshot"));
    drop(old);
    drop(new);
    assert_preserved(&fixture, &replaced);
}

#[test]
fn malformed_tail_fails_after_valid_page_and_latches_without_returning_partial_tail() {
    let fixture = Fixture::new();
    let entries = entries(&fixture, PAGE_ENTRIES + 2);
    fixture.save(&entries, None);
    let connection = fixture.database();
    assert_eq!(
        connection
            .execute(
                "UPDATE entries SET identity=x'00' WHERE sequence=?1",
                [(PAGE_ENTRIES + 1) as i64],
            )
            .expect("corrupt final row identity"),
        1
    );
    drop(connection);
    let original = fs::read(&fixture.cache).expect("read malformed catalog bytes");
    for _ in 0..2 {
        let mut reader = open(&fixture);
        assert_eq!(
            reader
                .next_page(&Cancellation::default())
                .expect("read valid page before malformed tail")
                .as_deref(),
            Some(&entries[..PAGE_ENTRIES])
        );
        assert!(
            reader
                .next_page(&Cancellation::default())
                .expect_err("malformed tail must fail the whole page")
                .contains("identity")
        );
        for _ in 0..2 {
            assert!(reader.next_page(&Cancellation::default()).is_err());
        }
        drop(reader);
        assert_preserved(&fixture, &original);
    }
}

#[test]
fn root_replacement_between_pages_fails_and_restoring_root_requires_reopen() {
    let fixture = Fixture::new();
    let entries = entries(&fixture, PAGE_ENTRIES + 1);
    fixture.save(&entries, None);
    let original = fs::read(&fixture.cache).expect("read original root catalog");
    let mut reader = open(&fixture);
    assert_eq!(
        reader
            .next_page(&Cancellation::default())
            .expect("read page before root replacement")
            .as_deref(),
        Some(&entries[..PAGE_ENTRIES])
    );
    let old_root = fixture.parent.join("OldLibrary");
    fs::rename(&fixture.root, &old_root).expect("move original fixture root aside");
    fs::create_dir(&fixture.root).expect("create replacement fixture root");
    assert!(
        reader
            .next_page(&Cancellation::default())
            .expect_err("replacement root must invalidate next page")
            .contains("root")
    );
    assert!(
        PagedReader::open(
            &fixture.cache,
            &fixture.root,
            MAX_ENTRIES,
            MAX_INDEX_BYTES,
            &Cancellation::default(),
        )
        .is_err()
    );
    fs::remove_dir(&fixture.root).expect("remove empty replacement root");
    fs::rename(&old_root, &fixture.root).expect("restore original fixture root");
    assert!(reader.next_page(&Cancellation::default()).is_err());
    drop(reader);
    assert_preserved(&fixture, &original);
    let mut reopened = open(&fixture);
    for chunk in entries.chunks(PAGE_ENTRIES) {
        assert_eq!(
            reopened
                .next_page(&Cancellation::default())
                .expect("read reopened catalog after root restoration")
                .as_deref(),
            Some(chunk)
        );
    }
    assert_eq!(
        reopened
            .next_page(&Cancellation::default())
            .expect("finish restored root catalog"),
        None
    );
    drop(reopened);
    assert_preserved(&fixture, &original);
}

#[test]
fn aggregate_memory_budget_can_fail_on_later_page_and_exact_budget_can_reopen() {
    let fixture = Fixture::new();
    let entries = entries(&fixture, PAGE_ENTRIES + 2);
    fixture.save(&entries, None);
    let original = fs::read(&fixture.cache).expect("read original budget catalog");
    // Charge decoded path capacities, which may differ from fixture construction.
    let entries = fixture
        .load()
        .expect("measure decoded catalog allocations")
        .0;
    let total: usize = entries.iter().map(entry_bytes).sum();
    let byte_limit = total - 1;
    let first_page_cost = entries.len() * size_of::<Entry>()
        + entries[..PAGE_ENTRIES]
            .iter()
            .map(|entry| entry_bytes(entry) - size_of::<Entry>())
            .sum::<usize>();
    assert!(first_page_cost <= byte_limit);
    for chunk in entries.chunks(PAGE_ENTRIES) {
        assert!(chunk.iter().map(entry_bytes).sum::<usize>() <= byte_limit);
    }
    let mut reader = PagedReader::open(
        &fixture.cache,
        &fixture.root,
        entries.len(),
        byte_limit,
        &Cancellation::default(),
    )
    .expect("structural allocation fits the limited budget");
    assert_eq!(
        reader
            .next_page(&Cancellation::default())
            .expect("first page fits the aggregate budget")
            .as_deref(),
        Some(&entries[..PAGE_ENTRIES])
    );
    assert!(
        reader
            .next_page(&Cancellation::default())
            .expect_err("later page exceeds the aggregate memory budget")
            .contains("memory budget")
    );
    assert!(reader.next_page(&Cancellation::default()).is_err());
    drop(reader);
    assert_preserved(&fixture, &original);
    let mut reopened = PagedReader::open(
        &fixture.cache,
        &fixture.root,
        entries.len(),
        total,
        &Cancellation::default(),
    )
    .expect("reopen with exact aggregate budget");
    for chunk in entries.chunks(PAGE_ENTRIES) {
        assert_eq!(
            reopened
                .next_page(&Cancellation::default())
                .expect("read page within exact aggregate budget")
                .as_deref(),
            Some(chunk)
        );
    }
    assert_eq!(
        reopened
            .next_page(&Cancellation::default())
            .expect("finish catalog within exact budget"),
        None
    );
    drop(reopened);
    assert_preserved(&fixture, &original);
}

#[test]
fn version_one_accepts_negative_sparse_sequence_values_across_page_boundary() {
    let fixture = Fixture::new();
    let entries = entries(&fixture, PAGE_ENTRIES + 2);
    fixture.save(&entries, Some("Version one"));
    let connection = fixture.database();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("read generated catalog version"),
        1
    );
    // Move all keys outside the original range before assigning sparse keys.
    connection
        .execute("UPDATE entries SET sequence=sequence+10000", [])
        .expect("move sequence keys away from target range");
    connection
        .execute("UPDATE entries SET sequence=(sequence-10000)*7-1800", [])
        .expect("assign negative and noncontiguous sequence keys");
    assert_eq!(
        connection
            .execute(
                "UPDATE entries SET sequence=?1 WHERE sequence=-1800",
                [i64::MIN],
            )
            .expect("include minimum signed sequence key"),
        1
    );
    assert_eq!(
        connection
            .execute(
                "UPDATE entries SET sequence=?1 WHERE sequence=-1",
                [i64::MAX],
            )
            .expect("include maximum signed sequence key"),
        1
    );
    drop(connection);
    let original = fs::read(&fixture.cache).expect("read version one catalog bytes");
    assert_eq!(
        fixture.load().expect("load compatible version one catalog"),
        (entries.clone(), Some("Version one".into()))
    );
    let mut reader = open(&fixture);
    for chunk in entries.chunks(PAGE_ENTRIES) {
        assert_eq!(
            reader
                .next_page(&Cancellation::default())
                .expect("read sparse signed sequence page")
                .as_deref(),
            Some(chunk)
        );
    }
    assert_eq!(
        reader
            .next_page(&Cancellation::default())
            .expect("finish sparse signed sequence catalog"),
        None
    );
    assert_eq!(reader.notice(), Some("Version one"));
    drop(reader);
    assert_preserved(&fixture, &original);
}
