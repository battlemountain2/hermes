// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use super::*;

#[test]
fn parses_standard_lrc_lines_with_fractional_seconds() {
    let lrc = r#"
[ti:Rolling Out the Red Carpet]
[ar:Hail the Sun]
[al:Wake]
[00:00.00] Allotting time just to dream them through
[00:20.50] Rage is the itch I'm not scratching
[01:25.40] Owning a red carpet
[01:31.00] Helps hide the secret
"#;

    let lyrics = parse_lrc(lrc).expect("must parse standard lrc");
    assert_eq!(lyrics.len(), 4);
    assert!(!lyrics.is_empty());
    assert_eq!(lyrics.lines[0].timestamp, Duration::from_secs(0));
    assert_eq!(lyrics.lines[0].text, "Allotting time just to dream them through");
    assert_eq!(lyrics.lines[1].timestamp, Duration::from_millis(20_500));
    assert_eq!(lyrics.lines[2].timestamp, Duration::from_millis(85_400));
    assert_eq!(lyrics.lines[2].text, "Owning a red carpet");
    assert_eq!(lyrics.lines[3].timestamp, Duration::from_millis(91_000));
}

#[test]
fn active_line_at_binary_search_accuracy() {
    let lrc = r#"
[00:10.00] Line One
[00:20.00] Line Two
[00:30.00] Line Three
"#;
    let lyrics = parse_lrc(lrc).expect("parse lyrics");

    // Before any line starts
    assert_eq!(lyrics.active_line_at(Duration::from_secs(5)), Some(0));

    // Exactly at line 1
    assert_eq!(lyrics.active_line_at(Duration::from_secs(10)), Some(0));

    // Midway through line 1
    assert_eq!(lyrics.active_line_at(Duration::from_secs(15)), Some(0));

    // Exactly at line 2
    assert_eq!(lyrics.active_line_at(Duration::from_secs(20)), Some(1));

    // Beyond line 3
    assert_eq!(lyrics.active_line_at(Duration::from_secs(45)), Some(2));
}

#[test]
fn empty_and_malformed_lrc_returns_none() {
    assert_eq!(parse_lrc(""), None);
    assert_eq!(parse_lrc("[ar: Artist Only]\n[ti: Title Only]"), None);
    assert_eq!(parse_lrc("not a lrc line at all"), None);
}

#[test]
fn rejects_unicode_fractions_and_overflowing_timestamps_without_panicking() {
    for tag in [
        "00:01.💫",
        "00:01.12💫",
        "00:01.éé",
        "00:01.１２３",
        "00:01.١٢٣",
        "00:01.",
        "00:01.123x",
        "+1:00",
        "00:+1",
        "00:60",
        "18446744073709551615:00",
        "307445734561825:59.999",
        "999999999999999999999999999999999999:00",
    ] {
        assert_eq!(parse_timestamp(tag), None, "{tag}");
        let content = format!("[{tag}]bad\n[00:02.50]valid 🎵");
        let lyrics = parse_lrc(&content).expect("valid line survives invalid timestamp");
        assert_eq!(lyrics.len(), 1);
        assert_eq!(lyrics.lines[0].text, "valid 🎵");
    }
}

#[test]
fn fractional_precision_and_timestamps_stay_within_signed_seek_bounds() {
    for (tag, millis) in [
        ("00:01.1", 1100),
        ("00:01.12", 1120),
        ("00:01.123", 1123),
        ("00:01.123456", 1123),
    ] {
        assert_eq!(parse_timestamp(tag), Some(Duration::from_millis(millis)));
    }
    let limit = i64::MAX as u64 / 1000;
    let format_timestamp = |millis: u64| {
        format!("{}:{:02}.{:03}", millis / 60_000, millis / 1000 % 60, millis % 1000)
    };
    let timestamp = parse_timestamp(&format_timestamp(limit)).expect("maximum seekable timestamp");
    assert!(timestamp.as_micros() <= i64::MAX as u128);
    assert_eq!(parse_timestamp(&format_timestamp(limit + 1)), None);
}

#[test]
fn bounds_input_bytes_and_individual_line_bytes() {
    assert!(parse_lrc(&"x".repeat(MAX_LRC_BYTES + 1)).is_none());
    let mut line = format!("[00:00]{}", "x".repeat(MAX_LINE_BYTES - 7));
    assert_eq!(parse_lrc(&line).expect("line at limit").len(), 1);
    line.push('x');
    assert!(parse_lrc(&line).is_none());
}

#[test]
fn bounds_timestamp_expansion_and_total_lyric_lines() {
    let mut timestamps = "[00:00]".repeat(MAX_TIMESTAMPS_PER_LINE);
    assert_eq!(parse_lrc(&timestamps).expect("timestamps at limit").len(), MAX_TIMESTAMPS_PER_LINE);
    timestamps.push_str("[00:00]");
    assert!(parse_lrc(&timestamps).is_none());
    let mut lines = "[00:00]x\n".repeat(MAX_LYRIC_LINES);
    assert_eq!(parse_lrc(&lines).expect("lines at limit").len(), MAX_LYRIC_LINES);
    lines.push_str("[00:00]x\n");
    assert!(parse_lrc(&lines).is_none());
}

#[test]
fn bounds_expanded_lyric_text_not_just_source_bytes() {
    let line = format!("{}{}\n", "[00:00]".repeat(32), "x".repeat(1024));
    let allowed = line.repeat(MAX_LYRIC_TEXT_BYTES / (32 * 1024));
    assert!(parse_lrc(&allowed).is_some());
    assert!(parse_lrc(&(allowed + &line)).is_none());
}

#[test]
fn sidecar_reader_caps_bytes_and_rejects_invalid_utf8() {
    struct CountingReader {
        bytes: usize,
    }
    impl std::io::Read for CountingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            buffer.fill(b'x');
            self.bytes += buffer.len();
            Ok(buffer.len())
        }
    }
    let mut reader = CountingReader { bytes: 0 };
    assert!(read_lrc(&mut reader).is_none());
    assert_eq!(reader.bytes, MAX_LRC_BYTES + 1);
    assert!(read_lrc(&b"[00:00]bad\xff"[..]).is_none());
    assert!(read_lrc(&b"[00:00]valid"[..]).is_some());
}

#[test]
fn sidecar_lookup_rejects_oversized_and_non_regular_files() {
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let fixture = Fixture(std::env::temp_dir().join(format!("hermes-lyrics-{}-{unique}", std::process::id())));
    fs::create_dir(&fixture.0).expect("fixture directory");
    let audio = fixture.0.join("track.flac");
    let sidecar = audio.with_extension("lrc");
    assert!(find_lyrics_for_audio_file(&audio).is_none());
    fs::create_dir(&sidecar).expect("non-regular sidecar");
    assert!(find_lyrics_for_audio_file(&audio).is_none());
    fs::remove_dir(&sidecar).expect("remove non-regular sidecar");
    fs::File::create(&sidecar)
        .expect("oversized sidecar")
        .set_len(MAX_LRC_BYTES as u64 + 1)
        .expect("sparse sidecar size");
    assert!(find_lyrics_for_audio_file(&audio).is_none());
    fs::write(&sidecar, "[00:00]fixture").expect("valid sidecar");
    assert_eq!(find_lyrics_for_audio_file(&audio).expect("valid lyrics").lines[0].text, "fixture");
    fs::write(&sidecar, b"[00:00]\xff").expect("invalid UTF-8 sidecar");
    assert!(find_lyrics_for_audio_file(&audio).is_none());
}
