use std::{ffi::OsString, rc::Rc};

use super::*;
use crate::model::{EntryKind, FileEntryInner, Location, MetadataValue, MusicTags};

pub(crate) fn entry(name: &str, disc: Option<u32>, track: Option<u32>) -> FileEntry {
    FileEntry(Rc::new(FileEntryInner {
        location: Location::local(format!("/album/{name}")),
        native_name: OsString::from(name),
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Known(1),
        modified_unix_seconds: MetadataValue::Known(1),
        music_tags: Some(MusicTags {
            album: Some("Example".into()),
            disc,
            track,
            ..MusicTags::default()
        }),
    }))
}

#[test]
fn metadata_wins_over_filenames_and_discs_precede_tracks() {
    let mut entries = vec![
        entry("1. Finale.flac", Some(2), Some(1)),
        entry("10. Opening.flac", Some(1), Some(1)),
        entry("2. Late.flac", Some(1), Some(10)),
        entry("cover.jpg", None, None),
    ];
    entries.sort_by(compare_tracks);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect::<Vec<_>>(),
        [
            "10. Opening.flac",
            "2. Late.flac",
            "1. Finale.flac",
            "cover.jpg"
        ]
    );
}

#[test]
fn filename_fallback_is_numeric_and_deterministic() {
    let mut entries = vec![
        entry("10. Song.flac", None, None),
        entry("2. Song.flac", None, None),
        entry("1. Song.flac", None, None),
        entry("Unknown.flac", None, None),
    ];
    entries.sort_by(compare_tracks);
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.display_name.as_str())
            .collect::<Vec<_>>(),
        [
            "1. Song.flac",
            "2. Song.flac",
            "10. Song.flac",
            "Unknown.flac"
        ]
    );
    assert_eq!(filename_track("2024song.flac"), None);
    assert_eq!(filename_track("00. Song.flac"), None);
    assert_eq!(natural_name("Song 2.flac", "Song 10.flac"), Ordering::Less);
}

#[test]
fn detects_album_folders_not_mixed_libraries_or_general_folders() {
    let tracks = vec![
        entry("one.flac", None, Some(1)),
        entry("two.FLAC", None, Some(2)),
    ];
    assert!(is_album(&tracks));
    let mut mixed = tracks.clone();
    Rc::make_mut(&mut mixed[1].0)
        .music_tags
        .as_mut()
        .unwrap()
        .album = Some("Other".into());
    assert!(!is_album(&mixed));
    let mut general = tracks;
    general.push(entry("movie.mp4", None, None));
    assert!(!is_album(&general));
    general.pop();
    general.push(entry("cover.JPG", None, None));
    assert!(is_album(&general));
    Rc::make_mut(&mut general[2].0).kind = EntryKind::Directory;
    assert!(!is_album(&general));
}
