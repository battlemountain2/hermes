use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use crate::services::{OperationEvent, OperationRequestId};

pub(super) const UPDATE_INTERVAL: Duration = Duration::from_millis(100);

/// Logical bytes processed, including the active staging attempt. Publication is
/// acknowledged separately by TransferItemCompleted, never by this counter.
pub(super) struct Progress {
    id: OperationRequestId,
    emit: Rc<dyn Fn(OperationEvent)>,
    total: usize,
    completed: Cell<usize>,
    total_bytes: Cell<Option<u64>>,
    finished_bytes: Cell<u64>,
    current_bytes: Cell<u64>,
    copied_bytes: Cell<u64>,
    last_update: Cell<Instant>,
}

impl Progress {
    pub fn new(
        id: OperationRequestId,
        emit: Rc<dyn Fn(OperationEvent)>,
        total: usize,
        total_bytes: Option<u64>,
    ) -> Self {
        Self {
            id,
            emit,
            total,
            completed: Cell::new(0),
            total_bytes: Cell::new(total_bytes),
            finished_bytes: Cell::new(0),
            current_bytes: Cell::new(0),
            copied_bytes: Cell::new(0),
            last_update: Cell::new(Instant::now()),
        }
    }

    pub fn invalidate_total(&self) {
        self.total_bytes.set(None);
    }

    pub fn update_file(&self, bytes: u64, reported_total: u64, expected_size: u64) {
        if (reported_total > 0 && reported_total != expected_size) || bytes > expected_size {
            self.invalidate_total();
        }
        self.current_bytes.set(self.current_bytes.get().max(bytes));
        if self.last_update.get().elapsed() >= UPDATE_INTERVAL {
            self.report();
        }
    }

    pub fn finish_file(&self, size: u64) {
        let copied = self.current_bytes.replace(0);
        self.copied_bytes
            .set(self.copied_bytes.get().saturating_add(copied));
        self.add_finished(copied.max(size));
        if self.last_update.get().elapsed() >= UPDATE_INTERVAL {
            self.report();
        }
    }

    pub fn abandon_file(&self) {
        // Keep observed work monotonic, but a retry/failure can no longer use
        // the original one-pass byte estimate for a percentage or ETA.
        let copied = self.current_bytes.replace(0);
        self.copied_bytes
            .set(self.copied_bytes.get().saturating_add(copied));
        self.add_finished(copied);
        self.invalidate_total();
        self.report();
    }

    fn add_finished(&self, bytes: u64) {
        if let Some(total) = self.finished_bytes.get().checked_add(bytes) {
            self.finished_bytes.set(total);
        } else {
            self.finished_bytes.set(u64::MAX);
            self.invalidate_total();
        }
    }

    pub fn finish_root(&self, completed: usize) {
        self.completed.set(completed);
        self.report();
    }

    pub fn report(&self) {
        self.last_update.set(Instant::now());
        let bytes = self
            .finished_bytes
            .get()
            .checked_add(self.current_bytes.get())
            .unwrap_or_else(|| {
                self.invalidate_total();
                u64::MAX
            });
        if self.total_bytes.get().is_some_and(|total| bytes > total) {
            self.invalidate_total();
        }
        (self.emit)(OperationEvent::TransferProgress {
            request_id: self.id,
            completed: self.completed.get(),
            total: self.total,
            bytes_transferred: bytes,
            bytes_copied: self
                .copied_bytes
                .get()
                .saturating_add(self.current_bytes.get()),
            total_bytes: self.total_bytes.get(),
        });
    }
}

#[cfg(test)]
mod tests;
