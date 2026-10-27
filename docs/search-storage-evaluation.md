# Search storage evaluation

Update October14: the first production storage increment now uses rusqlite0.40.2 with bundled
SQLite for private general-search snapshots. It shares the existing monitored catalog machinery
and delegates Home's Music subtree to the existing Music provider. The prototype findings below
still describe an experiment: FTS5 is not the production query path. A subsequent increment adds
exact-field candidate lookup and snapshot-aware pagination, retaining the bounded Rust scorer.

Measured October 14, 2026 with `python3 scripts/bench_search_catalog.py --repeats 21`.
This is a generated storage prototype, not the production Hermes search engine or a desktop
speed comparison. No existing files, music tags, user settings or collections were read or changed.

## Method

Generate 10,000 and 100,000 records with filenames, synthetic paths, song fields, types, extensions,
dates and sizes. Build a SQLite catalog in batches of 1,000 with exact-field indexes, an external
content FTS5 token/prefix index, and a filename trigram index. Close and reopen the database,
verify counts and index integrity, and check results against generated expected sets. Temporary
databases are removed on completion. The script has a 180-second time budget, a 192 MiB database
limit, an 8 MiB SQLite page cache and no memory mapping. These are prototype limits, not approved
production budgets; the Python record list and other allocations are additional memory.

This host used Python 3.14.8 and SQLite 3.54.0. Timings use a warm filesystem/cache and 21 samples
after one warmup; they include Python query/fetch overhead. They do not include crawling, file
metadata extraction, GTK rendering, watcher updates, production lock contention, or physical
cold-disk startup. Raw output from this run is `/tmp/hermes-search-storage-evaluation.json`.

| Operation, up to 100 returned results | 10k p50 / p95, ms | 100k p50 / p95, ms |
| --- | --- | --- |
| Token match ordered by row ID | 0.024 / 0.030 | 0.019 / 0.029 |
| Exact artist filter | 0.012 / 0.016 | 0.019 / 0.024 |
| Broad token match sorted by BM25 and ID | 3.173 / 3.454 | 32.329 / 33.002 |
| Filename infix candidates plus bounded Python reranking | 0.158 / 0.192 | 0.747 / 0.867 |
| No-hit substring scan with LIKE | 0.322 / 0.394 | 5.846 / 6.455 |

The 100k database occupied 37.7 MB for base records/indexes, 72.4 MB after token indexing, and
80.7 MB after filename trigrams. The 10k final database occupied 9.6 MB. Base/token/trigram build
stages took about 0.87/0.88/0.34 seconds at 100k. Reopen/count/first token query was about 2.1 ms
at 100k and 15.4 ms at 10k; these warm measurements and first-run effects are not cold-start
guarantees. The complete benchmark, including integrity and result checks, took 11.9 seconds.

## Findings and recommendation

SQLite with FTS5 is a promising durable catalog candidate. Indexed exact-field retrieval is cheap,
but broad relevance sorting has a visible cost that simple row-ID queries conceal. Prototype
production query plans and ranking in Rust before promising an end-to-end latency target.

The filename trigram experiment admits at most 500 candidates, checks literal substring matches,
and reranks within that bound. Broad matches are explicitly reported as truncated: taking the
first 500 row IDs cannot guarantee the best 100 matches over the entire catalog. Queries shorter
than three characters use a separate policy; trigram lookup does not handle them in this prototype.
Do not ship that bounded candidate ordering as complete relevance pagination.

Checks include Unicode/accent token behavior, punctuation and quotes, exact artist/album/title
sets, filter composition, persistence and index integrity. Token FTS intentionally returns no
results for the typo `aurra` or the infix `croarch`; the separate trigram path handles the latter.
Typo tolerance and filename substring behavior require deliberate retrieval/ranking policies.
These distinctions match the [SQLite FTS5 documentation](https://www.sqlite.org/fts5.html).

The included Python linear top-100 implementation is only a diagnostic approximation of existing
Hermes scoring. It measured about 19 ms p50 over 40k and 47 ms over 100k for one token query.
It is not the compiled Rust implementation, an equivalent query plan, or evidence of a Hermes
speedup. Keep it as a reproducible workload reference, not a comparative performance claim.

Next implementation gates: a reviewed Rust storage dependency, private cache ownership and
schema/rebuild recovery, bounded database/RAM budgets, shared identity-validated music enrichment,
watcher/reconciliation behavior, snapshot-aware pagination, cancellation during database queries,
coverage reporting, and stable exact/prefix/token/substring/fuzzy ranking. Authoritative ratings,
favorites, playlists and saved searches must remain independent of the disposable index.

The prototype itself did not modify the production backend. The subsequent storage increment
adds persistent general catalogs. Pagination now removes the100-result ceiling for shared search,
while retaining the40k/32 MiB catalog coverage limit.

## Production snapshot checks

General snapshots live under `$XDG_CACHE_HOME/strata/search-catalog/`, one SHA-256-derived file per
root. Directories/files require0700/0600 permissions and the current owner; symlinks, hard-linked
cache files and non-regular files are rejected. The64 MiB database limit is per root, not a global
cache-retention budget. SQL fields preserve opaque native path bytes and strong source identities.
Reads and writes use private staged copies, strict known schemas, bounded rows/pages/query settings,
atomic publication and cancellation checks. Unknown future versions are preserved; corrupt stores
fall back to a fresh scan with a save-failure notice rather than being silently erased. Known v1
snapshots can be replaced when the same root path is legitimately recreated. Original files and
authoritative collections/ratings are never modified.

New synthetic checks cover namespace/tag sharing, watcher updates, offline/recreated roots,
periodic reconciliation, cancellation, partial coverage, changed/deleted sources, native paths,
schema manipulation, hostile cache filesystem objects, rollback and future versions. A folder
query can reuse a complete active parent catalog; incomplete or unrepresented scopes start their
own catalog. Closed queries release their subscriptions and the last subscriber stops the worker.

Generated native debug workload:10,000 text files plus80 JSON-tagged synthetic audio names,
warm filesystem, low-priority build/test process. First cold names arrived~55 ms, reconciliation
finished~113 ms and80 synthetic tag reads occurred. Two reopened samples showed first snapshots
~64–65 ms, reconciliation~169–173 ms, and zero repeated tag reads. Twenty-one exact-filename
queries per run showed p50~19 ms and p95~20 ms, including worker scheduling; the database was
1,548,288 bytes. The fixture's JSON reader is intentionally cheap: warm filename display and
reconciliation were not faster than cold in this sample. Avoided tag work is verified, but real
sandbox music parsing, physical cold-disk startup, GTK painting and Dolphin parity are not measured.

Reproduce with the ignored `generated_catalog_restart_and_query_timings` test. Raw output is
`/tmp/hermes-catalog-native-benchmark.log`. Remaining gates include fuzzy/token query acceleration,
cache retention/clear controls and broader release corpus timings.

## Exact filters and snapshot pagination

The shared worker retains ranked item positions rather than cloning every matching source entry.
It scores all eligible entries before slicing a page. The dialog receives cumulative100-match pages;
the virtualized result-tab adapter explicitly requests all bounded-index matches. More requests carry
query generation, snapshot revision, shown count and total count. Obsolete/duplicate requests cannot
advance a different snapshot or replace a queued new query. Source changes reset dialog paging.

After publishing the first exact-filter query against a settled snapshot, a disposable in-memory
SQLite index is built lazily on the worker. Indexed fields are directory/type, extension, artist,
album and title, normalized with
Rust Unicode lowercasing to match the scorer. Parameter-bound intersections return every eligible
original item position in order, preserving content-extraction order and global relevance ranking.
Fuzzy/path/date/content checks remain with the original scorer. Failed/oversize candidate builds or
queries fall back to full scoring, never an empty or incomplete candidate set. Plain queries do not
pay the candidate build cost. Catalog updates discard this index; it is not another durable database.

Bounds:40k items,32 MiB database pages,2 MiB page cache,16 KiB rows, fixed bounded SQL, memory-only
temporary structures/journal and no attached databases. Cancellation is checked while building and
between returned rows; individual SQLite VM steps are not interrupted. The dialog preserves fixed
layout, unchanged rows and selection on page extension, and rejects old generations/revisions.
The32 MiB limit is the main candidate database's page budget, not total resident memory: source
items, ranking vectors, SQLite temporary structures and the memory journal have separate overhead.
These are bounded by item/query limits, not by a dedicated SQLite heap cap.
Initial results do not wait for index construction, but another request can briefly wait for that
worker job; a newer query cancels it. This is not yet a claim of faster fuzzy search, larger
Home coverage or Dolphin parity. Grouped album/artist entities are covered by the next increment;
Smart Folders are documented in the saved-query increment below.

Generated10k native debug fixture (21 warm retrieval samples per query): building the candidate
database took~104 ms and1,318,912 database bytes. Exact artist lookup (100 candidates) p95~0.033 ms,
title (one candidate)~0.008 ms, file-type (9,000 candidates)~2.47 ms; six-field intersection yielding
one candidate~1.45 ms. These measure candidate retrieval only, excluding scoring, worker scheduling,
source catalog loading and GTK rendering. No end-to-end speedup or cold-disk claim follows from them.
Run `generated_10k_exact_filter_retrieval_timings` explicitly; raw log
`/tmp/hermes-search-pages-benchmark.log`.

## Grouped query results

An optional grouped payload keeps typed virtual entities separate from physical SearchItems. The
same fully ranked eligible sources feed Files/Songs and metadata-derived Albums/Artists; entities
also match their own labels against free query terms. Counts are computed before page slicing.
All gives each nonempty group a balanced prefix before emitting section-ordered rows; each More
response extends each group's prefix. Typed selection keys preserve identity independently of a
representative artwork file. The original physical query path and result-tab adapter remain separate.

Album identity is normalized album-artist/title (track-artist fallback), not directory name. Artist
identity uses normalized track artist (album-artist fallback); absent/invalid metadata creates no
invented entities. Whitespace/case variants join; compilations and disc folders can join, and identical
album titles with different owners stay separate. Artist naming collisions/release editions are not
disambiguated by MusicBrainz IDs, which the current catalog does not retain.

Derived metadata is bounded to16MiB/40k entities, storing source positions rather than cloning all
representative SearchItems. Partial grouping has a distinct notice and never discards physical hits.
Source40k/32MiB coverage is unchanged; total grouped rows can exceed the source-entry count. Page
advancement therefore caps at the current result total, not the source-entry limit.

Entity activation searches that entity's eligible songs without autoplay or filesystem navigation.
Album song drilling/exports use disc/track order with filename-number fallback; other queries keep
their existing relevance ordering until explicitly sorted.
Files/Songs export URLs carry bounded normalized artist/album membership separately from the query;
legacy URLs still mean all physical results. Virtual-only exports and malformed/duplicate context
fields are rejected. Scoped membership, explicit tag filters and content consent stay independent.
This stage adds no persistent schema, external indexing, original-file writes or dependency changes.

## Authoritative saved queries

Named queries are separate from these disposable catalogs: private JSON in
`$XDG_DATA_HOME/strata/saved-searches/queries-v1.json`, version1, at most128 definitions/1MiB.
The saved name and canonical ScopedSearch URL preserve root/scope/query/physical result group and
optional normalized song membership. They do not store result files or create a new index.

A bounded worker owns all filesystem operations. Atomic publication, advisory single-writer
ownership, expected-byte checks and private directory/file permissions protect committed definitions.
Corrupt/future data and detected external changes block mutation until explicit Reload; paths on
offline drives are never silently pruned. UI waits for a committed snapshot before showing success;
ordinary validation errors leave editing available. Idle polling reads only a revision counter.
Application shutdown waits at most1s for pending commands; acknowledged saves have already synced.
Non-cooperating writers are not covered by a kernel-level compare-and-swap guarantee.

Opening a definition uses the existing monitored SearchFileSource, so current matches and coverage
notices retain their existing semantics. See `saved-searches.md` for interaction and UI-state details.
