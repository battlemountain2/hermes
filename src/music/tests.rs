use super::*;

#[test]
fn missing_metadata_uses_display_fallbacks_without_inventing_tags() {
    let track = Track {
        path: PathBuf::from("/Artist - Album/02 Filename.flac"),
        tags: MusicTags::default(),
    };
    assert!(track.needs_metadata());
    assert_eq!(track.title(), "02 Filename");
    assert_eq!(track.artist(), "Unknown artist");
    assert_eq!(track.album(), "Artist - Album");
    assert_eq!(track.tags, MusicTags::default());
}

#[test]
fn hydration_fills_only_missing_tags_and_never_uses_display_fallbacks() {
    let mut track = Track {
        path: PathBuf::from("/Not An Artist/02 Filename.flac"),
        tags: MusicTags {
            title: Some("Existing title".into()),
            artist: Some(" \t ".into()),
            disc: Some(2),
            ..Default::default()
        },
    };
    let tags = MusicTags {
        title: Some("Embedded title".into()),
        artist: Some(" Embedded artist ".into()),
        album_artist: Some("Embedded album artist".into()),
        album: Some("Embedded album".into()),
        disc: Some(1),
        track: Some(4),
    };
    assert!(track.hydrate_metadata(&tags));
    assert!(!track.needs_metadata());
    assert_eq!(track.title(), "Existing title");
    assert_eq!(track.artist(), "Embedded artist");
    assert_eq!(track.album(), "Embedded album");
    assert_eq!(track.tags.disc, Some(2));
    assert_eq!(track.tags.track, Some(4));
    assert!(!track.hydrate_metadata(&tags));
    assert!(!track.hydrate_metadata(&MusicTags::default()));
}

#[test]
fn blank_metadata_is_missing_and_album_artist_is_a_tagged_artist_fallback() {
    let mut track = Track {
        path: PathBuf::from("/Directory/filename.flac"),
        tags: MusicTags {
            title: Some("  ".into()),
            artist: Some(" ".into()),
            album_artist: Some(" Album artist ".into()),
            album: Some("\t".into()),
            ..Default::default()
        },
    };
    assert!(track.needs_metadata());
    assert_eq!(track.title(), "filename");
    assert_eq!(track.artist(), "Album artist");
    assert_eq!(track.album(), "Directory");
    track.hydrate_metadata(&MusicTags {
        title: Some("Title".into()),
        album: Some("Album".into()),
        track: Some(0),
        disc: Some(0),
        ..Default::default()
    });
    assert!(!track.needs_metadata());
    assert!(track.tags.artist.is_none());
    assert_eq!(track.tags.track, None);
    assert_eq!(track.tags.disc, None);
}

#[test]
fn moving_tracks_changes_next_order_without_changing_current_or_history() {
    let mut queue = Queue::default();
    queue.replace((1..=4).map(track).collect(), 0);
    assert!(queue.move_before(3, 1));
    assert_eq!(queue.playing(), Some(&track(1)));
    assert_eq!(queue.advance(false), Some(track(4)));
    assert_eq!(queue.previous(), Some(track(1)));
    assert!(queue.move_before(0, 4));
    assert_eq!(queue.playing(), Some(&track(1)));
    assert!(queue.advance(false).is_none());
    assert!(!queue.move_before(10, 0));
    assert!(!queue.move_before(0, 9));
}

#[test]
fn removal_remaps_current_history_and_order_including_duplicates() {
    let mut queue = Queue::default();
    queue.replace(vec![track(1), track(2), track(2), track(3)], 0);
    queue.select(2);
    assert!(queue.remove(1));
    assert_eq!(queue.current, Some(1));
    assert_eq!(queue.playing(), Some(&track(2)));
    assert_eq!(queue.previous(), Some(track(1)));
    assert_eq!(queue.advance(false), Some(track(2)));
    assert!(queue.remove(1));
    assert_eq!(queue.playing(), Some(&track(3)));
    assert!(queue.remove(1));
    assert!(queue.current.is_none());
    assert!(!queue.remove(99));
}

#[test]
fn shuffle_reorder_remove_and_play_next_share_the_displayed_order() {
    let mut queue = Queue::default();
    queue.replace((1..=8).map(track).collect(), 0);
    queue.set_shuffle(true, 42);
    let next_index = queue.order[1];
    assert!(queue.move_before(next_index, 8));
    queue.play_next(vec![track(9)]);
    assert_eq!(queue.advance(false), Some(track(9)));
    queue.remove(8);
    let expected = queue.playing().cloned().unwrap();
    assert_eq!(queue.ordered()[1].1, &expected);
    assert_eq!(queue.order.len(), 8);
    assert_eq!(
        queue
            .order
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        8
    );
}

#[test]
fn explicit_queue_selection_keeps_history_bounded() {
    let mut queue = Queue::default();
    queue.replace(vec![track(1), track(2)], 0);
    for index in 0..5000 {
        queue.select(index % 2);
    }
    assert_eq!(queue.history.len(), 1024);
}

fn track(number: u32) -> Track {
    Track {
        path: PathBuf::from(format!("/album/{number}.flac")),
        tags: MusicTags {
            track: Some(number),
            ..MusicTags::default()
        },
    }
}

#[test]
fn queue_finishes_without_restarting_and_explicit_next_overrides_repeat_one() {
    let mut queue = Queue::default();
    queue.replace(vec![track(1), track(2)], 0);
    queue.repeat = Repeat::One;
    assert_eq!(queue.advance(true), Some(track(1)));
    assert_eq!(queue.advance(false), Some(track(2)));
    queue.repeat = Repeat::Off;
    assert!(queue.advance(true).is_none());
    assert!(queue.current.is_none());
}

#[test]
fn play_next_and_append_preserve_current_track() {
    let mut queue = Queue::default();
    queue.replace(vec![track(1), track(2)], 0);
    queue.play_next(vec![track(3)]);
    queue.append(vec![track(4)]);
    assert_eq!(queue.playing(), Some(&track(1)));
    assert_eq!(queue.advance(false), Some(track(3)));
    assert_eq!(queue.advance(false), Some(track(2)));
    assert_eq!(queue.advance(false), Some(track(4)));
}

#[test]
fn shuffle_keeps_current_and_previous_uses_actual_history() {
    let mut queue = Queue::default();
    queue.replace((1..8).map(track).collect(), 3);
    queue.set_shuffle(true, 42);
    assert_eq!(queue.playing(), Some(&track(4)));
    let next = queue.advance(false).unwrap();
    assert_ne!(next, track(4));
    assert_eq!(queue.previous(), Some(track(4)));
    assert_eq!(
        queue
            .order
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        7
    );
}

#[test]
fn metadata_disc_track_order_wins_and_old_tags_still_deserialize() {
    let tags: MusicTags =
        serde_json::from_str(r#"{"album":"Example","disc":1,"track":2}"#).unwrap();
    assert_eq!(tags.title, None);
    let mut tracks = vec![track(10), track(2), track(1)];
    tracks.sort_by(compare_tracks);
    assert_eq!(tracks, vec![track(1), track(2), track(10)]);
}

#[test]
fn starting_at_track_three_queues_only_it_and_remaining_folder_tracks() {
    let mut another_folder = track(5);
    another_folder.path = PathBuf::from("/another-album/5.flac");
    let tracks = folder_queue(
        vec![
            track(10),
            track(1),
            track(4),
            track(3),
            track(2),
            another_folder,
        ],
        std::path::Path::new("/album/3.flac"),
    );
    assert_eq!(tracks, vec![track(3), track(4), track(10)]);
    let mut queue = Queue::default();
    queue.replace(tracks, 0);
    assert_eq!(queue.playing(), Some(&track(3)));
    assert_eq!(queue.advance(true), Some(track(4)));
    assert_eq!(queue.advance(true), Some(track(10)));
    assert!(queue.advance(true).is_none());
}

#[test]
fn folder_queue_uses_metadata_and_does_not_include_previous_discs() {
    let mut selected = track(3);
    selected.tags.disc = Some(2);
    let mut next = track(4);
    next.tags.disc = Some(2);
    next.path = PathBuf::from("/album/01-wrong-filename.flac");
    let tracks = folder_queue(
        vec![next.clone(), track(10), selected.clone()],
        &selected.path,
    );
    assert_eq!(tracks, vec![selected, next]);
    assert!(folder_queue(vec![track(1)], std::path::Path::new("/album/missing.flac")).is_empty());
    assert_eq!(
        folder_queue(
            vec![track(1), track(2)],
            std::path::Path::new("/album/2.flac")
        ),
        vec![track(2)]
    );
}

#[test]
fn folder_queue_with_folder_path_returns_all_tracks_in_order() {
    let t1 = track(1);
    let t2 = track(2);
    let t3 = track(3);
    let mut other = track(1);
    other.path = PathBuf::from("/other-album/1.flac");
    let tracks = folder_queue(
        vec![t3.clone(), other, t1.clone(), t2.clone()],
        std::path::Path::new("/album"),
    );
    assert_eq!(tracks, vec![t1, t2, t3]);
}
