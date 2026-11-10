# Search cache lifecycle

Issue #6 adds publication budgeting and lease-based recovery to the existing
private Linux search catalog files. These are derived caches in the active
Hermes profile's `search-catalog` directory.

## Saved database budget

Managed general and normalized source databases share a 512 MiB budget for their
published main files. Each publication accounts for replacing its previous
target. When space is needed, eligible databases are evicted in oldest-publication
order. A private publication lock serializes budgeting and replacement across
catalog roots. Other roots' acquisition locks are attempted without waiting.
Busy roots can prevent eviction and cause a visible save/fallback error.
An error before target replacement preserves that target, while earlier cache
evictions remain effective. An error synchronizing the directory after rename
can report failure with the complete new database already published.

Eviction accepts only recognized managed names and validated private regular
files with supported SQLite headers. Future or unknown formats are preserved.
Directory discovery is bounded and cancellable. Exhaustion or failed discovery
reports an error. Active readers use private snapshots, so public cache eviction
does not remove their current revision. A later visit can rebuild an evicted
saved database from the ordinary scan.

This budget covers published general and normalized main database bytes. Private
reader/build/publication copies, rollback journals, SQLite temporary workspaces,
Music's separate cache, artwork and process memory have independent accounting.
An application-wide peak-storage or RSS quota remains a separate task.

## Interrupted snapshot recovery

New private snapshots use `.snapshot-v2-PID-counter` names and hold an exclusive
kernel file lock on their directory throughout the Stage lifetime. This lease
continues after the catalog's parent acquisition lock is released. Process exit
releases the lease even when Rust destructors do not run.

The next acquisition of that catalog directory checks for abandoned versioned
stages under its parent lock. It attempts each stage lease without waiting.
Cleanup requires a current-user private 0700 directory and only the four known
SQLite payload/sidecar names, each an owned, single-link 0600 regular file.
Identity is rechecked before unlinking. Cleanup uses pinned directories and
individual unlinks, with directory synchronization after removal.

Live leased stages, older lease-free stages, unknown names or contents, shared
files and symlinks are preserved. An older stage's process ID alone cannot prove
it is abandoned. Recovery inspects at most 4,096 directory entries per acquisition
and checks cancellation between entries and file removals. Exhausting the
inspection budget defers further cleanup and allows ordinary catalog access.
A failed/cancelled
cleanup can leave a partially cleaned abandoned stage for a subsequent retry.
Directories for scopes that are never revisited require a later namespace-wide
maintenance pass.

## Evidence and remaining work

Focused fixtures cover quota replacement/eviction and lease recovery, including
a child process that exits with an open SQLite journal. The full regression suite
continues to cover saved catalogs, normalized source revisions and concurrent
readers. Exact-source test and artifact evidence belongs in this batch's PR.
Completed scale results stay in the [roadmap ledger](1.0-roadmap.md#completed-scale-checks-and-specific-remaining-work).

Producer consolidation, scan/metadata/grouping limit policy, temporary peak-space
accounting and physical target qualification remain separate release work.
