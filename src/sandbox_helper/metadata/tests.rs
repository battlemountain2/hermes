use super::*;

#[test]
fn media_details_keep_real_video_separate_from_attached_art_and_use_album_artist() {
    let data = br#"{"format":{"duration":"123.5","tags":{"title":"A song","album_artist":"An artist","album":"A record"}},"streams":[{"codec_type":"video","codec_name":"mjpeg","width":600,"height":600,"disposition":{"attached_pic":1}},{"codec_type":"audio","codec_name":"flac"},{"codec_type":"video","codec_name":"h264","width":1920,"height":1080}]}"#;
    let details = parse_media(data).unwrap();
    assert_eq!(details.artist.as_deref(), Some("An artist"));
    assert_eq!(details.title.as_deref(), Some("A song"));
    assert_eq!(details.width, Some(1920));
    assert_eq!(details.height, Some(1080));
    assert_eq!(details.duration_seconds, Some(123.5));
    assert_eq!(details.audio_codec.as_deref(), Some("flac"));
    assert_eq!(details.video_codec.as_deref(), Some("h264"));
}

#[test]
fn media_details_reject_unbounded_payloads_and_ignore_invalid_numbers() {
    assert!(parse_media(&vec![b' '; 65537]).is_err());
    assert!(parse_media(b"not json").is_err());
    let details = parse_media(br#"{"format":{"duration":"NaN"},"streams":[{"codec_type":"video","width":1000001,"height":0,"duration":"-1"}]}"#).unwrap();
    assert_eq!(details.width, None);
    assert_eq!(details.height, None);
    assert_eq!(details.duration_seconds, None);
}
