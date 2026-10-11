use std::{
    collections::VecDeque,
    fs::Metadata,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Instant,
};

use super::{CACHE_ENTRIES, CACHE_TTL, Operation};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl Identity {
    pub fn read(path: &Path) -> Option<Self> {
        path.metadata().ok().as_ref().map(Self::from_metadata)
    }

    pub fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

struct Entry {
    path: PathBuf,
    identity: Identity,
    operation: Operation,
    data: Vec<u8>,
    metadata: Option<Vec<u8>>,
    timestamp: Instant,
}

impl Entry {
    fn bytes(&self) -> usize {
        self.data
            .capacity()
            .saturating_add(self.metadata.as_ref().map_or(0, Vec::capacity))
    }
}

pub(super) struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
    budget: usize,
}

impl Cache {
    pub fn new(budget: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            budget,
        }
    }

    fn prune(&mut self, path: &Path, identity: Identity, operation: Operation) {
        self.entries.retain(|entry| {
            entry.timestamp.elapsed() < CACHE_TTL
                && !(entry.path == path
                    && entry.operation == operation
                    && entry.identity != identity)
        });
        self.bytes = self.entries.iter().map(Entry::bytes).sum();
    }

    pub fn get(
        &mut self,
        path: &Path,
        identity: Identity,
        operation: Operation,
    ) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
        self.prune(path, identity, operation);
        let index = self.entries.iter().position(|entry| {
            entry.path == path && entry.identity == identity && entry.operation == operation
        })?;
        let entry = self.entries.remove(index)?;
        let result = (entry.data.clone(), entry.metadata.clone());
        self.entries.push_back(entry);
        Some(result)
    }

    pub fn insert(
        &mut self,
        path: &Path,
        identity: Identity,
        operation: Operation,
        data: Vec<u8>,
        metadata: Option<Vec<u8>>,
    ) {
        self.prune(path, identity, operation);
        self.entries
            .retain(|entry| !(entry.path == path && entry.operation == operation));
        self.bytes = self.entries.iter().map(Entry::bytes).sum();
        let entry = Entry {
            path: path.to_owned(),
            identity,
            operation,
            data,
            metadata,
            timestamp: Instant::now(),
        };
        let bytes = entry.bytes();
        if bytes > self.budget {
            return;
        }
        while self.entries.len() >= CACHE_ENTRIES || self.bytes.saturating_add(bytes) > self.budget
        {
            let Some(removed) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(removed.bytes());
        }
        self.bytes += bytes;
        self.entries.push_back(entry);
    }
}

#[cfg(test)]
mod tests;
