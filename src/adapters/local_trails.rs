// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    model::TrailCollection,
    services::{StoredTrails, TRAIL_SCHEMA_VERSION, TrailStore},
};

#[derive(Deserialize, Serialize)]
struct TrailFile {
    schema_version: u32,
    #[serde(flatten)]
    collection: TrailCollection,
}

pub struct LocalTrailStore {
    path: PathBuf,
}

impl LocalTrailStore {
    pub fn xdg() -> Self {
        Self::at(gtk::glib::user_config_dir().join("strata/trails.toml"))
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn temporary_path(&self) -> PathBuf {
        self.path.with_extension("toml.tmp")
    }
}

impl TrailStore for LocalTrailStore {
    fn load(&self) -> io::Result<StoredTrails> {
        let source = match fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(StoredTrails::default());
            }
            Err(error) => return Err(error),
        };
        let mut stored: TrailFile = toml::from_str(&source).map_err(io::Error::other)?;
        if stored.schema_version > TRAIL_SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported Trail schema version {}", stored.schema_version),
            ));
        }
        stored.collection.normalize();
        Ok(StoredTrails {
            schema_version: stored.schema_version,
            collection: stored.collection,
        })
    }

    fn save(&self, trails: &StoredTrails) -> io::Result<()> {
        if trails.schema_version != TRAIL_SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "only the current Trail schema can be saved",
            ));
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = self.temporary_path();
        let encoded = toml::to_string_pretty(&TrailFile {
            schema_version: trails.schema_version,
            collection: trails.collection.clone(),
        })
        .map_err(io::Error::other)?;
        let result = write_and_replace(&temporary, &self.path, encoded.as_bytes());
        if result.is_err() {
            let _cleanup = fs::remove_file(&temporary);
        }
        result
    }
}

fn write_and_replace(temporary: &Path, destination: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(temporary)?;
    file.write_all(contents)?;
    file.sync_all()?;
    fs::rename(temporary, destination)
}

#[cfg(test)]
mod tests;
