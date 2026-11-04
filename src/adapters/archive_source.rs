// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    sync::Arc,
    time::SystemTime,
};

use crate::{
    model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue},
    services::{
        DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
    },
};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveNode {
    pub name: String,
    pub relative_path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Default)]
pub struct ArchiveIndex {
    // subpath (trimmed) -> list of direct children
    pub children: HashMap<String, Vec<ArchiveNode>>,
}

pub struct ArchiveFileSource {
    // Cached parsed index: archive path -> (mtime, index)
    cache: RefCell<HashMap<PathBuf, (SystemTime, Arc<ArchiveIndex>)>>,
}

impl ArchiveFileSource {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(HashMap::new()),
        }
    }

    pub fn get_or_build_index(&self, archive_path: &Path) -> Result<Arc<ArchiveIndex>, String> {
        let metadata = std::fs::metadata(archive_path)
            .map_err(|e| format!("Cannot read archive metadata: {e}"))?;
        let mtime = metadata
            .modified()
            .unwrap_or(SystemTime::UNIX_EPOCH);

        if let Some((cached_mtime, index)) = self.cache.borrow().get(archive_path) {
            if *cached_mtime == mtime {
                return Ok(index.clone());
            }
        }

        let index = Arc::new(parse_archive_contents(archive_path)?);
        self.cache
            .borrow_mut()
            .insert(archive_path.to_path_buf(), (mtime, index.clone()));
        Ok(index)
    }
}

pub fn parse_archive_contents(archive_path: &Path) -> Result<ArchiveIndex, String> {
    let output = Command::new("bsdtar")
        .arg("-tvf")
        .arg(archive_path)
        .output()
        .map_err(|e| format!("Unable to invoke bsdtar: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut raw_entries = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 9 {
            continue;
        }
        let mode = parts[0];
        let size: u64 = parts[4].parse().unwrap_or(0);
        // Find position of 9th token in line to preserve spaces in member paths
        let mut skipped = 0;
        let mut idx = 0;
        let bytes = line.as_bytes();
        let mut in_space = false;
        while idx < bytes.len() && skipped < 8 {
            if bytes[idx].is_ascii_whitespace() {
                if !in_space {
                    in_space = true;
                    skipped += 1;
                }
            } else {
                in_space = false;
            }
            idx += 1;
        }
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        let member_path = &line[idx..];
        let is_dir = mode.starts_with('d') || member_path.ends_with('/');
        raw_entries.push((member_path.trim_end_matches('/').to_string(), is_dir, size));
    }

    let mut dirs: HashSet<String> = HashSet::new();
    let mut files: HashMap<String, u64> = HashMap::new();

    for (p, is_dir, sz) in raw_entries {
        if p.is_empty() {
            continue;
        }
        let components: Vec<&str> = p.split('/').collect();
        for i in 1..components.len() {
            dirs.insert(components[..i].join("/"));
        }
        if is_dir {
            dirs.insert(p);
        } else {
            files.insert(p, sz);
        }
    }

    let mut children_map: HashMap<String, Vec<ArchiveNode>> = HashMap::new();

    for d in &dirs {
        let (parent, name) = match d.rsplit_once('/') {
            Some((p, n)) => (p.to_string(), n.to_string()),
            None => (String::new(), d.clone()),
        };
        children_map.entry(parent).or_default().push(ArchiveNode {
            name,
            relative_path: d.clone(),
            is_dir: true,
            size: 0,
        });
    }

    for (f, sz) in &files {
        let (parent, name) = match f.rsplit_once('/') {
            Some((p, n)) => (p.to_string(), n.to_string()),
            None => (String::new(), f.clone()),
        };
        children_map.entry(parent).or_default().push(ArchiveNode {
            name,
            relative_path: f.clone(),
            is_dir: false,
            size: *sz,
        });
    }

    // Sort direct children alphabetically with directories first
    for list in children_map.values_mut() {
        list.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }

    Ok(ArchiveIndex {
        children: children_map,
    })
}

impl FileSource for ArchiveFileSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        let Some((archive_path, _)) = location.archive_parts() else {
            return Err(LocationValidationError::Missing);
        };
        if archive_path.is_file() {
            Ok(())
        } else {
            Err(LocationValidationError::Missing)
        }
    }

    fn invalidate(&self, location: &Location) {
        if let Some((archive_path, _)) = location.archive_parts() {
            self.cache.borrow_mut().remove(&archive_path);
        }
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let request_id = request.id;
        let Some((archive_path, member_subpath)) = request.location.archive_parts() else {
            gtk::glib::idle_add_local_once(move || {
                emit(DirectoryEvent::Finished { request_id });
            });
            return LoadHandle::new(|| {});
        };

        let subpath = member_subpath.trim_matches('/').to_string();
        let archive_display = archive_path.display().to_string();

        let index_res = self.get_or_build_index(&archive_path);
        match index_res {
            Ok(index) => {
                let children = index.children.get(&subpath).cloned().unwrap_or_default();
                let items: Vec<FileEntry> = children
                    .into_iter()
                    .map(|node| {
                        let loc_str = if node.is_dir {
                            format!("archive://{}/{}/", archive_display, node.relative_path)
                        } else {
                            format!("archive://{}/{}", archive_display, node.relative_path)
                        };
                        FileEntry(Rc::new(FileEntryInner {
                            music_tags: None,
                            location: Location::uri(loc_str),
                            native_name: OsString::from(&node.name),
                            display_name: node.name,
                            kind: if node.is_dir {
                                EntryKind::Directory
                            } else {
                                EntryKind::File
                            },
                            size: MetadataValue::Known(node.size),
                            modified_unix_seconds: MetadataValue::Unknown,
                        }))
                    })
                    .collect();

                gtk::glib::idle_add_local_once(move || {
                    if !items.is_empty() {
                        emit(DirectoryEvent::Batch {
                            request_id,
                            entries: items,
                        });
                    }
                    emit(DirectoryEvent::Finished { request_id });
                });
            }
            Err(err) => {
                tracing::warn!(%err, "unable to enumerate archive");
                gtk::glib::idle_add_local_once(move || {
                    emit(DirectoryEvent::Finished { request_id });
                });
            }
        }

        LoadHandle::new(|| {})
    }
}
