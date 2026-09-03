// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, io, rc::Rc};

use crate::{
    model::{Location, Trail, TrailCollection, TrailId, TrailViewState},
    services::{StoredTrails, TrailStore},
};

pub struct Trails {
    store: Rc<dyn TrailStore>,
    stored: RefCell<StoredTrails>,
}

impl Trails {
    pub fn load(store: Rc<dyn TrailStore>) -> io::Result<Rc<Self>> {
        let stored = store.load()?;
        Ok(Rc::new(Self {
            store,
            stored: RefCell::new(stored),
        }))
    }

    pub fn empty(store: Rc<dyn TrailStore>) -> Rc<Self> {
        Rc::new(Self {
            store,
            stored: RefCell::new(StoredTrails::default()),
        })
    }

    pub fn all(&self) -> Vec<Trail> {
        self.stored.borrow().collection.trails.clone()
    }

    pub fn active_id(&self) -> Option<TrailId> {
        self.stored.borrow().collection.active.clone()
    }

    pub fn active_location(&self) -> Option<Location> {
        self.stored
            .borrow()
            .collection
            .active_trail()
            .and_then(Trail::active_location)
            .cloned()
    }

    pub fn active_view(&self) -> Option<TrailViewState> {
        self.stored
            .borrow()
            .collection
            .active_trail()
            .map(|trail| trail.view.clone())
    }

    pub fn ensure_default(&self, location: Location) -> io::Result<()> {
        if !self.stored.borrow().collection.trails.is_empty() {
            return Ok(());
        }
        let trail = Trail::new(
            TrailId::new("default").expect("static Trail identifier is valid"),
            location.display_name(),
            location.breadcrumbs(),
        )
        .expect("a location produces a valid Trail");
        let mut stored = self.stored.borrow_mut();
        stored.collection.active = Some(trail.id.clone());
        stored.collection.trails.push(trail);
        self.store.save(&stored)
    }

    pub fn create(
        &self,
        name: impl Into<String>,
        location: Location,
        view: TrailViewState,
    ) -> io::Result<TrailId> {
        let mut stored = self.stored.borrow_mut();
        let id = next_id(&stored.collection);
        let mut trail = Trail::new(id.clone(), name, location.breadcrumbs()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Trail name cannot be empty")
        })?;
        trail.view = view;
        stored.collection.active = Some(id.clone());
        stored.collection.trails.push(trail);
        self.store.save(&stored)?;
        Ok(id)
    }

    pub fn activate(&self, id: &TrailId) -> io::Result<Option<Trail>> {
        let mut stored = self.stored.borrow_mut();
        let trail = stored
            .collection
            .trails
            .iter()
            .find(|trail| &trail.id == id)
            .cloned();
        if trail.is_some() {
            stored.collection.active = Some(id.clone());
            self.store.save(&stored)?;
        }
        Ok(trail)
    }

    pub fn cycle(&self, offset: isize) -> io::Result<Option<Trail>> {
        let stored = self.stored.borrow();
        if stored.collection.trails.is_empty() {
            return Ok(None);
        }
        let current = stored
            .collection
            .active
            .as_ref()
            .and_then(|active| {
                stored
                    .collection
                    .trails
                    .iter()
                    .position(|trail| &trail.id == active)
            })
            .unwrap_or(0);
        let len = stored.collection.trails.len() as isize;
        let next = (current as isize + offset).rem_euclid(len) as usize;
        let id = stored.collection.trails[next].id.clone();
        drop(stored);
        self.activate(&id)
    }

    pub fn rename(&self, id: &TrailId, name: impl Into<String>) -> io::Result<bool> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Trail name cannot be empty",
            ));
        }
        let mut stored = self.stored.borrow_mut();
        let Some(trail) = stored
            .collection
            .trails
            .iter_mut()
            .find(|trail| &trail.id == id)
        else {
            return Ok(false);
        };
        trail.name = name;
        trail.custom_name = true;
        self.store.save(&stored)?;
        Ok(true)
    }

    pub fn toggle_pinned(&self, id: &TrailId) -> io::Result<bool> {
        let mut stored = self.stored.borrow_mut();
        let Some(trail) = stored
            .collection
            .trails
            .iter_mut()
            .find(|trail| &trail.id == id)
        else {
            return Ok(false);
        };
        trail.pinned = !trail.pinned;
        let pinned = trail.pinned;
        self.store.save(&stored)?;
        Ok(pinned)
    }

    pub fn close(&self, id: &TrailId) -> io::Result<Option<Trail>> {
        let mut stored = self.stored.borrow_mut();
        if stored.collection.trails.len() <= 1 {
            return Ok(None);
        }
        let Some(position) = stored
            .collection
            .trails
            .iter()
            .position(|trail| &trail.id == id)
        else {
            return Ok(None);
        };
        let was_active = stored.collection.active.as_ref() == Some(id);
        stored.collection.trails.remove(position);
        if was_active {
            let next = position.min(stored.collection.trails.len() - 1);
            stored.collection.active = Some(stored.collection.trails[next].id.clone());
        }
        let next = stored.collection.active_trail().cloned();
        self.store.save(&stored)?;
        Ok(was_active.then_some(next).flatten())
    }

    pub fn update_active_location(&self, location: Location) -> io::Result<()> {
        let mut stored = self.stored.borrow_mut();
        let active = stored.collection.active.clone();
        let Some(trail) = stored
            .collection
            .trails
            .iter_mut()
            .find(|trail| Some(&trail.id) == active.as_ref())
        else {
            return Ok(());
        };
        let locations = location.breadcrumbs();
        if trail.locations == locations {
            return Ok(());
        }
        trail.locations = locations;
        if !trail.custom_name {
            trail.name = location.display_name();
        }
        self.store.save(&stored)
    }

    pub fn update_active_view(&self, view: TrailViewState) -> io::Result<()> {
        let mut stored = self.stored.borrow_mut();
        let active = stored.collection.active.clone();
        let Some(trail) = stored
            .collection
            .trails
            .iter_mut()
            .find(|trail| Some(&trail.id) == active.as_ref())
        else {
            return Ok(());
        };
        if trail.view == view {
            return Ok(());
        }
        trail.view = view;
        self.store.save(&stored)
    }
}

fn next_id(collection: &TrailCollection) -> TrailId {
    let mut suffix = collection.trails.len() + 1;
    loop {
        let candidate = TrailId::new(format!("trail-{suffix}")).expect("generated id is valid");
        if !collection.trails.iter().any(|trail| trail.id == candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

#[cfg(test)]
mod tests;
