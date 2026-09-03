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
        .ensure_default(Location::local("/home/example"))
        .expect("create default");
    (trails, store)
}

#[test]
fn create_activate_rename_pin_and_close_round_trip() {
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
    assert!(trails.toggle_pinned(&created).expect("pin Trail"));
    assert_eq!(trails.all()[1].name, "Photos");
    assert!(trails.all()[1].pinned);

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
    let (trails, _) = manager();
    let mut view = trails.active_view().expect("active view");
    view.preview_open = true;

    trails.update_active_view(view.clone()).expect("save view");

    assert_eq!(trails.active_view(), Some(view));
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
