# Hermes: files, media, and music

Implementation update, 2026-10-09: scoped metadata search, full-length playback, music transport
and waveforms, Noctalia colors, the shared persistent/live-updating Music/search catalog, and
editable restart-persistent queues without autoplay are now implemented. Current validation
and remaining work are recorded in NEXT_AGENT.md. The code
findings below describe the earlier review; they are not a list of today's unresolved bugs.

Working proposal, 2026-10-02. This is a targeted codebase review and discussion document,
not a claim that all features below are implemented or that every source file has been audited.

## Agreed direction

- Give files, media, and music equal attention, using shared navigation and storage services.
- Keep the file browser useful independently of the player; keep playback alive across navigation.
- Song search should cover the whole library regardless of the current folder.
- Keep both hover peeking and click/Space previews. Hover behavior stays configurable through the
  existing Folder peeking setting; fixing crashes takes precedence over removing the feature.
- Pinned-folder centers remain file-transfer destinations. Gaps above/below pins add or arrange
  bookmarks without moving the underlying folders. Only an empty pinned section needs instructions.
- Immediate order: sidebar refinement, preview reliability and crash diagnosis, Noctalia colors,
  then the full music player. Column sizing stays deferred.
- Search reliability is part of the foundation and should not wait for the completed music UI.

## Findings from the current code

### Preview reliability

The failing portrait JPEGs are 2268x4032 pixels despite compressed sizes around 1 MB. Their full
RGBA raster exceeds the old 32 MiB decoded-frame guard. Several missing album covers have the
same problem. Compressed file size is not a useful predictor of decoder memory or CPU cost.

The local patch now uses JPEG reduced-size decoding with an aspect-correct size hint, a bounded
ImageMagick pixel cache, disabled disk/map caches, and the existing outer sandbox CPU, address-space,
and output limits. Oversized non-JPEG images are still rejected rather than blindly removing guards.
The large-JPEG path does not fall back to an unrestricted full-raster Pixbuf decode.

SCM_RIGHTS input descriptors are close-on-exec. Passing `/proc/self/fd/N` unchanged to an external
decoder therefore fails after exec. ImageMagick now receives an explicitly opened input stream on
stdin instead. Other external-decoder call sites still need a format-by-format audit.

Previously, ordinary decoder errors exited before writing a response header. The patch writes a
bounded, framed error and then reports unsuccessful completion, so the host can show the actual
decoder reason. A process killed by a resource limit can still produce EOF; this is a separate case.

Actual pooled-sandbox probes passed for three previously failing portrait JPEGs, the PNG sampled
from Important, an album cover, and a smaller JPEG. Successful samples took about 0.09–0.18 seconds
in one local run. Malformed input returned a framed error, and a subsequent valid request worked
on the same supervisor. These are engineering probes, not a universal performance benchmark.
No private image contents were displayed or copied into this document.

### Random crashes

Two stored desktop core dumps from this session both identify the main-thread hover-peek timeout
callback (`ViewState::schedule_peek`) as the abort boundary. This is stronger evidence than guessing
that music search or file size caused the crashes, but does not identify the original panic yet.

Next diagnosis: capture a fresh panic backtrace, reproduce rapid hover/navigation/drag/view changes,
audit synchronous observer reentrancy and timer cancellation, then add the smallest regression test.
Do not hide the failure by catching all panics or claim that every random crash has been fixed.
The dev launcher now preserves its log and enables Rust backtraces. Core dumps may contain private
data; investigate locally and never upload them automatically.

### Search

- The search dialog starts a new recursive index from the current location each time it opens.
  Searching from one album cannot find tracks in another album.
- The separate `search://` file source instead searches Home: two entry points have different scope.
- Matching currently uses names and paths, not embedded artist/album/title metadata.
- Walking follows neither directory symlinks nor all ignored/hidden content. These policies need
  explicit UI explanations, especially for externally mounted or linked music libraries.
- Search publishes a bounded top-100 result list, but uses an unbounded channel and builds an
  in-memory tree index. A bounded result list does not imply bounded indexing memory.
- Result updates destroy and recreate rows and select the first result again. Preserve selection
  by stable identity and apply only changed rows instead.

Proposed contract: one search service with visible scopes: This folder, Everywhere, Music library.
Music scope is global across configured roots, not relative to the selected album. It searches
filename, title, artist, album, and track metadata. Do not decode full audio to answer a query.
Show indexing progress, excluded/unavailable roots, and result limits instead of misleading emptiness.
Keep the index persistent and incrementally refreshed; use a latest-results slot or bounded channel.
Start with one local music root, then support additional roots without assuming they live in Home.

### Performance

Existing strengths: virtualized views, background directory enumeration, cancellation/generations,
separate thumbnail/preview worker pools, a byte-bounded UI thumbnail cache, and fixture scripts.
These are worth preserving rather than replacing with a large rewrite.

High-value follow-ups:

1. **Bound both sandbox caches by bytes.** Each pool currently caps entries, not retained bytes.
   Account for PNG plus metadata, use shared immutable buffers where useful, and benchmark actual PSS.
2. **Fix cache identity.** Sandbox keys use second-resolution mtime, size, path, and operation.
   Prefer nanosecond mtime plus device/inode identity; folder artwork must include the selected
   cover's identity so replacing a cover does not leave a stale album thumbnail.
3. **Prioritize visible work.** Navigation and the selected preview should outrank speculative
   thumbnails. Deduplicate identical in-flight jobs and cancel offscreen/stale work promptly.
4. **Move persistence off the interaction path.** Trail changes synchronously serialize and save
   from UI event callbacks. Coalesce changes, write atomically on a serialized worker, and flush
   deliberately on shutdown. Preserve latest-write ordering for bookmarks too.
5. **Reuse search indexes.** Avoid rebuilding a large tree on every dialog opening or rescan.
6. **Reduce rendering churn.** Preserve search-row selection and reuse rows. Profile blur/motion
   and texture upload costs before changing the design or blaming GTK/GPU drivers.

The historical fixture baseline in `performance-baseline.md` uses an optimized release build.
Today's debug watcher is not a fair release-performance comparison. Repeat cold/warm 1k/10k/100k
directory runs on this desktop, recording startup, first batch, sort/search/preview latency,
cancellation delay, dropped frames, and application-plus-worker PSS. Record median and tail
latency over repeated runs, not one flattering sample. Add a large music-library fixture separately.

## Feature parity without copying every feature

| Area | Hermes already has | Proposed next capability |
| --- | --- | --- |
| Navigation | Tabs/Trails, breadcrumbs, list columns, grid, details view | Restore closed tabs; consistent per-folder preferences; explicit split view |
| Organization | Pins, basic sorting, selection, transfers and conflict handling | Natural sorting, grouping, disc/track sorting, batch rename with dry-run |
| Safety | Trash and an undo service | Audit operation coverage; clear undo limits; recoverable job history |
| Discovery | Scoped recursive name/path search and filters | Global music metadata search; saved searches; optional local tags |
| Inspection | Sandboxed images/documents/media and waveform previews | Clear errors, zoom/fit controls, coherent media controls, gallery filmstrip |
| Integration | GIO locations, mounted devices, custom-action service | Validate network backends, desktop open-with behavior, MPRIS/media keys |
| Music | Artwork and static waveform groundwork | Persistent queue, full-track FLAC/MP3 transport, cached seekable waveforms |

These columns are not exhaustive certification of compatibility. Existing services may have
partial UI coverage or unsupported operation types; verify end-to-end before checking a box.

Useful references, checked against primary documentation:

- [Dolphin's views](https://docs.kde.org/stable_kf6/en/dolphin/dolphin/dolphin-view.html)
  provide split views, grouping, and audio-specific information/sorting.
- [Dolphin's per-folder settings](https://docs.kde.org/stable_kf6/en/dolphin/dolphin/view-properties.html)
  make view/sort preferences predictable across folders.
- [Dolphin commands](https://docs.kde.org/stable_kf6/en/dolphin/dolphin/command-reference.html)
  provide a useful comparison for undo, copy/move-to-other-pane, and optional panels.
- [Finder organization](https://support.apple.com/guide/mac-help/organize-your-files-in-the-finder-mchle9f0a1b2/mac)
  is a useful reference for gallery previews, quick actions, tags, and saved rule-based collections.
- Windows-style familiar operations, navigation, and readable details should be evaluated as a
  usability target, not treated as a reason to duplicate an entire Windows shell.
- [ImageMagick size hints](https://imagemagick.org/defines/) explain the JPEG reduced-decoding
  mechanism used in the current preview patch.

## A distinct Hermes experience

My recommendation is one trusted engine with three explicit lenses: Files, Media, and Music.
Lenses change available presentation and actions, not filesystem truth. A music collection can
appear as albums or files without moving or copying songs behind the user's back.

Keep a stable hierarchy: navigation and search at the top; view/size/sort/filter together above
content; inspection at the right; persistent transport at the bottom; operations in a quiet,
inspectable task tray. Use semantic Noctalia-driven colors throughout, including controls,
waveforms, disabled states, selection, and errors. Do not assume wallpaper changes should change
layout, font, density, or media ordering.

Context-aware features should be inspectable and reversible: suggest track order or a saved view,
show why an album cover was chosen, expose what a batch action will change, and keep search local.
AI is optional, not a dependency for opening folders, finding music, or applying safe file operations.
No silent content uploads, hidden moves, or automatic tag rewriting.

## Delivery gates

Latest requested refinements: keep tabs compact with title dragging and clear insertion feedback;
remove tab pinning until it has a useful purpose. F should open a loaded video preview fullscreen
without intercepting typing. These controls are implemented; pointer/visual acceptance is pending.
Images now use the same F entry/exit convention, with Fit/100%, resize-aware fit, cursor-anchored
wheel zoom, and drag pan. A synthetic desktop fixture checks layout and fullscreen lifecycle;
real-image visual acceptance is pending. The preview remains sandbox-bounded, so 100% refers to
the rendered preview rather than guaranteeing original resolution. GIF animation now uses bounded,
alpha-preserving sandbox conversion with autoplay, looping, pause/seek, and F fullscreen. Repeated
desktop fixtures verify loop restarts and lifecycle cleanup. Full-length video now bypasses clip
conversion: a persistent isolated GStreamer decoder emits bounded raw frames/audio, supports seeking,
and keeps one session across drawer/fullscreen. Posters are requested independently and superseded
by the decoder's first frame. PATD and Tidal Waves production-worker probes prepared their full
48/70-second timelines in about 140ms with warm plugin caches and sought beyond the old cutoff in
about 90ms. These are local engineering samples, not universal startup/sync guarantees. User visual,
audio-output, and lip-sync acceptance remains pending.

Two subsequent milestones need separate validation:
- **Album order — implemented, visual acceptance pending:** bounded sandboxed album/disc/track
  reading, asynchronous hydration, identity/nanosecond-time cache, and a shared numeric comparator
  for folder views and hover peeks. Handles totals such as `3/17`, multi-disc tags, filename fallback,
  artwork-last grouping, and manual sort overrides. Automatic detection is deliberately restricted
  to album-like leaf folders; other folders can select Album track explicitly. Actual sandbox
  worker verification read all 17 Blonde tracks successfully without modifying original files.
- **Full video playback — implemented, user acceptance pending:** the drawer now has a
  cancellable streaming/seek-capable decoder session. The user's updated requirement is full-length
  playback directly in the drawer; F only changes presentation, not the playback engine or length.
  Frame/audio queues and packet sizes are bounded, original-file parsing stays isolated, and
  closing/switching the preview tears down the worker. Playback has no cumulative duration/CPU
  cutoff, but initialization/seek and active packet stalls time out. The desktop fixture verifies
  playback across 30 seconds, late seeking, fullscreen continuity, natural end, replay, and cleanup.
  Reuse the service boundary for future music playback after audio clock/sync acceptance.

Unified video preview transport is now implemented using a persistent Hermes-styled bar and
a shared-stream fullscreen surface. A generated desktop clip passed a playback/lifecycle smoke
test. Visual acceptance and fullscreen idle fading remain to be checked; the full music service
and Noctalia integration are not implemented yet.

1. **Reliable foundation:** validate pin insertion/reordering gestures; finish decoder probes;
   reproduce/fix hover crashes; unify search scope; add byte budgets and useful diagnostics.
2. **Noctalia and visual coherence:** follow live palette/application mode; last-valid fallback;
   polished preview loading/error states and media controls; accessibility and reduced motion.
3. **Music service:** global metadata index, queue and full-track transport, then cached interactive
   waveform and library layouts. See `music-player-plan.md` for FLAC and sandbox decisions.
4. **Power-user parity:** per-folder presets, grouping, split view, batch rename, saved searches,
   tags, and fuller job/undo history, prioritized by real daily workflows.

Each gate needs tests plus a real UI check. Do not mark visual or drag behavior verified just
because the project compiles. Do not put column resizing back into the immediate scope.

## Design questions still worth exploring

- Both hover and click/Space previews are wanted. What delay makes hovering feel helpful rather
  than distracting, and should that differ between compact files and media layouts?
- Keep the cyber/monospace profile everywhere, or introduce a more readable UI font while retaining
  monospace filenames, metadata, and technical previews?
- Shared density across lenses, or compact Files and more spacious Media/Music?
- Should unknown preview formats show metadata/actions immediately rather than a blank drawer?
- Which power-user feature helps daily work most: split panes, batch rename, saved searches, or tags?
- Should Music search be the persistent default after selecting that scope, or a dedicated shortcut?

Preferences and priorities can change; this document should evolve with actual use and measurements.
