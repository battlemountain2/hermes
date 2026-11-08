// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::{
    cell::RefCell,
    future::poll_fn,
    rc::Rc,
    task::{Poll, Waker},
};

use crate::model::{FileEntry, Location};

use super::LoadHandle;

pub fn validate_basename(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        Err("Enter a name")
    } else if name.contains('/') {
        Err("Names cannot contain /")
    } else if matches!(name, "." | "..") {
        Err("That name is reserved")
    } else if name.contains('\0') {
        Err("Names cannot contain NUL characters")
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OperationRequestId(pub u64);

#[derive(Clone, Debug)]
pub struct RenameRequest {
    pub id: OperationRequestId,
    pub entry: FileEntry,
    pub new_name: String,
}

#[derive(Clone, Debug)]
pub struct CreateDirectoryRequest {
    pub id: OperationRequestId,
    pub parent: Location,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct CreateFileRequest {
    pub id: OperationRequestId,
    pub parent: Location,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct PasteRequest {
    pub id: OperationRequestId,
    pub destination: Location,
    pub sources: Vec<Location>,
    pub target_names: Vec<Option<String>>,
    pub move_sources: bool,
    pub overwrite_existing: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictAction {
    Cancel,
    Skip,
    KeepBoth,
    Replace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConflictResolution {
    pub action: ConflictAction,
    pub apply_to_remaining: bool,
}

#[derive(Debug, Default)]
struct ConflictResponse {
    resolution: Option<ConflictResolution>,
    waker: Option<Waker>,
    answered: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ConflictReply(Rc<RefCell<ConflictResponse>>);

impl ConflictReply {
    pub fn respond(&self, resolution: ConflictResolution) {
        let waker = {
            let mut response = self.0.borrow_mut();
            if response.answered {
                return;
            }
            response.answered = true;
            response.resolution = Some(resolution);
            response.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub(crate) async fn wait(&self) -> ConflictResolution {
        poll_fn(|context| {
            let mut response = self.0.borrow_mut();
            if let Some(resolution) = response.resolution.take() {
                Poll::Ready(resolution)
            } else {
                response.waker = Some(context.waker().clone());
                Poll::Pending
            }
        })
        .await
    }
}

#[derive(Clone, Debug)]
pub struct DeleteRequest {
    pub id: OperationRequestId,
    pub entries: Vec<FileEntry>,
    pub permanent: bool,
}

#[derive(Clone, Debug)]
pub struct RestoreRequest {
    pub id: OperationRequestId,
    pub entries: Vec<FileEntry>,
}

#[derive(Clone, Debug)]
pub struct ExtractArchiveRequest {
    pub id: OperationRequestId,
    pub archive: Location,
    pub destination: Location,
}

#[derive(Clone, Debug)]
pub struct CompressArchiveRequest {
    pub id: OperationRequestId,
    pub sources: Vec<Location>,
    pub output: Location,
}

#[derive(Clone, Debug)]
pub enum OperationEvent {
    Renamed {
        request_id: OperationRequestId,
    },
    Created {
        request_id: OperationRequestId,
    },
    Pasted {
        request_id: OperationRequestId,
    },
    TransferProgress {
        request_id: OperationRequestId,
        completed: usize,
        total: usize,
        /// Logical bytes processed, including an unpublished staging attempt.
        bytes_transferred: u64,
        /// Copy bytes reported by the backend, excluding rename-only completion.
        bytes_copied: u64,
        total_bytes: Option<u64>,
    },
    TransferConflict {
        request_id: OperationRequestId,
        source: FileEntry,
        existing: FileEntry,
        reply: ConflictReply,
    },
    TransferItemCompleted {
        request_id: OperationRequestId,
        source: Location,
        move_source: bool,
    },
    TransferCompletedWithErrors {
        request_id: OperationRequestId,
        message: String,
    },
    DeleteProgress {
        request_id: OperationRequestId,
        completed: usize,
        total: usize,
        deleted_location: Option<Location>,
    },
    RestoreProgress {
        request_id: OperationRequestId,
        completed: usize,
        total: usize,
        restored_location: Option<Location>,
    },
    ArchiveProgress {
        request_id: OperationRequestId,
        completed: usize,
        total: usize,
    },
    ArchiveCompleted {
        request_id: OperationRequestId,
        refresh: Location,
    },
    Deleted {
        request_id: OperationRequestId,
        locations: Vec<Location>,
    },
    CompletedWithErrors {
        request_id: OperationRequestId,
        deleted_locations: Vec<Location>,
        message: String,
    },
    Restored {
        request_id: OperationRequestId,
        locations: Vec<Location>,
    },
    RestoreCompletedWithErrors {
        request_id: OperationRequestId,
        restored_locations: Vec<Location>,
        message: String,
    },
    Failed {
        request_id: OperationRequestId,
        message: String,
    },
}

pub trait OperationProvider {
    fn rename(&self, request: RenameRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn create_directory(
        &self,
        request: CreateDirectoryRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle;
    fn create_file(
        &self,
        request: CreateFileRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle;
    fn paste(&self, request: PasteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn delete(&self, request: DeleteRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn restore(&self, request: RestoreRequest, emit: Rc<dyn Fn(OperationEvent)>) -> LoadHandle;
    fn extract_archive(
        &self,
        request: ExtractArchiveRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle;
    fn compress_archive(
        &self,
        request: CompressArchiveRequest,
        emit: Rc<dyn Fn(OperationEvent)>,
    ) -> LoadHandle;
}
