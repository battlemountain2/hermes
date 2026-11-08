# Live transfer progress

## Byte accounting

Paste, copy and move use the existing staged transfer and recursive merge paths.
Progress now includes the active file while it is being staged. Native copy
callbacks update one shared sample per worker. The owning main context reads
that sample at most every 100 ms and flushes the final changed sample when the
worker returns. A busy interface cannot accumulate a queue of block callbacks.
GIO-backed transfers use its asynchronous progress callback and the same
rate-limited accounting.

Two counters serve different purposes. Logical processed bytes drive the size
readout and percentage. Backend-reported copy bytes drive the speed readout.
A rename-only move can advance logical completion without fabricating copy
throughput. These counters describe backend work, including staging and retries.
The item-completed event separately acknowledges a successful publication.

Current-attempt samples use a high-water mark. Success absorbs the active sample
once. Failed or retried attempts retain observed work and invalidate the original
one-pass total. Skips, no-op roots and invalid preflight metadata also invalidate
the estimate. Changed root type or size cannot retain a known total. Checked
arithmetic prevents an overflowing total from becoming a valid estimate.

Only regular top-level files with known sizes supply an aggregate byte total.
Folder totals stay unknown. Hermes performs no recursive size walk before
copying. The HUD pulses for unknown totals and explains that the total size is
unavailable. Its widget-owned frame callback refreshes speed and ETA every
250 ms, including when a backend stalls. A sample interval without additional
copy bytes clears stale speed. Removing the HUD releases that callback with its
widget. The callback holds a weak browser reference.

## Safety and cancellation

The native worker retains ownership of its staging directory until writing
stops. Dropping the polling future cancels the shared GIO token. It creates no
detached polling task. Remote callbacks check the same cancellation token before
reporting. The existing request identity filter rejects late operation events.

Publication and source-retention checks remain in the staged transfer. A new
destination appearing during exclusive hard-link publication returns GIO's
conflict error, allowing the existing decision/retry path to handle it. The
competing destination and original source are retained.

## Automated evidence and remaining acceptance

Generated tests exercise intermediate progress before publication, final sample
flush, cancelled and dropped workers, actual GIO progress cancellation, monotonic
retry accounting, skipped items, changed sizes/types, folder totals, symlinks,
rename-only moves and exclusive-publication races. A private GTK fixture checks
known/unknown totals, speed expiry, rename accounting and HUD release.

Exact source identities, local test totals and hosted artifact qualification
belong in the batch PR and evidence report. Actual remote GIO backend cancellation
and callback disposal still require GVfs/backend qualification. The synchronous
GIO cancellation fixture verifies native staging. Real device removal, physical full
disks, broader cross-filesystem/backend coverage, audible playback while copying
and desktop clipboard/drag interoperability retain their release gates. Existing
Cancel dismisses the operation view immediately. A durable cancelled-item summary
and shutdown/transfer audit remain separate acceptance work.
