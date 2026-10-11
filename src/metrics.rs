// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

static STARTED: OnceLock<Instant> = OnceLock::new();
static FIRST_BATCH_RENDERED: AtomicBool = AtomicBool::new(false);

pub fn initialize() {
    let _started = STARTED.set(Instant::now());
}

pub fn mark_window_presented() {
    if let Some(started) = STARTED.get() {
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            "application window presented"
        );
    }
}

pub fn mark_batch_rendered(entries: usize, render_started: Instant) {
    let render_micros = render_started.elapsed().as_micros() as u64;
    tracing::debug!(entries, render_micros, "directory batch rendered");

    if !FIRST_BATCH_RENDERED.swap(true, Ordering::Relaxed)
        && let Some(started) = STARTED.get()
    {
        tracing::info!(
            entries,
            render_micros,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "first directory batch rendered"
        );
    }
}

pub fn mark_directory_merge(existing_entries: usize, incoming_entries: usize, started: Instant) {
    tracing::debug!(
        existing_entries,
        incoming_entries,
        merge_micros = started.elapsed().as_micros() as u64,
        "directory batch merged"
    );
}
