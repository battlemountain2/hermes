# Paged search source access

Issue #6 removes the shared search worker's second complete `SearchItem` copy of
its library snapshot. One immutable source adapter retains the library's original
`Arc<Vec<Entry>>`. Scoring, candidate construction, result materialization and
group representatives use the same fallible source interface.

## Scope and positions

Source construction checks the requested root and admits eligible entries under
the existing entry and logical transformed-item byte budgets. It excludes the
root itself and entries outside the requested root. Notices identify unavailable
roots and admission limits. This admission accounting bounds eligible search
work; it is not a measurement of total process resident memory.

Positions refer to the original immutable library snapshot. Out-of-scope rows
remain holes. Candidate and rank databases retain those original positions,
including when an unrelated entry precedes the first eligible result. Every
lookup checks the adapter's admitted range and scope. A source replacement
rebuilds ranks/candidates and advances the worker revision. An earlier source
handle continues to refer to its original immutable snapshot.

The adapter creates `SearchItem` values on demand with the same filename, relative
path, metadata normalization, tags and source identity as the previous worker.
Full scoring and candidate construction consume at most 256 eligible items per
source page. Single result and representative lookups materialize only the
requested item. The worker retains no additional complete item or position list.
Page slots reuse path and normalized ASCII string allocations. Unicode keeps
Rust's whole-string contextual lowercasing. Admission continues to use the
original fresh conversion, keeping its exact logical-budget boundaries.

## Failure and publication

The interface returns errors for reads and page visits so a future disk-backed
source can report failures through the existing revision-withdrawal protocol.
Cancelled work stops before subsequent page visits and publication. Invalid
ranked positions or grouped representatives fail the whole response. Failed
materialization clears results/counts under a newer revision, permitting the UI
to accept the withdrawal and reject stale pagination requests.

Grouped file/song position lists and bounded entity summaries remain in memory.
Flat exports can deliberately request all matching results. Group page creation
checks cancellation while resolving files and entity representatives. Candidate
failure still discards unpublished ranks and retries full scoring within the
same source revision, preserving reserved document-extraction outcomes.

## Remaining architecture

The current backing is the library's immutable `Entry` snapshot. Library scanning,
composition, saved-catalog restoration and publication still retain full entry
vectors. Direct reads from a persistent catalog revision require a disk-backed
implementation of this interface and atomic producer/publication integration.
The version-1 saved catalog and the existing private paged reader keep their
current contracts. Scan/group caps, source reconciliation, the two-reference-
machine 100k-file/latency gate and physical platform qualification remain open.

Test-only item-backed sources preserve crafted legacy scorer fixtures. Their
complete `SearchItem` vectors are excluded from the production adapter.

The adapter trades retained duplicate data for repeated conversion work. Generated
timing currently measures higher warm-query CPU/latency than the prior retained
item snapshot. Page-slot reuse reduces allocation churn, but direct persistent
normalized-row access remains a performance requirement. Exact comparative
measurements and their debug/synthetic scope belong in the batch PR.

## Verification

Adapter fixtures cover conversion, scopes, stable positions, source ownership,
limits, bounded page visits and cancellation. Integration fixtures exercise
out-of-scope holes through full scoring, SQL candidates, ranking, grouped
representatives, source replacement and failed materialization withdrawal.
Existing content-budget replay, literal/natural-language, Unicode/fuzzy, album
ordering and stale-cursor tests remain required. Exact-source local/hosted tests,
package provenance and optional generated timing belong in the batch PR.
