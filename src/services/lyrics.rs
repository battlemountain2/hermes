// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fs, io::Read, path::Path, time::Duration};

#[cfg(test)]
mod tests;

const MAX_LRC_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 4096;
const MAX_LYRIC_LINES: usize = 1024;
const MAX_TIMESTAMPS_PER_LINE: usize = 32;
const MAX_LYRIC_TEXT_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LyricLine {
    pub timestamp: Duration,
    pub text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
}

impl Lyrics {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn active_line_at(&self, position: Duration) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let next_idx = self
            .lines
            .partition_point(|line| line.timestamp <= position);
        if next_idx == 0 {
            Some(0)
        } else {
            Some(next_idx - 1)
        }
    }
}

pub fn parse_lrc(content: &str) -> Option<Lyrics> {
    if content.len() > MAX_LRC_BYTES {
        return None;
    }
    let mut lines = Vec::new();
    let mut text_bytes = 0usize;

    for raw_line in content.lines() {
        if raw_line.len() > MAX_LINE_BYTES {
            return None;
        }
        let trimmed = raw_line.trim();
        if !trimmed.starts_with('[') {
            continue;
        }

        let mut remaining = trimmed;
        let mut timestamps = Vec::new();

        while let Some(close_bracket) = remaining.find(']') {
            if !remaining.starts_with('[') {
                break;
            }
            let tag = &remaining[1..close_bracket];
            if let Some(timestamp) = parse_timestamp(tag) {
                if timestamps.len() == MAX_TIMESTAMPS_PER_LINE {
                    return None;
                }
                timestamps.push(timestamp);
            }
            remaining = remaining[close_bracket + 1..].trim_start();
        }

        let text = remaining.trim();
        if lines.len() + timestamps.len() > MAX_LYRIC_LINES {
            return None;
        }
        text_bytes = text_bytes.checked_add(text.len().checked_mul(timestamps.len())?)?;
        if text_bytes > MAX_LYRIC_TEXT_BYTES {
            return None;
        }
        for timestamp in timestamps {
            lines.push(LyricLine {
                timestamp,
                text: text.to_owned(),
            });
        }
    }

    if lines.is_empty() {
        return None;
    }

    lines.sort_by_key(|line| line.timestamp);
    Some(Lyrics { lines })
}

fn parse_timestamp(tag: &str) -> Option<Duration> {
    let (min_str, rest) = tag.split_once(':')?;
    if min_str.is_empty() || !min_str.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let minutes: u64 = min_str.parse().ok()?;

    let (sec_str, millis) = if let Some((sec_part, frac_part)) = rest.split_once('.') {
        if frac_part.is_empty() || !frac_part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let mut digits = frac_part.bytes();
        let millis = [100u64, 10, 1].into_iter().fold(0, |total, weight| {
            total + digits.next().map_or(0, |digit| u64::from(digit - b'0') * weight)
        });
        (sec_part, millis)
    } else {
        (rest, 0)
    };

    if sec_str.is_empty() || !sec_str.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let seconds: u64 = sec_str.parse().ok()?;
    if seconds >= 60 {
        return None;
    }
    let total = minutes
        .checked_mul(60_000)?
        .checked_add(seconds.checked_mul(1_000)?)?
        .checked_add(millis)?;
    if total > i64::MAX as u64 / 1_000 {
        return None;
    }
    Some(Duration::from_millis(total))
}

pub fn find_lyrics_for_audio_file(audio_path: &Path) -> Option<Lyrics> {
    let lrc_path = audio_path.with_extension("lrc");
    let metadata = fs::metadata(&lrc_path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_LRC_BYTES as u64 {
        return None;
    }
    let file = fs::File::open(&lrc_path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_LRC_BYTES as u64 {
        return None;
    }
    read_lrc(file)
}

fn read_lrc(reader: impl Read) -> Option<Lyrics> {
    let mut content = String::new();
    reader.take(MAX_LRC_BYTES as u64 + 1)
        .read_to_string(&mut content)
        .ok()?;
    parse_lrc(&content)
}
