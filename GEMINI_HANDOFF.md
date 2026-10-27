# Gemini handoff: smarter built-in Hermes search

## Start here

Work from the existing local checkout: `/home/bry/hermes`.
Read `AGENTS.md`, this handoff, then the latest sections of `NEXT_AGENT.md` before editing.
Current branch: `chore/2-antigravity-handoff`; last local commit when this handoff was prepared:
`70c44b2`. There are many modified AND untracked files containing completed work. The GitHub remote
is `git@github.com:battlemountain2/hermes.git`, but a fresh clone will not include this working tree's
uncommitted changes. Do not reset, clean, stash, replace, or wholesale regenerate this checkout.
Do not work concurrently with another coding agent editing these same files.

The app is branded Hermes; the Rust package/binary remains `strata`, version0.2.0. The user will
explicitly decide when1.0 is ready. Do not bump versions, commit, push, create branches, or open PRs
without separate authorization and the repository's issue/PR workflow.

## Latest user intent — this supersedes the earlier Smart Folder milestone

The user wanted better built-in fuzzy/natural-language searching, NOT a prominent saved-search
feature. Saved searches/Smart Folders were implemented before this misunderstanding was clarified.
Hide their sidebar section and Save search affordance by default, preserving their definitions and
backend. Do not delete saved queries, remove their storage, or rebuild them as another search mode.

Keep one existing search box. Improve its matching, understanding and appearance together. The user
also wants the filter panel to feel like Hermes, particularly its fonts. We agreed to a compact
visual mockup before implementation; present that first, obtain feedback, then build in increments.
No smarter-search engine or visual redesign has been implemented since this planning discussion.

## Agreed search behavior

1. Better word matching and ranking across filenames, song titles, artists, albums and paths.
   Support reordered words, partial names, punctuation/Unicode normalization and bounded typo
   tolerance. Strong name/title/tag matches should beat incidental directory/path matches.
2. A lightweight LOCAL interpreter for common phrases, not an LLM chat screen or cloud dependency.
   Initial examples:
   - `Beach House songs`: audio matching Beach House.
   - `PDFs in Downloads`: PDF files within the applicable Downloads location.
   - `videos from last week`: videos modified in the previous local calendar week.
   - `FLAC songs by Cameron Winter`: FLAC audio matching that artist.
3. Preserve unrecognized words as ordinary search terms. Quoted text stays literal. Do not silently
   turn uncertain text into strict exact artist/album constraints that discard useful matches.
   Protect real names/titles such as `Yesterday` and `Songs for the Deaf` from over-eager parsing.
   Existing explicit typed filters must keep working and take precedence over inferred intent.
4. Show inferred constraints visibly and make them removable/editable in the existing filter area.
   Dates currently mean MODIFIED time, not image capture/creation time: disclose that clearly.
   Do not switch result tabs or expand the chosen search root unexpectedly while the user types.
5. Names and metadata first. Document contents remain explicitly opt-in. No image recognition,
   OCR, semantic embeddings, new network service or promise to understand arbitrary language.

Dates should be evaluated consistently for a query generation using the user's local timezone.
Test week boundaries and daylight-saving transitions; distinguish calendar `last week` from rolling
`past 7 days`. Relative requests should retain understandable behavior when reopened/refreshed.
Do not hardcode the current date or Denver timezone into the implementation.

## Agreed visual direction

- Honor the selected Hermes interface font AND font size throughout input, filters, dropdowns,
  popover menus and results; update live with settings. Avoid fixed tiny10–12px filter labels.
  Inspect the existing theme-generated font overrides before assuming all hardcoded CSS wins.
- Keep readable input, restrained borders, square edges and less competing toolbar clutter.
- Reorganize existing filter controls into Files, Music and Contents sections. Keep content search
  opt-in. Avoid another settings page or a separate smart-search screen.
- Use compact, removable interpretations such as `Videos · Modified last week` in reserved space.
- Keep consistent icon/artwork slots, clear result names, quieter paths/metadata and type cues.
  Preserve useful All/Files/Songs/Albums/Artists grouping without pill/button overload.
- Fixed row heights and reserved status/filter space: no bouncing, resizing or expanding panels
  while typing, indexing, receiving metadata or paging results.
- Reuse semantic theme colors, existing controls and Lucide icons; support dark/light/Noctalia,
  hover, selection, focus and disabled states. No static theme colors or new hardcoded fonts.

## Current implementation / relevant files

- `src/services/search.rs`: ParsedSearchQuery, scoring, filter extraction, date bounds. Plain terms
  currently use exact substrings, cross-field term containment and ordered path subsequences;
  this is not general spelling correction. Date filters currently support Today/Yesterday only.
- `src/services/search/worker.rs`: generations, shared queries, ranking, cancellation and paging.
- `src/services/search/candidates.rs`: lazy in-memory SQLite EXACT-filter candidates. New matching
  semantics must not be accidentally narrowed by this optimization; fallback must stay correct.
- `src/services/search/groups.rs`: typed album/artist entities and physical Files/Songs grouping.
- `src/services/search/scope.rs`: scoped search URLs, physical group/entity membership and validation.
- `src/ui/search.rs`, `src/ui/search/results.rs`, `src/style.css`: search UI and filter popover.
- `src/ui/theme.rs`: interface-font settings, font sizes and generated theme CSS.
- `src/adapters/search_source.rs`: result-tab adapter using the same search engine.
- `src/services/saved_searches.rs` and adjacent disk/tests: authoritative saved definitions.
- `src/ui/saved_searches.rs`, `src/ui/window/saved_searches.rs`, `src/ui/window.rs`: editor/sidebar
  integration and the default-visible affordances to put out of the way, not destroy.

Keep one interpretation/matching contract across dialog results, grouped entities, exact-filter
candidates and result tabs. Do not bolt an unrelated search implementation onto the interface.
Keep real file identity separate from virtual album/artist identity; never invent filesystem paths
for entities. Preserve stable selection, stale-response guards and album disc/track ordering.

The shared catalog already persists and watches sources; Home reuses Music's metadata catalog.
Current coverage remains bounded40k entries/32MiB, metadata20k reads, with visible partial-coverage
notices. Do not silently raise limits, claim complete Home coverage, or build another index per
keystroke. Keep cancellation and bounded matching costs; measure latency rather than claiming speed.

## Safety and validation

DO NOT launch/restart/reposition the user's Hermes window. Do not run `cargo run`, `scripts/dev.sh`,
or restart-on-build watchers. Do not manipulate desktop audio, index personal Home for tests, or
change the user's preferences to make fixtures pass. Use generated temporary files and private
XDG config/data/cache/state/runtime paths. Use private GTK Broadway and D-Bus for UI tests, then
stop only the servers you started. Avoid rebuilding the debug main binary unnecessarily.

Run heavy validation sequentially, not competing processes:

```
nice -n 10 ionice -c 3 cargo test --bin strata --no-run --jobs 2
nice -n 10 ionice -c 3 cargo check --all-targets --jobs 2
nice -n 10 ionice -c 3 cargo build --release --jobs 2
```

Run the resulting unit-test executable with `--test-threads=2` and private XDG paths. Optional GTK
tests need separate processes/private displays; do not execute every ignored test indiscriminately.
Keep tests in adjacent modules per AGENTS.md. Format new/touched modules selectively, not entire
legacy browser/window files. Do not introduce inline code comments or unrelated cleanup.

Last completed baseline:733 normal binary tests pass/104 ignored; eight isolated integration/GTK
checks pass; all-target check/release build pass with pre-existing warnings. Logs are
`/tmp/hermes-saved-searches-{compile,unit,check,release,ui,layers}.log`. The latest volume recovery fix
and all prior media/performance/music work must remain intact.

Add regression cases for normal names, misspellings/reordered words, accented names, artist/title
ambiguity, literal dates, explicit filter precedence, constrained roots, local date boundaries,
shared dialog/tab semantics, stale query cancellation and stable panel geometry at multiple font
sizes. Compare ranking correctness and typing latency against a deterministic generated corpus.

## Deliver incrementally

1. Show the compact search/filter visual mockup and confirm it with the user.
2. Improve matching/ranking with focused tests.
3. Add bounded phrase interpretation, preserving literal/explicit behavior.
4. Integrate the agreed styling and visible/removable interpretations without layout animation.
5. Validate correctness, layout, performance and existing search/media regressions; update
   NEXT_AGENT.md with actual outcomes and remaining limits. Do not claim unperformed visual QA.

Full generic language understanding, bigger catalog coverage, filesystem-mounted virtual folders,
advanced rule builders, ratings and unrelated file-manager release gates are outside this increment.
