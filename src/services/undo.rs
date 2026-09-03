// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use crate::model::{FileEntry, Location};
use std::collections::VecDeque;

const MAX_UNDO_ENTRIES: usize = 20;

/// A reversible file operation that can be undone.
#[derive(Clone, Debug)]
pub enum UndoEntry {
    Rename {
        location: Location,
        old_name: String,
        new_name: String,
    },
    Move {
        sources: Vec<Location>,
        destination: Location,
    },
    Trash {
        entries: Vec<FileEntry>,
    },
    Create {
        location: Location,
    },
    Duplicate {
        created: Vec<Location>,
    },
    BatchRename {
        renames: Vec<(Location, String, String)>,
    },
}

impl UndoEntry {
    /// Returns a human-readable description of the operation that would be undone.
    pub fn description(&self) -> String {
        match self {
            Self::Rename { old_name, .. } => format!("Undo rename of {old_name}"),
            Self::Move { sources, .. } => {
                if sources.len() == 1 {
                    format!("Undo move of {}", sources[0].display_name())
                } else {
                    format!("Undo move of {} items", sources.len())
                }
            }
            Self::Trash { entries, .. } => {
                if entries.len() == 1 {
                    format!("Undo trash of {}", entries[0].display_name)
                } else {
                    format!("Undo trash of {} items", entries.len())
                }
            }
            Self::Create { location } => {
                format!("Undo creation of {}", location.display_name())
            }
            Self::Duplicate { created, .. } => {
                if created.len() == 1 {
                    format!("Undo duplicate of {}", created[0].display_name())
                } else {
                    format!("Undo duplicate of {} items", created.len())
                }
            }
            Self::BatchRename { renames } => {
                if renames.len() == 1 {
                    format!("Undo rename of {}", renames[0].2)
                } else {
                    format!("Undo rename of {} items", renames.len())
                }
            }
        }
    }
}

/// A bounded stack of reversible file operations.
#[derive(Debug, Default)]
pub struct UndoHistory {
    entries: VecDeque<UndoEntry>,
}

impl UndoHistory {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    pub fn push(&mut self, entry: UndoEntry) {
        if self.entries.len() >= MAX_UNDO_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn pop(&mut self) -> Option<UndoEntry> {
        self.entries.pop_back()
    }

    pub fn can_undo(&self) -> bool {
        !self.entries.is_empty()
    }

    pub fn describe_last(&self) -> Option<String> {
        self.entries.back().map(UndoEntry::description)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn undo(
        &mut self,
        provider: &dyn crate::services::OperationProvider,
        request_id: crate::services::OperationRequestId,
        emit: std::rc::Rc<dyn Fn(crate::services::OperationEvent)>,
    ) -> Option<crate::services::LoadHandle> {
        let entry = self.pop()?;
        Some(provider.undo(request_id, entry, emit))
    }
}
