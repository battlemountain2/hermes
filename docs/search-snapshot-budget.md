# Search snapshot admission

Issue #6 bounds private catalog snapshot main files across cooperating Hermes
instances in one profile. The shared reservation budget defaults to 2 GiB.
Published general and normalized databases retain their separate shared 512 MiB
[publication budget](search-cache-lifecycle.md).

## Reservation lifetime

A writable builder reserves 512 MiB before its stage directory or database is
created. Read copies reserve the source file's exact byte length. Admission and
marker changes use the private namespace-wide `.staging-budget-lock` with
descriptor-pinned paths, ownership/permission/link checks and bounded lock waits.

Version-3 stages retain the lifetime directory lease and add a fixed, private
16-byte `.reservation` record. Discovery rebuilds accounting from current records
after a process exits. The
record is written before copying data or opening writable SQLite. Copy loops
start at the source beginning, reject growth past their allowance and verify
the complete source length and identity.

After SQLite closes, the writer syncs its main file and seals the stage. Future
writable opens are disabled before the reservation is reduced to actual bytes.
Errors during or after the marker write preserve that read-only state. A
publication copy then reserves its exact size. Readers keep those reservations
until their private stages close. Drop removes the marker and stage, releasing
the charge. Accounting rechecks parent bindings so a concurrently dropped stage
contributes zero once detached.

SQLite's writable page ceiling is calculated from the actual database page size.
Existing allocated pages must already fit the requested byte allowance. The
returned ceiling is verified. This also fixes the older general catalog's fixed
page-count assumption for databases with larger pages.

## Recovery and compatibility

Admission inspects at most 16,384 entries across the namespace and recognized
normalized scope directories, with cancellation checks throughout. Incomplete
discovery refuses new admission with a visible cache error. Idle peer directories
use nonblocking acquisition locks. Reclamation follows the existing private
stage validation and lease rules.

At most 16 abandoned stages are reclaimed per admission. Live leases consume no
reclamation allowance. A later orphan can therefore be recovered even after a
prefix of live readers. Malformed orphan records can be removed with their
verified abandoned stage. Malformed live records fail closed.

Version-2 leased stages and older lease-free stages are conservatively charged
512 MiB each. Compatible abandoned leased stages can be reclaimed. Older
lease-free stages are preserved. A stage being dropped can briefly retain a
conservative charge until its parent entry disappears.

This admission protocol coordinates participating Hermes builds. Older writers
and changes made outside the protocol require separate compatibility review.

## Resource scope and evidence

The 2 GiB ceiling covers admitted snapshot main-file capacity. Rollback journals,
SQLite query/sort workspaces, the published cache, Music/artwork caches, allocated
filesystem blocks and process memory have separate accounting. Whole-application
disk and RSS ceilings remain release work.

Focused fixtures cover full reservations with tiny files, concurrent scope
builders, exact read-copy charging, sealing, failed marker writes/synchronization,
detached stages, older versions, cross-scope crash recovery, malformed objects,
cancellation and bounded discovery. SQLite capacity fixtures exercise 512-byte,
4 KiB and 64 KiB pages and real SQLITE_FULL results with small byte budgets.
Exact-source full-suite, hosted and package evidence belongs in the batch PR.
The [completed scale ledger](1.0-roadmap.md#completed-scale-checks-and-specific-remaining-work)
continues to carry the earlier results forward.
