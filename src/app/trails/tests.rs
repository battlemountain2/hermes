// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, io, rc::Rc};

use crate::services::{StoredTrails, TrailStore};

use super::*;

#[derive(Default)]
struct MemoryStore {
    value: RefCell<StoredTrails>,
}

impl TrailStore for MemoryStore {
    fn load(&self) -> io::Result<StoredTrails> {
        Ok(self.value.borrow().clone())
    }

    fn save(&self, trails: &StoredTrails) -> io::Result<()> {
        self.value.replace(trails.clone());
        Ok(())
    }
}

fn manager() -> (Rc<Trails>, Rc<MemoryStore>) {
    let store = Rc::new(MemoryStore::default());
    let trails = Trails::load(store.clone()).expect("load Trails");
    trails
        .create(
            "example",
            Location::local("/home/example"),
            TrailViewState { sidebar_open: true, ..TrailViewState::default() },
        )
        .expect("create initial tab");
    (trails, store)
}

#[test]
fn reordering_tabs_preserves_the_active_tab_and_saved_order() {
    let (trails, store) = manager();
    let first = trails.all()[0].id.clone();
    let second = trails.create("Music", Location::local("/home/example/Music"), TrailViewState::default()).expect("Create music tab");
    let third = trails.create("Videos", Location::local("/home/example/Videos"), TrailViewState::default()).expect("Create video tab");
    assert!(trails.move_relative(&third, &first, true).expect("Move before"));
    assert_eq!(trails.all().iter().map(|trail| &trail.id).collect::<Vec<_>>(), vec![&third, &first, &second]);
    assert_eq!(trails.active_id(), Some(third.clone()));
    assert!(trails.move_relative(&third, &second, false).expect("Move after"));
    assert_eq!(store.value.borrow().collection.trails.iter().map(|trail| &trail.id).collect::<Vec<_>>(), vec![&first, &second, &third]);
    assert!(!trails.move_relative(&second, &second, true).expect("Self drop"));
    assert!(!trails.move_relative(&third, &second, false).expect("Unchanged order"));
    let missing = TrailId::new("missing").expect("Valid id");
    assert!(!trails.move_relative(&missing, &second, true).expect("Missing source"));
}

#[test]
fn create_activate_rename_and_close_round_trip() {
    let (trails, store) = manager();
    let created = trails
        .create(
            "Pictures",
            Location::local("/home/example/Pictures"),
            TrailViewState::default(),
        )
        .expect("create Trail");
    assert_eq!(trails.active_id(), Some(created.clone()));

    assert!(trails.rename(&created, "Photos").expect("rename Trail"));
    assert_eq!(trails.all()[1].name, "Photos");

    let default = trails.all()[0].id.clone();
    let location = trails.activate(&default).expect("activate Trail");
    assert_eq!(
        location.and_then(|trail| trail.active_location().cloned()),
        Some(Location::local("/home/example"))
    );
    assert!(trails.close(&created).expect("close Trail").is_none());
    assert_eq!(store.value.borrow().collection.trails.len(), 1);
}

#[test]
fn closing_the_active_trail_selects_its_neighbor() {
    let (trails, _) = manager();
    let created = trails
        .create(
            "Temporary",
            Location::local("/tmp"),
            TrailViewState::default(),
        )
        .expect("create Trail");

    let location = trails.close(&created).expect("close active Trail");

    assert_eq!(
        location.and_then(|trail| trail.active_location().cloned()),
        Some(Location::local("/home/example"))
    );
    assert_eq!(trails.all().len(), 1);
}

#[test]
fn navigation_updates_only_the_active_trail() {
    let (trails, _) = manager();
    trails
        .create("Other", Location::local("/tmp"), TrailViewState::default())
        .expect("create Trail");

    trails
        .update_active_location(Location::local("/tmp/nested"))
        .expect("update location");

    assert_eq!(
        trails.all()[0].active_location(),
        Some(&Location::local("/home/example"))
    );
    assert_eq!(
        trails.all()[1].active_location(),
        Some(&Location::local("/tmp/nested"))
    );
}

#[test]
fn the_last_trail_cannot_be_closed() {
    let (trails, _) = manager();
    let only = trails.active_id().expect("active Trail");

    assert!(trails.close(&only).expect("close Trail").is_none());
    assert_eq!(trails.all().len(), 1);
}

#[test]
fn view_state_is_saved_for_only_the_active_trail() {
    let (trails, store) = manager();
    let original = trails.all()[0].clone();
    trails.create("Other", Location::local("/other"), TrailViewState::default()).expect("create other tab");
    let other = trails.all()[1].clone();
    trails.activate(&original.id).expect("activate original tab");
    let mut view = original.view.clone();
    view.preview_open = true;

    trails.update_active_view(view.clone()).expect("save view");

    let saved = trails.all();
    assert_eq!(saved[0].view, view);
    assert_eq!(saved[1], other);
    assert_eq!(store.value.borrow().collection.trails, saved);
}

#[test]
fn cycling_wraps_in_both_directions() {
    let (trails, _) = manager();
    trails
        .create("Second", Location::local("/tmp"), TrailViewState::default())
        .expect("create Trail");

    let first = trails.cycle(1).expect("cycle forward").expect("Trail");
    let second = trails.cycle(-1).expect("cycle backward").expect("Trail");

    assert_eq!(first.name, "example");
    assert_eq!(second.name, "Second");
}
