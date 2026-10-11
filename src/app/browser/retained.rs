use std::time::Instant;

use super::*;
use crate::app::navigation::LoadState;

const MAX_RETAINED_TABS: usize = 6;
const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;
const REVALIDATE_AFTER: Duration = Duration::from_secs(30);

pub(super) struct RetainedTab {
    id: TrailId,
    state: NavigationState,
    preferences: ViewPreferences,
    loads: Vec<LoadHandle>,
    guards: Vec<LoadHandle>,
    dirty: Rc<Cell<bool>>,
    saved: Instant,
    bytes: usize,
}

impl Browser {
    pub fn activate_tab(
        self: &Rc<Self>,
        id: TrailId,
        location: Location,
        preferences: ViewPreferences,
    ) {
        if self.active_tab.borrow().as_ref() == Some(&id) {
            return;
        }
        self.close_peek();
        self.monitor_epoch
            .set(self.monitor_epoch.get().wrapping_add(1));
        let target = {
            let mut tabs = self.retained_tabs.borrow_mut();
            tabs.iter()
                .position(|tab| tab.id == id)
                .map(|index| tabs.remove(index))
        };
        self.store_active_tab();
        if let Some((_, depth)) = self.pending_sort.take() {
            self.emit(BrowserEvent::SortingFinished { depth });
        }
        self.active_tab.replace(Some(id.clone()));
        if let Some(tab) = target {
            if tab.state.active_location().as_ref() == Some(&location) {
                let changed_preferences = !tab.state.retained_preferences_match(preferences);
                let reload = tab.dirty.get()
                    || tab.saved.elapsed() >= REVALIDATE_AFTER
                    || changed_preferences;
                let RetainedTab {
                    state,
                    preferences: stored_preferences,
                    loads,
                    guards,
                    ..
                } = tab;
                self.state.replace(state);
                self.preferences.set(stored_preferences);
                self.loads.replace(loads);
                let locations: Vec<_> = self
                    .state
                    .borrow()
                    .columns
                    .iter()
                    .map(|column| column.location.clone())
                    .collect();
                let monitors = locations
                    .iter()
                    .enumerate()
                    .map(|(depth, location)| self.install_monitor(depth, location.clone()))
                    .collect();
                self.monitors.replace(monitors);
                drop(guards);
                self.emit(BrowserEvent::TabRestored { id });
                if changed_preferences {
                    self.set_preferences(preferences);
                }
                if reload {
                    for depth in 0..locations.len() {
                        self.refresh_column(depth);
                    }
                }
                return;
            }
            self.emit(BrowserEvent::TabDiscarded { id: tab.id.clone() });
        }
        self.state.replace(NavigationState::default());
        self.set_preferences(preferences);
        self.navigate(location);
    }

    pub fn retain_open_tabs(&self, ids: &[TrailId]) {
        let removed = {
            let mut tabs = self.retained_tabs.borrow_mut();
            let mut removed = Vec::new();
            let mut index = 0;
            while index < tabs.len() {
                if ids.contains(&tabs[index].id) {
                    index += 1;
                } else {
                    removed.push(tabs.remove(index));
                }
            }
            removed
        };
        for tab in removed {
            self.emit(BrowserEvent::TabDiscarded { id: tab.id.clone() });
        }
    }

    fn store_active_tab(&self) {
        let Some(id) = self.active_tab.borrow().clone() else {
            self.loads.borrow_mut().clear();
            self.monitors.borrow_mut().clear();
            return;
        };
        let dirty = Rc::new(Cell::new(false));
        let mut guards = Vec::new();
        let bytes = {
            let state = self.state.borrow();
            if self.pending_sort.get().is_some()
                || state.columns.is_empty()
                || state.columns.iter().any(|column| {
                    column.location.native_path().is_none()
                        || !matches!(column.load_state, LoadState::Ready | LoadState::Empty)
                })
            {
                None
            } else {
                state.columns.iter().try_fold(0usize, |bytes, column| {
                    let invalidated = dirty.clone();
                    guards.push(self.source.watch_invalidation(
                        &column.location,
                        Rc::new(move || invalidated.set(true)),
                    )?);
                    let bytes = bytes.saturating_add(column.entries.iter().fold(
                        8192usize,
                        |sum, entry| {
                            sum.saturating_add(4096)
                                .saturating_add(entry.display_name.len().saturating_mul(4))
                                .saturating_add(entry.native_name.capacity())
                                .saturating_add(
                                    entry
                                        .location
                                        .native_path()
                                        .map_or(0, |path| path.as_os_str().len())
                                        .saturating_mul(4),
                                )
                                .saturating_add(entry.music_tags.as_ref().map_or(0, |tags| {
                                    [&tags.title, &tags.artist, &tags.album_artist, &tags.album]
                                        .into_iter()
                                        .flatten()
                                        .map(String::capacity)
                                        .sum::<usize>()
                                }))
                        },
                    ));
                    (bytes <= MAX_RETAINED_BYTES).then_some(bytes)
                })
            }
        };
        self.monitors.borrow_mut().clear();
        let loads = self.loads.take();
        if let Some(bytes) = bytes {
            self.emit(BrowserEvent::TabStored { id: id.clone() });
            let tab = RetainedTab {
                id,
                state: self.state.take(),
                preferences: self.preferences.get(),
                loads,
                guards,
                dirty,
                saved: Instant::now(),
                bytes,
            };
            let removed = {
                let mut tabs = self.retained_tabs.borrow_mut();
                tabs.push(tab);
                let mut removed = Vec::new();
                while tabs.len() > MAX_RETAINED_TABS
                    || tabs
                        .iter()
                        .fold(0usize, |sum, tab| sum.saturating_add(tab.bytes))
                        > MAX_RETAINED_BYTES
                {
                    removed.push(tabs.remove(0));
                }
                removed
            };
            for tab in removed {
                self.emit(BrowserEvent::TabDiscarded { id: tab.id.clone() });
            }
        }
    }

    pub(super) fn invalidate_retained_request(&self, event: &DirectoryEvent) -> bool {
        let request_id = match event {
            DirectoryEvent::Restarted { request_id }
            | DirectoryEvent::Snapshot { request_id, .. }
            | DirectoryEvent::MusicMetadata { request_id, .. }
            | DirectoryEvent::Batch { request_id, .. }
            | DirectoryEvent::Finished { request_id }
            | DirectoryEvent::Failed { request_id, .. } => *request_id,
        };
        let tabs = self.retained_tabs.borrow();
        if let Some(tab) = tabs.iter().find(|tab| {
            (0..tab.state.columns.len()).any(|depth| tab.state.accepts_request(depth, request_id))
        }) {
            if !matches!(event, DirectoryEvent::Finished { .. }) {
                tab.dirty.set(true);
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests;
