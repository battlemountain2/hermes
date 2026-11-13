use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::glib;

use crate::{
    app::{Browser, BrowserEvent, Trails},
    model::{
        Location, Trail, TrailBrowserDensity, TrailBrowserMode, TrailId, TrailViewState,
        ViewPreferences,
    },
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod fallback_tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Workspace {
    Files,
    Music,
}

pub(super) struct Selection {
    pub trail: Trail,
    pub workspace: Workspace,
}

pub(super) fn fresh_view(
    browser_mode: TrailBrowserMode,
    density: TrailBrowserDensity,
    show_hidden: bool,
) -> TrailViewState {
    TrailViewState {
        browser_mode,
        density,
        preferences: ViewPreferences {
            show_hidden,
            ..ViewPreferences::default()
        },
        sidebar_open: true,
        ..TrailViewState::default()
    }
}

fn usable(location: &Location) -> bool {
    location.is_absolute_native()
        || location
            .uri_value()
            .is_some_and(|uri| !uri.trim().is_empty())
}

pub(super) fn select(
    trails: &Trails,
    requested: Option<Location>,
    preference: &str,
    home: &Location,
    fresh_view: &TrailViewState,
) -> Selection {
    let workspace = if requested.is_none() && preference == "music" {
        Workspace::Music
    } else {
        Workspace::Files
    };
    let trail = if let Some(location) = requested {
        select_location(
            trails,
            if usable(&location) { &location } else { home },
            fresh_view,
        )
    } else if matches!(preference, "last" | "music") {
        let saved = trails.all();
        let active = trails.active_id();
        let available = |trail: &&Trail| trail.active_location().is_some_and(usable);
        let saved = saved
            .iter()
            .filter(available)
            .find(|trail| Some(&trail.id) == active.as_ref())
            .or_else(|| saved.iter().find(available));
        if let Some(trail) = saved {
            activate(trails, trail);
            trail.clone()
        } else {
            select_location(trails, home, fresh_view)
        }
    } else {
        select_location(trails, home, fresh_view)
    };
    Selection { trail, workspace }
}

fn activate(trails: &Trails, trail: &Trail) {
    if trails.active_id().as_ref() != Some(&trail.id)
        && let Err(error) = trails.activate(&trail.id)
    {
        tracing::warn!(%error, "unable to save startup Trail selection");
    }
}

fn select_location(trails: &Trails, location: &Location, view: &TrailViewState) -> Trail {
    let saved = trails.all();
    let active = trails.active_id();
    let matching = |trail: &&Trail| trail.active_location() == Some(location);
    if let Some(trail) = saved
        .iter()
        .filter(matching)
        .find(|trail| Some(&trail.id) == active.as_ref())
        .or_else(|| saved.iter().find(matching))
    {
        activate(trails, trail);
        return trail.clone();
    }
    if let Err(error) = trails.create(location.display_name(), location.clone(), view.clone()) {
        tracing::warn!(%error, "unable to save startup Trail");
    }
    trails
        .all()
        .into_iter()
        .find(|trail| Some(&trail.id) == trails.active_id().as_ref())
        .expect("a nonempty startup location creates an active Trail even if saving fails")
}

pub(super) struct Restore {
    target: Option<(TrailId, Location)>,
}

impl Restore {
    pub fn new(trail: &Trail) -> Self {
        Self {
            target: trail
                .active_location()
                .map(|location| (trail.id.clone(), location.clone())),
        }
    }

    pub fn preserves(&mut self, active: Option<&TrailId>, location: Option<&Location>) -> bool {
        let matches = self
            .target
            .as_ref()
            .is_some_and(|(id, target)| active == Some(id) && location == Some(target));
        if !matches {
            self.target = None;
        }
        matches
    }

    pub fn complete(
        &mut self,
        active: Option<&TrailId>,
        location: Option<&Location>,
        failed: bool,
        home: &Location,
    ) -> bool {
        let matches = self.preserves(active, location);
        self.target = None;
        matches && failed && location != Some(home)
    }
}

pub(super) fn install_fallback(
    browser: &Rc<Browser>,
    trails: Rc<Trails>,
    restore: Rc<RefCell<Restore>>,
    home: Location,
    fresh_view: TrailViewState,
    activate: Rc<dyn Fn(Trail)>,
) {
    let weak_browser = Rc::downgrade(browser);
    let navigation_generation = Rc::new(Cell::new(0u64));
    browser.observe(move |event| {
        if matches!(
            event,
            BrowserEvent::Reset
                | BrowserEvent::ColumnAdded { .. }
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::ColumnReloaded { .. }
                | BrowserEvent::TabRestored { .. }
        ) {
            navigation_generation.set(navigation_generation.get().wrapping_add(1));
            return;
        }
        let failed = match event {
            BrowserEvent::LoadFailed { depth, .. } => Some((depth, true)),
            BrowserEvent::LoadFinished { depth } => Some((depth, false)),
            _ => None,
        };
        let Some((depth, failed)) = failed else {
            return;
        };
        let Some(browser) = weak_browser.upgrade() else {
            return;
        };
        if browser.active_depth() != Some(depth) {
            return;
        }
        let active = trails.active_id();
        let location = browser.active_location();
        if !restore
            .borrow_mut()
            .complete(active.as_ref(), location.as_ref(), failed, &home)
        {
            return;
        }
        let weak_browser = weak_browser.clone();
        let trails = trails.clone();
        let home = home.clone();
        let fresh_view = fresh_view.clone();
        let activate = Rc::downgrade(&activate);
        let navigation_generation = navigation_generation.clone();
        let expected_generation = navigation_generation.get();
        glib::idle_add_local_once(move || {
            let Some(activate) = activate.upgrade() else {
                return;
            };
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            if navigation_generation.get() != expected_generation
                || trails.active_id() != active
                || browser.active_location() != location
            {
                return;
            }
            activate(select_location(&trails, &home, &fresh_view));
        });
    });
}
