use std::{
    cell::Cell,
    fs,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::*;
use crate::{model::MusicTags, services::music_library::FileIdentity};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "hermes-search-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("create source fixture");
        Self(root)
    }

    fn source(&self, entries: Arc<Vec<Entry>>, entry_limit: usize, byte_limit: usize) -> Source {
        Source::new(self.0.clone(), entries, entry_limit, byte_limit, &|| false)
            .expect("create source")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove source fixture");
    }
}

fn entry(path: PathBuf) -> Entry {
    Entry {
        path,
        is_directory: false,
        modified_unix_seconds: None,
        identity: None,
        music_tags: None,
    }
}

fn positions(source: &Source) -> Vec<usize> {
    let mut positions = Vec::new();
    source
        .for_each_page(&|| false, |page| {
            positions.extend(page.iter().map(|(position, _)| *position));
            Ok(())
        })
        .expect("read source pages");
    positions
}

#[test]
fn retains_the_original_arc_and_clones_only_source_ownership() {
    let fixture = Fixture::new();
    let entries = Arc::new(vec![entry(fixture.0.join("track.flac"))]);
    let original_row = &entries[0] as *const Entry;
    let source = fixture.source(entries.clone(), usize::MAX, usize::MAX);
    let reused = source.clone();
    for retained in [&source, &reused] {
        let Backing::Library(backing) = &retained.backing else {
            panic!("expected retained library snapshot")
        };
        assert!(Arc::ptr_eq(&entries, backing));
        assert_eq!(&backing[0] as *const Entry, original_row);
    }
    assert_eq!(Arc::strong_count(&entries), 3);
    drop(source);
    assert_eq!(positions(&reused), vec![0]);
    assert_eq!(Arc::strong_count(&entries), 2);
}

#[test]
fn installed_memory_source_rejects_a_replaced_root_without_a_new_snapshot() {
    let fixture = Fixture::new();
    let root = fixture.0.join("Selected");
    fs::create_dir(&root).expect("selected directory");
    let source = Source::new(
        root.clone(),
        Arc::new(vec![entry(root.join("file.txt"))]),
        usize::MAX,
        usize::MAX,
        &|| false,
    )
    .expect("memory source");
    assert!(source.get(0).expect("original lookup").is_some());
    fs::rename(&root, fixture.0.join("Previous")).expect("move original root");
    fs::create_dir(&root).expect("replacement directory");
    assert!(source.get(0).is_err());
    assert!(source.for_each_page(&|| false, |_| Ok(())).is_err());
}

#[test]
fn composed_component_binding_rejects_related_scope_and_preserves_unrelated_scope() {
    let fixture = Fixture::new();
    let music = fixture.0.join("Music");
    let pictures = fixture.0.join("Pictures");
    fs::create_dir(&music).expect("music directory");
    fs::create_dir(&pictures).expect("pictures directory");
    let binding = RootBinding {
        path: music.clone(),
        identity: FileIdentity::directory(&music).expect("music identity"),
    };
    let home = fixture
        .source(
            Arc::new(vec![entry(music.join("song.flac"))]),
            usize::MAX,
            usize::MAX,
        )
        .bind_roots(vec![binding.clone()]);
    let picture_source = Source::new(
        pictures.clone(),
        Arc::new(vec![entry(pictures.join("photo.jpg"))]),
        usize::MAX,
        usize::MAX,
        &|| false,
    )
    .expect("unrelated source")
    .bind_roots(vec![binding]);
    fs::rename(&music, fixture.0.join("PreviousMusic")).expect("replace component");
    fs::create_dir(&music).expect("new component");
    assert!(home.get(0).is_err());
    assert!(
        picture_source
            .get(0)
            .expect("unrelated binding filtered")
            .is_some()
    );
}

#[test]
fn scope_holes_keep_global_positions_and_the_raw_upper_bound() {
    let fixture = Fixture::new();
    let entries = Arc::new(vec![
        entry(fixture.0.clone()),
        entry(fixture.0.join("z.flac")),
        entry(fixture.0.with_extension("outside").join("not-scoped.flac")),
        entry(fixture.0.join("nested/a.flac")),
        entry(fixture.0.parent().expect("fixture parent").join("outside")),
    ]);
    let source = fixture.source(entries, 2, usize::MAX);
    assert_eq!(source.len(), 5);
    assert_eq!(positions(&source), vec![1, 3]);
    assert_eq!(source.notice(), None);
    for position in [0, 2, 4, 5, usize::MAX] {
        assert_eq!(source.get(position).expect("lookup"), None);
    }
    assert_eq!(
        source.get(3).expect("lookup").expect("scoped row").path,
        fixture.0.join("nested/a.flac")
    );
}

#[test]
fn pages_hold_256_eligible_rows_across_scope_holes_and_reuse_the_buffer() {
    let fixture = Fixture::new();
    let entries = Arc::new(
        (0..1201)
            .map(|position| {
                if position % 2 == 0 {
                    entry(fixture.0.clone())
                } else {
                    entry(fixture.0.join(format!("track-{position}.flac")))
                }
            })
            .collect(),
    );
    let source = fixture.source(entries, usize::MAX, usize::MAX);
    let mut sizes = Vec::new();
    let mut starts = Vec::new();
    let mut buffers = Vec::new();
    source
        .for_each_page(&|| false, |page| {
            sizes.push(page.len());
            starts.push(page[0].0);
            buffers.push(page.as_ptr());
            for (position, item) in page {
                assert_eq!(item.path, fixture.0.join(format!("track-{position}.flac")));
                assert_eq!(position % 2, 1);
            }
            Ok(())
        })
        .expect("bounded page stream");
    assert_eq!(source.len(), 1201);
    assert_eq!(sizes, vec![256, 256, 88]);
    assert_eq!(starts, vec![1, 513, 1025]);
    assert!(buffers.iter().all(|buffer| *buffer == buffers[0]));
}

#[test]
fn ascii_page_slots_reuse_path_and_normalized_string_allocations() {
    let fixture = Fixture::new();
    let source = fixture.source(
        Arc::new(
            (0..512)
                .map(|number| entry(fixture.0.join(format!("FILE-{number:04}.TXT"))))
                .collect(),
        ),
        usize::MAX,
        usize::MAX,
    );
    let mut buffers = Vec::new();
    source
        .for_each_page(&|| false, |page| {
            let item = &page[0].1;
            buffers.push((
                item.path.as_os_str().as_encoded_bytes().as_ptr(),
                item.name.as_ptr(),
                item.search_name.as_ptr(),
                item.search_path.as_ptr(),
            ));
            for (position, item) in page {
                assert_eq!(
                    item,
                    &source.get(*position)?.expect("independent source read")
                );
            }
            Ok(())
        })
        .expect("reused ASCII pages");
    assert_eq!(buffers.len(), 2);
    assert_eq!(buffers[0], buffers[1]);
}

#[test]
fn reused_rows_clear_old_tags_identity_and_preserve_contextual_unicode() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let fixture = Fixture::new();
    let mut entries = Vec::new();
    for number in 0..513 {
        let mut row = entry(fixture.0.join(format!("FILE-{number:04}.TXT")));
        if number == 0 {
            row.path = fixture.0.join("ΟΣ/İSTANBUL.FLAC");
            row.is_directory = true;
            row.modified_unix_seconds = Some(123);
            row.identity = Some(FileIdentity::from_metadata(
                &fs::metadata(&fixture.0).expect("root metadata"),
            ));
            row.music_tags = Some(MusicTags {
                title: Some("ΟΣ".into()),
                artist: Some(String::new()),
                album_artist: Some("İSTANBUL".into()),
                ..MusicTags::default()
            });
        }
        if number == 512 {
            row.path = fixture
                .0
                .join(OsString::from_vec(b"STRANGE-\xff.TXT".to_vec()));
        }
        entries.push(row);
        entries.push(entry(fixture.0.with_extension("outside").join("excluded")));
    }
    let source = fixture.source(Arc::new(entries), usize::MAX, usize::MAX);
    let mut sizes = Vec::new();
    source
        .for_each_page(&|| false, |page| {
            sizes.push(page.len());
            for (position, item) in page {
                assert_eq!(
                    item,
                    &source.get(*position)?.expect("independent source read")
                );
            }
            if sizes.len() == 1 {
                assert_eq!(page[0].1.search_metadata, "ΟΣ  İSTANBUL".to_lowercase());
                assert_eq!(page[0].1.search_path, "ΟΣ/İSTANBUL.FLAC".to_lowercase());
            } else {
                assert!(!page[0].1.is_directory);
                assert!(page[0].1.source_identity.is_none());
                assert!(page[0].1.music_tags.is_none());
                assert!(page[0].1.search_metadata.is_empty());
                assert!(page[0].1.modified_unix_seconds.is_none());
            }
            Ok(())
        })
        .expect("reused mixed source pages");
    assert_eq!(sizes, [256, 256, 1]);
}

#[test]
fn page_reuse_preserves_the_original_fresh_conversion_admission_budget() {
    let fixture = Fixture::new();
    let entries = Arc::new(
        ["a", "b", "ΟΣ.FLAC"]
            .into_iter()
            .map(|name| Entry {
                music_tags: Some(MusicTags {
                    title: Some("x".into()),
                    artist: Some(String::new()),
                    album_artist: Some("ΟΣ".into()),
                    album: Some(String::new()),
                    ..MusicTags::default()
                }),
                ..entry(fixture.0.join(name))
            })
            .collect::<Vec<_>>(),
    );
    let mut original_bytes = 0;
    for row in entries.iter() {
        let mut original = SearchItem::from_path(row.path.clone(), &fixture.0, false, None);
        original.music_tags = row.music_tags.clone();
        let tags = row.music_tags.as_ref().expect("fixture tags");
        original.search_metadata = [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        original_bytes += item_bytes(&original);
    }
    let exact = fixture.source(entries.clone(), usize::MAX, original_bytes);
    assert_eq!(positions(&exact), [0, 1, 2]);
    assert!(exact.notice().is_none());
    let limited = fixture.source(entries, usize::MAX, original_bytes - 1);
    assert_eq!(positions(&limited), [0, 1]);
    assert!(limited.notice().is_some());
}

#[test]
fn reused_scope_check_matches_fresh_lookup_for_path_component_aliases() {
    let fixture = Fixture::new();
    let entries = Arc::new(
        [
            fixture.0.clone(),
            fixture.0.join("."),
            fixture.0.join("./A.TXT"),
            fixture.0.join("nested//B.TXT"),
            fixture.0.join("nested/../C.TXT"),
            fixture.0.with_extension("outside").join("D.TXT"),
        ]
        .into_iter()
        .map(entry)
        .collect(),
    );
    let source = fixture.source(entries, usize::MAX, usize::MAX);
    let expected: Vec<_> = (0..source.len())
        .filter_map(|position| {
            source
                .get(position)
                .expect("fresh read")
                .map(|item| (position, item))
        })
        .collect();
    let mut observed = Vec::new();
    source
        .for_each_page(&|| false, |page| {
            observed.extend_from_slice(page);
            Ok(())
        })
        .expect("reused scoped read");
    assert_eq!(observed, expected);
    assert_eq!(
        observed
            .iter()
            .map(|(position, _)| *position)
            .collect::<Vec<_>>(),
        [2, 3, 4]
    );
}

#[test]
fn entry_cap_counts_eligible_rows_and_reports_only_real_omissions() {
    let fixture = Fixture::new();
    let entries = Arc::new(vec![
        entry(fixture.0.clone()),
        entry(fixture.0.join("first.flac")),
        entry(PathBuf::from("/unrelated/second.flac")),
        entry(fixture.0.join("last.flac")),
    ]);
    let source = fixture.source(entries.clone(), 1, usize::MAX);
    assert_eq!(source.len(), 3);
    assert_eq!(positions(&source), vec![1]);
    assert!(source.notice().expect("cap notice").contains("1 entries"));
    assert_eq!(source.get(3).expect("capped row"), None);
    let complete = fixture.source(entries, 2, usize::MAX);
    assert_eq!(complete.len(), 4);
    assert_eq!(complete.notice(), None);
}

#[test]
fn zero_caps_warn_only_when_a_scoped_row_is_omitted() {
    let fixture = Fixture::new();
    let holes = Arc::new(vec![
        entry(fixture.0.clone()),
        entry(PathBuf::from("/unrelated/row")),
    ]);
    for (entry_limit, byte_limit) in [(0, usize::MAX), (usize::MAX, 0), (0, 0)] {
        let empty = fixture.source(Arc::new(Vec::new()), entry_limit, byte_limit);
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.notice(), None);
        let source = fixture.source(holes.clone(), entry_limit, byte_limit);
        assert_eq!(source.len(), 2);
        assert!(positions(&source).is_empty());
        assert_eq!(source.notice(), None);
        let mut entries = holes.as_ref().clone();
        entries.push(entry(fixture.0.join("omitted.flac")));
        let capped = fixture.source(Arc::new(entries), entry_limit, byte_limit);
        assert_eq!(capped.len(), 2);
        assert!(positions(&capped).is_empty());
        assert!(
            capped
                .notice()
                .expect("omission notice")
                .contains("limited")
        );
    }
}

#[test]
fn byte_cap_counts_transformed_strings_tags_and_each_item_struct() {
    let fixture = Fixture::new();
    let mut tagged = entry(fixture.0.join("İSTANBUL.FLAC"));
    tagged.music_tags = Some(MusicTags {
        title: Some("Ⱥ TITLE".into()),
        artist: Some("Artist".into()),
        album_artist: Some("Album Artist".into()),
        album: Some("Album".into()),
        ..MusicTags::default()
    });
    let entries = Arc::new(vec![tagged, entry(fixture.0.join("second.flac"))]);
    let full = fixture.source(entries.clone(), usize::MAX, usize::MAX);
    let first_cost = item_bytes(&full.get(0).expect("lookup").expect("first row"));
    let second_cost = item_bytes(&full.get(1).expect("lookup").expect("second row"));
    assert!(first_cost > size_of::<SearchItem>());
    let rejected = fixture.source(entries.clone(), usize::MAX, first_cost - 1);
    assert_eq!(rejected.len(), 0);
    assert!(rejected.notice().is_some());
    let one = fixture.source(entries.clone(), usize::MAX, first_cost);
    assert_eq!(positions(&one), vec![0]);
    assert_eq!(one.len(), 1);
    assert!(one.notice().is_some());
    let missing_struct = fixture.source(entries.clone(), usize::MAX, first_cost + second_cost - 1);
    assert_eq!(positions(&missing_struct), vec![0]);
    let exact = fixture.source(entries, usize::MAX, first_cost + second_cost);
    assert_eq!(positions(&exact), vec![0, 1]);
    assert_eq!(exact.notice(), None);
}

#[test]
fn unavailable_roots_are_empty_with_a_descriptive_notice() {
    let fixture = Fixture::new();
    for root in [fixture.0.join("missing"), fixture.0.join("file")] {
        fs::write(fixture.0.join("file"), b"not a directory").expect("create non-directory");
        let source = Source::new(
            root.clone(),
            Arc::new(vec![entry(root.join("stale.flac"))]),
            usize::MAX,
            usize::MAX,
            &|| false,
        )
        .expect("unavailable source");
        assert_eq!(source.len(), 0);
        assert!(positions(&source).is_empty());
        assert_eq!(source.get(0).expect("empty lookup"), None);
        assert_eq!(
            source.notice(),
            Some(format!("Cannot search {}: folder unavailable", root.display()).as_str())
        );
    }
}

#[test]
fn conversion_preserves_tags_identity_and_normalized_metadata() {
    let fixture = Fixture::new();
    let identity =
        FileIdentity::from_metadata(&fs::metadata(&fixture.0).expect("fixture metadata"));
    let tags = MusicTags {
        title: Some("İSTANBUL".into()),
        artist: Some("ΑΘΗΝΑ".into()),
        album_artist: Some("ПЕСНЯ".into()),
        album: Some("Album Ⱥ".into()),
        disc: Some(2),
        track: Some(7),
    };
    let entries = Arc::new(vec![
        Entry {
            path: fixture.0.join("Sub Folder/İSTANBUL.FLAC"),
            is_directory: false,
            modified_unix_seconds: Some(-123),
            identity: Some(identity.clone()),
            music_tags: Some(tags.clone()),
        },
        Entry {
            is_directory: true,
            ..entry(fixture.0.join("Folder"))
        },
        Entry {
            music_tags: Some(MusicTags {
                title: Some(String::new()),
                album_artist: Some("Only Album Artist".into()),
                ..MusicTags::default()
            }),
            ..entry(fixture.0.join("Empty.MP3"))
        },
    ]);
    let source = fixture.source(entries, usize::MAX, usize::MAX);
    let expected = SearchItem {
        path: fixture.0.join("Sub Folder/İSTANBUL.FLAC"),
        name: "İSTANBUL.FLAC".into(),
        search_name: "i\u{307}stanbul.flac".into(),
        search_path: "sub folder/i\u{307}stanbul.flac".into(),
        is_directory: false,
        modified_unix_seconds: Some(-123),
        music_tags: Some(tags),
        source_identity: Some(identity),
        search_metadata: "i\u{307}stanbul αθηνα песня album ⱥ".into(),
    };
    assert_eq!(source.get(0).expect("converted row"), Some(expected));
    let folder = source.get(1).expect("lookup").expect("folder");
    assert!(folder.is_directory);
    assert_eq!(folder.search_metadata, "");
    assert_eq!(folder.music_tags, None);
    assert_eq!(folder.source_identity, None);
    let empty = source.get(2).expect("lookup").expect("empty tag row");
    assert_eq!(empty.search_metadata, " only album artist");
}

#[test]
fn test_backing_preserves_crafted_normalized_fields_and_shares_its_arc() {
    let mut item = SearchItem::from_path("/fixture/file.flac".into(), Path::new("/"), false, None);
    item.search_name = "crafted name".into();
    item.search_path = "crafted path".into();
    item.search_metadata = "Case Sensitive Crafted Metadata".into();
    let source = Source::from_items(vec![item.clone()]);
    let reused = source.clone();
    let (Backing::Items(left), Backing::Items(right)) = (&source.backing, &reused.backing) else {
        panic!("expected explicit test backing")
    };
    assert!(Arc::ptr_eq(left, right));
    assert_eq!(source.len(), 1);
    assert_eq!(source.get(0).expect("crafted item"), Some(item.clone()));
    assert_eq!(positions(&source), vec![0]);
    source
        .for_each_page(&|| false, |page| {
            assert_eq!(page, &[(0, item.clone())]);
            Ok(())
        })
        .expect("crafted page");
    assert_eq!(source.get(1).expect("past end"), None);
}

#[test]
fn cancellation_stops_initial_scans_including_long_out_of_scope_runs() {
    let fixture = Fixture::new();
    let entries = Arc::new(vec![entry(fixture.0.clone()); 1000]);
    assert!(Source::new(fixture.0.clone(), entries.clone(), 10, 10, &|| true).is_err());
    let checks = Cell::new(0);
    let result = Source::new(fixture.0.clone(), entries, 10, 10, &|| {
        checks.set(checks.get() + 1);
        checks.get() >= 20
    });
    assert!(result.is_err());
    assert_eq!(checks.get(), 20);
    let source = fixture.source(
        Arc::new(vec![entry(fixture.0.clone()); 1000]),
        usize::MAX,
        usize::MAX,
    );
    checks.set(0);
    assert!(
        source
            .for_each_page(
                &|| {
                    checks.set(checks.get() + 1);
                    checks.get() >= 20
                },
                |_| panic!("out-of-scope rows must not reach the visitor"),
            )
            .is_err()
    );
    assert_eq!(checks.get(), 20);
}

#[test]
fn cancellation_and_visitor_errors_stop_pages_and_allow_a_fresh_replay() {
    let source = Source::from_items(
        (0..600)
            .map(|position| {
                SearchItem::from_path(
                    PathBuf::from(format!("/fixture/{position}.flac")),
                    Path::new("/fixture"),
                    false,
                    None,
                )
            })
            .collect(),
    );
    let visits = Cell::new(0);
    assert!(
        source
            .for_each_page(&|| true, |_| panic!("cancelled visit"))
            .is_err()
    );
    let cancelled = Cell::new(false);
    let error = source.for_each_page(&|| cancelled.get(), |page| {
        assert_eq!(page.len(), 256);
        visits.set(visits.get() + 1);
        cancelled.set(true);
        Ok(())
    });
    assert!(error.expect_err("page cancellation").contains("cancelled"));
    assert_eq!(visits.get(), 1);
    let result = source.for_each_page(&|| false, |_| Err("visitor failed".into()));
    assert_eq!(result, Err("visitor failed".into()));
    assert_eq!(positions(&source), (0..600).collect::<Vec<_>>());
}

#[test]
fn empty_and_final_partial_pages_still_honor_cancellation() {
    let source = Source::empty();
    assert_eq!(source.len(), 0);
    assert_eq!(source.notice(), None);
    source
        .for_each_page(&|| false, |_| panic!("empty page"))
        .expect("empty source");
    assert!(source.for_each_page(&|| true, |_| Ok(())).is_err());
    let item = SearchItem::from_path("/fixture/row".into(), Path::new("/"), false, None);
    let source = Source::from_items(vec![item]);
    let cancelled = Cell::new(false);
    assert!(
        source
            .for_each_page(&|| cancelled.get(), |_| {
                cancelled.set(true);
                Ok(())
            })
            .is_err()
    );
}
