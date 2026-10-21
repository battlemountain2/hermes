use std::{cmp::Ordering, path::PathBuf};

use crate::model::MusicTags;

pub(crate) mod library;
pub(crate) mod session;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Track {
    pub path: PathBuf,
    pub tags: MusicTags,
}

impl Track {
    pub(crate) fn needs_metadata(&self) -> bool {
        tagged_text(&self.tags.title).is_none()
            || (tagged_text(&self.tags.artist).is_none()
                && tagged_text(&self.tags.album_artist).is_none())
            || tagged_text(&self.tags.album).is_none()
    }

    pub(crate) fn hydrate_metadata(&mut self, tags: &MusicTags) -> bool {
        let before = self.tags.clone();
        for (current, incoming) in [
            (&mut self.tags.title, &tags.title),
            (&mut self.tags.artist, &tags.artist),
            (&mut self.tags.album_artist, &tags.album_artist),
            (&mut self.tags.album, &tags.album),
        ] {
            if tagged_text(current).is_none() {
                *current = tagged_text(incoming).map(str::to_owned);
            }
        }
        self.tags.disc = self.tags.disc.or(tags.disc.filter(|number| *number > 0));
        self.tags.track = self.tags.track.or(tags.track.filter(|number| *number > 0));
        self.tags != before
    }

    pub fn title(&self) -> String {
        tagged_text(&self.tags.title)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                self.path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
    }

    pub fn artist(&self) -> &str {
        tagged_text(&self.tags.artist)
            .or_else(|| tagged_text(&self.tags.album_artist))
            .unwrap_or("Unknown artist")
    }

    pub fn album(&self) -> String {
        tagged_text(&self.tags.album)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                self.path
                    .parent()
                    .and_then(|path| path.file_name())
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
    }

    pub fn album_key(&self) -> PathBuf {
        self.path.parent().unwrap_or(&self.path).to_owned()
    }
}

fn tagged_text(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn compare_tracks(left: &Track, right: &Track) -> Ordering {
    let number = |track: &Track| {
        let fallback = track.path.file_stem().unwrap_or_default().to_string_lossy();
        let prefix: String = fallback.chars().take_while(char::is_ascii_digit).collect();
        (
            track.tags.disc.unwrap_or(1),
            track
                .tags
                .track
                .or_else(|| prefix.parse().ok())
                .unwrap_or(u32::MAX),
        )
    };
    number(left)
        .cmp(&number(right))
        .then_with(|| left.path.cmp(&right.path))
}

pub(crate) fn folder_queue(tracks: Vec<Track>, selected: &std::path::Path) -> Vec<Track> {
    if selected.is_dir() || (selected.extension().is_none() && !selected.as_os_str().is_empty()) {
        let mut tracks: Vec<_> = tracks
            .into_iter()
            .filter(|track| track.path.starts_with(selected))
            .collect();
        tracks.sort_by(compare_tracks);
        return tracks;
    }
    let folder = selected.parent();
    let mut tracks: Vec<_> = tracks
        .into_iter()
        .filter(|track| track.path.parent() == folder)
        .collect();
    tracks.sort_by(compare_tracks);
    let Some(position) = tracks.iter().position(|track| track.path == selected) else {
        return Vec::new();
    };
    tracks.drain(..position);
    tracks
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Default)]
pub(crate) struct Queue {
    pub tracks: Vec<Track>,
    pub current: Option<usize>,
    pub repeat: Repeat,
    pub shuffle: bool,
    order: Vec<usize>,
    history: Vec<usize>,
    seed: u64,
}

impl Queue {
    pub fn move_before(&mut self, index: usize, destination: usize) -> bool {
        let Some(source) = self.order.iter().position(|entry| *entry == index) else {
            return false;
        };
        if destination > self.order.len() {
            return false;
        }
        let destination = destination - usize::from(source < destination);
        if source == destination {
            return false;
        }
        self.order.remove(source);
        self.order.insert(destination, index);
        true
    }

    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.tracks.len() {
            return false;
        }
        if self.current == Some(index) {
            self.current = self
                .order
                .iter()
                .position(|entry| *entry == index)
                .and_then(|position| self.order.get(position + 1).copied());
        }
        self.tracks.remove(index);
        self.order.retain(|entry| *entry != index);
        self.history.retain(|entry| *entry != index);
        for entry in self.order.iter_mut().chain(self.history.iter_mut()) {
            *entry -= usize::from(*entry > index);
        }
        self.current = self.current.map(|entry| entry - usize::from(entry > index));
        true
    }

    pub fn replace(&mut self, tracks: Vec<Track>, start: usize) {
        self.tracks = tracks;
        self.current = (start < self.tracks.len()).then_some(start);
        self.history.clear();
        self.reset_order();
    }

    pub fn append(&mut self, tracks: Vec<Track>) {
        let first = self.tracks.len();
        self.tracks.extend(tracks);
        self.order.extend(first..self.tracks.len());
    }

    pub fn play_next(&mut self, tracks: Vec<Track>) {
        let first = self.tracks.len();
        self.tracks.extend(tracks);
        let insert = self
            .current
            .and_then(|current| self.order.iter().position(|index| *index == current))
            .map_or(0, |position| position + 1);
        self.order.splice(insert..insert, first..self.tracks.len());
    }

    pub fn set_shuffle(&mut self, shuffle: bool, seed: u64) {
        self.shuffle = shuffle;
        self.seed = seed.max(1);
        self.reset_order();
    }

    fn reset_order(&mut self) {
        self.order = (0..self.tracks.len())
            .filter(|index| Some(*index) != self.current)
            .collect();
        if self.shuffle {
            for position in (1..self.order.len()).rev() {
                self.seed ^= self.seed << 13;
                self.seed ^= self.seed >> 7;
                self.seed ^= self.seed << 17;
                self.order
                    .swap(position, (self.seed as usize) % (position + 1));
            }
        } else if let Some(current) = self.current {
            let position = self
                .order
                .iter()
                .position(|index| *index > current)
                .unwrap_or(self.order.len());
            self.order.insert(position, current);
            return;
        }
        if let Some(current) = self.current {
            self.order.insert(0, current);
        }
    }

    pub fn can_advance(&self) -> bool {
        match self
            .current
            .and_then(|current| self.order.iter().position(|index| *index == current))
        {
            Some(position) => position + 1 < self.order.len() || self.repeat == Repeat::All,
            None => !self.order.is_empty(),
        }
    }

    pub fn advance(&mut self, natural_end: bool) -> Option<Track> {
        if natural_end && self.repeat == Repeat::One {
            return self.playing().cloned();
        }
        let position = self
            .current
            .and_then(|current| self.order.iter().position(|index| *index == current));
        let next = match position {
            Some(position) if position + 1 < self.order.len() => Some(self.order[position + 1]),
            Some(_) if self.repeat == Repeat::All => self.order.first().copied(),
            None => self.order.first().copied(),
            _ => None,
        };
        if let Some(previous) = self.current {
            self.history.push(previous);
            if self.history.len() > 1024 {
                self.history.remove(0);
            }
        }
        self.current = next;
        self.playing().cloned()
    }

    pub fn previous(&mut self) -> Option<Track> {
        let previous = self.history.pop().or_else(|| {
            let position = self
                .current
                .and_then(|current| self.order.iter().position(|index| *index == current))?;
            position.checked_sub(1).map(|position| self.order[position])
        })?;
        self.current = Some(previous);
        self.playing().cloned()
    }

    pub fn select(&mut self, index: usize) -> Option<Track> {
        if index >= self.tracks.len() {
            return None;
        }
        if let Some(previous) = self.current
            && previous != index
        {
            self.history.push(previous);
            if self.history.len() > 1024 {
                self.history.remove(0);
            }
        }
        self.current = Some(index);
        self.playing().cloned()
    }

    pub fn playing(&self) -> Option<&Track> {
        self.tracks.get(self.current?)
    }

    pub fn ordered(&self) -> Vec<(usize, &Track)> {
        self.order
            .iter()
            .filter_map(|index| Some((*index, self.tracks.get(*index)?)))
            .collect()
    }
}

#[cfg(test)]
mod tests;
