// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fs, path::Path, time::Duration};

#[cfg(test)]
mod tests;

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
    let mut lines = Vec::new();

    for raw_line in content.lines() {
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
                timestamps.push(timestamp);
            }
            remaining = remaining[close_bracket + 1..].trim_start();
        }

        let text = remaining.trim().to_owned();
        for timestamp in timestamps {
            lines.push(LyricLine {
                timestamp,
                text: text.clone(),
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
    let minutes: u64 = min_str.parse().ok()?;

    let (sec_str, millis) = if let Some((sec_part, frac_part)) = rest.split_once('.') {
        let seconds: u64 = sec_part.parse().ok()?;
        let millis: u64 = match frac_part.len() {
            1 => frac_part.parse::<u64>().ok()? * 100,
            2 => frac_part.parse::<u64>().ok()? * 10,
            3 => frac_part.parse::<u64>().ok()?,
            _ => frac_part[..3].parse::<u64>().ok()?,
        };
        (seconds, millis)
    } else {
        let seconds: u64 = rest.parse().ok()?;
        (seconds, 0)
    };

    Some(Duration::from_millis(
        minutes * 60_000 + sec_str * 1_000 + millis,
    ))
}

pub fn find_lyrics_for_audio_file(audio_path: &Path) -> Option<Lyrics> {
    let lrc_path = audio_path.with_extension("lrc");
    if lrc_path.is_file()
        && let Ok(content) = fs::read_to_string(&lrc_path)
        && let Some(lyrics) = parse_lrc(&content)
    {
        return Some(lyrics);
    }
    None
}
