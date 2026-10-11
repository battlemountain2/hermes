use super::Library;
use gio::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

pub(super) struct Monitors {
    library: Weak<Library>,
    monitors: HashMap<PathBuf, gio::FileMonitor>,
    wanted: HashSet<PathBuf>,
    limited: bool,
}

impl Monitors {
    pub(super) fn new(library: &Arc<Library>) -> Self {
        Self {
            library: Arc::downgrade(library),
            monitors: HashMap::new(),
            wanted: HashSet::new(),
            limited: false,
        }
    }

    pub(super) fn begin(&mut self) {
        self.wanted.clear();
        self.limited = false;
    }

    pub(super) fn parent(&mut self, root: &Path) {
        let mut parent = root.parent();
        while let Some(path) = parent {
            if path.is_dir() {
                self.add(path);
                break;
            }
            parent = path.parent();
        }
    }

    pub(super) fn add(&mut self, path: &Path) {
        let Some(library) = self.library.upgrade() else {
            return;
        };
        if self.wanted.len() >= library.configuration.monitor_limit && !self.wanted.contains(path) {
            self.limited = true;
            return;
        }
        self.wanted.insert(path.to_owned());
        if self
            .monitors
            .get(path)
            .is_some_and(|monitor| !monitor.is_cancelled())
        {
            return;
        }
        let monitor = match gio::File::for_path(path)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        {
            Ok(monitor) => monitor,
            Err(_) => {
                self.limited = true;
                return;
            }
        };
        monitor.set_rate_limit(100);
        let weak = self.library.clone();
        let root = library.configuration.root.clone();
        monitor.connect_changed(move |_, file, other, _| {
            let relevant = |file: &gio::File| {
                file.path()
                    .is_some_and(|path| path.starts_with(&root) || root.starts_with(&path))
            };
            if (relevant(file) || other.as_ref().is_some_and(|file| relevant(file)))
                && let Some(library) = weak.upgrade()
            {
                library.request(false);
            }
        });
        self.monitors.insert(path.to_owned(), monitor);
    }

    pub(super) fn finish(&mut self, notice: &mut Option<String>) {
        self.monitors.retain(|path, monitor| {
            let keep = self.wanted.contains(path);
            if !keep {
                monitor.cancel();
            }
            keep
        });
        if self.limited {
            super::append_notice(
                notice,
                "Some folders cannot be watched; automatic verification runs every 60 seconds",
            );
        }
    }
}

impl Drop for Monitors {
    fn drop(&mut self) {
        for monitor in self.monitors.values() {
            monitor.cancel();
        }
    }
}
