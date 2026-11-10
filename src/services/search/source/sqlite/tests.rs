use std::{
    cell::Cell,
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::DirBuilderExt},
    sync::atomic::{AtomicUsize, Ordering},
};

use super::*;
use crate::services::music_library::FileIdentity;

struct Fixture {
    parent: PathBuf,
    root: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let parent = std::env::temp_dir().join(format!(
            "hermes-normalized-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&parent)
            .expect("create fixture");
        let root = parent.join("Library");
        fs::create_dir(&root).expect("create root");
        Self {
            path: parent.join("cache/source.sqlite"),
            parent,
            root,
        }
    }

    fn entry(&self, name: impl AsRef<Path>) -> Entry {
        Entry {
            path: self.root.join(name),
            is_directory: false,
            modified_unix_seconds: None,
            identity: None,
            music_tags: None,
        }
    }

    fn build(&self, entries: &[Entry]) -> Store {
        self.counted(entries, &Cell::new(0))
    }

    fn counted(&self, entries: &[Entry], conversions: &Cell<usize>) -> Store {
        Store::build(
            &self.path,
            &self.root,
            entries,
            usize::MAX,
            usize::MAX,
            &Cancellation::default(),
            |entry| {
                conversions.set(conversions.get() + 1);
                convert(entry, &self.root)
            },
        )
        .expect("build normalized source")
    }

    fn limited(&self, entries: &[Entry], entry_limit: usize, byte_limit: usize) -> Store {
        Store::build(
            &self.path,
            &self.root,
            entries,
            entry_limit,
            byte_limit,
            &Cancellation::default(),
            |entry| convert(entry, &self.root),
        )
        .expect("build limited source")
    }

    fn edit_public(&self) -> Connection {
        Connection::open_with_flags(
            &self.path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .expect("open fixture cache for deliberate corruption")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.parent);
    }
}

fn convert(entry: &Entry, root: &Path) -> SearchItem {
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
                .flatten()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        })
        .unwrap_or_default();
    item
}

fn all(store: &mut Store) -> Vec<(usize, SearchItem)> {
    let mut page = Vec::new();
    let mut after = None;
    let mut result = Vec::new();
    loop {
        let used = store
            .read_page(after, &mut page, &|| false)
            .expect("read source page");
        if used == 0 {
            break;
        }
        result.extend_from_slice(&page[..used]);
        after = Some(page[used - 1].0);
    }
    result
}

// Deliberately mutate the private fixture's file to exercise mid-stream row
// failure. Production owns this reader's stage exclusively and never writes it.
fn edit_reader(store: &Store) -> Connection {
    Connection::open_with_flags(
        store
            .database
            .connection()
            .path()
            .expect("reader stage path"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .expect("open private fixture reader for fault injection")
}

#[test]
fn round_trips_unicode_native_paths_and_every_raw_field() {
    let fixture = Fixture::new();
    let mut tagged = fixture.entry("ΟΣ/İSTANBUL.FLAC");
    tagged.modified_unix_seconds = Some(-123);
    tagged.identity = Some(FileIdentity::from_metadata(
        &fs::metadata(&fixture.root).expect("root metadata"),
    ));
    tagged.music_tags = Some(MusicTags {
        title: Some("ΟΣ".into()),
        artist: Some(String::new()),
        album_artist: Some("İSTANBUL".into()),
        album: Some("東京".into()),
        disc: Some(u32::MAX),
        track: Some(17),
    });
    let mut directory = fixture.entry("nested");
    directory.is_directory = true;
    directory.music_tags = Some(MusicTags::default());
    let entries = vec![
        tagged,
        fixture.entry(OsString::from_vec(b"NATIVE-\xff.TXT".to_vec())),
        directory,
    ];
    let expected: Vec<_> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (index, convert(entry, &fixture.root)))
        .collect();
    let mut store = fixture.build(&entries);
    assert_eq!(store.end(), 3);
    assert_eq!(store.count(), 3);
    assert_eq!(all(&mut store), expected);
    for (position, item) in &expected {
        assert_eq!(store.get(*position).expect("read row"), Some(item.clone()));
    }
    assert_eq!(expected[0].1.search_name, "i\u{307}stanbul.flac");
    assert_eq!(expected[0].1.search_metadata, "ος  i\u{307}stanbul 東京");
    assert_eq!(store.get(3).expect("past bound"), None);
    assert_eq!(store.get(usize::MAX).expect("past bound"), None);
    let disk_identity: Vec<u8> = store
        .database
        .connection()
        .query_row(
            "SELECT identity FROM records WHERE path = ?1",
            [entries[0].path.as_os_str().as_bytes()],
            |row| row.get(0),
        )
        .expect("stored binary identity");
    assert_eq!(disk_identity.len(), 56);
    assert_eq!(
        decode_identity(&disk_identity).expect("decode stored identity"),
        entries[0].identity.clone().expect("source identity")
    );
    let stored_signature: Vec<u8> = store
        .database
        .connection()
        .query_row(
            "SELECT signature FROM records WHERE path = ?1",
            [entries[1].path.as_os_str().as_bytes()],
            |row| row.get(0),
        )
        .expect("stored signature");
    assert_eq!(
        stored_signature,
        serde_json::to_vec(&(false, None::<i64>, None::<FileIdentity>, None::<MusicTags>))
            .expect("raw signature")
    );
    drop(store);
    let conversions = Cell::new(0);
    assert_eq!(all(&mut fixture.counted(&entries, &conversions)), expected);
    assert_eq!(conversions.get(), 0);
}

#[test]
fn native_root_bytes_and_component_aliases_preserve_source_scope() {
    let mut fixture = Fixture::new();
    fixture.root = fixture
        .parent
        .join(OsString::from_vec(b"Library-\xff".to_vec()));
    fs::create_dir(&fixture.root).expect("create native root");
    let entries: Vec<_> = ["", ".", "./A.TXT", "nested//B.TXT", "nested/../C.TXT"]
        .into_iter()
        .map(|name| fixture.entry(name))
        .collect();
    let mut store = fixture.build(&entries);
    assert_eq!(store.end(), 5);
    assert_eq!(
        all(&mut store),
        entries
            .iter()
            .enumerate()
            .skip(2)
            .map(|(index, entry)| (index, convert(entry, &fixture.root)))
            .collect::<Vec<_>>()
    );
    assert_eq!(store.get(0).expect("root hole"), None);
    assert_eq!(store.get(1).expect("root alias hole"), None);
}

#[test]
fn pages_cross_257_boundary_with_sparse_original_positions_and_reused_slots() {
    let fixture = Fixture::new();
    let entries: Vec<_> = (0..1029)
        .map(|position| {
            if position % 2 == 0 {
                fixture.entry("")
            } else {
                fixture.entry(format!("FILE-{position:04}.TXT"))
            }
        })
        .collect();
    let mut store = fixture.build(&entries);
    let mut page = Vec::new();
    assert_eq!(store.end(), 1029);
    assert_eq!(store.count(), 514);
    assert_eq!(
        store
            .read_page(None, &mut page, &|| false)
            .expect("first page"),
        256
    );
    assert_eq!((page[0].0, page[255].0), (1, 511));
    let buffers = (
        page.as_ptr(),
        page[0].1.path.as_os_str().as_bytes().as_ptr(),
        page[0].1.name.as_ptr(),
        page[0].1.search_name.as_ptr(),
        page[0].1.search_path.as_ptr(),
    );
    assert_eq!(
        store
            .read_page(Some(511), &mut page, &|| false)
            .expect("second page"),
        256
    );
    assert_eq!((page[0].0, page[255].0), (513, 1023));
    assert_eq!(
        buffers,
        (
            page.as_ptr(),
            page[0].1.path.as_os_str().as_bytes().as_ptr(),
            page[0].1.name.as_ptr(),
            page[0].1.search_name.as_ptr(),
            page[0].1.search_path.as_ptr(),
        )
    );
    assert_eq!(
        store
            .read_page(Some(1023), &mut page, &|| false)
            .expect("last page"),
        2
    );
    assert_eq!((page[0].0, page[1].0), (1025, 1027));
    assert_eq!(page.len(), 256);
    assert_eq!(
        store
            .read_page(Some(1027), &mut page, &|| false)
            .expect("EOF"),
        0
    );
    assert_eq!(page.len(), 256);
    assert_eq!(
        store
            .read_page(Some(512), &mut page, &|| false)
            .expect("start after hole"),
        256
    );
    assert_eq!(page[0].0, 513);
    assert_eq!(store.get(512).expect("hole"), None);
    assert_eq!(all(&mut store).len(), 514);
}

#[test]
fn exactly_257_entries_require_two_pages() {
    let fixture = Fixture::new();
    let entries: Vec<_> = (0..257)
        .map(|position| fixture.entry(format!("{position}.flac")))
        .collect();
    let mut store = fixture.build(&entries);
    let mut page = Vec::new();
    assert_eq!(
        store
            .read_page(None, &mut page, &|| false)
            .expect("first page"),
        256
    );
    assert_eq!(
        store
            .read_page(Some(255), &mut page, &|| false)
            .expect("second page"),
        1
    );
    assert_eq!(page[0].0, 256);
    assert_eq!(
        store
            .read_page(Some(256), &mut page, &|| false)
            .expect("EOF"),
        0
    );
}

#[test]
fn reused_page_clears_old_identity_tags_directory_and_unicode_fields() {
    let fixture = Fixture::new();
    let mut entries: Vec<_> = (0..257)
        .map(|position| fixture.entry(format!("FILE-{position:04}.TXT")))
        .collect();
    entries[0].path = fixture.root.join("ΟΣ/İSTANBUL.FLAC");
    entries[0].is_directory = true;
    entries[0].modified_unix_seconds = Some(123);
    entries[0].identity = Some(FileIdentity::from_metadata(
        &fs::metadata(&fixture.root).expect("root metadata"),
    ));
    entries[0].music_tags = Some(MusicTags {
        title: Some("ΟΣ".into()),
        ..MusicTags::default()
    });
    entries[256].path = fixture
        .root
        .join(OsString::from_vec(b"LAST-\xff.TXT".to_vec()));
    let mut store = fixture.build(&entries);
    let mut page = Vec::new();
    assert_eq!(
        store
            .read_page(None, &mut page, &|| false)
            .expect("first page"),
        256
    );
    assert_eq!(page[0].1, convert(&entries[0], &fixture.root));
    assert_eq!(
        store
            .read_page(Some(255), &mut page, &|| false)
            .expect("last page"),
        1
    );
    assert_eq!(page[0].1, convert(&entries[256], &fixture.root));
}

#[test]
fn unchanged_restart_one_change_reorder_and_deletion_only_convert_changed_inputs() {
    let fixture = Fixture::new();
    let mut entries = vec![
        fixture.entry("a.flac"),
        fixture.entry("b.flac"),
        fixture.entry("c.flac"),
    ];
    let conversions = Cell::new(0);
    drop(fixture.counted(&entries, &conversions));
    assert_eq!(conversions.replace(0), 3);
    drop(fixture.counted(&entries, &conversions));
    assert_eq!(conversions.replace(0), 0);
    entries[1].music_tags = Some(MusicTags {
        title: Some("changed".into()),
        ..MusicTags::default()
    });
    drop(fixture.counted(&entries, &conversions));
    assert_eq!(conversions.replace(0), 1);
    entries.swap(0, 2);
    let mut reordered = fixture.counted(&entries, &conversions);
    assert_eq!(conversions.replace(0), 0);
    assert_eq!(
        all(&mut reordered),
        entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (index, convert(entry, &fixture.root)))
            .collect::<Vec<_>>()
    );
    drop(reordered);
    let removed = entries.remove(1);
    let store = fixture.counted(&entries, &conversions);
    assert_eq!(conversions.get(), 0);
    let retained: i64 = store
        .database
        .connection()
        .query_row("SELECT COUNT(*) FROM records", [], |row| row.get(0))
        .expect("retained record count");
    assert_eq!(retained, 2);
    let deleted: bool = store
        .database
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM records WHERE path = ?1)",
            [removed.path.as_os_str().as_bytes()],
            |row| row.get(0),
        )
        .expect("deleted record");
    assert!(!deleted);
}

#[test]
fn each_raw_signature_field_invalidates_only_its_record() {
    let fixture = Fixture::new();
    let mut entries = vec![fixture.entry("changed"), fixture.entry("unchanged")];
    let conversions = Cell::new(0);
    drop(fixture.counted(&entries, &conversions));
    conversions.set(0);
    entries[0].is_directory = true;
    drop(fixture.counted(&entries, &conversions));
    assert_eq!(conversions.replace(0), 1);
    entries[0].modified_unix_seconds = Some(-1);
    drop(fixture.counted(&entries, &conversions));
    assert_eq!(conversions.replace(0), 1);
    entries[0].identity = Some(FileIdentity::from_metadata(
        &fs::metadata(&fixture.root).expect("root metadata"),
    ));
    drop(fixture.counted(&entries, &conversions));
    assert_eq!(conversions.replace(0), 1);
    entries[0].music_tags = Some(MusicTags::default());
    drop(fixture.counted(&entries, &conversions));
    assert_eq!(conversions.replace(0), 1);
    drop(fixture.counted(&entries, &conversions));
    assert_eq!(conversions.get(), 0);
}

#[test]
fn exact_fresh_byte_budget_and_notices_survive_cached_reuse() {
    let fixture = Fixture::new();
    let mut tagged = fixture.entry("İSTANBUL.FLAC");
    tagged.music_tags = Some(MusicTags {
        title: Some("Ⱥ TITLE".into()),
        artist: Some("Artist".into()),
        album_artist: Some("Album Artist".into()),
        album: Some("Album".into()),
        ..MusicTags::default()
    });
    let entries = vec![
        fixture.entry(""),
        tagged,
        fixture.entry("second.flac"),
        fixture.entry(""),
    ];
    let first = super::super::item_bytes(&convert(&entries[1], &fixture.root));
    let second = super::super::item_bytes(&convert(&entries[2], &fixture.root));
    drop(fixture.build(&entries));
    for (budget, end, count) in [
        (first + second, 4, 2),
        (first + second - 1, 2, 1),
        (first - 1, 1, 0),
        (first, 2, 1),
    ] {
        let mut store = fixture.limited(&entries, usize::MAX, budget);
        assert_eq!((store.end(), store.count()), (end, count));
        assert_eq!(all(&mut store).len(), count);
        assert_eq!(store.notice().is_some(), count < 2);
        if count < 2 {
            assert!(
                store
                    .notice()
                    .expect("limit notice")
                    .contains(&format!("{budget} bytes"))
            );
        }
    }
    let mut exact_entries = fixture.limited(&entries, 2, usize::MAX);
    assert_eq!(exact_entries.end(), 4);
    assert_eq!(exact_entries.notice(), None);
    assert_eq!(all(&mut exact_entries).len(), 2);
    let mut capped = fixture.limited(&entries, 1, usize::MAX);
    assert_eq!((capped.end(), capped.count()), (2, 1));
    assert!(capped.notice().expect("entry notice").contains("1 entries"));
    assert_eq!(capped.get(2).expect("omitted row"), None);
}

#[test]
fn empty_holes_zero_budgets_and_duplicate_unchanged_paths_are_honest() {
    let fixture = Fixture::new();
    for entries in [
        Vec::new(),
        vec![
            fixture.entry(""),
            Entry {
                path: fixture.parent.join("outside"),
                ..fixture.entry("unused")
            },
        ],
    ] {
        let mut store = fixture.limited(&entries, 0, 0);
        assert_eq!(store.end(), entries.len());
        assert_eq!(store.count(), 0);
        assert_eq!(store.notice(), None);
        assert!(all(&mut store).is_empty());
    }
    let row = fixture.entry("duplicate.flac");
    let conversions = Cell::new(0);
    let mut store = fixture.counted(&[row.clone(), row], &conversions);
    assert_eq!(conversions.get(), 1);
    assert_eq!(store.count(), 2);
    assert_eq!(
        all(&mut store)
            .iter()
            .map(|(position, _)| *position)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    let mut omitted = fixture.limited(&[fixture.entry("omitted")], 0, 0);
    assert_eq!((omitted.end(), omitted.count()), (0, 0));
    assert!(omitted.notice().is_some());
    assert!(all(&mut omitted).is_empty());
}

#[test]
fn old_reader_is_unchanged_while_another_thread_publishes_reordered_rows() {
    let fixture = Fixture::new();
    let entries = vec![fixture.entry("old-a.flac"), fixture.entry("old-b.flac")];
    let mut old = fixture.build(&entries);
    let path = fixture.path.clone();
    let root = fixture.root.clone();
    let next = vec![entries[1].clone(), fixture.entry("new-c.flac")];
    let expected = next.clone();
    let writer = std::thread::spawn(move || {
        let mut store = Store::build(
            &path,
            &root,
            &next,
            usize::MAX,
            usize::MAX,
            &Cancellation::default(),
            |entry| convert(entry, &root),
        )
        .expect("publish next revision");
        all(&mut store)
    });
    assert_eq!(
        all(&mut old),
        entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (index, convert(entry, &fixture.root)))
            .collect::<Vec<_>>()
    );
    let published = writer.join().expect("publication thread");
    assert_eq!(
        published,
        expected
            .iter()
            .enumerate()
            .map(|(index, entry)| (index, convert(entry, &fixture.root)))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        old.get(0).expect("old row").expect("present").path,
        entries[0].path
    );
    assert_eq!(all(&mut old).len(), 2);
    assert_eq!(all(&mut fixture.build(&expected)), published);
}

#[test]
fn query_cancellation_does_not_latch_and_later_queries_reuse_the_store() {
    let fixture = Fixture::new();
    let entries: Vec<_> = (0..257)
        .map(|position| fixture.entry(format!("{position}.flac")))
        .collect();
    let mut store = fixture.build(&entries);
    let calls = Cell::new(0);
    let mut page = Vec::new();
    assert!(
        store
            .read_page(None, &mut page, &|| {
                calls.set(calls.get() + 1);
                calls.get() > 10
            })
            .is_err()
    );
    assert!(page.is_empty());
    assert_eq!(all(&mut store).len(), 257);
    assert!(store.read_page(None, &mut page, &|| true).is_err());
    assert_eq!(
        store.get(256).expect("lookup after cancellation"),
        Some(convert(&entries[256], &fixture.root))
    );
}

#[test]
fn cancelled_builds_preserve_the_public_database_bytes() {
    let fixture = Fixture::new();
    let entries = vec![fixture.entry("original")];
    drop(fixture.build(&entries));
    let before = fs::read(&fixture.path).expect("read published bytes");
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        Store::build(
            &fixture.path,
            &fixture.root,
            &entries,
            usize::MAX,
            usize::MAX,
            &cancel,
            |entry| convert(entry, &fixture.root)
        )
        .is_err()
    );
    assert_eq!(fs::read(&fixture.path).expect("unchanged bytes"), before);
    let cancel = Cancellation::default();
    assert!(
        Store::build(
            &fixture.path,
            &fixture.root,
            &[fixture.entry("replacement")],
            usize::MAX,
            usize::MAX,
            &cancel,
            |entry| {
                cancel.cancel();
                convert(entry, &fixture.root)
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(&fixture.path).expect("unchanged bytes"), before);
    assert_eq!(all(&mut fixture.build(&entries)).len(), 1);
}

#[test]
fn oversized_and_invalid_rows_fail_without_replacing_public_bytes() {
    let fixture = Fixture::new();
    drop(fixture.build(&[fixture.entry("original")]));
    let before = fs::read(&fixture.path).expect("original bytes");
    let mut oversized = fixture.entry("large-tags");
    oversized.music_tags = Some(MusicTags {
        title: Some("X".repeat(MAX_TAG_BYTES + 1)),
        ..MusicTags::default()
    });
    let conversions = Cell::new(0);
    assert!(
        Store::build(
            &fixture.path,
            &fixture.root,
            &[oversized],
            usize::MAX,
            usize::MAX,
            &Cancellation::default(),
            |entry| {
                conversions.set(conversions.get() + 1);
                convert(entry, &fixture.root)
            }
        )
        .is_err()
    );
    assert_eq!(conversions.get(), 0);
    assert_eq!(fs::read(&fixture.path).expect("unchanged bytes"), before);
    let row = fixture.entry("bad-conversion");
    assert!(
        Store::build(
            &fixture.path,
            &fixture.root,
            &[row],
            usize::MAX,
            usize::MAX,
            &Cancellation::default(),
            |entry| {
                let mut item = convert(entry, &fixture.root);
                item.search_name = "X".repeat(MAX_FIELD_BYTES + 1);
                item
            }
        )
        .is_err()
    );
    assert_eq!(fs::read(&fixture.path).expect("unchanged bytes"), before);
    let nul = fixture.entry(OsString::from_vec(b"invalid\0name".to_vec()));
    assert!(
        Store::build(
            &fixture.path,
            &fixture.root,
            &[nul],
            usize::MAX,
            usize::MAX,
            &Cancellation::default(),
            |entry| convert(entry, &fixture.root)
        )
        .is_err()
    );
    assert_eq!(fs::read(&fixture.path).expect("unchanged bytes"), before);
}

#[test]
fn sql_and_late_short_page_failures_are_explicit() {
    let fixture = Fixture::new();
    let entries: Vec<_> = (0..257)
        .map(|position| fixture.entry(format!("{position}.flac")))
        .collect();
    let mut store = fixture.build(&entries);
    let mut page = Vec::new();
    assert_eq!(
        store
            .read_page(None, &mut page, &|| false)
            .expect("first page"),
        256
    );
    edit_reader(&store)
        .execute("DELETE FROM positions WHERE position = 256", [])
        .expect("remove final row");
    assert!(
        store
            .read_page(Some(255), &mut page, &|| false)
            .expect_err("short committed page must fail")
            .contains("committed count")
    );
    assert!(page.is_empty());
    store
        .database
        .connection()
        .set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 1)
        .expect("inject SQL failure");
    assert!(store.get(0).is_err());
    assert!(store.read_page(None, &mut page, &|| false).is_err());
}

#[test]
fn missing_join_targets_and_malformed_rows_never_become_partial_success() {
    let fixture = Fixture::new();
    let entries = vec![
        fixture.entry("a.flac"),
        fixture.entry("b.flac"),
        fixture.entry("c.flac"),
    ];
    let mut store = fixture.build(&entries);
    let writer = edit_reader(&store);
    writer
        .execute(
            "DELETE FROM records WHERE path = ?1",
            [entries[1].path.as_os_str().as_bytes()],
        )
        .expect("remove target");
    assert!(store.get(1).is_err());
    assert!(store.read_page(None, &mut Vec::new(), &|| false).is_err());
    drop(writer);
    drop(store);
    let mut store = fixture.build(&entries);
    edit_reader(&store)
        .execute(
            "UPDATE records SET name = CAST(X'FF' AS TEXT) WHERE path = ?1",
            [entries[0].path.as_os_str().as_bytes()],
        )
        .expect("corrupt UTF-8");
    assert!(store.get(0).is_err());
    assert!(store.read_page(None, &mut Vec::new(), &|| false).is_err());
}

#[test]
fn root_replacement_fails_every_lookup_and_page_even_past_eof() {
    let fixture = Fixture::new();
    let mut store = fixture.build(&[fixture.entry("row")]);
    fs::rename(&fixture.root, fixture.parent.join("old-root")).expect("move original root");
    fs::create_dir(&fixture.root).expect("replace root");
    assert!(store.get(0).is_err());
    assert!(store.get(usize::MAX).is_err());
    assert!(store.read_page(None, &mut Vec::new(), &|| false).is_err());
    assert!(
        store
            .read_page(Some(usize::MAX), &mut Vec::new(), &|| false)
            .is_err()
    );
}

#[test]
fn newer_schema_returns_an_error_and_preserves_public_bytes() {
    let fixture = Fixture::new();
    let entries = [fixture.entry("original")];
    drop(fixture.build(&entries));
    {
        let connection = fixture.edit_public();
        connection
            .pragma_update(None, "user_version", VERSION + 1)
            .expect("set future version");
        connection
            .execute_batch("CREATE TABLE future(payload BLOB) STRICT")
            .expect("add future schema");
    }
    let before = fs::read(&fixture.path).expect("future bytes");
    let conversions = Cell::new(0);
    let failure = Store::build(
        &fixture.path,
        &fixture.root,
        &entries,
        usize::MAX,
        usize::MAX,
        &Cancellation::default(),
        |entry| {
            conversions.set(conversions.get() + 1);
            convert(entry, &fixture.root)
        },
    )
    .err()
    .expect("reject future version");
    assert!(failure.contains("newer schema"));
    assert_eq!(conversions.get(), 0);
    assert_eq!(
        fs::read(&fixture.path).expect("preserved future bytes"),
        before
    );
}

#[test]
fn corrupt_or_old_schema_and_bad_metadata_rebuild_only_the_private_stage() {
    let fixture = Fixture::new();
    let entries = [fixture.entry("original")];
    for corruption in [
        "PRAGMA user_version=0",
        "CREATE TABLE unexpected(payload BLOB) STRICT",
        "UPDATE metadata SET source_end=0",
        "UPDATE metadata SET eligible=0",
        "UPDATE metadata SET item_bytes=item_bytes+1",
        "UPDATE metadata SET root=X'FF'",
        "UPDATE metadata SET root_key=zeroblob(16)",
    ] {
        drop(fixture.build(&entries));
        fixture
            .edit_public()
            .execute_batch(corruption)
            .expect("corrupt old fixture");
        let before = fs::read(&fixture.path).expect("corrupt bytes");
        let cancel = Cancellation::default();
        assert!(
            Store::build(
                &fixture.path,
                &fixture.root,
                &entries,
                usize::MAX,
                usize::MAX,
                &cancel,
                |entry| {
                    cancel.cancel();
                    convert(entry, &fixture.root)
                }
            )
            .is_err()
        );
        assert_eq!(
            fs::read(&fixture.path).expect("old bytes preserved"),
            before
        );
        let conversions = Cell::new(0);
        let mut store = fixture.counted(&entries, &conversions);
        assert_eq!(conversions.get(), 1);
        assert_eq!(all(&mut store).len(), 1);
    }
    drop(fixture.build(&entries));
    fs::write(&fixture.path, b"corrupt sqlite header").expect("corrupt header");
    assert_eq!(all(&mut fixture.build(&entries)).len(), 1);
}

#[test]
fn cached_cost_must_be_nonnegative_bounded_and_consistent_with_metadata() {
    let fixture = Fixture::new();
    let entries = [fixture.entry("original")];
    drop(fixture.build(&entries));
    fixture.edit_public().execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE records SET cost=-1; UPDATE metadata SET item_bytes=-1").expect("corrupt signed costs");
    let conversions = Cell::new(0);
    drop(fixture.counted(&entries, &conversions));
    assert_eq!(conversions.get(), 1);
    fixture
        .edit_public()
        .execute_batch(
            "UPDATE records SET cost=1000000000; UPDATE metadata SET item_bytes=1000000000",
        )
        .expect("corrupt huge cost");
    conversions.set(0);
    drop(fixture.counted(&entries, &conversions));
    assert_eq!(conversions.get(), 1);
    let unchanged = Cell::new(0);
    drop(fixture.counted(&entries, &unchanged));
    assert_eq!(unchanged.get(), 0);
}

#[test]
fn position_reads_use_primary_key_ranges_and_pruning_has_a_path_index() {
    let fixture = Fixture::new();
    let store = fixture.build(&[fixture.entry("row")]);
    let connection = store.database.connection();
    let mut statement = connection.prepare(
        "EXPLAIN QUERY PLAN SELECT p.position, p.ordinal, r.* FROM positions p JOIN records r ON r.path = p.path WHERE p.position > ?1 ORDER BY p.position LIMIT 256",
    ).expect("query plan");
    let plans: Vec<String> = statement
        .query_map([0], |row| row.get(3))
        .expect("plan rows")
        .collect::<rusqlite::Result<_>>()
        .expect("read plan");
    assert!(
        plans
            .iter()
            .any(|plan| plan.contains("SEARCH p USING INTEGER PRIMARY KEY")),
        "{plans:?}"
    );
    assert!(
        !plans.iter().any(|plan| plan.contains("TEMP B-TREE")),
        "{plans:?}"
    );
    let columns: Vec<String> = connection
        .prepare("PRAGMA index_info(positions_path)")
        .expect("path index")
        .query_map([], |row| row.get(2))
        .expect("index columns")
        .collect::<rusqlite::Result<_>>()
        .expect("read index");
    assert_eq!(columns, ["path"]);
    assert_eq!(
        connection
            .query_row("PRAGMA cache_size", [], |row| row.get::<_, i64>(0))
            .expect("cache cap"),
        -2048
    );
    assert_eq!(
        connection
            .query_row("PRAGMA mmap_size", [], |row| row.get::<_, i64>(0))
            .expect("mmap disabled"),
        0
    );
    assert_eq!(
        connection
            .query_row("PRAGMA temp_store", [], |row| row.get::<_, i64>(0))
            .expect("file-backed temp"),
        1
    );
    assert_eq!(
        connection
            .limit(Limit::SQLITE_LIMIT_LENGTH)
            .expect("row limit"),
        MAX_ROW_BYTES as i32
    );
    assert_eq!(
        connection
            .limit(Limit::SQLITE_LIMIT_COLUMN)
            .expect("column limit"),
        32
    );
}

#[test]
fn corrupt_cached_rows_are_repaired_once_without_normalizing_unchanged_peers() {
    let fixture = Fixture::new();
    let entries = [fixture.entry("repair"), fixture.entry("unchanged")];
    for corruption in [
        "UPDATE records SET identity=X'FF' WHERE name='repair'",
        "UPDATE records SET name=CAST(X'FF' AS TEXT) WHERE name='repair'",
        "UPDATE records SET search_name=replace(hex(zeroblob(9000)), '0', 'X') WHERE name='repair'",
        "UPDATE records SET title='unexpected' WHERE name='repair'",
        "UPDATE records SET directory=1 WHERE name='repair'",
        "UPDATE records SET modified=123 WHERE name='repair'",
        "UPDATE records SET tagged=1, title='changed' WHERE name='repair'",
    ] {
        drop(fixture.build(&entries));
        fixture
            .edit_public()
            .execute_batch(corruption)
            .expect("corrupt cached row");
        let conversions = Cell::new(0);
        let mut repaired = fixture.counted(&entries, &conversions);
        assert_eq!(conversions.get(), 1);
        assert_eq!(
            all(&mut repaired),
            entries
                .iter()
                .enumerate()
                .map(|(index, entry)| (index, convert(entry, &fixture.root)))
                .collect::<Vec<_>>()
        );
        drop(repaired);
        conversions.set(0);
        drop(fixture.counted(&entries, &conversions));
        assert_eq!(conversions.get(), 0);
    }
}

#[test]
fn contradictory_duplicate_paths_fail_atomically_but_omitted_duplicates_only_warn() {
    let fixture = Fixture::new();
    let original = fixture.entry("duplicate");
    let mut conflicting = original.clone();
    conflicting.modified_unix_seconds = Some(123);
    drop(fixture.build(std::slice::from_ref(&original)));
    let before = fs::read(&fixture.path).expect("published bytes");
    let entries = [original, conflicting];
    let failure = Store::build(
        &fixture.path,
        &fixture.root,
        &entries,
        usize::MAX,
        usize::MAX,
        &Cancellation::default(),
        |entry| convert(entry, &fixture.root),
    )
    .err()
    .expect("reject ambiguous records");
    assert!(failure.contains("Conflicting"));
    assert_eq!(fs::read(&fixture.path).expect("preserved bytes"), before);
    let store = fixture.limited(&entries, 1, usize::MAX);
    assert_eq!((store.end(), store.count()), (1, 1));
    assert!(store.notice().is_some());
}

#[test]
fn root_change_during_build_cannot_publish_over_the_previous_database() {
    let fixture = Fixture::new();
    drop(fixture.build(&[fixture.entry("original")]));
    let before = fs::read(&fixture.path).expect("published bytes");
    let entries = [fixture.entry("replacement")];
    assert!(
        Store::build(
            &fixture.path,
            &fixture.root,
            &entries,
            usize::MAX,
            usize::MAX,
            &Cancellation::default(),
            |entry| {
                fs::rename(&fixture.root, fixture.parent.join("old-root"))
                    .expect("move root during conversion");
                fs::create_dir(&fixture.root).expect("replace root during conversion");
                convert(entry, &fixture.root)
            },
        )
        .is_err()
    );
    assert_eq!(fs::read(&fixture.path).expect("preserved bytes"), before);
}
