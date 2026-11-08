# Bounded search candidate paging

## Implemented increment

The existing shared search catalog remains responsible for scope scanning and
metadata. Its worker previously built an auxiliary in-memory SQL index limited
to 40,000 entries and collected every matching candidate ID before ranking.
This increment removes the auxiliary entry cutoff and streams IDs to the
existing scorer in ordered pages of at most 256 positions.

Explicit directory, category, extension, artist, album and title constraints
use indexed keyset queries. Cursor and limit parameters share the existing
bounded SQL parameter policy. Literal/inferred queries keep their existing
candidate interpretation and full fuzzy/accent/content scoring. Result revision,
selection, grouped entity and disc/track ordering behavior is preserved.

The auxiliary database keeps its existing 32 MiB size limit and 2 MiB page cache.
Input accounting covers its stored columns. Unindexed strings and spare capacity
in the shared snapshot no longer consume this separate input estimate. Source
rows, normalized rows, SQLite pages and values retain explicit bounds. The
database uses SQLite-owned temporary storage with file-backed spill capability
and connection-bound cleanup. [SQLite temporary databases](https://www.sqlite.org/inmemorydb.html)

## Failure and cancellation

Each query starts its own increasing cursor. The worker discards a failed
candidate pass before full-scoring the snapshot. Errors after earlier pages
cannot publish a successfully truncated result or duplicate their prior ranks.
Cancellation checks run at row/page boundaries and during ranking. Existing
generation checks suppress cancelled publications and stale page requests.

Budget-consuming document extraction outcomes have reserved replay capacity.
Ordinary text hits can occupy at most the normal result-limit admission range.
The bounded document-extraction budget supplies at most 32 additional outcomes.
This permits a failed indexed pass to replay PDF/Office results without spending
the extraction budget again. Known source identities retain equality checks
across snapshots. Unknown identities can replay only within the same nonzero
snapshot revision. Legacy non-shared scans retain their existing identity policy.

## Evidence and next gate

Generated tests cover 100,000 auxiliary rows, six simultaneous filters, all 64
filter subsets, sparse matches, empty/exact page boundaries, actual SQL failure
between pages, visitor failure and cancellation. Worker fixtures compare complete
ranking above the former cutoff and document-content replay with identified and
unidentified sources. Existing grouped/literal/pagination fixtures remain gates.

Query ranks now use [paged SQLite storage](search-query-ranking.md), and shared
queries use [source paging](search-source-access.md) over original Entry snapshots.
The shared 100k scan/metadata caps, grouping budgets and catalog reconciliation still
need the next architecture increment. Complete 100k-file Home coverage,
restart/offline correctness and the under-100-ms warm first-page p95 goal on both
reference machines remain release gates. Exact source, local/hosted counts and
package evidence are recorded in the batch PR.
