use super::*;
use crate::services::{
    SearchResult,
    music_library::tests::{Fixture, ready},
    search::UnavailableTextExtraction,
};

#[test]
fn literal_smart_queries_keep_grouped_and_result_tab_matches_with_sql_candidates() {
    let root = Path::new("/synthetic");
    let mut literal = SearchItem::from_path(root.join("01.flac"), root, false, None);
    literal.search_metadata = "video games an artist".into();
    literal.music_tags = Some(MusicTags {
        title: Some("Video Games".into()),
        album: Some("Video Games".into()),
        artist: Some("An Artist".into()),
        ..Default::default()
    });
    let video = SearchItem::from_path(root.join("Games.mp4"), root, false, None);
    let mut worker = super::pagination_tests::synthetic_worker(vec![literal, video]);
    worker.query.text = "Video Games".into();
    worker.prepare_candidates();
    assert!(worker.candidates.is_none());
    worker.candidates = Some(super::super::candidates::Candidates::build(&worker.shared_items, &|| false).unwrap());
    for virtual_results in [true, false] {
        worker.query.grouping = Some(Grouping {
            group: SearchGroup::Songs,
            entity: None,
            virtual_results,
        });
        worker.rescore_shared();
        let indexed = worker.ranked.clone();
        let candidates = worker.candidates.take();
        worker.rescore_shared();
        assert_eq!(worker.ranked, indexed);
        worker.candidates = candidates;
        if virtual_results {
            let groups = worker.groups.as_ref().unwrap();
            assert_eq!(groups.counts().songs, 1);
            assert_eq!(groups.counts().albums, 1);
        } else {
            assert_eq!(worker.ranked.len(), 1);
            assert_eq!(worker.shared_items[worker.ranked[0].1].name, "01.flac");
        }
    }
    worker.query.text = "Video Games type:video".into();
    worker.rescore_shared();
    assert!(worker.ranked.is_empty());
}

#[test]
fn entity_pages_can_extend_past_the_source_entry_limit() {
    let root = Path::new("/synthetic");
    let items = (0..100)
        .map(|number| {
            let mut item =
                SearchItem::from_path(root.join(format!("{number:03}.flac")), root, false, None);
            item.music_tags = Some(MusicTags {
                artist: Some("Fixture artist".into()),
                album: Some("Album".into()),
                ..MusicTags::default()
            });
            item
        })
        .collect();
    let mut worker = super::pagination_tests::synthetic_worker(items);
    worker.limits.entries = 100;
    worker.query.limit = 100;
    worker.query.grouping = Some(Grouping {
        group: SearchGroup::All,
        entity: None,
        virtual_results: true,
    });
    worker.rescore_shared();
    worker.publish(false);
    let SearchEvent::Results {
        page: Some(page),
        groups: Some(groups),
        ..
    } = worker.events.take().unwrap()
    else {
        panic!("Expected grouped first page")
    };
    assert_eq!(
        (page.shown, page.total, groups.results.len()),
        (100, 102, 100)
    );
    worker.commands.put(Command::More(page));
    worker.pending_shared(false);
    let SearchEvent::Results {
        page: Some(page),
        groups: Some(groups),
        ..
    } = worker.events.take().unwrap()
    else {
        panic!("Expected extended page")
    };
    assert_eq!(
        (page.shown, page.total, groups.results.len()),
        (102, 102, 102)
    );
    assert!(!page.has_more());
}

#[test]
fn album_drilling_and_export_follow_disc_track_tags_before_filenames() {
    let root = Path::new("/synthetic");
    let items = [("a.flac", 2, 1), ("b.flac", 1, 2), ("z.flac", 1, 1)]
        .into_iter()
        .map(|(name, disc, track)| {
            let mut item = SearchItem::from_path(root.join(name), root, false, None);
            item.music_tags = Some(MusicTags {
                artist: Some("Fixture artist".into()),
                album: Some("Album".into()),
                disc: Some(disc),
                track: Some(track),
                ..MusicTags::default()
            });
            item
        })
        .collect();
    let mut worker = super::pagination_tests::synthetic_worker(items);
    for virtual_results in [true, false] {
        worker.query.grouping = Some(Grouping {
            group: SearchGroup::Songs,
            entity: Some(SearchEntityKey::Album {
                artist: "fixture artist".into(),
                album: "album".into(),
            }),
            virtual_results,
        });
        worker.rescore_shared();
        let names: Vec<_> = worker
            .ranked
            .iter()
            .map(|(_, position)| worker.shared_items[*position].name.as_str())
            .collect();
        assert_eq!(names, ["z.flac", "b.flac", "a.flac"]);
    }
}

fn grouped_page(
    receiver: &SearchReceiver,
    generation: u64,
    group: SearchGroup,
    shown: usize,
    total: usize,
) -> (SearchGroups, SearchPage) {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut last = None;
    while Instant::now() < deadline {
        if let Ok(SearchEvent::Results {
            generation: current,
            items,
            indexing,
            page,
            groups,
            notice,
            ..
        }) = receiver.recv_timeout(TICK)
        {
            last = Some((
                current,
                indexing,
                page,
                groups.as_ref().map(|groups| (groups.group, groups.counts)),
                notice,
            ));
            if current == generation
                && !indexing
                && let (Some(page), Some(groups)) = (page, groups)
                && groups.group == group
                && page.shown == shown
                && page.total == total
            {
                assert!(items.is_empty());
                assert_eq!(groups.results.len(), shown);
                return (groups, page);
            }
        }
    }
    panic!("Expected {group:?} page {shown}/{total}; last={last:?}");
}

#[test]
fn grouped_catalog_uses_all_eligible_tags_and_exports_only_real_scoped_songs() {
    let fixture = Fixture::new();
    for number in 0..150 {
        std::fs::write(
            fixture
                .root
                .join(format!("performer-notes-{number:04}.txt")),
            b"Synthetic",
        )
        .unwrap();
    }
    for number in 0..245 {
        let album = if number < 120 { "Alpha" } else { "Beta" };
        let path = fixture
            .root
            .join(album)
            .join(format!("track-{number:04}.flac"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let tags = MusicTags {
            title: Some(format!("Song {number}")),
            artist: Some("Performer".into()),
            album: Some(album.into()),
            ..MusicTags::default()
        };
        std::fs::write(path, serde_json::to_vec(&tags).unwrap()).unwrap();
    }
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .filter(|entry| entry.music_tags.is_some())
            .count()
            == 245
    });
    let (handle, receiver) = start_shared(
        fixture.root.clone(),
        Arc::new(UnavailableTextExtraction),
        crate::services::music_library::subscribe(fixture.root.clone()),
    );
    let query = "performer";
    let generation = handle.query_group(query, SearchGroup::All, None);
    let (first, cursor) = grouped_page(&receiver, generation, SearchGroup::All, 100, 398);
    assert_eq!(
        (
            first.counts.files,
            first.counts.songs,
            first.counts.albums,
            first.counts.artists
        ),
        (150, 245, 2, 1)
    );
    for group in [
        SearchGroup::Files,
        SearchGroup::Songs,
        SearchGroup::Albums,
        SearchGroup::Artists,
    ] {
        assert!(first.results.iter().any(|result| result.group() == group));
    }
    handle.more(cursor);
    let (second, _) = grouped_page(&receiver, generation, SearchGroup::All, 200, 398);
    for group in [
        SearchGroup::Files,
        SearchGroup::Songs,
        SearchGroup::Albums,
        SearchGroup::Artists,
    ] {
        let prefix: Vec<_> = first
            .results
            .iter()
            .filter(|result| result.group() == group)
            .map(SearchResult::key)
            .collect();
        let extended: Vec<_> = second
            .results
            .iter()
            .filter(|result| result.group() == group)
            .map(SearchResult::key)
            .collect();
        assert!(extended.starts_with(&prefix));
    }
    let generation = handle.query_group(query, SearchGroup::Albums, None);
    let (albums, _) = grouped_page(&receiver, generation, SearchGroup::Albums, 2, 2);
    let SearchResult::Album(album) = albums
        .results
        .iter()
        .find(|result| matches!(result, SearchResult::Album(entity) if entity.title == "Alpha"))
        .unwrap()
    else {
        panic!("Expected album")
    };
    assert_eq!(album.songs, 120);
    let entity = album.key.clone();
    let generation = handle.query_group(query, SearchGroup::Songs, Some(entity.clone()));
    let (songs, _) = grouped_page(&receiver, generation, SearchGroup::Songs, 100, 120);
    assert!(songs.results.iter().all(|result| {
        result
            .file()
            .unwrap()
            .path
            .parent()
            .unwrap()
            .ends_with("Alpha")
    }));
    let generation = handle.query_all_scoped(query, SearchGroup::Songs, Some(entity));
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Ok(SearchEvent::Results {
            generation: current,
            items,
            indexing: false,
            groups: None,
            page: Some(page),
            ..
        }) = receiver.recv_timeout(TICK)
            && current == generation
        {
            assert_eq!(items.len(), 120);
            assert_eq!(page.total, 120);
            assert!(
                items
                    .iter()
                    .all(|item| item.path.parent().unwrap().ends_with("Alpha"))
            );
            break;
        }
        assert!(Instant::now() < deadline);
    }
    let generation = handle.query_all_scoped(query, SearchGroup::Files, None);
    loop {
        if let Ok(SearchEvent::Results {
            generation: current,
            items,
            indexing: false,
            groups: None,
            ..
        }) = receiver.recv_timeout(TICK)
            && current == generation
        {
            assert_eq!(items.len(), 150);
            assert!(
                items
                    .iter()
                    .all(|item| item.path.extension().unwrap() == "txt")
            );
            break;
        }
        assert!(Instant::now() < deadline);
    }
}

#[test]
fn compilation_drilldown_uses_album_artist_without_replacing_track_artist_filters() {
    let fixture = Fixture::new();
    for (number, artist) in [(1, "First performer"), (2, "Second performer")] {
        let tags = MusicTags {
            title: Some(format!("Song {number}")),
            artist: Some(artist.into()),
            album_artist: Some("Various Artists".into()),
            album: Some("Compilation".into()),
            ..MusicTags::default()
        };
        std::fs::write(
            fixture.root.join(format!("{number}.flac")),
            serde_json::to_vec(&tags).unwrap(),
        )
        .unwrap();
    }
    let mut subscription = fixture.subscribe();
    ready(&mut subscription, |snapshot| {
        snapshot
            .entries
            .iter()
            .filter(|entry| entry.music_tags.is_some())
            .count()
            == 2
    });
    let (handle, receiver) = start_shared(
        fixture.root.clone(),
        Arc::new(UnavailableTextExtraction),
        crate::services::music_library::subscribe(fixture.root.clone()),
    );
    let generation = handle.query_group("compilation", SearchGroup::Albums, None);
    let (results, _) = grouped_page(&receiver, generation, SearchGroup::Albums, 1, 1);
    let SearchResult::Album(album) = &results.results[0] else {
        panic!("Expected compilation")
    };
    assert_eq!(album.songs, 2);
    let generation = handle.query_group(
        "artist:\"First performer\"",
        SearchGroup::Songs,
        Some(album.key.clone()),
    );
    let (songs, _) = grouped_page(&receiver, generation, SearchGroup::Songs, 1, 1);
    assert_eq!(songs.results[0].file().unwrap().name, "1.flac");
    let generation = handle.query_group("", SearchGroup::Songs, Some(album.key.clone()));
    let (songs, _) = grouped_page(&receiver, generation, SearchGroup::Songs, 2, 2);
    assert!(songs.results.iter().all(|result| result.file().is_some()));
}
