# Performance Baseline

## Tab corners, scrubbing and JPEG/MOV loading — 2026-10-10 follow-up

File-tab surfaces and selection buttons now have square corners; compact tab widths, control
alignment, top accents and drag ordering are unchanged. One shared seek interaction manages file
audio/video/fullscreen controls and the main music transport. Pointer/touch scrubs retain the thumb
while playback timestamps arrive, commit on release and coalesce outstanding targets. The scale
stays enabled during seeking. Keyboard/click changes remain immediate; paused streams stay paused.
Cancelled/unmapped controls and changed streams discard pending targets; programmatic time updates
do not trigger seeks. Deterministic GTK state-machine tests and native paused-music checks pass.
These tests do not inject physical mouse events; confirm actual thumb dragging in the user's session.

JPEG thumbnails now check an embedded EXIF JPEG before original-image decoding. Read at most 128 KiB
of headers, honor both primary and thumbnail orientation, require adequate dimensions and matching
aspect, retain an available embedded ICC profile, and keep the existing decoded-frame limit.
Malformed/missing/too-small/mismatched thumbnails fall back safely. Full 1400px JPEG previews use
bounded original decoding even above the full-frame budget, not the old stretched EXIF shortcut.
ImageMagick outputs now use PNG compression level 1, without reducing pixel resolution or increasing
sandbox memory/output/time budgets. Existing identity-checked thumbnail/preview caches are unchanged.

Generated 2400×1600 JPEG with 384×256 embedded thumbnail, three debug renderer samples:
embedded path about 2.2 ms versus original-image ImageMagick about 9.8–11.1 ms. Production pooled
sandbox sample: thumbnail cold ~44 ms / warm ~20 µs; full preview cold ~53 ms / warm ~11 µs.
All eight EXIF orientations are compared to ImageMagick; small/missing thumbnails, cancellation,
source replacement and original-image full previews (including a 3600×2400 source) are checked.
These generated colors are not representative noisy camera photos and timings exclude UI paint.

Separate three-sample shell renderer comparison on generated textured 2400×1600 JPEG, 1400px output:
default PNG compression 141–149 ms; compression 1 about 86–101 ms. These are warm filesystem-cache
renderer samples, not cold-disk or end-to-end UI measurements; PNG files can be larger.

Video playback now starts the decoder immediately but delays fallback poster work by 200 ms.
If a frame is ready, selection is cancelled or the decoder failed, no extra poster job starts.
Grid/movie thumbnails still use the existing cached ffmpegthumbnailer path and representative
10%-position selection. A slow decoder can show its fallback poster up to 200 ms later; this is a
contention tradeoff, not a guaranteed speedup for every MOV. There is no whole-file conversion or
duration limit. Generated 720p H.264 MOV native first-frame samples were ~127 ms (three runs), and
the 45-second MP4 seek/fullscreen/replay fixture passed. No comparable before/after first-frame
baseline was captured; these samples do not establish HEVC/4K, disk-cold, Wayland or Dolphin parity.

Logs: `/tmp/hermes-media-qa`. The older application-shared-controller optional GTK test stops at its
stale `Button::label()` assertion for the now-icon-only Repeat button, before its seek checks;
that unrelated test was left unchanged. A new focused native multi-view seek test covers this work.

## Retained loaded tabs — 2026-10-10 follow-up

Ready/empty, monitored native tabs now keep their NavigationState and existing GTK presentation
mounted but hidden. Unchanged returns do not enumerate or replace the row models. Selection,
filters, history, thumbnail size and visible scrollable-axis positions survive. Each window retains
up to six inactive tabs within a conservative estimated 64 MiB accounting budget, not a hard RSS
or GPU-texture cap. Dirty folders, late metadata, changed preferences and 30-second revalidation
reload contents with location-based selection restoration. Unsupported/incomplete/error/oversized
tabs load normally; close/eviction releases both views and monitors.

Three release samples of the generated two-folder 1k-text-file actual TabBar fixture, private
HOME/XDG/session bus and isolated Broadway display measured model completion/restoration:

| Presentation / visit | Median | Range |
| --- | ---: | ---: |
| Columns, first cold | 40.04 ms | 38.42–42.84 ms |
| Columns, second cold | 23.49 ms | 23.20–24.08 ms |
| Columns, first warm | 1.71 ms | 1.59–1.78 ms |
| Columns, second warm | 1.29 ms | 1.29–1.32 ms |
| Grid, first cold | 107.33 ms | 105.89–115.76 ms |
| Grid, second cold | 143.59 ms | 143.15–145.55 ms |
| Grid, first warm | 8.10 ms | 8.06–8.40 ms |
| Grid, second warm | 7.45 ms | 7.39–8.23 ms |
| Explorer, first cold | 63.84 ms | 62.54–65.23 ms |
| Explorer, second cold | 51.96 ms | 49.62–52.92 ms |
| Explorer, first warm | 2.67 ms | 2.65–3.02 ms |
| Explorer, second warm | 2.15 ms | 2.14–2.63 ms |

Warm runs assert no entry-insertion event. Timing ends after BrowserView restores its presentation,
at the restoration notification, before any final tab-strip work or completed paint. The fixture
also verifies additions while inactive and rename/delete while active. These synthetic text-file
model timings do not measure cold storage, MOV/JPG/HEIF decoding, full production-window handlers,
native Wayland painting, real personal folders or Dolphin parity. The test display had no connected
browser client; older connected-client samples below are not a controlled before/after comparison.

A separate generated GTK fixture checks original model/shell/mode widget identity, actual saved
scroll adjustments, filters, selection and release of closed tabs across all three modes. Three
debug repetitions passed, as did the existing discarded-navigation lifetime fixture. Repeat with
`desktop_retained_tabs_preserve_presentations_and_release_closed_views` and
`desktop_warm_tab_directory_reuse_and_live_changes` (`HERMES_TEST_TAB_MODE=grid` / `explorer`).
Cold movie poster generation, image decoding and cheaper Grid binding remain separate targets.
Validation includes normal unit tests, all-target compilation and debug/release builds; older
unrelated integration failures remain documented in NEXT_AGENT.md.

## Lazy alternate views and HEIF decoding — 2026-10-10 follow-up

Folder navigation now constructs only the selected Grid or Explorer pane, instead of also building
hidden alternate panes. Switching modes releases hidden presentations; requesting the same mode
does not rebuild it. Columns are still maintained as the underlying navigation presentation.
Three further release samples of the same generated two-folder 1k-text-file tab fixture, private
Broadway display and connected browser client measured complete GTK model times:

| Presentation / visit | Median | Range |
| --- | ---: | ---: |
| Columns, first cold | 67.85 ms | 63.90–73.93 ms |
| Columns, second cold | 50.60 ms | 50.28–60.30 ms |
| Columns, first warm | 27.49 ms | 23.93–34.86 ms |
| Columns, second warm | 23.11 ms | 21.52–23.83 ms |
| Grid, first cold | 195.94 ms | 185.69–232.83 ms |
| Grid, second cold | 200.62 ms | 185.10–287.76 ms |
| Grid, first warm | 120.69 ms | 117.26–122.89 ms |
| Grid, second warm | 116.18 ms | 112.39–140.83 ms |
| Explorer, first warm | 55.21 ms | 49.16–58.28 ms |
| Explorer, second warm | 53.57 ms | 47.54–65.55 ms |

These are subsequent samples, not an isolated A/B experiment or proof of native desktop/Dolphin
paint parity. The all-view lifecycle fixture verifies that unused alternate models are absent,
discarded objects release and changing modes keeps complete folder data. Full loaded-tab GTK model
and scroll retention was not implemented in that sample (see the follow-up above); cheaper Grid binding and native Wayland paint measurements
remain next targets.

A separate debug fixture generates a 4032×3024 HEVC image with a rotated 320px embedded thumbnail.
Direct native thumbnail decoding measured 18.94 ms at first decoder initialization, then 2.58/2.81
ms, versus 107–115 ms for full-image ImageMagick conversions. Inside the actual sandbox, its first
thumbnail request was 51.06 ms and repeat cache lookup 22 µs; full 1400px preview was 264.18 ms and
repeat lookup 11 µs. These are decoder/cache timings, not UI upload/paint or noisy camera-photo
benchmarks. The generated image has simple color quadrants; real photos and absent thumbnails
can cost more. Full preview resolution is unchanged. Tests verify thumbnail orientation and color
against the primary image, bounded selection, no-thumbnail fallback, cancellation and replacement
invalidation. Reproduce with ignored `generated_embedded_heif_thumbnail_orientation_fallback_and_cache`
and a built `strata` helper. Requires HEVC codec plugins and ImageMagick, but no personal files.

## Real tab-directory reuse — 2026-10-10

An explicit GTK/TabBar fixture switches between two generated 1,000-text-file folders. Measurements
start at navigation/tab activation, observe the first GTK-model insertion after BrowserView handles
it, and finish at the directory's model-completion event. Three optimized-build samples on the
Ryzen 5 7600X / GTK 4.24.1 host use an isolated Broadway display, private HOME/XDG state and session
bus, warm filesystem cache, and fresh application state per sample:

| Presentation / same process | First insertion, median (range) | Complete model, median (range) |
| --- | ---: | ---: |
| Columns, first cold folder | 50.66 ms (48.96–51.72) | 98.83 ms (98.04–105.23) |
| Columns, second cold folder | 30.77 ms (27.85–32.64) | 74.74 ms (72.60–86.18) |
| Columns, first warm return | 29.16 ms (28.04–32.31) | 51.37 ms (45.99–52.16) |
| Columns, second warm return | 23.25 ms (21.35–23.87) | 38.82 ms (37.15–39.44) |
| Grid, first cold folder | 53.86 ms (52.23–57.72) | 263.47 ms (252.22–272.54) |
| Grid, second cold folder | 71.70 ms (67.15–76.65) | 255.40 ms (252.18–271.49) |
| Grid, first warm return | 72.00 ms (70.53–78.27) | 167.18 ms (160.80–167.49) |
| Grid, second warm return | 72.18 ms (69.23–75.94) | 162.88 ms (159.10–173.74) |

This does not measure finished painting, cold disk I/O, image decoding, actual user directories,
full tab-scroll restoration or Dolphin. Grid's warm first insertion is not faster than its first
cold insertion; it still has significant binding/layout cost. Provider-data reuse improves total
model completion in these samples, not every interaction. Repeat with realistic media and native
Wayland frame measurements before claiming desktop speed parity.

The investigation also found controller/list-item/ancestor reference cycles retaining discarded
GTK presentations, plus per-icon registration scans. Weak event-time references and keyed icon
registration remove those costs; a generated lifecycle fixture checks widgets/models/items from
all three views are released across nine navigations and on teardown. Earlier debug probes grew
from ~370 to ~1,200 ms across four Grid navigations; a post-fix debug sample was ~296, 284, 225,
217 ms. Those are diagnostic samples, not a controlled release before/after percentage.

The folder cache keeps at most 12 watched native listings for 30 seconds with a conservative
estimated 32 MiB data budget. Failed/mutated/incomplete loads and unmonitored/search/remote sources
are not reused. Active changes and inactive-folder additions, rename/removal, symlink retargeting,
partial-replay invalidation and cancellation have fixture coverage. GTK models still rebuilt in
those historical samples; loaded-tab retention is implemented in the follow-up above. Cheaper
cold Grid binding remains open. Lazy construction is covered above.
Reproduce with the ignored `desktop_warm_tab_directory_reuse_and_live_changes` test; set
`HERMES_TEST_TAB_MODE=grid` or `explorer` for alternate views and use a private display/session.

This document defines the reproducible local performance check for Strata. Results are machine-specific and should be compared against earlier runs on the same system rather than treated as universal benchmarks.

## Fixtures

Generate deterministic mixed directories containing 1,000, 10,000, and 100,000 entries, plus deep-tree and native-path edge cases:

```bash
./scripts/generate-fixture.sh target/fixtures --clean
```

Omit `--clean` to fill in missing fixture entries without deleting existing ones. On the baseline machine, clean generation of all fixtures takes approximately one second.

## Instrumented run

Build once so compilation is excluded from startup measurements:

```bash
cargo build --release
RUST_LOG=strata=debug target/release/strata target/fixtures/100000
```

Strata accepts a startup directory on the command line. Structured logs report:

- Time until the application window is presented
- First provider batch latency
- Time until the first directory batch is rendered
- Complete enumeration time and entry count
- Per-batch UI append duration
- Cancelled directory request IDs

Capture sampled RSS and proportional set size (PSS) with:

```bash
STRATA_BINARY=target/release/strata \
  ./scripts/profile-fixture.sh target/fixtures/100000
```

The profiling script now isolates HOME, XDG config/state/cache and the session bus, so it cannot
forward the fixture to an existing personal instance or overwrite its saved navigation. It samples
the application process tree, including decoder descendants; isolated bus services are excluded.
This is a Linux/Python 3 tool requiring dbus-run-session and pidfd support. Incomplete/timeout runs
return a nonzero status. PSS avoids charging each process fully for shared pages.
Optional `STRATA_PROFILE_HEADLESS=1` uses a private GTK Broadway display (gtk4-broadwayd required).
Headless results measure provider/model/GTK insertion, not finished paint or interactive scrolling.
The probe uses local GIO VFS/volume monitoring and fresh settings/cache; it does not benchmark
remote backends, personal themes, restored workspaces or every installed desktop service.

## Navigation follow-up — 2026-10-09

Trail snapshots are now serialized/written/fsynced on a shared coalescing worker. Unchanged tab
widgets survive activation/refresh rather than being recreated. The generated two-tab desktop
fixture performed 200 warm tab-strip refreshes in roughly 0.8–1.1 ms total in debug samples;
this excludes directory loading, view replacement and painting. Object-identity assertions verify
reuse; rename, reorder, close and edit cancellation still work.

Directory enumeration now emits up to 32 visible entries first, followed by normal bounded batches.
On this host, three optimized-build runs of a generated 10,000-empty-text-file directory gave:

| Same release probe | First GTK insertion, median (range) | Complete enumeration, median (range) |
| --- | ---: | ---: |
| Initial 128-entry batch | 21.69 ms (21.39–22.32) | 150 ms (146–152) |
| Initial 32-entry batch | 6.13 ms (5.70–6.17) | 161 ms (143–164) |

This deliberately shows fewer entries sooner; it is not a faster decode or a four-times-faster
complete folder load. Full enumeration varied and its median slightly increased. The probe uses
a headless Broadway display, warm filesystem cache and isolated fresh application state. Startup
included private portal activation (~0.55 seconds) and is not a normal running-instance tab-switch
measurement. UI insertion excludes finished painting. The before/after runs use the same AMD Ryzen
5 7600X (6 cores / 12 threads) host, not the 9950X3D used in the historical August baseline below;
do not compare those hardware samples as a regression claim. These are engineering samples, not
Dolphin parity or universal guarantees. Fixtures remain under target/performance-fixtures/10000
and target/performance-fixtures/100000 for local reproduction.
The follow-up host has 30.5 GiB RAM, GTK 4.24.1 and Rust 1.99.0; the August environment differs.

One 100,000-file release sample after the change reported first insertion 6.57 ms, complete
enumeration 1,329 ms and sampled process-tree PSS 131,237 KiB (~128 MiB). This empty-file/Columns
case does not establish a photo-grid/music-directory memory budget. Cached decoded payloads now
have 16 MiB thumbnail-worker / 64 MiB preview-worker limits in addition to the UI thumbnail budget.

## Ordered merge probe — 2026-10-09

Directory debug logs now report `directory batch merged` / `merge_micros`, separately from GTK
insertion. Empty batches retain existing storage; strictly ordered non-overlapping batches append
without copying every preceding entry. Interleaved/equal batches retain the existing merge path.

The ignored `profile_ordered_directory_merges` test constructs 100,000 in-memory entries in
128-entry ordered batches. One debug-build sample before the change took 3.25 seconds; one after
took 15.17 milliseconds. This favorable model-only probe excludes filesystem enumeration, GTK
updates, painting, media workers and Dolphin. It does not replace the historical release baseline
below or establish overall browser speed parity. Next measurements need repeated cold/warm release
samples, interleaved batches, interaction/paint latency and complete process-tree PSS. See the
[quality review](quality-and-parity-review.md) for scope, remaining bottlenecks and proposed budgets.

## Initial baseline — 2026-08-29

Environment:

- AMD Ryzen 9 9950X3D, 60 GiB RAM
- Omarchy, Wayland/Hyprland
- GTK 4.22.4
- Rust 1.97.1
- Optimized `--release` build
- Warm filesystem cache

Each timing below is a single engineering sample. “First UI batch” includes process startup; the parenthesized value is the time spent appending that 128-entry batch.

| Fixture | Window presented | First provider batch | First UI batch | Complete enumeration | Peak RSS / PSS |
|---:|---:|---:|---:|---:|---:|
| 1,000 | 90 ms | 72 ms | 105 ms (2.5 ms) | 104 ms | 241.9 / 147.9 MiB |
| 10,000 | 89 ms | 67 ms | 103 ms (2.3 ms) | 287 ms | 247.4 / 153.2 MiB |
| 100,000 | 91 ms | 68 ms | 107 ms (2.3 ms) | 2,131 ms | 278.0 / 183.9 MiB |

### Findings

- Initial interaction is effectively independent of directory size because results arrive in bounded 128-entry batches.
- UI batch insertion is well below one 16.7 ms frame in these samples.
- Removing the UI’s duplicate `FileEntry` storage reduced the 100,000-entry sample from approximately 315 MiB RSS / 219 MiB PSS to 278 MiB RSS / 184 MiB PSS.
- The 100,000-entry memory result still exceeds the provisional 150 MiB target and remains an open optimization item. The current model retains one application entry and one GTK string object per result.
- Complete enumeration scales approximately linearly and remains asynchronous, but smooth scrolling must also be evaluated manually under sustained input.
- After globally stable incremental sorting was introduced, the 100,000-entry sample completed in 3,755 ms at 286.3 MiB RSS / 191.4 MiB PSS. Individual GTK insertion batches remained below 4 ms, but the application-side merge cost is a future optimization target.

## Regression budgets

Use these provisional guardrails:

- The first provider batch should remain below 100 ms on the target machine.
- A 100,000-entry directory must remain scrollable while enumeration continues.
- UI batch rendering should not routinely exceed one 16.7 ms frame.
- Navigation away from a loading fixture must cancel its request without stale rows appearing.
- Long-term peak memory remains targeted below 150 MiB for the 100,000-entry fixture.

Record hardware, build profile, GTK version, cache state, and notable environmental load whenever replacing the baseline table. Use multiple samples before treating small timing differences as regressions.
