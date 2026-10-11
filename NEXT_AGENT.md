# Next agent: Hermes performance, file features and media polish

## Square tabs, draggable seek handles and JPEG/MOV loading (2026-10-10)

Current user asked for square tab indicators, draggable preview timeline thumbs and then MOV/JPG
loading work. Only file-tab surfaces/selection buttons were squared; compact tabs/top accents and
their control layout remain. Shared `ui/seek.rs` now handles preview/fullscreen audio/video and main
Music timeline interactions. Capture-phase raw pointer/touch observation does not claim GTK's
gesture. Native scale value changes move the thumb; playback timestamps cannot overwrite an active
scrub. Commit on release, coalesce later targets during an outstanding seek, keep scale enabled
while seeking, keep paused playback paused. Cancel/unmap/stream replacement clears pending targets.
The preview volume range no longer stops GTK's normal value-change handling. No queue semantics changed.

`sandbox_helper/jpeg.rs` adds bounded adequate embedded-JPEG thumbnails (128 KiB header read,
primary/thumbnail EXIF orientation, no tiny-thumbnail upscaling or aspect mismatch, decoded-frame
budget, available ICC retention). Missing/malformed/undersized thumbnails fall back. Full JPEG
previews use original-image ImageMagick, including above-budget originals with bounded JPEG decode
hints; the older unvalidated EXIF shortcut is no longer used for those. PNG compression 1 saves
renderer time without reducing pixel detail; all sandbox budgets remain. Thumbnail/preview worker
caches still validate identity and cancellation. Generated eight-orientation, fallback, original
quality, over-budget source, cache and invalidation tests cover the path.

MOV preview startup now gives the existing native decoder 200 ms before optional poster work.
Skip that extra decode if a real frame is ready, selection cancelled or stream failed. Slow-file
fallback posters may appear 200 ms later; no universal speed claim. Grid posters still use the
existing cached ffmpegthumbnailer path. Full-length playback remains unlimited and streaming.
Synthetic H264 720p MOV first-frame samples ~127 ms, without a controlled before/after comparison.
JPEG direct embedded thumbnail ~2.2 ms vs original renderer ~10 ms; production cold worker ~44 ms
and warm ~20 µs. Generated textured JPEG 1400px render ~86–101 ms vs ~141–149 ms with old PNG
compression. These are fixtures, not personal-photo, cold-disk, GTK paint or Dolphin parity results.
More HEVC/4K MOV and real-photo profiling remains useful. See docs/performance-baseline.md.

Private fixture logs are `/tmp/hermes-media-qa`; no user files/app/session changed. Deterministic
seek lifecycle, native main-player seek, audio preview, tab layout, MOV startup/seek and 45-second
MP4 fullscreen/replay checks pass. Physical pointer dragging still needs user confirmation.
The optional older application-shared-controller GTK fixture fails before seek checks on an obsolete
Repeat `Button::label()` expectation (Repeat is now icon-only); left unchanged as unrelated. New
focused multi-view native seek fixture passes. Previously documented integration failures remain.
Final validation: 521 normal unit tests pass, 46 optional tests ignored by default; the targeted
GTK/native/JPEG fixtures above pass, as do all-target checking and debug/release compilation.
New standalone modules pass scoped formatting and diff whitespace checks. Existing lint warnings
remain; broad integration tests were not rerun this turn.
Favorites/manual playlists/Most played/Recently added are still approved next steps, not implemented.
No Save queue as playlist. Gapless and column sizing remain deferred. No branch/commit/PR created.

## Retained native tabs and approved library roadmap (2026-10-10)

User prioritized retained tabs; reports some MOV/JPG loads remain slow. Favorites, manual playlists,
Most played, Recently added and cleaner Albums/Artists/Songs navigation are approved next steps.
Do NOT add Save queue as playlist. Existing queue restart persistence remains. Gapless is later.
These library features were planned, not implemented this turn. See docs/music-player-plan.md.

Browser now caches NavigationState and existing GTK presentations by TrailId, not path. Duplicate
tabs at the same path keep separate selections. Up to six inactive ready/empty native tabs are
retained within estimated 64 MiB accounting; this is not a hard RSS/texture cap. Directory/ancestor
invalidation monitors must install successfully. Dirty/expired (30 seconds)/preference-changed tabs
restore their views then reload all columns, preserving location-based selections. Late metadata
for inactive request IDs marks the tab dirty instead of changing the active tab. Old monitor and
pending-sort callbacks are gated by tab epoch. Incomplete/error/unmonitored/oversized/search/remote
tabs use normal loading. Eviction and closing prune both models and monitors outside cache borrows.

Retained ModeViews remain mounted as hidden children of a per-window GTK Stack: no row/model
reconstruction on clean activation. Each presentation owns its Columns root/scroller, active alternate
view, filter controls and thumbnail-size state; cut/transfer/click-preference handlers are shared.
Temporary rename/new-folder/hover/entry animations are cancelled safely. Scroll snapshots cover
only the visible view and scrollable axes; Never-policy adjustments are layout internals, not saved
scroll positions. Restoration uses generation-guarded ticks and yields to real scrolling input.
File preview sessions close on switching tabs; main music/queue/bottom transport are unchanged.
Window activation configures cold mode/density before model events, saves final view state and
routes new search-result tabs through the same activation path. Column sizing is still deferred.

Generated fixtures check original model/shell/mode identity, filters, selection, scroll, inactive
addition, active rename/delete, duplicate-path independence, history, guard release, memory/count
eviction, preference/hidden policy changes, unsupported/incomplete fallback and stale callbacks.
Private GTK logs are /tmp/hermes-retained-qa. No personal files were used and no user app restarted.
Final validation: 518 normal unit tests pass, 41 optional tests ignored by default; explicit retained
view/lifetime fixtures and nine release TabBar/live-change samples pass. All-target compilation,
debug/release builds, scoped formatting and diff whitespace checks pass. Warm restored-model
medians are ~1.3–1.7 ms Columns, ~7.5–8.1 ms Grid, ~2.1–2.7 ms Explorer on synthetic 1k-text-file
folders; they exclude final painting and production-window handlers. See docs/performance-baseline.md.
Automatic album track order is compatible with saved default Name preferences, avoiding needless
album reloads. Previously documented seven integration failures remain
unrelated and unchanged. No branch, commit or PR was created.

Remaining cold MOV/JPG work: JPEG currently uses ImageMagick with JPEG downsample hints (embedded
EXIF thumbnail shortcut only above the decoded-frame guard); video posters use ffmpegthumbnailer.
Full video playback already uses the streaming engine. Measure poster/decode/startup/UI scheduling
separately; do not restore duration limits or bypass sandbox guards to chase speed.

## Music context menus, waveform visibility and HEIF fast path (2026-10-10)

Current user asked for Music track/album right-click queue menus, an option to hide the Music
waveform and more loading-speed work, especially HEIC. Dedicated Music rows now offer Play,
Play next and Add to queue; Menu/Shift+F10 works for selected rows. Album actions capture all
metadata-ordered songs, ignoring song-list filters. Track Play captures the remaining folder queue;
track queue actions add only the clicked song. Opening menus does not select/browse/play, and
queue additions preserve current playback. Targets are immutable snapshots, closures hold weak
state/menu references, and close/unbind/unmap unparents popovers. The generated GTK fixture
exercises the actual row gesture, changed filters/library, queue preservation and popup release.

The Music header eye toggle hides waveform/status and stays effective through controller updates;
showing it again waits for real data. It is per-window presentation state, not a persisted setting
or an analysis-disable option. Existing generated playback fixture checks stream/queue unchanged.
Track file previews still show artwork/playback/metadata rather than waveforms.

HEIF thumbnails now use libheif-rs 2.7 without default latest-version features, requiring libheif
>=1.17. CI/release/dev dependency installation and README were updated. The decoder runs only
inside the existing sandbox: bounded embedded-thumbnail enumeration, smallest adequate matching
aspect, container orientation, checked RGBA dimensions/stride, retained ICC profile, and fast PNG.
32 MiB direct decoded-frame budget remains; unsupported/large/thumbnail-less photos fall back to
ImageMagick. Full previews remain original-image 1400px ImageMagick, never upscaled thumbnail.
New wire operations 20/21 route both through existing identity-checked/bounded worker caches, so
repeat visits no longer rerun one-shot HEIF conversions. Generated 12MP rotated HEIF tests check
orientation/color quadrants, fallback, cancellation, cold/warm requests and replacement invalidation.

Grid/Explorer panes now construct only in the selected mode, and mode switching releases hidden
alternate presentations; same-mode requests do not rebuild. The navigation lifecycle fixture checks
only the appropriate alternate model is alive across all three presentations. Provider-data cache
is unchanged; full loaded-tab GTK model/scroll retention is STILL not implemented. Column sizing
remains deferred. Performance methodology and measured samples are in docs/performance-baseline.md.

Validation: 512 normal unit tests pass; generated Music context-menu/waveform/playback,
all-view navigation lifecycle and HEIF decoder/cache fixtures pass. Debug/release builds,
all-target checks, scoped formatting, shell syntax and diff whitespace checks pass. Three release
tab/live-change samples each of Columns/Grid/Explorer pass. See performance baseline for timings.
Clippy could not run: the installed stable toolchain lacks its component. Existing unfulfilled
lint-expectation warnings remain. Broad integration tests were deliberately inspected too:
M1 (19), M2 empirical (11), M3.1 (12), M3.2 (15), E2E (165) and export samples (1) pass.
Seven failures remain in unchanged paths: M2 stress oversized_no_exif / forged_oversized_exif
expect a frame-budget diagnostic but receive a safe ImageMagick failure; M4.2 has four ODS
fixture parsing failures (rows, columns, extraction, formulas) and one legacy waveform duration
expectation failure. Do not claim the entire suite is green or blindly weaken those assertions.
Logs are in /tmp/hermes-context-qa; these unrelated failures were left unchanged this round.
Private GTK artifacts are /tmp/hermes-context-qa; the temporary display/browser were closed after
validation. No personal media was used or app restarted.
Older pending feature/parity work below remains open; no commit, branch or PR was created.

## Music cleanup, audio metadata previews and navigation lifecycle (2026-10-10)

User wants album double-click playback, cleaner Music/bottom controls, a hideable queue and
Dolphin-like audio preview information instead of the preview waveform. Single-click an album
only browses; double-click plays its entire disc/track-ordered album, even with filtered results.
Music now has Play album/results, secondary actions in More actions, compact shuffle/repeat,
Show/Hide queue and transport Stop in More. Hiding queue never changes playback. The Music
waveform/status appears only with actual peak data; zero-valued peaks for silence are valid data.

File audio previews now use artwork, full-length single-track playback and read-only embedded
metadata/properties. PreviewAudioMetadata is operation 19, with bounded/sanitized ffprobe output;
it does not generate a waveform. Embedded cover then bounded named neighboring covers are tried
independently. Missing properties remain unknown; extra tags are expandable. These are not editable
filesystem tags, ratings or a metadata editor. Actual preview Play pauses shared music without
replacing its queue; selection does not pause/autoplay. Starting main music pauses audio preview;
closing/replacing preview stops its own worker and never auto-resumes music. Production coordination
is install_audio_playback_coordination in ui/window.rs, exercised by the generated audio fixture.

Actual tab-directory measurements exposed retained GTK rows/views. Strong controller captures of
ListItems, rows, rename fields, marquee ancestors and hover anchors were changed to weak/event-time
references in ui/browser.rs and browser_modes.rs. Alternate selection callbacks no longer strongly
retain Browser; programmatic replacement suppresses selection callbacks. The generated lifecycle
test checks Columns/Grid/Explorer widgets, models and items disappear across nine navigations and
final teardown. It must navigate to a different location: Browser.navigate intentionally ignores
the already-active folder. Context/clipboard popovers unparent when their owner is destroyed.
Icon registries now use keyed weak references with periodic cleanup, replacing O(n) scans per bind;
live recoloring and rebinding still work, with generated regression coverage.

RoutedFileSource previously omitted watch forwarding; local live directory updates now propagate.
CachedFileSource retains up to 12 successfully monitored native listings for 30 seconds within an
estimated 32 MiB budget. Directory and ancestor monitors protect alias/rename/symlink retarget cases;
no UI-thread stat. Failed/incomplete/unmonitored/remote/search listings are not reused. Refresh
explicitly invalidates. Cancelled/changed replay falls back safely; DirectoryEvent::Restarted clears
partial models/peeks before fresh enumeration and preserves pending selection, pruning missing
locations on finish. Metadata after enumeration and missing-tag enrichment remain supported.
Pending local music-entry accumulation also has count/byte bounds with a bounded rescan fallback.

Three release samples of two generated 1k-text-file folders measured complete warm tab models:
Columns medians 51/39 ms; Grid 167/163 ms. These exclude completed paint and decoding, and do not
prove Dolphin parity. Earlier debug Grid navigation grew ~370→1,200 ms; a post-fix sample stayed
~296→217 ms. See docs/performance-baseline.md for exact timings, methodology and limitations.
Provider data is cached, not complete GTK models. Next performance targets: lazy alternate-view
construction, loaded-tab model/scroll retention with live revisions, cheaper grid binding and
real media/native Wayland paint measurements. Column sizing remains deferred.

Validation: 509 normal tests pass; optional generated Music, audio playback/metadata/coordination,
icon cleanup, all-view lifecycle, native watcher/symlink and actual tab/live-change fixtures pass.
Columns/Grid each have three release timing samples; Explorer's generated tab/live-change fixture
passes too. Theme/CSS parsing and screenshots checked using private Broadway, generated silent
FLAC/cover files and private HOME/XDG/D-Bus. Do not set GSK_RENDERER=cairo for Broadway's live
renderer: its browser command stream can fail; snapshot helpers use Cairo independently.
Debug/release builds and all-target checks pass. Existing unfulfilled lint-expectation warnings
remain. No personal files/session restart, commit, branch or pull request was made.

Remaining file parity work from the review is unchanged: runtime undo, complete folder/remote
transfer recovery, shared ordered sidebar persistence, natural/type sorting, properties pin
integration, per-folder presentation and closed-tab recovery. Music tag editing, gapless/ReplayGain
and cover/MPRIS identity audits remain separate tasks; don't imply they were added here.

## Transfer safety and folder/tab performance follow-up (2026-10-09)

User approved the review priorities and clarified that opening folders/tabs feels slow. Native
file/link copies now stage in private sibling directories and publish only after success, without
predeleting destinations. Overwrite-approved same-device moves rename atomically without copying.
Source-removal failures after cross-device commit report that both destination and source remain.
Full folder replacement/type conflicts and remote replacement intentionally fail closed with
guidance; no silent merges. New-folder transfers still work but can leave partial new output on
failure/cancel. Native non-overwrite publication currently needs hard-link support; exFAT/FAT
publication needs a capability-aware safe alternative. Cancellation racing atomic commit may
complete it. This is not a power-loss durability or hostile-path-race guarantee.

Paste continues after item failures and reports successful roots separately. Cut state/clipboard
only removes confirmed moved items and protects newer clipboard content via generation/provider
ownership. Cancelled/failed items remain for retry. Overlapping operations are rejected with guidance
instead of silently cancelling old handles; completion handles synchronous/reentrant providers.
Adjacent tests simulate write failure/disk-full, disappeared/changed sources, precommit cancellation,
link/type/folder conflicts, copy/move success and inode-preserving fast moves. The GTK/provider
fixture verifies partial moves, clipboard replacement, cancellation and retained progress controls.

LocalTrailStore now shares an ordered/coalescing writer per path across windows. Serialization,
write and fsync are off GTK; private unique 0600 temps atomically replace the unchanged TOML format.
save() acknowledges enqueueing, not durability; asynchronous failures log. load() waits for prior
saves to preserve read-your-last-save; explicit flush/shutdown/final drop also wait deliberately.
Tests cover slow/inflight writer, ordering, coalescing, multiple stores, failure/recovery, atomic
private writes, shutdown and last-drop flush. Sidebar bookmark/order saves remain separate racing
workers and are a next persistence target, not fixed by the Trail writer.

TabBar reuses unchanged tab widgets and updates active styles instead of recreating all controls
on activation; rename/reorder/close/rename cancellation retain behavior. An explicit generated
two-tab desktop probe performed 200 warm strip refreshes in ~0.8–1.1 ms total (debug; no directory
loading/paint). Folder enumeration emits up to 32 visible entries first, then normal bounded
batches; empty filtered batches do not consume the first-visible fast path. Native-byte names,
incremental sort/selection/cancellation and metadata ordering remain unchanged.

Preview-worker caches now account for payload capacity, LRU/TTL/replacement and separate 16 MiB
thumbnail / 64 MiB preview budgets. Identities include dev/inode/size and nanosecond mtime/ctime;
changed-during-render/cancelled results are rejected. UI thumbnail identity checks and cover
discovery run off GTK; keys include actual cover identity, warm covers skip rescanning, stale or
rebound results cannot paint or cache. The 64 MiB/256-entry UI budget remains. Desktop-launcher icon
lookup also moved off GTK and never executes launchers. Alternate-cover fallback, MPRIS cover
identity revalidation, and shared immutable worker payloads remain open.

scripts/profile-fixture.sh now wraps an isolated Python/private-D-Bus runner and includes decoder
descendants in memory sampling; timeouts/incomplete runs fail. Optional STRATA_PROFILE_HEADLESS=1
uses Broadway, not the user's visible app. Three release 10k empty-file samples reduced median
first insertion from 21.69 ms (128 entries) to 6.13 ms (32 entries); complete enumeration medians
were 150/161 ms, not an overall speedup claim. A 100k sample finished enumeration in 1329 ms at
~128 MiB sampled process-tree PSS. No finished-paint, actual tab-directory timing or Dolphin
benchmark is implied. Full context and caveats: docs/performance-baseline.md.

Optimized auto-updating dev launch is now optional: HERMES_DEV_RELEASE=1 make start-dev. Default
debug behavior is unchanged. Close an existing Dev instance before changing profile; GApplication
otherwise forwards the launch to the old executable. Both target/debug/strata and target/release/strata
are build outputs; no user session/playback was forcibly restarted. Column sizing still deferred.

Final validation: 470 regular binary tests pass, 31 optional tests ignored by default; eight
explicit desktop/provider fixtures pass for transfer/cut clipboard, tab reuse/layout/activation,
first-batch enumeration, visible-only search artwork, shared-player shutdown/restore and real
MPRIS playback. Two profiler tests pass. Debug/release builds, all-target check, scoped formatting,
shell syntax and diff checks pass; existing lint-expectation/user GTK CSS warnings remain.
Next prioritize measured actual tab-directory switching: models currently reload across tabs;
any warm snapshot/cache must reconcile live changes and retain selection/cancellation guards.
Then ordered bookmark/order persistence, safe cross-backend/folder transactions and remaining
cover/performance audits. No undo/batch-rename/split-pane feature has been newly wired in here.

## Shared player, MPRIS and quality review (2026-10-09)

User approved desktop integration followed by a safety/performance/parity review. Music now has
one application-owned controller, queue, stream and session writer. Window UI remains separate;
closing one window does not terminate music in another. Controller callbacks do not retain the
originating view. Application shutdown saves, cancels workers and releases playback once; restart
restores without autoplay. Stable queue-occurrence IDs distinguish duplicate tracks and survive
reordering. Paused/stopped next/previous preserve state. Idle controllers do not notify every tick,
and unchanged transport icons are not recreated on each position update.

Native GIO MPRIS exports org.mpris.MediaPlayer2.hermes (development suffix .Dev), Root/Player
interfaces, controls, validated local-audio OpenUri, metadata, position, repeat/shuffle and volume.
PropertiesChanged contains only changes, never periodic Position; Seeked follows actual seeks.
SetPosition rejects stale occurrence IDs. Registration does not take over another bus owner and
is cleaned up on name loss/shutdown. Missing bus access does not disable in-app playback.
Music only: ephemeral video previews are not exported. No shell settings/global key grabs changed.
Artwork decoding is asynchronous and sandboxed, with private bounded temporary PNGs and cleanup;
restoring a stopped queue does not start cover decoding. Installed Noctalia/media-key acceptance
is still needed; protocol fixtures are not a claim that the user's shell selected Hermes.

Directory merges reuse storage for empty or strictly ordered append batches; equal/interleaved
entries retain prior merge semantics. New merge timing separates this from GTK insertion. One
synthetic debug 100k-entry/128-batch sample improved from 3.25 seconds to 15.17 milliseconds; this
is a favorable model-only case, not end-to-end Dolphin speed parity or a release benchmark.

Validation: 414 regular tests pass, 28 optional tests ignored; explicit private-bus export/signal,
real generated-media remote playback, shared-controller multiwindow/shutdown/restore and queue
drag/removal/restart fixtures pass. Production build passes. Existing lint-expectation warnings and
missing user GTK CSS imports remain unrelated. No user application/playback was restarted.
An additional production smoke used a private session bus, GTK Broadway display and temporary
HOME/XDG config/state/cache with generated silent FLAC and cover PNG. It verified real startup/name
ownership, OpenUri playback, sandbox-decoded artwork URL, Pause/Quit, temporary-cover cleanup and
unregistration. This does not substitute for installed Noctalia/media-key acceptance.

Read docs/quality-and-parity-review.md for the prioritized review and primary feature references.
Highest open risk: replacement paste permanently removes the destination before the new transfer
succeeds. NOT FIXED in this update. Next: safe staged replacement and explicit folder semantics,
per-item transfer recovery, then byte-budgeted/identity-correct caches and ordered off-thread
navigation/bookmark persistence. Avoid overlapping operation handle loss and premature cut clearing.
Undo/batch-rename drafts are not runtime features. Natural/type grouping, per-folder settings,
closed-tab recovery, independent split panes, tags/Smart Folders and optional terminal/Git remain
useful parity slices. Mounted devices, remote GIO sources and archive create/extract already exist.
Do not blindly delete draft modules or whole-format compressed legacy files. Column sizing deferred.
Do not run scripts/profile-fixture.sh beside the user's app unchanged: isolate state/session bus
and account for the full process tree before comparing optimized builds with Dolphin.

## Music navigation, visual search and PDF fullscreen (2026-10-09)

User requested a left-side Music button, artwork-to-player navigation, clearer search visuals,
and F-fullscreen for PDF previews. Clarification: switching file tabs should close the Music
screen while music and the bottom transport continue. The header Music toggle is now beside
the sidebar toggle, before breadcrumbs. Clicking transport artwork opens Music at the playing
album without restarting playback. Tab switching, cycling, active-tab closure and new-tab
creation return to Files through the shared activation hook; background-tab closure does not.

Search dialog rows now show distinct themed type icons, type/extension badges, song titles,
separate artist/album details and a subdued path. Existing sandboxed thumbnails and installed
desktop-launcher icons load only when rows enter the viewport, after layout has settled.
Unmapping cancels requests and invalidates deferred ticks. An initial desktop regression caught
offscreen rows being mistaken for visible rows before GTK allocation; the layout guard fixes it.
Launcher files are never executed to retrieve icons. This is not a game catalog or an Artist
entity index. Search-to-preview and search-tab entries preserve hydrated song tags and known size.

PDF previews have a reader expand button and use the existing F shortcut. Fullscreen reparents
the same scrollable reader rather than parsing the document again; loaded pages, zoom and text
selection remain available. F/Escape or minimize returns it to the drawer. Scroll-fraction
restoration preserves approximate reading position across viewport-size changes. Preview
replacement, closure and object teardown close the fullscreen window. Parser isolation and
resource limits are unchanged; no private user documents were needed for validation.

Validation: 399 regular tests pass, 24 optional tests ignored. Explicit desktop fixtures pass
for PDF fullscreen entry/exit and actual PreviewDrawer cleanup with generated pages; viewport-only
search artwork and nonexecution of generated launchers; tab activation without transport hiding;
search refresh/reentrant rendering; artwork navigation with the same live music stream; and
compact tabs/full theme CSS parsing. Production build, targeted formatting and diff checks pass.
Known test lint-expectation warnings and missing user GTK CSS imports remain unrelated.
The refreshed binary is target/debug/strata; no user playback or application was restarted.
Hands-on visual/pointer acceptance is still needed.

User disliked the bottom artwork's outline. Its button now has no border, outline or shadow;
hover and keyboard focus use subtle artwork opacity/background changes instead. Clicking still
opens Music, and keyboard focus remains visible without a ring.

Next: shared application player ownership, MPRIS/media keys and Noctalia's player widget; then
remaining cover/cache/performance and random-crash audits. Column sizing remains deferred.

## Queue polish and restart persistence (2026-10-09)

User approved queue reorder/removal and restart persistence after the shared-library update.
Both the Music queue and transport popover now have themed drag handles with row previews,
top/bottom insertion indicators, up/down buttons, remove buttons and selected-row keyboard
actions (Alt+Up/Down, Delete). Queue order is the playback order, including Shuffle/Play-next;
reordering and removing other songs leave the active stream untouched. Index remapping keeps
the current song and playback history correct, including duplicate paths. Removing the current
song advances to the next order entry and preserves paused state; removing the final current
entry stops. No queue action deletes source files. Music has Resume queue and Clear queue.
Drag tokens are per-player and revision-bound, rejecting stale/cross-player drops.

music/session.rs persists validated native Locations, tags, explicit order, bounded history,
current song, position, repeat/shuffle and volume in user_state_dir()/strata/music-queue-v1.json.
Limits: 20k songs, 16 MiB JSON, 1024 history entries, 4096-byte absolute audio paths and bounded
metadata strings. Non-local/parent-traversal paths, malformed ordering, unknown versions, invalid
volume/position and nonregular state files are rejected. Missing songs are pruned off-thread;
if the current song disappeared its successor starts at zero, not the removed song's offset.
Non-UTF8 native paths and duplicate songs round-trip. Writes coalesce in a dedicated worker,
use unique 0600 temporary files with sync+atomic rename and report failures without breaking
the live queue. Position checkpoints every ten seconds; graceful shutdown captures the final
position and flushes/joins the writer (only shutdown waits for disk I/O). Abrupt crashes may lose
the last few seconds of position. Loading happens off-thread and cannot overwrite user actions
made while restoration is pending. An untouched window does not overwrite a restored session.

Restoration never creates a stream or shows the bottom transport. Resume queue explicitly
starts the saved song at the saved position; starting a different song/folder resets that offset.
The player remains per-window, not a global application controller: independent windows can
save independent queues to this same session file; latest completed save wins. Consolidate that
controller before global MPRIS/media-key integration. These changes do not alter sandbox limits.

Validation: 397 regular tests pass, 21 optional tests ignored; default parallel and serial runs
pass. Production build, changed-module formatting and diff checks pass. Queue unit tests cover moving/removing,
history remapping, shuffle/Play-next order, duplicates and bounded selection history. Session tests
cover restart data, missing songs, byte-safe paths, corruption/limits, private atomic writes,
coalescing/final flush and failed-write recovery. The generated-media desktop test drives actual
GTK drag-source/drop-target signals, row buttons and keyboard actions; verifies the same stream
continues during other-song edits; removes current songs playing/paused; resumes a saved offset;
closes/reopens without autoplay; and clears the persisted queue. User pointer/visual acceptance
is still needed. Existing full-track/library-update/seek/retry fixture also passes.

Next: shared application player ownership, MPRIS/media keys and Noctalia's player widget; then
remaining cover/cache/performance and random-crash audits. Column sizing remains deferred.

## Shared music library update (2026-10-09)

User approved the shared-library step before queue polish. services/music_library.rs now owns
the local catalog, sandboxed tag hydration, bounded snapshots, persistence and folder monitoring.
Music's Scan is a thin subscription/Track projection. Music-root and inside-Music folder searches
subscribe to the same catalog; search result tabs keep receiving live snapshots after initial
completion. General Home/outside-Music search retains its generic recursive worker/manual Refresh,
but metadata reads can reuse identity-validated tags from an active shared library. Multiple
consumers have independent revision cursors; the last subscriber cancels the worker/parser and
releases directory watches. Metadata/source files remain read-only and sandbox limits are unchanged.

The existing v1 Music cache shape/path is retained, including byte-safe native locations. Successful
empty tags are now cached too. Reads validate root containment, paths, fields, identity and size;
writes use uniquely named 0600 temporary files, sync and atomic rename. Limits: 40k entries/32 MiB,
20k tracks, 100k walked entries, 128 levels, 16 MiB persisted tags, and 4096 directory watches.
GIO monitoring runs on the library worker's private main context, not the UI. Changes debounce;
parent monitoring handles missing/recreated roots, new folders gain watches, and obsolete watches
are removed. Periodic verification every 60 seconds covers missed/unsupported/over-limit watches.
Callbacks are processed between bounded tag reads; a slow individual parser can delay watch
delivery until it completes or times out. This is one local root, not a multi-root database.

Music skips unchanged library rendering and preserves/falls back from removed album selections.
Automatic updates do not replace playback queues or restart streams. Content-result memoization
in the search worker keeps up to 132 identity-bound scored items so library revisions do not
repeatedly spend the per-query document-extraction allowance or erase unchanged content results.
Changing a query clears that memoization. A regression also caught/fixed hidden-filter ordering
in the migrated walker; standard_filters(false) must precede hidden(true).

Validation: 386 regular tests pass, 20 optional tests ignored, in serial and default parallel runs.
Explicit player desktop regression verifies automatic additions/album rename while the same stream
keeps playing, plus queue/seek/error-retry/stop; search refresh/reentrant rendering and all-view
snapshot-selection fixtures pass too. Read-only real-library integration finds Nikes/Space Song/Juna
in 2–24 ms, hydrates 994 songs exactly once across Search and Music in 61.73 seconds, then drops all
consumers and reopens from disk in 60.36 ms with zero decoder reads. The real fixture uses a temporary
cache and the production sandbox helper; it does not alter the user's songs or their existing cache.
Local measurements are not universal speed guarantees. Build/format/diff checks pass.

At that stage the next priority was queue reorder/removal and restart persistence (now implemented
above), then desktop media-key/Noctalia
player integration. Cover audits, remaining random-crash diagnosis, thumbnail-cache budgets and
hands-on drag/Trash/image-zoom/lip-sync acceptance are still open. Column sizing remains deferred.

## Whole-library search follow-up (2026-10-09)

User authorized the next search reliability/performance pass. Reopening search now rewalks the
scope rather than trusting a stale cached listing. Successful cached tags are reused only when
device/inode/size/nanosecond mtime/ctime match; changed-during-read tags are rejected, failed reads
are retried on refresh. The bounded four-root/32 MiB in-memory cache retains tags for ten minutes.
Old cache/map references are dropped after enumeration instead of surviving the idle worker.
Title/artist words match filename and folder fields in either order even before tags arrive.
Search tabs now receive replacement snapshots while indexing, preserving selection by location
and replacing superseded top-100 results. Stale requests and reentrant cancellation are guarded;
GTK model replacement suppresses selection callbacks in Columns, Grid and Explorer. The dialog
has Refresh / Ctrl+R, retains the query and pending selection through refresh, and does not rebuild
unchanged result rows. Audio results use the existing themed music icon.

Latest validation: 369 regular tests pass, 19 optional/desktop tests ignored. Explicit desktop
search scope/refresh/reentrant-rendering and all-view snapshot-selection fixtures pass. Real
read-only library check found Nikes, Space Song and Juna in 7–23 ms before tag completion, parsed
994 audio files through the production sandbox in 61.42 seconds, then refreshed warm in 10.70 ms
with zero tag rereads. These are local sample timings, not guarantees. An initial real-file test
failed because current_exe was the Rust test runner, not Hermes's helper; one-shot parsing now has
an explicit executable seam used by that fixture. Production still supplies current_exe and keeps
the same sandbox/limits. Build, changed-search-module formatting and diff checks pass.

At that stage there was no live library watcher or shared persistent search/Music catalog; the
shared-library section above supersedes that limitation. No blanket crash fix is claimed.
The refreshed development binary is target/debug/strata; Hermes Dev still launches the existing
auto-rebuild script. It was not running at the end of this pass. Column sizing remains deferred.

## Big update in progress (2026-10-04)

Noctalia is now enabled in the user's settings and the user confirmed the colors look good.
Follow-up: a labeled Queue button in the bottom transport opens a live, themed queue popover
without leaving Files; it shares the Music view's model and double-click activation. Desktop
regression covers opening the popover, matching rows, jumping, closing, and error/retry with
the original queue intact. Play now retries failed streams and clears stale error tooltips.
Space Song reproduced a decoder disconnection after ~16 seconds in the actual Music path.
Cause: bubblewrap --die-with-parent was attached to the temporary GLib startup thread, which
retires after 15 idle seconds. Playback now spawns the worker on its dedicated monitor thread
and keeps that thread alive until killing/reaping the sandbox. No isolation/limits were removed.
A startup-thread-exit regression failed before the fix and passes
after it. Direct Space Song startup/middle/late seeking passed too. The silent desktop soak
played the complete real 320-second Space Song, advanced to Beyond Love and played two more
seconds, passing in 323.23 seconds. Latest regular suite: 358 pass/17 ignored. Queue popover,
retry, startup-thread-exit and malformed-media desktop/worker checks pass. The user subsequently
confirmed the queue/playback updates seem to work well.
Video replay follow-up: the apparent EOS/replay failure was a desktop-test polling bug, not
a reproduced decoder failure. A busy fullscreen event loop drained 128 events before checking
the predicate again, advancing beyond the test's two-second replay window. The pump now checks
its predicate, error and deadline between every event. The expanded fixture tests replay by
the real Play button twice, rapid replacement seeks, seeking/finished status, fullscreen and
cleanup. Video controls now expose Replay from beginning and a finished/seeking notice.
Three consecutive expanded video fixtures pass (16.85s, 11.01s, 16.76s). Music queue/retry
and GIF looping desktop regressions also pass. Build, changed-module formatting and diff checks
pass; the regular suite remains 358 passing/17 ignored. No decoder/protocol limits were changed
for this follow-up, and no replay-specific engine workaround was needed.

User order is now stability, whole-library song search, full music player, then Noctalia.
Column sizing stays deferred. They want a persistent transport plus a dedicated Music view,
using ~/Music (their shorthand /music does not exist as an absolute directory here).
The transport must be hidden before the first actual playback, remain available while paused,
and hide on Stop/queue completion. This behavior is implemented and the generated desktop
FLAC/MP3 fixture passes. Song activation is now agreed: double-click/Enter plays the selected
song and queues only it plus the remaining songs in that folder. Starting at track 3 queues
3, 4, 5 onward, not earlier tracks or another album. Single-click continues to select/preview.
Native audio activation is routed into Hermes, while non-audio/default Open With behavior is
preserved. File activation prepares metadata order off-thread with cancellation; Music view
activation uses the same folder-only suffix rule even when filtering the library.

New music.rs queue and music/library.rs bounded local scan feed ui/music.rs. Ctrl+M switches
Files/Music; the bottom transport is outside both views and survives navigation/drawer closure.
Audio uses the same isolated full-length PlaybackStream (without a video poster request), with
audio-preroll seek acknowledgments. Dedicated Music has album covers, metadata tracks, library
filtering, queue, next/previous, shuffle/repeat, explicit Play/Add/Play-next actions, volume,
and an interactive numeric waveform. Ctrl+Enter/context Play in Hermes plays selected audio;
context Add to music queue appends it. Queue persistence/reordering, MPRIS/media keys, gapless,
ReplayGain, additional roots and final visual polish remain unfinished. It is per-window,
not yet a single shared application-wide music controller. Real speaker acceptance is pending.

MusicTags now includes title/artist/album_artist with backwards-compatible serde defaults.
The shared music catalog persists validated tags by device/inode/size/nanosecond mtime/ctime;
Refresh rescans and folder monitoring updates open consumers automatically. The waveform helper streams complete decoded PCM
into bounded numeric peaks, not a colored PNG or lossy whole-track intermediate; the identity
cache is capped at 256 entries/8 MiB. Analysis budgets can reject long/expensive tracks without
affecting playback. Source files are unchanged.

Search has explicit This folder/Home/Music scopes (Music default), global metadata matching,
coalesced query/result mailboxes, index budgets, cancellation, stable result selection and
an identity-validated in-memory tag cache with a ten-minute age limit. Each reopening or Refresh
rescans the file listing. Ctrl+Shift+K forces Music scope. Show
results in tab uses the same scoped URI contract. Music-scope and inside-Music folder search now
share Music's persistent/live-updating catalog; general recursive searches retain a separate index.

Noctalia opt-in is implemented via the verified GTK4 export, validated semantic tokens,
atomic-replacement monitoring, debounce and session-local last-valid fallback. It does not
modify Noctalia configuration or auto-enable itself. See docs/themes.md.

Hover-peek patches and new regression tests are present: stale anchor/row/navigation checks,
released UI RefCell borrows before GTK callbacks, and reentrant close/replacement guards around
enumeration. The historical core dumps identify the timer abort boundary, not the original
panic; do not claim every random crash is fixed. The stability/search subagents were interrupted
before final reports, so finish reviewing/running their desktop regressions. Current production
cargo check/build pass; latest music and hover-peek desktop fixtures pass. Latest regular run:
357 pass/14 ignored. Folder-queue, full FLAC/MP3 playback/auto-next, and registered song-activation
desktop checks pass. UI pointer/speaker acceptance and further architectural review are pending.

Still hanging from earlier requests: pin/tab drag acceptance; Trash/GVfs backend verification;
cover-candidate fallback/embedded artwork audit and cache byte budgets/identity; image zoom
acceptance; real video lip-sync/full-length/seek checks and occasional decoder disconnection.
Do not present this ongoing big update as a finished release or silently change activation UX.

## Playback follow-up (2026-10-03)

Full-length local video is now connected through playback.rs + playback/worker.rs + the custom
GtkMediaStream in ui/preview/stream.rs. LocalPreviewProvider returns StreamingMedia (a path to
the session factory, never GTK MediaFile::for_file). Session startup runs off the UI thread. The
persistent bubblewrap GStreamer playbin worker reads one canonicalized original, negotiates bounded
RGBA (1280x720/30fps) and F32LE stereo/48kHz PCM, and paces sinks on its playback clock. Only validated
raw packets cross stdout; the GUI coalesces frames and uses a small/leaky raw-audio appsrc queue.
No GUI decoder parses the original. The video 10/30-second clip limits are gone; GIF normalization
and the music waveform/player path remain separate. F only changes presentation of the same stream.

Host plugin registry is mounted read-only when available, cutting repeated sandbox plugin scans.
Actual PATD and Tidal Waves probes prepared their full 48/70-second timelines in ~140ms and sought
to 80% in ~90ms (warm caches, local samples, not a universal guarantee). Initial cold scanning was
~3.3 seconds. Three consecutive desktop full-length fixtures passed playback across 30 seconds,
late seeking, fullscreen continuity, EOS/replay, and cleanup. Seeking pauses/settles the worker,
rejects old generations, flushes host audio, and acknowledges preroll as well as ASYNC_DONE: relying
only on ASYNC_DONE intermittently stalled replay. Worker resets to READY after EOS before seeking.
The legacy clip-conversion helper remains CLI-only and is not in the normal video's service route.

Playback retains namespace/input isolation, 1.25GB address-space/32MB file-size/128-FD guards,
two libav decoder threads when supported, bounded IPC, and 12s startup/seek/active-packet-stall
watchdogs. It intentionally has no cumulative CPU/movie-length cutoff. Stop/drop kills/reaps the
bubblewrap tree off the UI thread, including late startup completions. Posters are now owned by
the stream, independently fetched and cancelled when the real frame arrives; the old provider
Poster event was removed. CI/release build dependencies and README/sandbox docs are updated.
User visual, audio-speaker and lip-sync acceptance remains pending. Do not claim hardware decoding,
zero-copy frames, or a completed music service. Earlier notes below describe the superseded clip path.
GIF regression checks exposed an end/seek/play race: animation restart now waits for seeking=false
and resumes through a coalesced idle action, rather than playing immediately after an async seek.
Three consecutive GIF fixtures passed after that change; pause/remap/fullscreen/stop checks remain.
Final regular suite: 289 passing, eight optional/desktop fixtures ignored. Build and changed-module
format checks pass. Explicit full-length, GIF, legacy playback, request-lifecycle, malformed-input,
and three real-file worker probes passed. Clippy is not installed in the selected toolchain.

Poster-first video loading is connected: LocalPreviewProvider requests a bounded 256px video
thumbnail independently of media conversion through the existing thumbnail pool/cache. A new
nonterminal Poster event displays an aspect-preserving picture and Preparing playback notice;
it does not finish/cancel the media request. Poster decode errors are ignored, playback failures
remain visible, and request IDs reject stale/late events. Completion cancels the poster's separate
sandbox token and aborts its GLib task; LoadHandle cancellation stops both jobs. Full-length playback
is not implemented yet. An explicit desktop fixture checks mismatched IDs, invalid PNG, nonterminal
posters, terminal completion, late events, and drawer cleanup. User-facing timing/visual acceptance
is pending; no claim of faster decoding, only earlier visual feedback.
The user clarified the next engine's behavior: full-length playback belongs directly in the drawer;
F ONLY toggles fullscreen, never upgrades a limited clip to full playback. Keep one shared session,
position, mute/volume, and transport across both surfaces. Do not implement the previous proposal
of limited drawer playback with full-length playback reserved for fullscreen. The existing 30/10s
limits are still present until a cancellable, bounded streaming/seek-capable engine replaces conversion.

GIF/PATD follow-up: GIF MIME/extension detection now routes previews through the new sandbox
PreviewAnimation operation, not static PNG rendering. Thumbnails remain images. FFmpeg converts
one GIF cycle to bounded 30fps VP8 WebM with alpha (up to 30 seconds, no audio). MediaPreview
autoplays once after both prepare/map, preserves pause across remapping, hides silent audio
controls, and schedules coalesced idle restarts on ended notifications to avoid reentrant
decoder calls. Native GtkMediaFile loop mode intermittently
stalled at the last frame; do not switch back to relying on set_loop(true). stop disables
autoplay/restarts before pausing and closing fullscreen. An explicit desktop animation test
verifies actual timestamp wraps, pause/resume, remapping, fullscreen, and stop cleanup.
Three consecutive desktop animation runs and the video playback smoke test passed. The regular
suite passes 280 tests with four desktop fixtures ignored by default; the development build passes.

PATD's MP4 is 888x1514, 58fps HEVC with AAC and a 90-degree display rotation. Both previous
30-second conversion profiles hit SIGKILL under the decoder CPU guard. Compact retries now
probe video codec/pixel rate in the sandbox and shorten high pixel-rate HEVC (>60MP/s) to ten
seconds. Normal quality/other compact retries remain thirty seconds; existing per-process,
output, and overall deadline limits are unchanged. Actual production sandbox conversion passed
in 5.3 seconds: 640x376 VP8 + Opus, 10.008 seconds. The real at-the-zoo GIF normalized in 0.13
seconds to a 720x480, 0.6-second alpha-preserving WebM. Source files were unchanged. Full-length
streaming/player work and poster-first loading still remain separate future milestones.

Album order is now connected. `app/music_order.rs` shares numeric disc/track ordering between
folders and hover peeks; artwork follows tracks even descending, and missing tags use numbered
filename fallback. LocalFileSource emits visible batches/Finished before asynchronous MusicMetadata
chunks (32 tracks). Tags use sandbox workers/FFprobe and a 512-entry LRU identity cache with inode,
device, size, nanosecond mtime/ctime; metadata bypasses the older coarse preview cache. Dropping a
load cancels parsing; stale request IDs, changed sizes/times, and removed entries are rejected.
Automatic album detection is limited to leaf folders with at least two audio files, allowed
artwork/companions, and no conflicting known album tags. Manual sort choices override automatic
sorting for the current column; automatic defaults do not leak into other folders. `Album track`
is also available explicitly. Peeks now replace sorted snapshots instead of appending raw batches.
Selection remains tied to locations during hydration. Tests cover sorting, multi-disc metadata,
fallback, selections, stale requests, manual overrides, cache invalidation, and late UI events.
Actual sandbox worker probe verified all 17 Blonde FLAC tags (tracks 1–17) in about 0.52 seconds.
278 unit tests pass (three desktop tests ignored); the development binary builds. User visual
confirmation in the live app remains pending. Older album-pending notes below describe
the preceding work-in-progress, not the current implementation.

Image previews now share F/fullscreen entry with videos, with F/Escape exit and lifecycle cleanup
when the drawer closes or changes files. Rasterized images and GeoTIFF images use a shared viewer
with Fit/100%, wheel zoom, and drag pan. Replaced the old Picture sizing/one-shot fit with explicitly
scaled canvas dimensions, resize-aware fit, centered-margin-aware cursor anchoring, and pending
adjustment updates after layout. Tiny fitted previews no longer jump to the old 10% lower bound.
100% describes preview pixels, not original-file resolution. A synthetic desktop image fixture
checks fit, actual-size centering, wheel zoom, fullscreen F exit, and close cleanup. No private
images were used. Unit suite: 267 passing, three explicit desktop fixtures ignored by default.

Album-order groundwork is present but NOT connected to browsing: optional MusicTags on FileEntry,
bounded FFprobe album/disc/track JSON parsing and a sandbox operation. No Track sort is exposed
until the comparator and async hydration are implemented. The temporary non-exhaustive Track
variant that caused the user's build failure has been removed; the obsolete Pixbuf conversion
was replaced with TextureDownloader. GIF playback, the PATD MP4 failure, and full-length video
streaming remain pending. User selected album ordering as the next major priority.

Latest UI direction: the user wants Hermes-specific polish, not a Dolphin clone. Tab pinning was
only a persisted flag with no protective behavior; at the user's request its UI and toggle action
are removed. The serialized flag remains readable for compatibility. All tabs now have internal
drag-to-reorder on their title, edge insertion feedback, and persisted relative ordering that
preserves the active tab. Pointer gesture acceptance is pending; order/payload tests pass.
F opens an already loaded video preview fullscreen; F or Escape exits. Text editing and modified
shortcuts are excluded. The explicit desktop playback test verifies fullscreen opening and F exit.

Album order remains unresolved. FileEntry carries no disc/track metadata, the generic comparator
uses Name/Type/Size/Modified, and hover peeks append raw filesystem batches. Nikes in the Blonde
folder really has embedded track=1/disc=1/album=Blonde. The next fix must share a bounded sandboxed
tag service and numeric disc/track comparator across normal views and hover peeks, not merely
sort filename strings. Do not claim that metadata ordering or full-length playback is implemented.
The user is choosing which of these two milestones to prioritize next.

Tab polish: the user rejected stretched full-width tabs and an outlined selected tab. The revised
strip uses compact non-expanding tabs, capped/ellipsized labels, centered action buttons, opaque
semantic surfaces, and only a thin top accent for the active tab. Active tabs scroll into view
after allocation. An explicit themed desktop fixture passed compact-width and vertical alignment
checks; user visual acceptance remains pending.

The user identified `early dgd Tidal Waves.mp4` as still failing. Its 1080p conversion was killed
by SIGKILL under the decoder guard. Limiting filter threads alone did not help. Media conversion
now tries one lighter 640x480/24fps profile only after SIGKILL/SIGXCPU, retaining normal quality
otherwise. The actual helper succeeded in about six seconds with a 640x360, 30-second WebM and
Opus audio. Original files, per-process limits, output limit, and overall 12-second deadline are
unchanged; two attempts can consume more aggregate CPU than one. Full-length in-app playback is
still not implemented: open the original in the default player for that.

The user now reports all video previews render; the remaining issue was silent audio. A live
PipeWire/Pulse stream for the dev process showed 0% application volume despite GtkMediaFile
reporting unmuted volume 1.0 and audio=true. Restored only the confirmed Hermes stream to 50%;
The user confirmed audio is audible again. Do not globally force system volume at startup.
The preview of driving with neft retains an Opus track with non-silent sample levels in the
unchanged sandbox. The desktop smoke test now uses MemoryInputStream, matching the actual drawer,
and asserts audio detection, initial mute/volume, transport, seeking, and lifecycle cleanup.
These checks do not establish what reaches the user's speakers.

One-shot sandbox failures now retain bounded decoder stderr in the private temporary output
directory and display it with exit status instead of discarding it. All existing sandbox limits
remain unchanged. Playback prepared/playing/error diagnostics are logged without per-frame noise.
257 regular tests pass; the desktop smoke test is explicit/ignored in the normal suite.

## Desktop development update (2026-10-02)

Unified video preview controls now live in `src/ui/preview/media.rs`: persistent drawer transport,
seek/time, mute/volume, fullscreen sharing the same stream, idle fullscreen controls, Escape exit,
and bounded-preview/loading/error messaging. Uses bundled Lucide icons and semantic theme colors.
Decoder limits and original files are unchanged. An explicit desktop smoke test with a generated
clip passed preparation, playback, seeking, mute, fullscreen creation, and stop/close cleanup.
Visual polish and fullscreen fade still need the user's check. Full music playback remains planned.

Latest user direction and targeted code review: `docs/hermes-direction.md`. Equal focus on files,
media, and music; song search should cover the whole library regardless of current folder.
Immediate order remains preview reliability/crash diagnosis, Noctalia colors, then full music.
Column sizing stays deferred.

The user confirmed folder pinning works. The empty prompt is now shown only without visible pins;
subtle gaps above/between/below pins accept new folders or reorder existing bookmarks. Pinned row
centers still transfer files. The new insertion/reorder gestures need real desktop verification.

The Important screenshot failed on a JPEG, not a PNG. Three portrait JPEGs exceeded the full-raster
guard, as did some covers. Reduced JPEG decoding plus stdin transport now succeeds in actual
pooled-worker probes; the sampled PNG and existing HEIC probe also pass. Ordinary decoder errors
now send bounded framed responses instead of looking like missing wire headers. Resource-killed
decoders can still produce EOF. The most recent library-only run passes 254 tests.

Two saved core dumps (21:14 and 21:17 MDT) abort in the hover-peek timeout callback. Root panic is
not yet reproduced or fixed; do not claim random crashes are solved. The dev script enables Rust
backtraces and appends to its local log. A temporary extracted core was deleted after local review.
Search currently uses the current location in the dialog, but Home in the search URI adapter;
scope unification and music metadata indexing are proposed, not implemented.
The user wants both hover peeking and click/Space previews, with hover optional in Settings.
An existing Folder peeking toggle already implements that preference surface; do not duplicate it.

The user resumed development with Codex and deferred column sizing. Today's scope is sidebar
drag and drop plus a development build that automatically rebuilds and restarts.

- Sidebar folder rows, pinned places, recent folders, and mounted devices now accept file drops.
  Internal drags default to move, external drags default to copy; Ctrl requests copy and Shift
  requests move. Existing transfer conflict handling is reused.
- Trash accepts move drops only and uses the existing trash confirmation and operation provider.
  Today/Yesterday search rows remain non-destinations.
- A dedicated PINNED drop area adds local folder bookmarks without moving files. Metadata is
  queried asynchronously, non-folders are filtered out, and repeated folders are deduplicated.
- File drops are excluded from the separate sidebar reorder controller. Hover feedback uses
  semantic theme colors, and Music is now included among standard places.
- `make start-dev` uses a user-local Rust toolchain when available and `STRATA_DEV=1`, and watches
  source, assets, the manifest, lockfile, and build script. On this desktop, the Hermes Dev launcher
  is installed in `~/.local/share/applications/hermes-dev.desktop`.
- Preview decoder fixes and column sizing remain pending. The historical notes below describe
  the original handoff; the sidebar work is implemented in the current local diff.

## Handoff status

Latest follow-up: Grid's thumbnail popover now includes List/Grid/Explorer options; List and
Explorer also have a nearby view selector. A shared window action persists mode choices and
keeps menu checks synchronized. The dev script captures output in
`~/.local/state/hermes/dev.log`; stale duplicate watchers on this desktop were stopped.

Runtime pin-drop traces showed MOVE-only offers. The pin area now accepts internal MOVE-only
file drops as bookmarks (Hermes drag-end never deletes the source); external MOVE-only drops
remain rejected to prevent external sources from deleting files. Actual gesture verification
is still pending, so do not report pinning as verified working.

Video previews now normalize to at most 960x720 at 30 fps rather than preserving high input
frame rates. Under the unchanged 10-second CPU budget, `Videos/driving to bea.mp4` failed before
and now produces a valid 30-second WebM in about 3.6 seconds. Source files are not changed.
Custom Hermes video controls, broader sample testing, artwork fixes, and Noctalia are still pending.

The proposed music workspace, FLAC player, cached seekable waveforms, Noctalia theme integration,
sorting, and preview/artwork work are described in `docs/music-player-plan.md`; not implemented.
Existing view modes were found by the user; improve discoverability rather than duplicating them.
Pin drops now explicitly negotiate COPY on enter/motion and have a larger hit area; the pointer
gesture still needs user verification. Trash also fails outside Hermes (`gio list trash:///`),
and this desktop lacks GVfs. The error now explains the missing backend. Installing GVfs needs
the user's system administrator credentials.

This repository snapshot preserves the ongoing Antigravity work as of 2026-10-01. The user requested an upload and this handoff; the fixes below are the next task, not a claim that they have been implemented or verified. Read AGENTS.md before working. Older agent notes are preserved in .agents/ for context; this note states the latest user priorities.

## Reported root causes

- **Preview failures ("Incomplete browser header"):** The pooled sandbox worker passes files as anonymous `/proc/self/fd/N` file descriptors. Several format decoders (3D models, comics/eBooks, spreadsheets, audio, image decoders) relied on file extension strings (`.extension()`), which return empty in the worker. When decoders errored out, the worker process exited without writing a header to the output pipe, producing `Unable to read wire response: Incomplete browser header`.
- **Missing formats:** `pptx` is missing from format classification and thumbnail extractors, and HEIC requires robust stdin decoding.
- **Column squishing:** When the preview drawer opens on smaller screens or multi-level folders, columns are clipped on the left because horizontal scroll adjustments only fire on column creation, not on preview drawer layout changes.
- **Sidebar drag & drop:** Sidebar place rows currently only accept internal String drops for reordering, but lack `gdk::FileList` drop targets to accept dropped files into folders or pin new folders.

## User review / requirements

Keep all changes self-contained within Hermes (`~/src/hermes`), fully covered by unit and integration tests, and strictly preserve non-modal navigation. The supplied plan says the changes are fully covered; the next agent must verify actual coverage and results rather than assume that statement is an established result.

## Proposed changes

### 1. Robust magic-byte sniffing in sandboxed parsers

Eliminate extension dependencies in the worker so all formats render reliably from anonymous file descriptors.

- **`src/sandbox_helper/model.rs`:** Check for `PK\x03\x04` ZIP magic header: if present, parse as 3MF; otherwise parse as ASCII/binary STL.
- **`src/sandbox_helper/archive_cover.rs`:** Sniff container headers: RAR signature (`Rar!\x1a\x07`) for CBR; for ZIP files, inspect metadata (`mimetype` containing epub or `content.opf`) to parse EPUB, falling back to sequential image extraction for CBZ.
- **`src/sandbox_helper/table.rs` and `src/services/table.rs`:** Remove `calamine::open_workbook_auto` extension dependence. Detect container structure: ZIP with `xl/` → `calamine::Xlsx`; ZIP with `content.xml` → `calamine::Ods`; OLE2 header → `calamine::Xls`. Try XLSX → ODS → XLS fallback chain.
- **`src/sandbox_helper/audio.rs`:** Pipe audio streams via stdin (`pipe:0`) to FFmpeg and FFprobe. Ensure WAV header metadata fallback functions correctly on in-memory buffers so duration is never incorrectly 0.0s.
- **`audio.rs` / `src/services/formats.rs` / `src/adapters/local_preview.rs` (confirm relevant extraction module):** Add `pptx` to `FormatFamily::OfficeDocument`. Support extracting `docProps/thumbnail.jpeg` from PPTX files for visual slide 1 preview.
- **HEIC:** Support robust stdin decoding.
- **`src/sandbox/browser/worker.rs`:** Ensure `execute_job` always transmits a framed response, even on decode failure, returning a structured error so the host never receives an EOF / Incomplete browser header.

### 2. Auto-shrink / collapse ancestor columns for maximum preview space

Modify `src/ui/browser.rs` and `src/ui/preview.rs`:

- **Ancestor column shrinking:** When the preview drawer opens, automatically shrink/collapse previous ancestor columns (all columns before the active folder) down to compact slim rails (~48px) or hide them so the active folder and preview drawer get maximum horizontal real estate.
- **Auto-restore on close / navigate:** When the preview drawer is closed, or when clicking an ancestor column / navigating left with Left Arrow, immediately restore ancestor columns back to their full browsing width (240px).
- **Splitter prioritization:** Prioritize preview width so documents, 3D models, tables, and waveforms render large and unconstrained.

### 3. Sidebar drag & drop integration

Modify `src/ui/window.rs`:

- Install `gtk::DropTarget` for `gtk::gdk::FileList` on system places (Home, Documents, Downloads, Music, Pictures, Videos, Trash), pinned places, and Recent folders.
- Connect file drop handlers to trigger file transfer (move or copy) into the targeted folder. Preserve appropriate Trash semantics.
- Allow dropping folders onto the sidebar to pin them directly into the PINNED section.
- Provide visual drop feedback (highlight row on hover).

## Verification plan

### Automated

- `cargo check --all-targets`: verify compilation and inspect warnings; the requested target is zero compiler warnings.
- `cargo test`: run the complete suite (the supplied plan expects 560+ tests) and verify no regressions. Record actual counts.
- Verify `export_samples` exports and tests all sample formats cleanly. Inspect its output paths before running on the destination PC.

### Manual

Launch Hermes (SUPER + E) and test:

1. Click `mechanical_pyramid.3mf` → renders 3D model.
2. Click `sample_novel.epub` and `comic_issue_01.cbz` → renders cover art.
3. Click `quarterly_budget.ods` and `metrics_report.xlsx` → renders spreadsheet table.
4. Click `test_tone_440hz.wav` and MP3s in `~/Music/` → renders audio waveform.
5. Click PNG, HEIC, and PPTX files → renders previews.
6. Open 3+ columns, then toggle preview (Space): verify columns smoothly auto-scroll and remain visible without being clipped off.
7. Drag a file from a column onto Downloads or Music in the sidebar: verify file moves/copies into the folder.

## Upload-time validation (2026-10-01)

- `cargo check --all-targets --offline` passed, with existing compiler warnings (including unfulfilled lint expectations). Zero-warning target is not yet met.
- Full tests, sample export, and manual UI verification were not run during this upload-only task. Historical test claims in other notes are not fresh verification.
- `git diff --check` found an existing trailing blank line in `src/sandbox_helper/tests.rs`; source was preserved as received.
- The ignored `target/` build directory is excluded from GitHub; source, tests, and existing `.agents/` notes are included.
