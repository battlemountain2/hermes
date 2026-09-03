// SPDX-License-Identifier: GPL-3.0-or-later

use std::io;

use crate::model::TrailCollection;

pub const TRAIL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredTrails {
    pub schema_version: u32,
    pub collection: TrailCollection,
}

impl Default for StoredTrails {
    fn default() -> Self {
        Self {
            schema_version: TRAIL_SCHEMA_VERSION,
            collection: TrailCollection::default(),
        }
    }
}

pub trait TrailStore {
    fn load(&self) -> io::Result<StoredTrails>;
    fn save(&self, trails: &StoredTrails) -> io::Result<()>;
}
