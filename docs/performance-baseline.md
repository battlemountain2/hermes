# Performance Baseline

## Folder transitions and selected music artwork — 2026-10-20

Columns, Grid and Explorer share a150ms loading-feedback grace period. Quick loads no longer
crossfade through a skeleton/spinner; pending rows are non-actionable, and delayed/error/empty
states remain explicit. Rapid navigation, canceled timers, destroyed panes, stale batches and
view changes retain generation guards. This reduces transition flashes; it is not a measured
cold-directory speedup or a promise that every compositor paints without flicker.

Selected immersive covers no longer use the fixed256px thumbnail operation. Adjacent artwork
uses the existing1400px image-preview bound; embedded artwork has a separate1024px selected
worker operation. A generated1400px embedded cover proves256px thumbnail versus1024px selected
output through the real sandbox. No decoder/resource budget is raised. Low-resolution sources
still have their original limits, and high-resolution cold decoding still costs time.

Palette analysis and64px softened-backdrop preparation run off GTK once per selected source.
The background is two retained native nodes (base color and an opacity-wrapped texture), with
no per-frame full-screen Cairo aura, live blur or additional animation timer. Cover-derived hue
retains theme-background luminance, with adjusted visualizer colors for extreme covers. Control
opacity changes reserve layout space; keyboard-focus and stable bounds are checked. These are
structural improvements, not a newly measured GPU/onscreen-FPS or audio-latency claim.

Validation:927 regular cases across926 serial cases plus the known process-global GLib thumbnail
case in a separate process;187 optional cases remain excluded from that normal run. All21 exact
isolated GTK/sandbox cases pass: folder transitions, tag hydration, mini Escape/close/icon states,
live theme contrast, lyrics mode/identity/cancellation, metadata-only retention, native backdrop
nodes, selected artwork resolution and real one-shot DOCX/ODT/PPTX extraction. Generated Office
unit cases also cover hyperlink labels, relocated parts, implicit cells and allocation bounds.

Production build and all-target check pass; the test target retains five old unfulfilled lint
expectations. Artifacts: /tmp/hermes-polish-path. A Graphite control snapshot was inspected;
the hidden Broadway display did not reliably deliver later repaint screenshots, so light/dark
coverage is computed style/palette and lifecycle coverage, not a full visual/compositor sign-off.
Actual pointer dragging/fades, user Office documents, cold media and live audio remain user checks.
No user media/tags/settings were changed, no online lyrics request was sent and no user window
was restarted, focused, moved or closed. One reported FLAC was probed read-only to confirm its
title/artist/album tags exist; generated fixtures exercise the actual hydration/sandbox path.

## Prism / Orbit and mode-switch polish — 2026-10-19

The user still reported Tunnel/Cosmic stutter and unreliable returns to Lyrics after the native
snapshot migration below. Native nodes alone were not sufficient: the old elapsed-since-render
limiter produced about48updates/second on144Hz displays, and raw audio bins moved geometry abruptly.
The new deadline scheduler maintains approximately60updates across60/144/240/1000Hz synthetic
clocks without catch-up bursts; time-based attack/release smooths the32 bins. Hidden/paused/
motion-disabled modes still stop ticks, and leaving Visualizer clears its retained frame.

Prism uses six nested rectangular frames (24 strokes). Orbit uses two sparse arcs,12 stable stars,
six short trails and16 grouped spectrum columns. Audio changes brightness/size, not star positions.
Both avoid the old large translucent ring/particle paint regions, full-screen blur and new timers.
Lyrics/Visualizer buttons synchronize modes, missing lyrics keep stable panel geometry, unchanged
empty states reuse widgets, and active lyrics no longer animate font dimensions or weight.

Initial isolated Vulkan offscreen render-plus-readback medians at3840×2160: Prism5.70ms and
Orbit6.15ms, with CPU snapshot recording around0.04/0.07ms. Ribbons5.81ms. These debug measurements
include synchronization/download and do NOT measure onscreen FPS or hardware-decoder activity.
The old Cairo comparator draws the old designs, so it is not a like-for-like speedup comparison.
After visual inspection, long crossing Orbit links were shortened to trails; final profiling
artifacts, generated PNGs and validation are recorded in NEXT_AGENT.md. No user's running window
was presented, moved or restarted; the renderer fixture realizes an explicitly unmapped window.

Final sparse-design3840×2160 medians: Vulkan render-plus-readback Prism5.10ms/Orbit5.21ms;
explicit Cairo fallback1.27ms/1.01ms. GPU synchronization/readback can dominate this synthetic test,
so these numbers are not a reason to force a backend or a claim that GPU rendering always wins.
Production keeps GTK's automatic selection. Both fixture paths pass, including bounded native
geometry and normal widget disposal; actual onscreen cadence still needs user verification.

## Native immersive scene rendering — 2026-10-18

Ribbons, Tunnel and Cosmic now snapshot bounded native GTK4.12 GSK color, transform, border and
rounded-clip nodes instead of repainting a viewport-sized Cairo surface every frame. GTK keeps
its normal Vulkan/GL/software renderer selection; no forced backend, unsafe code, dependency,
custom GLArea or shader was added. The static ambient aura remains cached Cairo. Native frames
contain no Cairo nodes, texture uploads/downloads, timers, I/O or spectrum analysis. Animation
only runs when mapped, enabled, motion-enabled and playing, capped near60fps; pause keeps a
static frame and suspension clears it. Theme/style/size changes still redraw correctly.

An initial CPU-only debug probe (reused Cairo image surface,20 frames,3 warmup frames excluded)
found4K Tunnel drawing~4.535ms median versus~0.220ms for the1024-sample spectrum analysis. That
motivated examining render/upload cost rather than replacing the analyzer or audio pipeline.

Paired offscreen probe:15 frames per style/size/path, first3 excluded, same synthetic32-band input,
palette and time steps, debug build. Measures snapshot construction plus render_texture and a
completed TextureDownloader readback. Final native Wayland run selected GskVulkanRenderer without
GSK_RENDERER overrides. The window was realized solely to obtain a surface, never presented or
mapped; assertions enforce this. No personal media/audio was opened or played.

| Style,3840x2160 | Cairo-node median / p95 | Native GSK median / p95 |
| --- | --- | --- |
| Ribbons |31.092 /33.322ms |8.031 /8.648ms |
| Tunnel |35.260 /36.338ms |8.035 /10.160ms |
| Cosmic |29.524 /32.022ms |7.383 /8.328ms |

These include synchronization/readback and are NOT pure GPU timestamps, actual onscreen frame
times, guaranteed FPS or a universal speedup. DRM engine counters were unavailable. Native node
construction remains CPU work: final4K Ribbons~0.276ms, Tunnel~0.037ms and Cosmic~0.038ms median.
Ribbons use at most128 segments per curve to bound scene complexity; this is a polyline
approximation, not pixel-identical Cairo antialiasing. Synthetic side-by-side PNGs were inspected.

Explicit GskCairoRenderer/Broadway fallback passed the same comparison. A representative fallback
run at4K had Ribbons2.588→2.523ms, Tunnel5.757→5.600ms and Cosmic0.555→0.703ms median. Some small
software-rendered cases are slightly slower (1080p Ribbons0.971→1.354ms); do not promise a CPU
fallback speedup. Output stays bounded, clipped and theme-driven on all paths.

Separately, library hero speaker/repeat icons now update only on state changes instead of being
recreated every100ms, including while hidden behind immersive. Hidden waveform avoids redraw.
827 regular tests pass;137 optional ignored. Seventeen focused isolated GTK checks pass, including
bounded native nodes, invalid input, live palette, clear/reuse/allocation/drop and transport icon
identity. Synthetic profile tests are optional/ignored and run explicitly, never on app startup.
Production and all-target checks pass (existing test-only lint-expectation warnings remain).

Tests: src/ui/music/immersive/performance_tests.rs and canvas/tests.rs. Logs/screenshots are in
the directory recorded by /tmp/hermes-render-path: cairo-baseline.log, native-comparison.log
(prototype), native-final.log (final), software-comparison.log, software-final.log, unit-final.log
and ui-final.log.
Private HOME/XDG/DBus and a private hidden Broadway connection isolate these from the user's
running Hermes/preferences. Real compositor presentation/frame pacing and live audio under load
remain user-verification gates; neither an application restart nor renderer setting was forced.

## Heavy first-load photo/video follow-up — 2026-10-13

Eligible large JPEG/standard8-bit HEIF selected previews now try installed optional libvips inside
the existing sandbox before ImageMagick. Input is seekable stdin, not an exposed original path.
Two processing threads, operation cache0, downsize-only1400px output, encoded input128MiB and
existing source/output ceilings. Direct host decoded-frame budgets are unchanged. Missing tools,
failed/invalid output, HDR/high precision/unsupported NCLX or uncertain HEIF primary/thumbnail
selection retain fallback. No Rust dependency or automatic package installation was added.

Generated12MP debug/warm-filesystem samples: JPEG scaled~64–91ms vs~81–105ms after tool startup;
first scaled calls~140–154ms vs~84–127ms were sometimes slower. HEIC scaled~159–202ms vs~212–265ms.
Optional process/plugin startup still costs time: this is not a universal first-file speedup.
1400x1050 output and all-eight-EXIF rotations checked; representative meanRGB differences0.34
JPEG/0.046 HEIC against prior output. Different resamplers are not bit-identical or exhaustive
ICC/HDR/camera tests. Actual pooled/one-shot rendering and missing-tool fallback are verified.

Video now drops excess display-rate frames before transforms, then scales before RGB conversion.
Same generated4K60 two-second clip, no-clock fakesink: early-order drain~0.78–0.79s vs~1.22–1.34s,
user CPU~1.08–1.10s vs~1.98–2.02s. A fused conversion/scaling alternative was slower and rejected.
These are processing/CPU samples, not playback FPS or cold startup. Full-length playback, audio,
1280x720/30fps limits, rotation and seek epochs remain. Codec reference frames still decode.

Native first frames after warming the private registry~149–173ms across4K H.264, rotated MOV,
HEVC and VFR; seek/end pass and rotated output405x720. Generated4K color bars have meanRGBA
difference0.24 with opaque alpha. Real desktop rendering and first registry costs remain separate.
The first portrait fixture lacked rotation because an older ffmpeg metadata option was ignored;
the corrected display_rotation fixture has a verified90-degree matrix and passes. No renderer
rotation defect was inferred from that failed fixture.

Validation:597 normal pass,88 optional ignored;12 distinct targeted optional tests run explicitly.
All-target/debug/release/scoped formatting/diff checks pass. Logs /tmp/hermes-heavy-preview-qa and
/tmp/hermes-preview-audit.J7eOla. No personal media/config changes, app restart, version bump or
branch/commit/PR. Existing warnings and unrelated broad stress failures are outside this pass.
No cold-disk/physical-pointer/Dolphin parity claim; ask for real remaining slow camera samples.

## Visible-first shared thumbnail jobs — 2026-10-12

Thumbnail source resolution, cache lookup and rendering now enter one bounded UI-owned queue
instead of each bound image starting its own independent background pipeline. Admission uses
the default sandbox worker limit (at most four); canceled workers keep their slots until they
actually complete. Visible targets rank before a half-viewport prefetch band. Far-offscreen and
unmapped/hidden-tab requests wait. Popovers form their own clipping boundary. Map/unmap/completion
wake the queue immediately; mapped pending visibility is sampled every32ms for scrolling, with
no polling when all waiting targets are unmapped. Folder-nonempty and desktop icon checks are
unchanged and are not part of this admission limit.

Equal resolved requests share one render only when original and artwork identities, operation,
kind and logical size match. Weak subscribers retain individual cancellation/request guards.
Rebinding one leaves a visible peer working; disappearing eligible subscribers cancel the shared
job. Returning subscribers re-resolve the source rather than reviving an old result. All metadata,
source validation, disk I/O and parsing stay off GTK; cache budgets and selected-preview workers
are unchanged. Shared results save to the optional persistent cache once, not once per widget.
This does not yet coalesce different tracks that happen to reference the same cover file.

A generated private GTK fixture showed two visible images for one source after~70–100ms with one
lookup/render/decode; warm rebind samples took~7.5–7.8ms without decoding. Distant and hidden-tab images did
no work until scrolling/showing them; final total was three original decodes for three sources.
These debug, warm-filesystem/Broadway observations include window/helper initialization and are
not a before/after speed percentage, physical paint benchmark or Dolphin parity. A separate shared
render/rebind test proves the old result cannot overwrite the rebound image or cancel its peer.

595 normal tests pass,85 optional ignored by default; fourteen distinct targeted optional tests
pass for the new behavior and existing cache/media/zoom/artwork/layout/peek/tab lifecycles. A pure
1,000-job admission test verifies the four-slot bound even while a canceled job has not returned.
The selected-photo-under-occupied-thumbnail-pool fixture remains independent (~136ms in one run).
All-target checking, debug/release builds, scoped formatting and diff checks pass. Existing warnings
and historical unrelated integration failures remain. Logs: /tmp/hermes-thumbnail-scheduling-qa
and /tmp/hermes-preview-audit.4h33ra/scheduling-*.log. No personal media/config changes, app restart,
version bump, branch/commit/PR. Heavy first-load photo/video, model/cover tiers and actual desktop
confirmation remain; avoiding unnecessary work is not a claim to accelerate every individual codec.

## Ghost-free transitions and persistent media thumbnails — 2026-10-12 follow-up

Loading thumbnails and full photos now share the image viewport/footer layout, fit on the first
draw and paint an opaque semantic background within the viewport. This prevents old letterbox
pixels showing through. GSK/Cairo, layout, request-lifecycle and zoom/fullscreen regressions pass;
actual user-session compositor/pointer confirmation is still needed for the supplied recording.

A private asynchronous raw-RGBA thumbnail store survives app restarts, with strong file/cover
identity checks, bounded validation and atomic writes: 256px raster limit, 512 records / 128 MiB,
eight queued saves, private directory/files. Corrupt, changed, symlinked or FIFO records miss safely.
Cache reads never send attacker-controlled encoded cache bytes directly to native image parsers.
Larger model/comic rasters are not persisted; optional pending writes can be dropped on exit.

Sandboxed selected JPEG/HEIF previews now have bounded native paths with the original fallback.
JPEG supports RGB 8-bit within the decoded-frame budget; HEIF decodes the primary image for
supported 8-bit/NCLX profiles. Grayscale/CMYK/high precision, large and HDR/wide-gamut cases still
need the fallback. Generated orientation/dimension/color checks and real pooled/one-shot routes
pass. These are not exhaustive camera/ICC tests or a change to original selected-image quality limits.

Fresh debug/private Broadway samples on this host, warm filesystem: persisted thumbnail reload
~1.03ms after RAM clearing; deliberately pending cached-photo display ~9.9ms; generated JPEG
eight orientations ~41–55ms native versus ~65–76ms previous; rotated HEIF primary ~40–55ms
versus ~68–74ms. These small generated inputs differ from the large textured fixtures below.
The additional sandbox-routing rerun under higher load varied to JPEG ~72–191ms versus
~161–213ms, HEIF ~63–108ms versus ~211–276ms; all quality/routing assertions still passed.

Media initialization starts once off-thread 1.2 seconds after window presentation, without
autoplay. A fresh private registry warmup took ~4.09s, followed by a first HEVC frame in ~100ms.
The scan still costs time/CPU and immediate selection may pay it. Native video decoding is already
isolated in its own worker; host initialization/audio output are distinct from user-media decoding.

Validation: 588 normal tests pass, 83 optional ignored by default; ten targeted private optional
tests pass, with the two native tests additionally rerun for actual pooled/one-shot routing.
All-target check and debug/release builds pass. Existing warnings remain; unrelated broad stress
tests were not rerun. Logs: /tmp/hermes-preview-resume-qa and /tmp/hermes-preview-audit.4h33ra.
Earlier temporary logs vanished between sessions; the earlier 60-format matrix was not regenerated.
Only the explicitly supplied recording was inspected; no personal media/config changes or app
restart, version bump, branch, commit or PR. Next: visible-first/in-flight work coalescing, distinct
model/cover tiers, heavy first-load photo/video profiling and user-session ghosting confirmation.

## Preview format audit and progressive loading — 2026-10-12

See [the format/performance audit](preview-format-audit.md) for all registry families,
representative generated fixtures, Dolphin/KIO comparisons and the latest Strata review.
Upstream 1.0 was released today; Hermes intentionally stays at 0.2.0.

Selected photos reuse an existing source-validated thumbnail while the full preview loads.
Videos reuse cached posters and skip redundant fallback rendering when a real frame or valid
poster is already present. Stale sources, errors, cancellation and completed full previews
cannot receive late placeholder results. No thumbnail means the cold decode still runs.
The private GTK photo fixture displayed a cached image in ~4.4ms while its full provider
was deliberately pending; this does not measure full-resolution decoding or physical paint.

Worker-cache expiry increases from 30 to 300 seconds with unchanged entry/byte limits and
strong identity checks; a real 31-second revisit remains cached. There is no new persistent
disk cache. Pooled PDF thumbnails use a 256px bounding box instead of full-page rendering;
selected full-size PDF/text layers remain. AVIF/SVG/JPE/JFIF get thumbnail requests.

Format transport fixes cover typed model/ebook/comic operations and sanitized one-shot input
suffixes. An additional adapter bug supplied the PDF page number as the office/archive text
limit: previews now receive the actual bounded text budget, avoiding blank office results and
one-byte archive listings. XLSX descriptor detection already worked and was not rewritten.

Repeated final debug adapter runs: textured JPEG ~148–272ms first / ~0.5–1.4ms cache hits;
textured HEIC ~739–1128ms first / ~0.5–2.7ms cache hits; MOV posters ~38–66ms H.264 / ~51–102ms HEVC.
Earlier native HEVC probes measured ~6.86s with a fresh private GStreamer registry, then
~145–147ms decoded frames. These generated, warm-filesystem timings do not prove cold-disk
improvements or Dolphin parity. First-use video initialization and heavy HEIC decode remain.

Validation: 579 normal binary tests pass (76 optional ignored), targeted generated pooled
3MF/EPUB/CBZ/PDF and one-shot DOCX/ODT pass, plus private GTK photo/poster guards, MOV startup/
seek, full-length fullscreen and image zoom. Generated matrix: 60 fixtures, 321 measurements;
malformed/over-budget and unsupported files are expected rejections, not universal successes.
Cancellation/source replacement/revisit/contention probes pass. Real CBR/RAR generation is
unavailable, so its optional fixture is not counted as tested. All-target check and debug/
release build logs: /tmp/hermes-preview-final-*.log; generated logs: /tmp/hermes-preview-audit.q2u1x6.
Existing warnings and historical unrelated broad integration failures remain; no personal
media/config edits, user-app restart or commit/branch/PR.

## Clipboard, volume and music-collection stability — 2026-10-11

Volume changes no longer capture the entire queue and refresh every Music view for each slider
event. The existing 100ms tick coalesces notifications, persistence waits 250ms after the latest
motion, and graceful shutdown captures the final value. Native gain/mute property writes run
off GTK with atomic newest state and one bounded wakeup, preventing a stalled audio-property
writer from blocking slider input. Startup reapplies the latest settings; gain is set before
unmuting. This is a bounded-work improvement, not a measured production frame-rate claim.

Music collections persist on a bounded background worker, with atomic private writes, retry/backoff
and explicit graceful-shutdown flush. Independent processes cannot simultaneously write the same
store; external byte changes/corruption/symlinks are preserved rather than silently overwritten.
Playlist and group models retain unchanged content, including on favorite changes and save
acknowledgments; drag versions change with membership, not global storage revisions. Rejected
catalog observations coalesce and retry in bounded batches without changing first-seen dates.
Path-based membership intentionally does not infer renamed files. Abrupt process termination and
noncooperating external edits racing the final rename are not covered by the shutdown guarantees.

Clipboard feedback uses a compact count badge and explicit item-list activation, avoiding the
previous simultaneous hover popover and tooltip. Explicit URI/KDE-URI export is checked against
actual Dolphin for single/multiple generated folders, nested contents and empty folders under
isolated X11 and native Wayland sessions. The old FileList baseline also worked privately; the
user's original folder failure was not reproduced, and portal involvement remains unconfirmed.
Photo drawer/fullscreen menus capture the preview file and release on close/replacement without
altering left-button zoom/pan.

Validation: all-target checking, debug/release builds and 565 normal unit tests pass; 62 helper/
desktop tests are ignored by default. Targeted private GTK checks cover the clipboard badge,
single/multi-selection URI export/import, cut outcomes, photo menus/fullscreen teardown, volume
multiview/MPRIS/latest-state synchronization, separate playlist editor/drag/model stability,
music context-menu snapshot targeting, meaningful main-player history/preview exclusion, and
image zoom/fullscreen. Store tests cover file-lock ownership in an independent process, external
edits/corruption, retry, bounds and immediate shutdown flush. Logs: /tmp/hermes-polish-qa.TPAYfn,
/tmp/hermes-directory-clipboard-qa, /tmp/hermes-volume-qa and /tmp/hermes-photo-menu-qa.KCbtF0.
Fixtures are generated and isolated; no personal media or user-app restart. Existing lint
expectations and local missing libadwaita stylesheet warnings remain unrelated. Historical broad
integration failures were not revisited. Version remains 0.2.0; no Dolphin-parity claim.

## Thumbnail isolation, pooled MOV posters and JPEG encoding — 2026-10-11

Thumbnail operations now use the thumbnail pool; selected photo/document previews use their
separate pool. Media thumbnails use pooled ffmpegthumbnailer workers with seekable input passed
through stdin and identity-checked caching, preserving representative 10%-position posters.
Cancellation while waiting for either pool does not spawn a fallback renderer. A generated fixture
holds every thumbnail lease while a selected JPEG preview still completes independently.

Generated 1080p MOV, 256px poster, debug production paths:

| Path | Fixture time |
| --- | ---: |
| One-shot, three samples | 80.4–80.7ms |
| First pooled request | 75–78ms |
| Cached request | 1.5–15µs |
| Selected JPEG preview while thumbnail pool is full | ~86ms |

The main benefit is reuse and avoiding thumbnail contention, not eliminating the first decode.
Replacement, cancellation, invalid media and decoded-pixel equivalence are covered. Embedded
MP3 artwork and adjacent-cover fallback also use these bounded paths; fallbacks validate both
track and cover identities and do not rewrite tags.

JPEG rendering now selects PNG compression strategy 2 (Huffman-only), retaining lossless decoded
pixels, output dimensions, orientation and existing limits. Three warm-filesystem shell renderer
samples on a generated textured 4000×3000 JPEG at 1400px: previous ~130–132ms versus ~105–107ms,
roughly 18–20% less renderer time. This fixture's PNG output was ~3.5MiB versus ~2.1MiB, with
ImageMagick absolute pixel error 0. Output size and gains depend on image content.

Native video decoding is unchanged. Isolated generated 720p H.264 MOV checks showed a first
GStreamer registry initialization of ~3.87s, then frames in ~106–127ms. First-use registry cost,
HEVC/4K, real storage and compositor behavior remain separate work. Logs/fixtures are under
`/tmp/hermes-cold-media-qa`; no personal media used. These are generated renderer/state timings,
not cold-disk, completed UI paint, physical-pointer FPS or Dolphin parity measurements.

## Preview sizing and bounded image zoom — 2026-10-11

Thumbnail-size changes now reach mounted Grid cards through their actual Overlay wrapper.
Preview splits no longer intentionally reserve 10% of unused space; Columns and Explorer
reserve their content, while Grid uses a stable half-workspace reservation. Explorer's initial
Name width is 320px and the details table reserves 684px including separators. Narrow windows
retain the preview minimum and allow horizontal Explorer scrolling; user divider drags remain.

Image pixels are converted to an immutable Cairo source once and reused during zoom/pan and
fullscreen, with a safe allocation fallback. Painting is bounded to the visible viewport instead
of a zoom-sized DrawingArea.
Virtual scroll extents still track zoom, but the painted canvas stays viewport-sized. Clear old
pixels, clip the canvas, ease pointer-anchored zoom and hand off projected offsets to panning
before adjustment/layout updates complete. Reduced motion keeps immediate scaling. Generated
Cairo pixels verify clearing; GTK fixtures cover rapid zoom/pan, canvas bounds, fit/100%/fullscreen,
thumbnail slider changes, detail-column visibility and split updates/manual sizing.

Logs: /tmp/hermes-refresh-qa. These are synthetic isolated GTK/state/pixel checks, with explicit
public frame phases for headless Broadway where needed. They do not establish physical-pointer
smoothness, production compositor painting, real HEIC/MOV/JPG cold-load speed or Dolphin parity.
Representative cold-media profiling remains after the approved library/playlist work.

## Tab corners, scrubbing and JPEG/MOV loading — 2026-10-10 follow-up

File-tab surfaces and selection buttons now have square corners; compact tab widths, control
alignment, top accents and drag ordering are unchanged. One shared seek interaction manages file
audio/video/fullscreen controls and the main music transport. Pointer/touch scrubs retain the thumb
while playback timestamps arrive, commit on release and coalesce outstanding targets. The scale
stays enabled during seeking. Keyboard/click changes remain immediate; paused streams stay paused.
Cancelled/unmapped controls and changed streams discard pending targets; programmatic time updates
do not trigger seeks. Deterministic GTK state-machine tests and native paused-music checks pass.
These tests do not inject physical mouse events; the user subsequently confirmed real thumb dragging
feels good. Follow-up tab work removes strip padding/centering and inter-tab gaps, not just corner
radius. Tabs stay compact horizontally while filling the strip vertically; the expanded GTK fixture
checks wide/narrow/one-tab layouts, adjacent boundaries and overflow without a connected browser.

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
