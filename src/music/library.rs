use std::path::PathBuf;

use super::Track;
use crate::services::music_library::Subscription;

pub(crate) struct Snapshot {
    pub tracks: Vec<Track>,
    pub indexing: bool,
    pub notice: Option<String>,
}

pub(crate) struct Scan {
    subscription: Subscription,
}

impl Scan {
    pub(crate) fn start(root: PathBuf) -> Self {
        Self {
            subscription: crate::services::music_library::subscribe(root),
        }
    }

    pub(crate) fn refresh(&self) {
        self.subscription.refresh();
    }

    pub(crate) fn poll(&mut self) -> Option<Snapshot> {
        let snapshot = self.subscription.poll()?;
        let mut tracks: Vec<_> = snapshot
            .entries
            .iter()
            .filter(|entry| entry.is_audio())
            .map(|entry| Track {
                path: entry.path.clone(),
                tags: entry.music_tags.clone().unwrap_or_default(),
            })
            .collect();
        tracks.sort_by(|left, right| {
            left.album_key()
                .cmp(&right.album_key())
                .then_with(|| super::compare_tracks(left, right))
        });
        Some(Snapshot {
            tracks,
            indexing: snapshot.indexing,
            notice: snapshot.notice.clone(),
        })
    }
}
