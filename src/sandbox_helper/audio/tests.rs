use super::{AudioDetails, AudioMetadata, parse_audio_details, parse_music_tags};

#[test]
fn test_audio_metadata_serialization() {
    let meta = AudioMetadata {
        format: "FLAC".to_string(),
        duration_seconds: 184.2,
        sample_rate: 96000,
        channels: 2,
        bitrate: Some(1500000),
    };
    let json = serde_json::to_string(&meta).unwrap();
    let decoded: AudioMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(meta, decoded);
}

#[test]
fn reads_case_insensitive_album_disc_and_track_tags() {
    let tags = parse_music_tags(
        br#"{"format":{"tags":{"ALBUM":"Blonde","TRACKNUMBER":"3/17","DISC":"2/2"}}}"#,
    )
    .expect("Valid probe");
    assert_eq!(tags.album.as_deref(), Some("Blonde"));
    assert_eq!(tags.track, Some(3));
    assert_eq!(tags.disc, Some(2));
}

#[test]
fn uses_stream_tags_and_rejects_invalid_numbers() {
    let tags = parse_music_tags(
        br#"{"streams":[{"tags":{"album":"Example","track":"0","discnumber":"999999999999"}}]}"#,
    )
    .expect("Valid probe");
    assert_eq!(tags.album.as_deref(), Some("Example"));
    assert_eq!(tags.track, None);
    assert_eq!(tags.disc, None);
    assert!(parse_music_tags(b"invalid").is_err());
}

#[test]
fn rich_metadata_preserves_totals_and_useful_tags_without_changing_music_tags() {
    let details = parse_audio_details(br#"{"format":{"format_name":"flac","duration":"184.2","bit_rate":"1500000","tags":{"TITLE":"Example","ARTIST":"Artist","ALBUM":"Album","ALBUM_ARTIST":"Album artist","TRACK":"3/17","DISC":"2/2","GENRE":"Ambient","DATE":"2025-05-04","COMPOSER":"Composer","COMMENT":"Notes"}},"streams":[{"codec_name":"flac","sample_rate":"96000","channels":2,"bits_per_raw_sample":"24"}]}"#).unwrap();
    assert_eq!(details.tags.track, Some(3));
    assert_eq!(details.tags.disc, Some(2));
    assert_eq!(details.embedded_tags["track"], "3/17");
    assert_eq!(details.embedded_tags["composer"], "Composer");
    assert_eq!(details.embedded_tags["comment"], "Notes");
    assert_eq!(details.properties.duration_seconds, Some(184.2));
    assert_eq!(details.properties.sample_rate, Some(96000));
    assert_eq!(details.properties.bitrate, Some(1500000));
    assert_eq!(details.properties.bit_depth, Some(24));
    let json = serde_json::to_vec(&details).unwrap();
    assert_eq!(
        serde_json::from_slice::<AudioDetails>(&json).unwrap(),
        details
    );
    assert_eq!(
        serde_json::from_slice::<crate::model::MusicTags>(&json).unwrap(),
        details.tags
    );
}

#[test]
fn unknown_or_invalid_audio_properties_are_not_invented() {
    let details = parse_audio_details(br#"{"format":{"duration":"NaN","bit_rate":"0"},"streams":[{"duration":"inf","sample_rate":"999999999999","channels":65537,"bits_per_raw_sample":"0"}]}"#).unwrap();
    assert_eq!(details.properties, super::AudioProperties::default());
    assert_eq!(parse_audio_details(b"{}").unwrap(), AudioDetails::default());
    assert!(parse_audio_details(b"invalid").is_err());
}

#[test]
fn stream_properties_win_and_format_tags_have_stable_precedence() {
    let details = parse_audio_details(br#"{"format":{"duration":"50","bit_rate":"200","tags":{"ARTIST":"Format artist"}},"streams":[{"duration":"40","bit_rate":"100","tags":{"artist":"Stream artist","genre":"Rock"}}]}"#).unwrap();
    assert_eq!(details.properties.duration_seconds, Some(40.0));
    assert_eq!(details.properties.bitrate, Some(100));
    assert_eq!(details.embedded_tags["artist"], "Format artist");
    assert_eq!(details.embedded_tags["genre"], "Rock");
}

#[test]
fn rich_embedded_tags_are_bounded_and_control_characters_are_removed() {
    let tags = (0..100)
        .map(|index| {
            (
                format!("tag_{index}"),
                format!("\n{}\u{0}\t", "é".repeat(2000)),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let json = serde_json::to_vec(&serde_json::json!({"format":{"tags": tags}})).unwrap();
    let details = parse_audio_details(&json).unwrap();
    assert_eq!(details.embedded_tags.len(), 64);
    for value in details.embedded_tags.values() {
        assert_eq!(value.chars().count(), 1024);
        assert!(!value.chars().any(char::is_control));
    }
}
