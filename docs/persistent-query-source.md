# Persistent normalized query sources

Issue #6 gives completed Linux search snapshots a direct SQLite-backed Source.
The library still scans and composes Entry snapshots. The worker publishes one
private, immutable normalized revision after its build succeeds. Indexing progress
continues through the existing memory adapter.

## Stored rows and incremental updates

Each root has a profile-separated cache under
`search-catalog/normalized-<native-root-sha256>/source.sqlite`. Its own private
parent directory provides the publication lock, separate from the legacy saved
Entry catalog and other roots.

A records table keys native path bytes to raw input signatures, normalized
filename/relative-path/metadata text, display values, identity, tags and the
original fresh-conversion admission cost. Signatures compare directory kind,
modified time, complete identity and tags. Unchanged inputs reuse normalized
rows across restarts and reordering. A new or changed path is converted once.
Invalid cached raw fields are rebuilt from the incoming snapshot. Removed paths
are pruned. Positions and their dense ordinals are rebuilt transactionally for
the new snapshot, preserving scoped holes and original source indices.
Schema version 2 includes a covering copy of normalized scoring fields in the
position map. Plain text scoring reads that map sequentially. Tag/date/rating,
content, inferred-literal and entity-dependent queries retain full rows, and all
displayed results resolve complete records. The scorer and ranking rules remain
the acceptance authority for both projections.

The schema/application ID, root bytes and root device/inode key are validated.
Metadata counts, ordinals, references, row costs and limits are checked before
publication. A newer schema is preserved. Private malformed copies can be reset
or repaired, leaving the public cache untouched until a complete build publishes.
Normalization changes require a schema/version decision before reusing old rows.

## Query ownership and cancellation

Source lookup and indexed keyset pages read stored normalized columns directly.
Page decoding reuses up to 256 item slots and string allocations. A completed
SQL-backed Source retains no Entry or SearchItem collection. Visitors run after
the database mutex is released, permitting representative lookups through the
same Source. Query cancellation discards partial work and leaves the immutable
source available to the next query.

A build-only SQLite progress callback checks cancellation every 1,000 virtual
machine operations, including aggregate validation and pruning. Its connection
closes before publication opens the independent query connection. An expired
build token cannot cancel a later query. Copy loops check cancellation every
64 KiB. Filesystem synchronization remains a synchronous operating-system call.

## Root binding and fallback

A library configuration captures its directory identity before scanning starts.
Registry reuse requires that binding to remain current. A selected child scope
also requires the snapshot's directory identity to match the current directory.
The search worker checks binding before building, after building and after a
fallback is prepared. Replacement withdraws old results under a new revision and
restarts the producer or waits for a verified child-scope refresh. Missing roots
cannot trigger an endless refresh loop.
Root bindings accompany each shared snapshot revision atomically. Memory sources
retain those bindings and validate active reads and publication even without a
new snapshot event. Relevant ancestor and composed Music identities participate;
unrelated sibling scopes do not. Music composition retains its component
producer's binding and withdraws mismatched component rows.

Cache failures with a verified root fall back to the memory adapter and expose a
notice. Root changes cannot turn a rejected SQL build into stale memory results.
The Mac implementation currently retains memory-backed querying while its native
filesystem/storage adapters await qualification.

## Atomic files and commit boundary

The filesystem wrapper reuses the saved catalog's descriptor-pinned, no-follow
paths, private 0700 directories and 0600 single-link files. SQLite writes only a
private copied stage. Publication closes and syncs the writer, copies and syncs a
second stage, validates the root, then atomically replaces the public cache and
syncs its directory. Readers retain their original private inode. The directory
lock is explicitly released before returning the reader.

Cancellation/errors before replacement preserve the prior public bytes. After
rename, a directory-sync or unlock failure may report an error even though a
complete new public file exists. The caller can use a verified memory snapshot;
a later build validates the public cache before reuse. Cleanup removes only the
current operation's owned stages after closing their SQLite connections.

## Resource scope and remaining gates

Each normalized database has a 512 MiB main-file ceiling, a 2 MiB configured page
cache, disabled mmap and file-backed temporary storage. Encoded rows are bounded
to 64 KiB, native paths and individual normalized fields to 16 KiB, and combined
raw tags to 8 KiB. Exceeding cache capabilities produces an explicit fallback
notice. Entry and logical transformed-item admission limits preserve the existing
coverage notice behavior.

This derived cache coexists with the legacy saved Entry catalog during migration.
The [cache lifecycle policy](search-cache-lifecycle.md) shares a 512 MiB budget
across published general and normalized main files. Rank/candidate workspaces,
SQLite journals and private revision copies have separate accounting. An old reader, old public cache,
new private build and publication copy can approach 2 GiB for this cache alone,
before journals or additional readers. There is no application-wide disk/RSS
quota. Consolidating producer storage and temporary peak-space accounting remain
release gates. New stages carry kernel leases and abandoned stages are recovered
on the next acquisition of their directory. Older lease-free stages and unvisited
scope cleanup remain separate work. Normal closure and recoverable failures
clean up their own stages.

The producer's complete Entry vectors, scan/group caps, grouped position/entity
collections, end-to-end eligible-file accounting and the two physical reference-machine
latency target remain open. The [roadmap scale ledger](1.0-roadmap.md#completed-scale-checks-and-specific-remaining-work)
records completed 100k checks, including this source's build/read/reopen fixture.
Exact-source tests, generated timing, hosted attempts
and native package provenance belong in the batch PR. Synthetic debug timing
alone does not qualify physical Linux, Asahi or macOS performance.

## Dependency gate

The updated advisory database flagged bundled SQLite in the prior
`libsqlite3-sys 0.38.2` dependency with RUSTSEC-2026-0459. This batch upgrades
rusqlite to 0.41.1 and libsqlite3-sys to 0.39.1, which bundles SQLite 3.54.1.
A test checks the linked engine meets the patched 3.54.0 version floor. The
advisory is retained in validation evidence, with no policy exclusion added.
