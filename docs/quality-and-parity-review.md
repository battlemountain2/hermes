# Hermes quality, performance and feature review

Reviewed 2026-10-10 against the current working tree. This is a prioritized engineering review,
not a claim of feature or speed parity with Dolphin. Column sizing remains deferred.

## What is already here

Three virtualized browser views, Miller-column navigation, hover peeks, persistent/reorderable
tabs, pinned places, mounted-device monitoring, copy/move/conflict UI, Trash/restore, properties,
Open With, archive creation/extraction, scoped metadata/content search, rich sandboxed previews,
image/PDF/video fullscreen, full-length music/video playback and a shared live music catalog.
Search result tabs can preserve a query; the player has an editable, persisted queue and waveforms.
These should be hardened and connected, not rebuilt under new names.

## Highest-priority findings

1. **Replacement safety follow-up implemented for native files/links.** Paste no longer predeletes
   destinations. Copies stage beside the target and publish atomically; overwrite-approved same-device
   moves rename directly. Missing/changed sources, simulated write failure and precommit cancellation
   preserve old targets. Folder replacement and remote replacement fail closed with guidance; no
   silent folder merge. Non-overwrite staging needs hard-link support. New-folder/remote failures may
   leave partial new output; commit races can complete despite cancellation. This is not a power-loss
   or hostile-path-race guarantee. Full folder transactions and backend-specific recovery remain open.
2. **Transfer outcomes and recovery follow-up implemented.** Successful roots are reported separately;
   failures do not abandon the remainder. Only confirmed successful moves clear owned cut items, with
   failed/cancelled items retained and newer clipboard content protected. Overlapping operations are
   rejected instead of dropping active handles. Undo remains an unconnected draft, not runtime support.
3. **Cache memory and identity partially addressed.** Sandbox payload caches now enforce 16 MiB
   thumbnail / 64 MiB preview budgets plus entry limits, LRU reuse, device/inode/nanosecond mtime/ctime
   identity and changed-during-render rejection. UI thumbnails now validate the actual selected cover
   as well as the directory, off-thread; warm folder hits skip rescanning and stale/cancelled bindings
   cannot populate the cache. Cached Vec payloads are still cloned. Bounded alternate cover fallback
   and MPRIS artwork revalidation remain useful further work.
4. **Navigation persistence partially addressed.** Trail saves now serialize/write/fsync off the UI
   thread through one ordered/coalescing writer per path, using private unique temporary files and
   atomic replacement. Load/flush/shutdown/final drop wait deliberately; asynchronous failures log.
   Independent sidebar bookmark/order writers can still finish out of order and need the same treatment.
5. **Large-directory and library hot paths.** The first visible batch is now capped at 32, reducing
   measured first insertion from ~22 ms to ~6 ms in a release fixture without claiming faster complete
   enumeration. Tab widgets are reused during activation/unchanged refresh. Remaining batches can
   still rebuild sorted vectors for interleaved entries.
   Sort changes remain synchronous; Music projects/clones/sorts changed snapshots and rebuilds search
   strings during filtering. Measure first, then add adaptive batches, normalized search fields,
   cached album membership and revision-checked background projection. Retain virtualization,
   incremental results, selection by native location and cancellation guards.
6. **Polish discrepancies.** Properties reports `PINNED: No` and disables its Pin action despite
   working sidebar pinning. Name sorting is case-sensitive lexical ordering; Type sorts entry kind
   rather than useful file types. File audio previews now use artwork, single-track playback and
   embedded metadata instead of a separate waveform renderer. Music retains its real-data waveform.
   Correct the remaining inconsistencies rather than adding another visual layer.

Evidence entry points: `src/adapters/local_operations.rs`, `src/ui/browser.rs`,
`src/app/browser.rs`, `src/app/navigation.rs`, `src/adapters/local_trails.rs`,
`src/ui/window.rs`, `src/sandbox/browser.rs`, `src/ui/thumbnail.rs`,
`src/music/library.rs`, `src/ui/music.rs`, `src/services/mod.rs`, `src/ui/mod.rs`.
The follow-up fixes above have generated-fixture coverage; remaining limitations are explicit.

## Performance work started in this update

The 2026-10-10 follow-up fixes strong GTK controller/list-item/ancestor references that kept
discarded folder views alive across navigation. Generated weak-reference lifecycle probes now
cover Columns, Grid and Explorer widgets, models and items. Icon registration uses keyed weak
references and periodic cleanup rather than scanning every live icon on every bind; live theme
updates and icon replacement remain supported.

Native folder snapshots can now be reused for up to 30 seconds within a 12-directory / estimated
32 MiB budget, but only with successful directory/ancestor monitors. The routed provider also
forwards live directory changes again. Changes, cancellation, refresh, failed/incomplete loads,
symlink retargeting and eviction invalidate reuse. Invalidated partial replay resets before a
fresh enumeration; stale request IDs remain rejected. Remote/search directories are not cached.
That provider cache is separate from the new per-window loaded-tab retention. Up to six inactive
native tabs now keep their existing GTK presentations, selection, filters, scroll and history within
an estimated 64 MiB accounting budget. Watched changes, late metadata, changed preferences or
30-second revalidation reload contents before treating them as ready. Closing/evicting tabs releases
their views and monitors. Incomplete, failed, unmonitored, oversized and remote/search views reload
normally. Hidden alternate Grid/Explorer panes are still built only when selected; cold Grid binding
and MOV/JPG/HEIF decoding remain performance targets. See the updated performance baseline for
model timing methodology and limitations; retention is not a hard process-memory cap or paint benchmark.

Music now supports complete-album double-click playback, a hideable queue and compact secondary
controls. File previews expose read-only embedded tags/properties and independent playback that
pauses main music without replacing its queue. These are not filesystem tagging, ratings,
editable music tags, gapless playback or Dolphin feature parity.

The Music follow-up adds right-click/keyboard track and album queue menus and per-window waveform
visibility. HEIF thumbnails can decode an adequate embedded thumbnail, with bounded primary-image
fallback; full previews keep their original-image resolution. Distinct cached sandbox operations
make repeat HEIF visits cheap without sharing thumbnail output with full previews. First decoding
of thumbnail-less camera images remains an open performance cost.

Directory merges now return unchanged storage for empty batches and append non-overlapping,
already-ordered batches into the existing vector rather than draining/copying all previous entries.
Strict ordering preserves the old incoming-before-equal behavior; interleaved batches retain the
existing merge algorithm. Selection handling and emitted splices remain unchanged.
The debug log now reports `directory batch merged`, separate from GTK model insertion timing.

A synthetic debug-build probe merges 100,000 in-memory entries in 128-entry ordered batches:
one before sample was 3.25 seconds; one after sample was 15.17 milliseconds. This isolates the
favorable append case. It excludes enumeration, parser workers, GTK model updates, painting,
filesystem cache effects and Dolphin. It is **not** a 200-times-faster file browser claim, a release
benchmark or a universal latency guarantee. Random/interleaved batches still need measurement.

Reproduce with the ignored `profile_ordered_directory_merges` test. Storage-reuse and existing
selection/sort tests cover correctness. Broader targets need realistic measurements before tuning.

Music windows now share one application-owned controller, stream, queue and session writer instead
of competing per-window players. Idle transport notifications stop without an active stream;
unchanged play/pause/speaker icons are reused. MPRIS capabilities avoid allocating a full ordered
queue on every snapshot. These are targeted ownership/hot-path improvements, not a broad rewrite.

Validation: 414 regular tests and four explicit desktop/private-bus fixtures pass; 28 optional tests
remain excluded from the default suite. Production build/all-target checks pass with existing
unrelated warnings. A separate production smoke uses temporary HOME/XDG state, a private bus and
GTK Broadway display with generated silent media. It verifies real service ownership, playback,
sandboxed artwork delivery and shutdown cleanup without restarting the personal application.
Installed desktop-widget/media-key acceptance remains separate from protocol validation.

## Gaps compared with mature file managers

| Workflow | Hermes now | Useful next slice |
| --- | --- | --- |
| Independent split browsing | Columns represent one navigation path, not two independent folders | Optional split panes with a clear active pane and cross-pane transfers |
| Reliable operation recovery | Conflict/progress UI exists; partial outcomes and replacement safety need work | Safe replacement, per-item retry, then tested undo |
| Large batch rename | Draft module is not wired into runtime | Preview names/conflicts before applying; cancellation and recovery |
| Folder-specific presentation | Tab preferences exist, not full per-folder presets | Persist view, sorting/grouping, thumbnail size per folder |
| Useful file grouping | Basic name/size/modified/kind and album-track sorting | Natural names, meaningful type groups, date grouping |
| Finder-style organization | Search tabs exist; file tags and dedicated Smart Folders do not | Local tags and explicit saved collections, backed by a shared index |
| Desktop development tools | External Open With/actions exist | Optional terminal panel and Git status/actions, loaded only when requested |
| Archive navigation | Create/extract and bounded previews exist | Browse an archive as a virtual folder without whole extraction |
| Tab recovery | Reordering/persistence exist | Reopen closed tab, predictable overflow and keyboard navigation |
| Remote/cloud workflow | GIO-backed sources and mounts exist | Audit protocol operations and errors; explicit discovery/auth/capability UI |

Dolphin's official overview documents split panes, an embedded terminal, remote/cloud access and
extension-based Git integration: [KDE Dolphin](https://apps.kde.org/dolphin/).
Its [view-properties reference](https://docs.kde.org/stable_kf6/en/dolphin/dolphin/view-properties.html)
documents folder-specific presentation. Finder supports
[file tags](https://support.apple.com/guide/mac-help/tag-files-and-folders-mchlp15236/mac) and
[Smart Folders](https://support.apple.com/guide/mac-help/create-or-change-a-smart-folder-on-mac-mchlp2804/mac).
Windows File Explorer's [official guide](https://support.microsoft.com/en-us/windows/experience/fileexplorer/file-explorer-in-windows)
describes tabs, Home/Quick Access and Gallery; Hermes already covers some equivalent navigation and
preview workflows, but not Windows' platform/cloud ecosystem. Feature references are not speed evidence.

## Getting genuinely quick

Benchmark optimized builds on the same machine, directories, view, thumbnail policy and cache state.
Use synthetic small/10k/100k directories and a generated media library; never benchmark private IDs
or other personal documents. Record cold/warm median and tail times for first usable content, full
enumeration, tab switching, search results, scrolling/frame stalls and selected-preview startup.
Measure complete process-tree PSS, parser churn, cache hit rates and stale-work cancellation.

The updated `scripts/profile-fixture.sh` isolates HOME/XDG config/state/cache and the session bus,
samples full application descendants, and reports incomplete runs as failures. Optional Broadway
mode allows headless provider/model probes. This is not an actual interaction/paint benchmark;
existing insertion timings omit finished painting. See docs/performance-baseline.md for three-run
release comparisons and their scope. Keep a selected generated-media desktop regression gate;
regular CI currently skips ignored desktop fixtures.

Proposed targets—not measured promises: common warm tab/folder changes below 100 ms, cached query
updates below 50 ms, and visible UI work normally within one 16.7 ms frame at 60 Hz. Slow decoding
must not freeze browsing. Refine targets using real baseline measurements and hardware constraints.

## Direction beyond parity

One coherent local-first workspace: files, photographs, music and documents share a reliable catalog,
search and preview language. Add saved collections and context-aware actions without making folders
depend on a player, internet service or AI. Keep metadata-based search useful offline. Prefer immediate
results with progressive enrichment, undoable bulk work, resumable workspace state, keyboard access,
and visible explanations of partial indexing/decoder failures. Optional new tools should not add idle
parsers, background scans or mandatory network traffic.

Trim duplicated execution paths and dead drafts only after checking callers/tests. Splitting a large
file can improve ownership but does not itself make Hermes smaller or faster. The shared music
controller is a useful ownership reduction; broad rewrites and blind deletion of handoff work are not.

Order after MPRIS: transfer safety/outcomes → cache budgets/identity and ordered persistence →
measured navigation/search/thumbnail bottlenecks → natural/type sorting, tab recovery and properties
polish → optional split panes, tags/collections and developer tools. Column sizing stays deferred.
