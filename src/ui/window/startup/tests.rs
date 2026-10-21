use std::{cell::RefCell, io, rc::Rc};

use crate::{
    model::{SortDirection, SortKey, TrailCollection},
    services::{StoredTrails, TrailStore},
};

use super::*;

#[derive(Default)]
struct MemoryStore {
    stored: RefCell<StoredTrails>,
    fail_saves: bool,
}

impl TrailStore for MemoryStore {
    fn load(&self) -> io::Result<StoredTrails> {
        Ok(self.stored.borrow().clone())
    }

    fn save(&self, stored: &StoredTrails) -> io::Result<()> {
        if self.fail_saves {
            return Err(io::Error::other("in-memory save failure"));
        }
        self.stored.replace(stored.clone());
        Ok(())
    }
}

fn home() -> Location {
    Location::local("/startup-fixture/home")
}

fn defaults() -> TrailViewState {
    fresh_view(
        TrailBrowserMode::Columns,
        TrailBrowserDensity::Compact,
        false,
    )
}

fn saved_trail(id: &str, location: Location) -> Trail {
    let mut trail = Trail::new(
        TrailId::new(id).expect("fixture identifier"),
        format!("Pinned {id}"),
        vec![Location::local("/startup-fixture/ancestor"), location],
    )
    .expect("fixture Trail");
    trail.custom_name = true;
    trail.pinned = true;
    trail.view = TrailViewState {
        browser_mode: TrailBrowserMode::Explorer,
        density: TrailBrowserDensity::Airy,
        preferences: ViewPreferences {
            show_hidden: true,
            folders_first: false,
            sort_key: SortKey::Size,
            sort_direction: SortDirection::Descending,
        },
        sidebar_open: false,
        preview_open: true,
    };
    trail
}

fn session(saved: Vec<Trail>, active: Option<TrailId>) -> (Rc<Trails>, Rc<MemoryStore>) {
    let store = Rc::new(MemoryStore {
        stored: RefCell::new(StoredTrails {
            collection: TrailCollection {
                trails: saved,
                active,
            },
            ..StoredTrails::default()
        }),
        fail_saves: false,
    });
    (Trails::load(store.clone()).expect("load fixture"), store)
}

#[test]
fn explicit_location_wins_over_every_startup_mode_and_preserves_saved_trails() {
    for preference in ["home", "music", "last", "unknown"] {
        let saved = saved_trail("saved", Location::local("/startup-fixture/projects"));
        let (trails, store) = session(vec![saved.clone()], Some(saved.id.clone()));
        let requested = Location::local("/startup-fixture/requested");

        let selected = select(
            &trails,
            Some(requested.clone()),
            preference,
            &home(),
            &defaults(),
        );

        assert_eq!(selected.workspace, Workspace::Files);
        assert_eq!(selected.trail.active_location(), Some(&requested));
        assert_eq!(selected.trail.view, defaults());
        assert_eq!(trails.active_id(), Some(selected.trail.id.clone()));
        assert_eq!(store.stored.borrow().collection.trails[0], saved);
        assert_eq!(store.stored.borrow().collection.trails.len(), 2);
    }
}

#[test]
fn explicit_location_reuses_the_matching_trail_without_resetting_its_view() {
    let target = saved_trail("target", Location::local("/startup-fixture/requested"));
    let other = saved_trail("other", home());
    let saved = vec![target.clone(), other.clone()];
    let (trails, store) = session(saved.clone(), Some(other.id));

    let selected = select(
        &trails,
        target.active_location().cloned(),
        "music",
        &home(),
        &defaults(),
    );

    assert_eq!(selected.workspace, Workspace::Files);
    assert_eq!(selected.trail, target);
    assert_eq!(store.stored.borrow().collection.trails, saved);
}

#[test]
fn home_reuses_a_saved_home_tab_and_keeps_the_other_session_tabs() {
    let home_tab = saved_trail("home", home());
    let other = saved_trail("other", Location::uri("smb://unavailable/share"));
    let saved = vec![home_tab.clone(), other.clone()];
    let (trails, store) = session(saved.clone(), Some(other.id));

    let selected = select(&trails, None, "home", &home(), &defaults());

    assert_eq!(selected.workspace, Workspace::Files);
    assert_eq!(selected.trail, home_tab);
    assert_eq!(store.stored.borrow().collection.trails, saved);
}

#[test]
fn home_adds_a_tab_instead_of_overwriting_the_active_saved_trail() {
    let saved = saved_trail("saved", Location::local("/startup-fixture/projects"));
    let (trails, store) = session(vec![saved.clone()], Some(saved.id.clone()));

    let selected = select(&trails, None, "home", &home(), &defaults());

    assert_eq!(selected.trail.active_location(), Some(&home()));
    assert_eq!(selected.trail.view, defaults());
    assert_eq!(store.stored.borrow().collection.trails[0], saved);
    assert_eq!(trails.all().len(), 2);
}

#[test]
fn music_selects_only_the_library_workspace_without_replacing_the_file_session() {
    let saved = saved_trail("saved", Location::local("/startup-fixture/projects"));
    let (trails, store) = session(vec![saved.clone()], Some(saved.id.clone()));
    let before = store.stored.borrow().clone();

    let selected = select(&trails, None, "music", &home(), &defaults());

    assert_eq!(selected.workspace, Workspace::Music);
    assert_eq!(selected.trail, saved);
    assert_eq!(*store.stored.borrow(), before);
}

#[test]
fn last_preserves_native_and_uri_sessions_without_local_path_conversion() {
    for location in [
        Location::local("/startup-fixture/not-present"),
        Location::uri("trash:///"),
        Location::uri("smb://unavailable/share"),
        Location::uri("search:///saved-query"),
    ] {
        let saved = saved_trail("saved", location);
        let (trails, store) = session(vec![saved.clone()], Some(saved.id.clone()));
        let before = store.stored.borrow().clone();

        let selected = select(&trails, None, "last", &home(), &defaults());

        assert_eq!(selected.workspace, Workspace::Files);
        assert_eq!(selected.trail, saved);
        assert_eq!(*store.stored.borrow(), before);
    }
}

#[test]
fn changing_startup_modes_keeps_saved_paths_views_and_tab_order() {
    let first = saved_trail("first", Location::local("/startup-fixture/projects"));
    let second = saved_trail("second", Location::uri("trash:///"));
    let saved = vec![first.clone(), second.clone()];
    let (trails, store) = session(saved.clone(), Some(first.id));

    for preference in ["music", "last", "home", "last", "music", "home"] {
        select(&trails, None, preference, &home(), &defaults());
        assert_eq!(
            &store.stored.borrow().collection.trails[..2],
            saved.as_slice()
        );
    }
    assert_eq!(trails.all().len(), 3);
    let reloaded = Trails::load(store).expect("reload session");
    assert_eq!(reloaded.all(), trails.all());
    assert_eq!(reloaded.active_id(), trails.active_id());
}

#[test]
fn fresh_sessions_use_the_supplied_hidden_default_in_every_mode() {
    for show_hidden in [false, true] {
        let view = fresh_view(
            TrailBrowserMode::Grid,
            TrailBrowserDensity::Airy,
            show_hidden,
        );
        for preference in ["home", "music", "last", "unknown"] {
            let (trails, store) = session(Vec::new(), None);

            let selected = select(&trails, None, preference, &home(), &view);

            assert_eq!(selected.trail.active_location(), Some(&home()));
            assert_eq!(selected.trail.view, view);
            assert_eq!(
                selected.workspace == Workspace::Music,
                preference == "music"
            );
            assert_eq!(
                store.stored.borrow().collection.trails,
                vec![selected.trail]
            );
        }
    }
}

#[test]
fn saved_hidden_preference_overrides_the_current_default_in_both_directions() {
    for show_hidden in [false, true] {
        let mut saved = saved_trail("saved", home());
        saved.view.preferences.show_hidden = !show_hidden;
        let (trails, _) = session(vec![saved.clone()], Some(saved.id.clone()));
        let view = fresh_view(
            TrailBrowserMode::Grid,
            TrailBrowserDensity::Compact,
            show_hidden,
        );
        for preference in ["home", "music", "last"] {
            assert_eq!(
                select(&trails, None, preference, &home(), &view).trail,
                saved
            );
        }
    }
}

#[test]
fn duplicate_destinations_prefer_the_active_trail() {
    let first = saved_trail("first", home());
    let second = saved_trail("second", home());
    let (trails, _) = session(vec![first, second.clone()], Some(second.id.clone()));

    assert_eq!(
        select(&trails, None, "home", &home(), &defaults()).trail,
        second
    );
}

#[test]
fn missing_active_identifier_recovers_an_existing_session_without_discarding_tabs() {
    let saved = saved_trail("saved", Location::uri("trash:///"));
    for active in [None, TrailId::new("missing")] {
        let (trails, store) = session(vec![saved.clone()], active);
        let selected = select(&trails, None, "last", &home(), &defaults());
        assert_eq!(selected.trail, saved);
        assert_eq!(trails.active_id(), Some(saved.id.clone()));
        assert_eq!(store.stored.borrow().collection.trails, vec![saved.clone()]);
    }
}

#[test]
fn invalid_saved_locations_fall_back_without_deleting_the_saved_trails() {
    for location in [
        Location::local("relative"),
        Location::local(""),
        Location::uri(""),
    ] {
        let saved = saved_trail("saved", location);
        let (trails, store) = session(vec![saved.clone()], Some(saved.id.clone()));

        let selected = select(&trails, None, "last", &home(), &defaults());

        assert_eq!(selected.trail.active_location(), Some(&home()));
        assert_eq!(store.stored.borrow().collection.trails[0], saved);
        assert_eq!(trails.all().len(), 2);
    }
}

#[test]
fn unavailable_destination_falls_back_once_and_retains_the_failed_saved_trail() {
    let saved = saved_trail("offline", Location::uri("smb://unavailable/share"));
    let (trails, store) = session(vec![saved.clone()], Some(saved.id.clone()));
    let selected = select(&trails, None, "last", &home(), &defaults());
    let mut restore = Restore::new(&selected.trail);

    assert!(restore.preserves(trails.active_id().as_ref(), saved.active_location()));
    assert!(restore.complete(
        trails.active_id().as_ref(),
        saved.active_location(),
        true,
        &home()
    ));
    assert!(!restore.complete(
        trails.active_id().as_ref(),
        saved.active_location(),
        true,
        &home()
    ));
    let fallback = select_location(&trails, &home(), &defaults());
    assert_eq!(fallback.active_location(), Some(&home()));
    assert_eq!(store.stored.borrow().collection.trails[0], saved);
    let mut restore = Restore::new(&fallback);
    assert!(!restore.complete(
        Some(&fallback.id),
        fallback.active_location(),
        true,
        &home()
    ));
}

#[test]
fn successful_loads_and_user_navigation_cancel_startup_fallback() {
    let saved = saved_trail("saved", Location::local("/startup-fixture/projects"));
    let mut restore = Restore::new(&saved);
    assert!(!restore.complete(Some(&saved.id), saved.active_location(), false, &home()));
    assert!(!restore.complete(Some(&saved.id), saved.active_location(), true, &home()));

    let mut restore = Restore::new(&saved);
    assert!(!restore.preserves(Some(&saved.id), Some(&home())));
    assert!(!restore.complete(Some(&saved.id), saved.active_location(), true, &home()));

    let other = saved_trail(
        "other",
        saved.active_location().expect("saved location").clone(),
    );
    let mut restore = Restore::new(&saved);
    assert!(!restore.preserves(Some(&other.id), other.active_location()));
    assert!(!restore.complete(Some(&saved.id), saved.active_location(), true, &home()));
}

#[test]
fn startup_restoration_does_not_flatten_nested_paths_or_overwrite_saved_view_state() {
    let saved = saved_trail(
        "nested",
        Location::local("/startup-fixture/projects/nested"),
    );
    let (trails, store) = session(vec![saved.clone()], Some(saved.id.clone()));
    let selected = select(&trails, None, "last", &home(), &defaults());
    let mut restore = Restore::new(&selected.trail);
    let before = store.stored.borrow().clone();

    for _ in 0..3 {
        if !restore.preserves(
            trails.active_id().as_ref(),
            selected.trail.active_location(),
        ) {
            trails
                .update_active_location(
                    selected
                        .trail
                        .active_location()
                        .expect("nested location")
                        .clone(),
                )
                .expect("save flattened location");
            trails
                .update_active_view(defaults())
                .expect("save fresh view");
        }
        assert_eq!(*store.stored.borrow(), before);
    }
    assert!(!restore.complete(Some(&saved.id), saved.active_location(), false, &home()));
    assert_eq!(
        Trails::load(store).expect("reload fixture").all(),
        vec![saved]
    );
}

#[test]
fn startup_destination_remains_effective_if_persistence_fails() {
    let store = Rc::new(MemoryStore {
        fail_saves: true,
        ..MemoryStore::default()
    });
    let trails = Trails::load(store).expect("load fixture");
    let requested = Location::local("/startup-fixture/requested");

    let selected = select(
        &trails,
        Some(requested.clone()),
        "music",
        &home(),
        &defaults(),
    );

    assert_eq!(selected.workspace, Workspace::Files);
    assert_eq!(selected.trail.active_location(), Some(&requested));
    assert_eq!(trails.active_id(), Some(selected.trail.id));
}
