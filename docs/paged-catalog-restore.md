# Paged saved-catalog restore

## Reader contract

`catalog_disk::PagedReader` owns a private copy of one version-1 catalog,
its SQLite connection, source-root handle, cursor and cumulative budgets.
Opening validates ownership, permissions, schema, root identity and row count.
The existing 512 MiB database limit and configured entry/memory limits apply.
The restore accumulator reserves the validated row count to match charged entry
slots. SQLite's bounded cache, transient pages and the first published prefix
are separate working memory from the aggregate snapshot-row budget.

Each call returns at most 256 source-validated entries in stored sequence order.
Signed and sparse sequences remain compatible with existing version-1 reading.
The cursor advances by raw rows, including stale entries. A fully stale page
returns an empty vector inside `Some`. Successful `None` is the completion signal.
Decoding, aggregate byte accounting, source checks and root checks precede cursor
advancement. Errors and cancellation invalidate the reader until it is reopened.

The cache-directory acquisition lock is released after the private snapshot is
secured. A writer can then publish a replacement while existing readers finish
their original snapshot. The SQLite connection closes before its private stage
is removed. Readers preserve the public catalog file, including malformed and
future-version data encountered during validation.

## Restart integration

The normal library startup path consumes this reader. It publishes one bounded
nonempty cached prefix while verifying the remaining saved rows. Cached output
retains `indexing=true` and the verification notice. The completed saved restore
also stays provisional until the normal filesystem scan reconciles current data.

A late decode, budget or root error withdraws the provisional prefix and reports
that the saved index is being rebuilt. The ordinary scan follows. Cancellation
stops later publications and releases the private reader. A new attempt can reopen
the preserved saved file. Stale-source notices are retained through restore.
Music supplied by the shared music subscription retains its existing scope rules.

## Verification and remaining architecture

Generated reader fixtures cover exact page boundaries, all-stale pages, aggregate
budget exhaustion after a valid page, malformed tails, cancellation, root changes,
concurrent publication, signed sparse sequences and stage cleanup. Integration
fixtures change a later source after the early publication, reject an invalid
tail with prefix withdrawal, and cancel/retry through the startup helper.

The scanner still owns an in-memory library snapshot. Query ranks now use
[paged SQLite storage](search-query-ranking.md) and shared workers use the
[paged source adapter](search-source-access.md). Moving source reads directly
to persistent catalog revisions, removing
scan/group limits and verifying full Home coverage and warm latency remain the
next search architecture gates. First-result and completion timings need measured
reference-machine evidence. Native Mac filesystem/profile and XPC qualification,
physical Asahi and other release gates remain separate. Exact source identities,
test counts and package evidence belong in the batch PR.
