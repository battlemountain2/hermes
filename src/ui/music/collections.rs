use super::*;
use crate::services::music_collections::Collections;

mod editor;

pub(super) struct PendingObservation {
    paths: Vec<PathBuf>,
    next: usize,
    timestamps: Vec<u64>,
}

impl PendingObservation {
    fn new(paths: Vec<PathBuf>, timestamp: u64) -> Self {
        Self {
            timestamps: vec![timestamp; paths.len()],
            paths,
            next: 0,
        }
    }

    fn with_latest(self, paths: Vec<PathBuf>, timestamp: u64) -> Self {
        let previous: std::collections::BTreeMap<_, _> = self
            .paths
            .into_iter()
            .zip(self.timestamps)
            .skip(self.next)
            .collect();
        let mut entries: Vec<_> = paths
            .into_iter()
            .map(|path| (previous.get(&path).copied().unwrap_or(timestamp), path))
            .collect();
        entries.sort();
        let (timestamps, paths) = entries.into_iter().unzip();
        Self {
            paths,
            timestamps,
            next: 0,
        }
    }

    fn try_submit(
        &mut self,
        submit: impl FnOnce(Vec<PathBuf>, u64) -> Result<(), String>,
    ) -> Result<bool, String> {
        if self.next == self.paths.len() {
            return Ok(true);
        }
        let timestamp = self.timestamps[self.next];
        let end = (self.next..(self.next + 512).min(self.paths.len()))
            .take_while(|index| self.timestamps[*index] == timestamp)
            .last()
            .map_or(self.next, |index| index + 1);
        submit(self.paths[self.next..end].to_vec(), timestamp)?;
        self.next = end;
        Ok(self.next == self.paths.len())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Section {
    Albums,
    Artists,
    Songs,
    Favorites,
    Playlists,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Group {
    Artist(String),
    Playlist(u64),
    RecentlyAdded,
    MostPlayed,
}

pub(super) fn connect(state: &Rc<State>, create: &gtk::Button, edit: &gtk::Button) {
    for (section, control) in &state.navigation {
        let section = *section;
        let weak = Rc::downgrade(state);
        control.connect_toggled(move |control| {
            if !control.is_active() {
                return;
            }
            if let Some(state) = weak.upgrade() {
                state.section.set(section);
                state.album_scroll.set_visible(section == Section::Albums);
                state
                    .group_scroll
                    .set_visible(matches!(section, Section::Artists | Section::Playlists));
                state
                    .playlist_editor
                    .set_visible(section == Section::Playlists);
                state.filter.set_text("");
                state.render_groups();
                state.render_tracks();
            }
        });
    }
    let weak = Rc::downgrade(state);
    state.group_selection.connect_selected_notify(move |_| {
        if let Some(state) = weak.upgrade()
            && !state.rendering_library.get()
        {
            state.render_tracks();
        }
    });
    let weak = Rc::downgrade(state);
    create.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            editor::open(&state, None);
        }
    });
    let weak = Rc::downgrade(state);
    edit.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            if let Some(id) = state.current_playlist() {
                editor::open(&state, Some(id));
            } else {
                state.collection_message(
                    "Select a manual playlist to edit; smart collections update automatically",
                );
            }
        }
    });
}

impl State {
    pub(super) fn observe_library_paths(&self, tracks: &[Track]) {
        let paths: std::collections::BTreeSet<_> =
            tracks.iter().map(|track| track.path.clone()).collect();
        if *self.catalog_paths.borrow() == paths {
            return;
        }
        let latest = paths.iter().cloned().collect();
        let timestamp = crate::services::music_collections::now();
        let previous = self.pending_observation.borrow_mut().take();
        let observation = match previous {
            Some(previous) => previous.with_latest(latest, timestamp),
            None => PendingObservation::new(latest, timestamp),
        };
        self.pending_observation.replace(Some(observation));
        self.catalog_paths.replace(paths);
    }

    pub(super) fn retry_observation(&self) {
        let mut pending = self.pending_observation.borrow_mut();
        let Some(observation) = pending.as_mut() else {
            return;
        };
        let completed = match observation.try_submit(|paths, timestamp| {
            self.collection_store
                .send(CollectionCommand::Observe(paths, timestamp))
        }) {
            Ok(true) => {
                pending.take();
                true
            }
            Ok(false) => false,
            Err(error) => {
                self.collection_message(&format!("Recently added update is waiting: {error}"));
                false
            }
        };
        drop(pending);
        if completed {
            self.collection_hint(self.tracks.borrow().len());
        }
    }

    pub(super) fn track_playback(&self, selected: &std::path::Path) -> Vec<Track> {
        if !self
            .library
            .borrow()
            .iter()
            .any(|track| track.path == selected)
        {
            return Vec::new();
        }
        if let Some(id) = self.current_playlist() {
            let mut tracks = visible_tracks(
                &self.library.borrow(),
                Section::Playlists,
                None,
                Some(&Group::Playlist(id)),
                &self.collection_snapshot.borrow().data,
                "",
            );
            let available: std::collections::BTreeSet<_> = self
                .library
                .borrow()
                .iter()
                .map(|track| track.path.clone())
                .collect();
            tracks.retain(|track| available.contains(&track.path));
            if let Some(position) = tracks.iter().position(|track| track.path == selected) {
                tracks.drain(..position);
                return tracks;
            }
            return Vec::new();
        }
        let filtered = !self.filter.text().is_empty() || self.section.get() != Section::Albums;
        if filtered {
            let mut tracks = self.available_tracks();
            if let Some(position) = tracks.iter().position(|track| track.path == selected) {
                tracks.drain(..position);
                return tracks;
            }
        }
        if let Some(album_path) = self.selected_album.borrow().as_ref() {
            let mut tracks = super::album_tracks(&self.library.borrow(), album_path);
            if let Some(position) = tracks.iter().position(|track| track.path == selected) {
                tracks.drain(..position);
                return tracks;
            }
        }
        crate::music::folder_queue(self.library.borrow().clone(), selected)
    }

    pub(super) fn available_tracks(&self) -> Vec<Track> {
        let available: std::collections::BTreeSet<_> = self
            .library
            .borrow()
            .iter()
            .map(|track| track.path.clone())
            .collect();
        self.tracks
            .borrow()
            .iter()
            .filter(|track| available.contains(&track.path))
            .cloned()
            .collect()
    }

    pub(super) fn current_group(&self) -> Option<Group> {
        self.groups
            .borrow()
            .get(self.group_selection.selected() as usize)
            .cloned()
    }

    pub(super) fn current_playlist(&self) -> Option<u64> {
        if self.section.get() != Section::Playlists {
            return None;
        }
        match self.current_group()? {
            Group::Playlist(id) => Some(id),
            _ => None,
        }
    }

    pub(super) fn collection_message(&self, message: &str) {
        self.collection_status.set_text(message);
        self.collection_status.set_visible(!message.is_empty());
    }

    pub(super) fn collection_command(&self, command: CollectionCommand) {
        if let Err(error) = self.collection_store.send(command) {
            self.collection_message(&error);
        }
    }

    pub(super) fn poll_collections(&self) {
        let snapshot = self.collection_store.snapshot();
        if snapshot.revision == self.collection_snapshot.borrow().revision {
            return;
        }
        let created =
            snapshot.data.playlists.len() > self.collection_snapshot.borrow().data.playlists.len();
        let latest = snapshot.data.playlists.last().map(|playlist| playlist.id);
        self.collection_snapshot.replace(snapshot);
        let favorites = self.collection_snapshot.borrow().data.favorite_paths();
        self.favorite_buttons.borrow_mut().retain(|(button, path)| {
            if let Some(button) = button.upgrade() {
                let active = favorites.contains(path);
                button.set_active(active);
                if let Some(image) = button.child().and_downcast::<gtk::Image>() {
                    assets::set_primary_icon(
                        &image,
                        if active {
                            icons::HEART_SELECTED
                        } else {
                            icons::HEART
                        },
                    );
                }
                button.update_property(&[gtk::accessible::Property::Label(if active {
                    "Remove song from favorites"
                } else {
                    "Add song to favorites"
                })]);
                true
            } else {
                false
            }
        });
        self.favorite_paths.replace(favorites);
        self.render_groups();
        if created && self.section.get() == Section::Playlists {
            let position = self
                .groups
                .borrow()
                .iter()
                .position(|group| Some(group) == latest.map(Group::Playlist).as_ref());
            if let Some(position) = position {
                self.group_selection.set_selected(position as u32);
            }
        }
        self.render_tracks();
    }

    pub(super) fn render_groups(&self) {
        if self.section.get() == Section::Artists
            && self.group_section.get() == Some(Section::Artists)
            && self.group_library_revision.get() == self.library_revision.get()
        {
            return;
        }
        let selected = self.current_group();
        let snapshot = self.collection_snapshot.borrow();
        let (groups, labels): (Vec<_>, Vec<_>) = match self.section.get() {
            Section::Artists => {
                let mut artists = std::collections::BTreeMap::<String, usize>::new();
                for track in self.library.borrow().iter() {
                    *artists.entry(track.artist().to_owned()).or_default() += 1;
                }
                artists
                    .into_iter()
                    .map(|(artist, count)| {
                        let label = format!("{artist}\n{count} songs");
                        (Group::Artist(artist), label)
                    })
                    .unzip()
            }
            Section::Playlists => snapshot
                .data
                .playlists
                .iter()
                .map(|playlist| {
                    (
                        Group::Playlist(playlist.id),
                        format!("{}\n{} songs", playlist.name, playlist.tracks.len()),
                    )
                })
                .chain([
                    (
                        Group::RecentlyAdded,
                        "Recently added\nFirst seen in Music".into(),
                    ),
                    (
                        Group::MostPlayed,
                        "Most played\nMeaningful main-player listening".into(),
                    ),
                ])
                .unzip(),
            _ => (Vec::new(), Vec::new()),
        };
        self.group_section.set(Some(self.section.get()));
        self.group_library_revision.set(self.library_revision.get());
        if *self.groups.borrow() == groups && *self.group_labels.borrow() == labels {
            return;
        }
        let position = selected
            .and_then(|selected| groups.iter().position(|group| group == &selected))
            .or_else(|| (!groups.is_empty()).then_some(0));
        self.rendering_library.set(true);
        self.groups.replace(groups);
        replace_model(&self.group_model, &labels);
        self.group_labels.replace(labels);
        self.group_selection
            .set_selected(position.map_or(gtk::INVALID_LIST_POSITION, |position| position as u32));
        self.rendering_library.set(false);
    }

    pub(super) fn collection_hint(&self, visible: usize) {
        let snapshot = self.collection_snapshot.borrow();
        if let Some(error) = &snapshot.error {
            self.collection_message(&format!(
                "Collections: {error} · Your audio files are unchanged"
            ));
        } else if !snapshot.ready {
            self.collection_message("Loading saved favorites and playlists…");
        } else if let Some(id) = self.current_playlist() {
            let library = self.library.borrow();
            let available: std::collections::BTreeSet<_> =
                library.iter().map(|track| track.path.as_path()).collect();
            let playlist = snapshot.data.playlist(id);
            let missing = playlist.map_or(0, |playlist| {
                playlist
                    .tracks
                    .iter()
                    .filter(|track| {
                        track
                            .native_path()
                            .is_none_or(|path| !available.contains(path))
                    })
                    .count()
            });
            self.collection_message(&format!("{visible} entries shown · {missing} unavailable in the current library · Edit playlist to rename, add, remove or reorder. Unavailable paths stay saved; playback skips them. Rename matching is manual."));
        } else {
            self.collection_message(match (self.section.get(), visible) {
                (Section::Favorites, 0) => "No favorites shown · Use a song's heart or right-click a song or album to add favorites.",
                (Section::Favorites, _) => "Unavailable paths remain saved; playback skips them. Use the heart to remove a favorite. Renamed paths must be added manually.",
                (Section::Playlists, _) if self.current_group() == Some(Group::MostPlayed) => "Most played counts once after 30 seconds of actual main-player listening, or half a shorter track. Previews, pause time and seeking do not count. Missing or renamed paths are not matched automatically.",
                (Section::Playlists, _) => "Recently added uses when Hermes first indexes a path, not its modification date. First-import dates are tied; missing or renamed paths are not matched automatically.",
                _ => "",
            });
        }
    }
}

pub(super) fn visible_tracks(
    library: &[Track],
    section: Section,
    album: Option<&std::path::Path>,
    group: Option<&Group>,
    collections: &Collections,
    query: &str,
) -> Vec<Track> {
    let query = query.to_lowercase();
    let favorites = collections.favorite_paths();
    let mut tracks: Vec<_> = match (section, group) {
        (Section::Favorites, _) => {
            let lookup: std::collections::BTreeMap<_, _> = library
                .iter()
                .map(|track| (track.path.as_path(), track))
                .collect();
            favorites
                .iter()
                .map(|path| saved_track(path, &lookup))
                .collect()
        }
        (Section::Playlists, Some(Group::Playlist(id))) => {
            let lookup: std::collections::BTreeMap<_, _> = library
                .iter()
                .map(|track| (track.path.as_path(), track))
                .collect();
            collections
                .playlist(*id)
                .into_iter()
                .flat_map(|playlist| &playlist.tracks)
                .filter_map(|path| {
                    let path = path.native_path()?;
                    Some(saved_track(path, &lookup))
                })
                .collect()
        }
        _ => library
            .iter()
            .filter(|track| match section {
                Section::Albums => {
                    !query.is_empty() || album.is_some_and(|album| track.album_key() == album)
                }
                Section::Artists => {
                    matches!(group, Some(Group::Artist(artist)) if artist == track.artist())
                }
                Section::Songs => true,
                Section::Favorites => favorites.contains(&track.path),
                Section::Playlists => {
                    matches!(group, Some(Group::RecentlyAdded | Group::MostPlayed))
                }
            })
            .cloned()
            .collect(),
    };
    tracks.retain(|track| {
        let metadata = format!(
            "{} {} {} {}",
            track.title(),
            track.artist(),
            track.album(),
            track.path.to_string_lossy()
        )
        .to_lowercase();
        query.split_whitespace().all(|term| metadata.contains(term))
    });
    if section == Section::Playlists && matches!(group, Some(Group::RecentlyAdded)) {
        let dates = collections.added_dates();
        tracks.retain(|track| dates.contains_key(&track.path));
        tracks.sort_by(|left, right| {
            dates[&right.path]
                .cmp(&dates[&left.path])
                .then_with(|| left.path.cmp(&right.path))
        });
    } else if section == Section::Playlists && matches!(group, Some(Group::MostPlayed)) {
        let counts = collections.play_counts();
        tracks.retain(|track| counts.contains_key(&track.path));
        tracks.sort_by(|left, right| {
            counts[&right.path]
                .cmp(&counts[&left.path])
                .then_with(|| left.path.cmp(&right.path))
        });
    } else if section == Section::Songs || section == Section::Favorites {
        tracks.sort_by_cached_key(|track| {
            (
                track.title().to_lowercase(),
                track.artist().to_lowercase(),
                track.path.clone(),
            )
        });
    } else if section != Section::Playlists {
        tracks.sort_by(|left, right| {
            left.album_key()
                .cmp(&right.album_key())
                .then_with(|| crate::music::compare_tracks(left, right))
        });
    }
    tracks
}

fn saved_track(
    path: &std::path::Path,
    lookup: &std::collections::BTreeMap<&std::path::Path, &Track>,
) -> Track {
    lookup.get(path).map_or_else(
        || Track {
            path: path.to_owned(),
            tags: crate::model::MusicTags {
                title: Some(format!(
                    "Unavailable: {}",
                    path.file_stem().unwrap_or_default().to_string_lossy()
                )),
                ..Default::default()
            },
        },
        |track| (*track).clone(),
    )
}

#[cfg(test)]
mod tests;
