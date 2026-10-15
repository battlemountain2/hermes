# Preview formats, speed and upstream review

Audit: October 12, 2026 (America/Denver). Hermes remains **0.2.0**; upstream releases
do not change Hermes's version, license, design choices or music-player behavior.

## What changes in this pass

### Heavy photo/video follow-up — October13

Large supported JPEG and standard8-bit HEIF native-fallback selections can use installed optional
libvips inside the sandbox. Fixed arguments, seekable stdin, two threads and no upscaling; absent/
failed tools or invalid output use ImageMagick. No host decoded-frame budget increase or mandatory
dependency. HEIF HDR/high precision/unsupported NCLX, multi-image and potentially adequate embedded
thumbnail cases retain the fallback to protect primary-image behavior. The original1400px bounding
box and EXIF orientation remain; resampling filters differ. [libvips thumbnailing](https://www.libvips.org/API/current/using-vipsthumbnail.html).

Video display-rate dropping now runs before rotation/scale, and scaling precedes RGBA conversion.
This avoids transforming discarded frames and full-resolution color conversion before downsizing.
Full-length playback/audio, rotation, seek epochs,1280x720/30fps and isolation remain; codec reference
frames still decode. A fused videoconvertscale alternative was measured but slower on this host and
was not shipped. [GStreamer video conversion/scaling](https://gstreamer.freedesktop.org/documentation/videoconvertscale/videoconvertscale.html).

Generated12MP warm-filesystem/debug samples: JPEG64–91ms vs81–105ms after optional-tool startup;
first scaled calls140–154ms vs84–127ms were sometimes slower. HEIC159–202ms vs212–265ms. This is not
a promise that every first selection speeds up. All-eight-EXIF, output dimensions, representative
color/detail differences and actual pool/one-shot/missing-tool routes are checked, not a complete
camera/ICC corpus. The4K60 no-clock processing probe drained in0.78–0.79s vs1.22–1.34s with less CPU;
this is not real-time FPS. Private native first frames after registry warming149–173ms, with portrait,
HEVC, VFR, seek/end and color-bar checks passing. See the latest performance baseline for details.

Latest validation:597 normal passed/88 optional ignored;12 targeted optional tests explicitly pass,
all-target/debug/release and formatting/diff checks pass. Logs /tmp/hermes-heavy-preview-qa and
/tmp/hermes-preview-audit.J7eOla. No new types, duration cutoff, device permissions, install/version
changes, personal media/config edits or user app restart. Previous full format matrix not regenerated.

### Latest follow-up: visible-first work and duplicate sharing

Thumbnail source resolution/cache/render work now has bounded admission (at most four active
jobs), visible-first ordering and a half-viewport nearby prefetch band. Far-offscreen and hidden
tabs wait; mapping or scrolling makes them eligible. Popovers use their own clipping boundary.
Completion/map/unmap wake promptly; mapped visibility is sampled every32ms and all-unmapped
waiting queues stop polling. Existing folder-nonempty and desktop icon checks are unchanged.

Matching resolved requests share an in-flight render only with equal original/artwork identities,
operation, kind and logical size. Subscribers retain weak images and their own cancellation and
request IDs. Removing/rebinding one cannot cancel or repaint its visible peer; canceled workers
retain admission slots until completion, and returning subscribers re-resolve the current source.
Persistent output is saved once per shared render. All metadata/disk/parser work stays off GTK;
selected-preview queues and cache budgets are unchanged. Different original tracks referencing
one common cover are not yet coalesced by this UI queue.

Private generated GTK observations: duplicate visible pair~70–100ms with one lookup/render/decode,
warm rebind~7.5–7.8ms with no decode; distant/hidden requests start only after scroll/tab activation.
These debug/warm-filesystem/headless observations are not cold-disk or physical paint guarantees.
Ordering, bounded admission, identity separation, per-subscriber cancellation, stale rebinding,
popover visibility, cache/media/zoom/layout and retained-tab lifecycles are covered by regressions.

### Follow-up: ghosting, persistent thumbnails and native photo paths

The user-supplied October 12 recording showed a brief old-image/position transition when
switching photos. Loading thumbnails now use the same image viewport/footer layout as the
full preview, fit on their first draw, and paint an opaque semantic theme background inside
the viewport. Old letterbox pixels cannot show through that surface. Source/request guards
remain; stale, failed or closed requests cannot replace the current image. Tiny original
photos are not enlarged just because they are loading. Headless GSK/Cairo and GTK regressions
pass; the user's actual compositor/pointer behavior still needs confirmation after restart.

Small thumbnails now persist under `$XDG_CACHE_HOME/strata/thumbnails-rgba-v1`. Disk I/O,
hashing and pruning run off GTK. The store is private (0700 directory, 0600 files), bounded
to 512 records / 128 MiB with an eight-item writer queue, and atomically replaces records.
Reads reject symlinks, FIFOs, malformed/checksum-invalid records and changed sources. Both
track and adjacent-cover identities include device/inode/size and nanosecond mtime/ctime.
Non-UTF-8 source paths work; unrelated Unicode cache names do not break pruning.

Cache files contain bounded raw sRGB RGBA (at most 256×256), not untrusted PNGs handed to a
native decoder. Validated pixels are encoded into a fresh PNG for the existing UI path.
This thumbnail color tier does not preserve HDR/wide-gamut precision; full originals keep
their existing path. Models/comic covers returning larger rasters are not persisted yet.
Busy/full writers skip optional saves, and abrupt exit may drop queued cache writes. The
store is not a shared Freedesktop cache and has no new settings/clear button in this pass.

Selected JPEG previews try sandboxed codec-scaled decoding for 8-bit three-component images
within the existing 32 MiB decoded-frame budget. Eight EXIF orientations, dimensions and
representative generated color samples match the previous renderer. Grayscale, CMYK,
high-precision, oversized and failed native decodes retain the fallback. Selected HEIF
previews decode the primary image, not its embedded thumbnail, for bounded 8-bit images
with supported NCLX primaries/transfer functions. Larger, HDR/wide-gamut or failed inputs
retain ImageMagick. ICC propagation remains; these generated tests are not an exhaustive
real-camera/colorspace corpus. Pooled and one-shot JPEG/HEIF routes are explicitly tested.

GStreamer initialization starts once on a background thread 1.2 seconds after the window
appears. It creates no playback session and does not autoplay. The registry scan cost still
exists; immediate first selections can still pay it. The existing isolated video worker can
reuse the resulting registry; this is not a replacement playback engine or a duration limit.

Fresh private debug samples (generated files, warm filesystem, not physical paint):

| Workload | Follow-up observation |
| --- | --- |
| Disk thumbnail after clearing RAM | ~1.03ms; downloaded RGBA matches the original thumbnail |
| Cached photo with full provider deliberately pending | ~9.9ms; same viewport height before/after replacement |
| Generated 2400×1600 JPEG, eight orientations | Native ~41–55ms versus previous ~65–76ms |
| Generated rotated 1600×1200 HEIF primary | Native ~40–55ms versus previous ~68–74ms |
| Fresh private GStreamer registry warmup | ~4.09s in background, then first HEVC frame ~100ms |

The large textured 12 MP HEIC/JPEG cases below are different inputs and often use fallback;
do not compare their times as an overall speedup. No Dolphin parity or universal lag fix is claimed.
An additional sandbox-routing rerun under greater host load measured JPEG ~72–191ms versus
~161–213ms, and HEIF ~63–108ms versus ~211–276ms. All quality/routing assertions still passed;
the variation is why these small engineering samples are not a universal latency promise.

### Initial format/progressive-loading pass

- A selected photo can show its existing, identity-validated thumbnail while the full
  preview loads. This is temporary, not a reduced-quality replacement. The full image
  retains the existing orientation, resolution limits, zoom/pan and fullscreen controls.
  GIFs keep their animation path. No thumbnail exists on a genuinely cold selection,
  so that case still waits for decoding.
- Video startup reuses a valid cached poster before starting another poster job. A real
  decoded frame supersedes it; stopping, errors and changed sources cannot restore an
  old poster. The delayed fallback remains for misses; playback stays full length.
- Encoded worker results survive five minutes rather than thirty seconds, within the
  same 64-entry and byte budgets (16 MiB thumbnail pool, 64 MiB preview pool). Every hit
  still validates device/inode, size and nanosecond mtime/ctime. This is RAM reuse, not
  persistence across app restarts.
- Pooled PDF thumbnails now render at a 256px bounding box instead of rendering a full
  selected page. Selected pages, text layers and PDF fullscreen are unchanged.
- AVIF, SVG and JPEG aliases JPE/JFIF now get thumbnail requests. JPEG aliases also
  receive the image classification used by file-type filtering and search.
- Model and ebook/comic requests carry their format through descriptor transport:
  3MF is no longer mistaken for STL, and EPUB/CBZ/CBR no longer depend on an extension
  that `/proc/self/fd/N` cannot supply. CBR subprocesses receive seekable stdin.
- One-shot helpers receive only a short sanitized format suffix (`/input.docx`), not
  the original basename or browsing directory. DOCX/ODT parsing therefore works when
  the one-shot route is needed, without broadening mounts or resource limits.
- Document/archive preview requests now supply their text limit, not the PDF page
  number. This also fixes successful-but-blank office previews and one-byte archive
  listings. One-shot model/cover fallbacks receive their proper selected-preview size.

## Coverage and limits

The complete filename registry was inspected: **121 extension rules**, four special
text filenames, and **60 thumbnail extension rules** after these changes. The Dolphin
comparison examined 186 filename/MIME lookups and the installed provider inventory.
Those are dispatch checks, not successful decoding of every possible encoding.

| Family | Hermes behavior | Verification / remaining boundary |
| --- | --- | --- |
| PNG, JPEG/JPE/JFIF, WebP, BMP, AVIF | Sandboxed bounded raster preview and thumbnail | Generated representative files and malformed variants; not every colorspace or camera variant |
| HEIC/HEIF | Embedded-thumbnail path where available; separate bounded full preview | Generated HEVC-encoded HEIC, including textured 4000×3000 image; full decode remains expensive |
| GIF | Static browser thumbnail; loopable animation preview | Generated animation/corruption/cancellation; animated WebP/APNG are not equivalent supported animation paths |
| TIFF / GeoTIFF | Raster thumbnail; selected geospatial image/metadata path | Generated ordinary TIFF; specialized geospatial metadata has existing tests |
| SVG | Rasterized preview and new thumbnail; source text can be searched | Generated SVG; still uses the existing image renderer, not upstream's native resvg path; SVGZ not enabled |
| Camera RAW (23 extensions) | Dedicated embedded-thumbnail attempt; selected preview uses generic image fallback | Registry reviewed; no representative real RAW corpus was available for this audit |
| MP4/MOV and other video extensions | Poster thumbnail plus native full-length playback | Generated H.264/HEVC MP4/MOV, real decoded frames, seek/fullscreen; a streaming-path Ready event alone is not decoder success |
| FLAC/MP3/OGG/Opus/M4A/AAC/WAV/WMA | Metadata preview, native playback and embedded/adjacent artwork when present | All eight generated metadata formats; embedded MP3 cover; a bare audio file without art correctly uses an icon |
| PDF | 256px first-page thumbnail, full selected page and text layer | Generated single/multipage, corruption, full-size/text preservation; not every encrypted/complex PDF |
| DOCX/ODT | Bounded extracted text | Generated files through real adapter and one-shot sandbox; not formatted office-document rendering |
| XLS/XLSX/ODS | Bounded spreadsheet table, first sheet | Generated XLSX plus existing table tests; no new promise for every workbook; descriptor XLSX sniffing already worked |
| EPUB/CBZ/CBR | Package cover / first suitable comic image | Real pooled generated EPUB/CBZ; CBR dispatch/stdin hardened, but real RAR fixture test was not run because a RAR writer is unavailable |
| STL/3MF | Bounded model raster or embedded model thumbnail | Generated descriptor, pooled and malformed fixtures; FreeCAD/OBJ are not enabled |
| ZIP/TAR and compressed archives | Bounded listing; file operations remain separate | Generated ZIP/TAR/TAR.GZ and malformed archives; not automatic image thumbnails for every archive |
| Text/source/config/markup | Bounded source/text preview and optional content search | Includes Markdown/HTML/CSV as source, not newly rendered rich documents |
| Desktop entries / unknown | Existing application icon handling, text when detected / unsupported fallback | Generated inert desktop entry never launched; PPTX, AIFF/APE, DjVu and other extra providers were not added |

Non-local binary preview limitations, parser deadlines, output/decoded-pixel ceilings
and cancellation remain. Native GStreamer decoding uses a separate isolated worker,
distinct from the image/document worker. Host-side audio output and registry initialization
are not the original-media decoder.

## Measured behavior

All inputs were generated in private temporary directories with isolated HOME/XDG and
session buses. Runs used the actual debug adapter, built sandbox helper and native
playback. Filesystem caches were warm; these are **not cold-disk, physical compositor
paint or real-user-library benchmarks**. Three per-format requests separate first
request from cache hits. CPU load and codec initialization affect comparisons.

| Workload | Representative observation |
| --- | --- |
| Cached photo shown while full provider is deliberately pending | ~4.4ms in the private GTK regression; a state/display-readiness test, not universal decoding time |
| Textured 4000×3000 JPEG, selected 1400×1050 raster | Repeated final runs ~148–272ms first request, ~0.5–1.4ms encoded-cache revisits |
| Textured HEIC, selected 1400×1050 raster | Repeated final runs ~739–1128ms first request, ~0.5–2.7ms encoded-cache revisits |
| H.264 / HEVC MOV poster, 256×144 | Repeated final runs ~38–66ms / ~51–102ms first request; encoded-cache calls below 0.1ms |
| Native HEVC decoded frame | Fresh private GStreamer registry ~6.86s; subsequent samples ~145–147ms; unrelated to the fast streaming-path event |
| Revisit after 31 seconds | Cache result remains reusable; original thirty-second expiry caused unnecessary decoding |
| Generated PDF thumbnail | Final runs 198×256 in ~11–15ms, versus full-page thumbnail work before the fix |

These numbers do not establish an overall percentage improvement or Dolphin parity.
The heavyweight first HEIC decode and first-use video registry scan are **not solved**
by showing a cached thumbnail/poster.

## Lessons from Dolphin

Dolphin uses MIME-specific providers through KIO; thumbnails, information-panel
metadata and actual playback are different workloads. KIO exposes scaled-and-cached
previews and provider discovery rather than one decoder for everything.
[KIO PreviewJob](https://api.kde.org/kio-previewjob.html).

Source inspection highlights visible-first scheduling and asynchronous cache reuse.
JPEG-specific decoding is worth evaluating instead of full decode followed by scaling.
[Dolphin roles updater](https://invent.kde.org/system/dolphin/-/blob/v26.08.2/src/kitemviews/kfileitemmodelrolesupdater.cpp),
[KIO cache implementation](https://invent.kde.org/frameworks/kio/-/blob/v6.30.0/src/widgets/previewjob.cpp),
[JPEG thumbnail provider](https://invent.kde.org/network/kio-extras/-/blob/v26.08.2/thumbnail/jpegcreator.cpp).

Private comparisons used installed Gear 26.08.2 / Frameworks 6.30.0. A fresh-process
KIO JPEG disk-cache hit took ~6ms in one fixture; Hermes at that initial comparison retained
only RAM thumbnails. The follow-up now adds private persistent reuse. Equal-size JPEG renderer tests favored KIO on the
synthetic input, but changed quality characteristics prevent a blanket speed/quality
claim. Keep Hermes's stronger source identity checks rather than copying coarse
mtime-only invalidation.

## Recent Strata work worth evaluating

Upstream released **1.0.0 on October 12**. Relevant changes include icon-cache LRU,
precomputed sort keys, less large-selection/drag overhead, configurable search
exclusions, per-folder view preferences, and document selection/find. Review these
selectively; do not replace Hermes's compact square tabs, retained views, music
collections, full-length playback or user-selected defaults.
[Strata 1.0 release](https://github.com/lgse/strata/releases/tag/v1.0.0).

The preview architecture also warrants separate review: native SVG rendering,
persistent thumbnail reuse and stronger per-job isolation. Hermes already has pools;
blindly importing another pool is not a performance plan. The release source/docs
were checked at `0a086aa3bdb251842a1e8413e2a6b35970d0b409`; earlier detailed inspection
used RC1 `0049bbb9a13de3119e871822981a4a895a7e0740`, not a complete latest-tree audit.
[Pinned preview architecture](https://github.com/lgse/strata/blob/0a086aa3bdb251842a1e8413e2a6b35970d0b409/docs/preview-sandbox.md).

## Next work, in priority order

1. Split model/cover thumbnail size from the full selected-preview operation; investigate
   sharing common album covers across different original tracks. Matching UI thumbnail
   requests now coalesce and mapped/visible entries have priority.
2. Expand first-load photo profiling to large real-camera/ICC/HDR inputs, preserving quality,
   safety and fallback behavior. Current bounded native paths do not solve every HEIC/JPEG.
3. Profile video initialization under contention and immediate first selection; idle warmup
   moves registry cost rather than eliminating it. Keep full-length playback and no transcoding.
4. Selective upstream safety/performance backports, search exclusions, per-folder preferences,
   then document find, richer RAW/SVG
   handling and optional file types backed by fixtures, not just extension registration.

Separate outstanding product items include playlist/favorite continuity across renamed
files, real-session UI/audio confirmation and historical unrelated stress-test failures.
Gapless and detailed column sizing remain deferred; Save queue as playlist is unwanted.

## Validation and artifacts

Latest scheduling follow-up: **595 passed, 85 optional tests ignored**; all-target/debug/release
and scoped formatting/diff checks pass. Fourteen distinct targeted optional tests passed for
shared/rebound/visible/hidden thumbnails, cache/photo/poster/zoom/artwork, selected-preview
contention, slider/details, hover peeks, discarded presentations and retained tabs/transport.
Logs: `/tmp/hermes-thumbnail-scheduling-qa` and `/tmp/hermes-preview-audit.4h33ra/scheduling-*.log`.
The original full format matrix was not regenerated; no new file type or playback cutoff added.

Earlier persistent/native follow-up: **588 passed, 83 optional tests ignored**; all-target checking and
debug/release builds pass. Ten targeted optional tests were run privately, including GSK/Cairo
letterbox clearing, stable loading layout/stale-request guards, persistent real GTK thumbnails,
FIFO rejection, native JPEG/HEIF quality, idle warmup, image zoom and full-length video seek/
fullscreen. The two native tests were rerun with additional real pooled and one-shot sandbox
assertions. Existing warnings and historical unrelated integration failures remain unaddressed.

The initial pass's explicit private tests
cover pooled 3MF/EPUB/CBZ/PDF, one-shot office formats/text limits, cached photo/poster
replacement and errors, native MOV startup/seek, full-length fullscreen and image zoom.
The generated matrix covers 60 fixtures / 321 measurements, including corrupted and
over-budget files; expected rejection or unsupported output is not a success claim.
Separate checks cover cancellation, source replacement and revisit/contention.

Latest follow-up logs: `/tmp/hermes-preview-resume-qa` and `/tmp/hermes-preview-audit.4h33ra`.
Earlier all-target checking and debug/release builds were recorded alongside logs. Detailed
research/fixtures: `/tmp/hermes-dolphin-audit.wTdETK`,
`/tmp/hermes-preview-audit.q2u1x6`, `/tmp/strata-readonly-audit.IwFi4L`.
The earlier temporary directories disappeared between interrupted sessions; they are not durable
test assets or user libraries. The earlier 60-fixture matrix was not regenerated in this follow-up.
No personal
media/configuration changes, user-app restart, commit, branch or PR were made.
