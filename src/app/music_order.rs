use std::cmp::Ordering;

use crate::{
    model::FileEntry,
    services::{FormatFamily, classify_by_name},
};

#[cfg(test)]
pub(crate) mod tests;

pub(crate) fn is_audio(entry: &FileEntry) -> bool {
    !entry.is_directory() && classify_by_name(&entry.native_name) == FormatFamily::Audio
}

pub(super) fn is_album(entries: &[FileEntry]) -> bool {
    let mut tracks = 0;
    let mut album: Option<String> = None;
    for entry in entries {
        if entry.is_directory() {
            return false;
        }
        if is_audio(entry) {
            tracks += 1;
            if let Some(name) = entry
                .music_tags
                .as_ref()
                .and_then(|tags| tags.album.as_ref())
            {
                let name = name.trim().to_lowercase();
                if !name.is_empty() {
                    if album.as_ref().is_some_and(|previous| previous != &name) {
                        return false;
                    }
                    album = Some(name);
                }
            }
        } else {
            let artwork = matches!(
                classify_by_name(&entry.native_name),
                FormatFamily::Image
                    | FormatFamily::Heif
                    | FormatFamily::RawImage
                    | FormatFamily::Svg
                    | FormatFamily::Pdf
            );
            let companion = entry
                .location
                .native_path()
                .and_then(|path| path.extension())
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    ["cue", "m3u", "m3u8", "nfo", "log", "txt", "lrc"]
                        .iter()
                        .any(|allowed| extension.eq_ignore_ascii_case(allowed))
                });
            if !artwork && !companion {
                return false;
            }
        }
    }
    tracks >= 2
}

pub(super) fn compare_tracks(left: &FileEntry, right: &FileEntry) -> Ordering {
    let audio_order = is_audio(right).cmp(&is_audio(left));
    if audio_order != Ordering::Equal {
        return audio_order;
    }
    if is_audio(left) {
        let numbers = |entry: &FileEntry| {
            let tags = entry.music_tags.as_ref();
            let disc = tags
                .and_then(|tags| tags.disc)
                .filter(|number| *number > 0)
                .unwrap_or(1);
            let track = tags
                .and_then(|tags| tags.track)
                .filter(|number| *number > 0)
                .or_else(|| filename_track(&entry.display_name))
                .unwrap_or(u32::MAX);
            (disc, track)
        };
        let order = numbers(left).cmp(&numbers(right));
        if order != Ordering::Equal {
            return order;
        }
    }
    natural_name(&left.display_name, &right.display_name)
        .then_with(|| left.location.compare(&right.location))
}

fn filename_track(name: &str) -> Option<u32> {
    let digits: String = name
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    let suffix = name.trim_start().get(digits.len()..)?;
    if !suffix.starts_with(['.', '-', '_', ' ']) {
        return None;
    }
    digits
        .parse::<u32>()
        .ok()
        .filter(|number| *number > 0 && *number <= 1_000_000)
}

fn natural_name(left: &str, right: &str) -> Ordering {
    let left_lower = left.to_lowercase();
    let right_lower = right.to_lowercase();
    let mut left_chars = left_lower.chars().peekable();
    let mut right_chars = right_lower.chars().peekable();
    loop {
        match (left_chars.peek().copied(), right_chars.peek().copied()) {
            (Some(left_character), Some(right_character))
                if left_character.is_ascii_digit() && right_character.is_ascii_digit() =>
            {
                let left_number: String =
                    std::iter::from_fn(|| left_chars.next_if(char::is_ascii_digit)).collect();
                let right_number: String =
                    std::iter::from_fn(|| right_chars.next_if(char::is_ascii_digit)).collect();
                let left_number = left_number.trim_start_matches('0');
                let right_number = right_number.trim_start_matches('0');
                let order = left_number
                    .len()
                    .cmp(&right_number.len())
                    .then_with(|| left_number.cmp(right_number));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(left_character), Some(right_character)) => {
                let order = left_character.cmp(&right_character);
                if order != Ordering::Equal {
                    return order;
                }
                left_chars.next();
                right_chars.next();
            }
            (None, None) => return left.cmp(right),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}
