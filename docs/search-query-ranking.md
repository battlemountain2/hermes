# Disk-backed query ranking

Issue #6 moves shared-catalog query ranks out of the complete Rust rank vector.
Each query owns one disposable SQLite database tied to its worker revision.
The saved catalog's private reader and on-disk version-1 format are separate.

## Publication and ordering

The scorer still reads the current immutable in-memory library snapshot and
retains fuzzy, accent-folded, literal/natural-language and content behavior.
Eligible scores enter the query database one at a time. Only a successfully
finished rank build becomes visible. Candidate-read failure discards the entire
unpublished database before a full-scoring replay. Reserved document-extraction
outcomes remain attached to the same source revision during that replay.

Score order is descending, followed by Rust path-component order and source
sequence for deterministic ties. Album drilldowns use existing disc/track and
filename-number defaults before path order. Source indices refer only to the
snapshot that supplied them. Generation, source revision, shown count and total
continue to guard pagination requests.

Rank reads use bounded keyset pages. Flat publication collects only the displayed
prefix. Group construction consumes the same paged rank stream and commits its
counts/entities only when the stream succeeds. A rank build or publication read
error clears the affected results and exposes a storage notice. A subsequent
query or source revision constructs a fresh database. Cancelled queries do not
publish results or failures after their cancellation is observed.

## Resource ownership

SQLite owns the temporary query file and closes it with the revision. The database
has a 512 MiB page ceiling, a 2 MiB configured page cache, disabled mmap and
file-backed temporary storage. Raw paths are limited to 16 KiB, with a separate
worst-case allowance for component-key escaping. This disposable workspace has its own ceiling,
separate from the persistent catalog and candidate database. It is not a setting
for the total process resident-memory or disk budget. Rows and reads are bounded,
and failed/unfinished databases cannot expose a readable prefix.

The worker still owns complete `SearchItem` snapshots. Grouped file/song source
positions and bounded entity metadata also remain in memory. Export/result-tab
APIs may deliberately request and materialize every result. Scanner/metadata/group
caps and source reconciliation are unchanged by this increment. The persistent
catalog as the sole paged source and the two-reference-machine 100k-file/latency
acceptance remain release gates.

## Verification

Storage fixtures compare SQL order with the previous Rust comparator across page
boundaries, score extremes, non-UTF-8 paths and album track defaults. Worker
fixtures cover ordinary scorer parity, late read withdrawal/recovery, stale cursor
rejection, grouped-stream failure and cancellation. Existing candidate-failure
content extraction tests protect same-revision replay without spending the
extraction budget twice. Exact local, hosted and package evidence belongs in the
batch pull request. Physical Linux/Asahi and native Mac acceptance remain pending.
