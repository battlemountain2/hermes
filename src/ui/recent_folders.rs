use std::fs;
use std::path::PathBuf;

use crate::model::Location;

#[derive(Clone, Debug, Default)]
pub struct RecentFolders {
    locations: Vec<Location>,
}

impl RecentFolders {
    pub fn load() -> Self {
        let path = Self::storage_path();
        let locations: Vec<Location> = fs::read_to_string(&path)
            .ok()
            .and_then(|contents| toml::from_str(&contents).ok())
            .unwrap_or_default();
        Self {
            locations: locations
                .into_iter()
                .filter(is_folder_history_location)
                .collect(),
        }
    }

    pub fn save(&self) {
        if let Some(parent) = Self::storage_path().parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(contents) = toml::to_string(&self.locations) {
            let _ = fs::write(Self::storage_path(), contents);
        }
    }

    pub fn record(&mut self, location: &Location, limit: usize) {
        if !is_folder_history_location(location) {
            return;
        }
        if let Some(path) = location.native_path() {
            let home = gtk::glib::home_dir();
            if path == home {
                return;
            }
        }

        self.locations.retain(|loc| loc != location);
        self.locations.insert(0, location.clone());
        if self.locations.len() > limit {
            self.locations.truncate(limit);
        }
        self.save();
    }

    pub fn remove(&mut self, location: &Location) {
        self.locations.retain(|l| l != location);
        self.save();
    }

    pub fn clear(&mut self) {
        self.locations.clear();
        self.save();
    }

    pub fn iter(&self) -> impl Iterator<Item = &Location> {
        self.locations.iter()
    }

    fn storage_path() -> PathBuf {
        crate::paths::recent_folders_file()
    }
}

fn is_folder_history_location(location: &Location) -> bool {
    !location.is_home_dashboard()
        && !location
            .uri_value()
            .is_some_and(|uri| uri.starts_with("search://"))
}

#[cfg(test)]
mod tests;
