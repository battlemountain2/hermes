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
