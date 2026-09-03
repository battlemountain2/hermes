// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Serialize};

use super::{Location, ViewPreferences};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct TrailId(String);

impl TrailId {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        (!value.trim().is_empty()).then_some(Self(value))
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMode {
    #[default]
    Columns,
    Grid,
    Explorer,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserDensity {
    #[default]
    Compact,
    Airy,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct TrailViewState {
    pub browser_mode: BrowserMode,
    pub density: BrowserDensity,
    pub preferences: ViewPreferences,
    pub sidebar_open: bool,
    pub preview_open: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Trail {
    pub id: TrailId,
    pub name: String,
    pub locations: Vec<Location>,
    #[serde(default)]
    pub view: TrailViewState,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub custom_name: bool,
}

impl Trail {
    pub fn new(id: TrailId, name: impl Into<String>, locations: Vec<Location>) -> Option<Self> {
        let name = name.into();
        (!name.trim().is_empty() && !locations.is_empty()).then_some(Self {
            id,
            name,
            locations,
            view: TrailViewState {
                sidebar_open: true,
                ..TrailViewState::default()
            },
            pinned: false,
            custom_name: false,
        })
    }

    pub fn active_location(&self) -> Option<&Location> {
        self.locations.last()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct TrailCollection {
    pub trails: Vec<Trail>,
    pub active: Option<TrailId>,
}

impl TrailCollection {
    pub fn active_trail(&self) -> Option<&Trail> {
        let active = self.active.as_ref()?;
        self.trails.iter().find(|trail| &trail.id == active)
    }

    pub fn normalize(&mut self) {
        self.trails.retain(|trail| !trail.locations.is_empty());
        if self
            .active
            .as_ref()
            .is_some_and(|active| !self.trails.iter().any(|trail| &trail.id == active))
        {
            self.active = self.trails.first().map(|trail| trail.id.clone());
        }
    }
}

#[cfg(test)]
mod tests;
