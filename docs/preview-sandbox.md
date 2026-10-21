# Preview sandbox

Strata treats files shown while browsing as untrusted. Native parsers do not receive the user's normal filesystem or network access.

## Sandboxed providers

The following providers run in a short-lived helper process:

- GDK Pixbuf image and camera RAW loaders;
- libheif HEIC/HEIF thumbnail decoding through safe Rust bindings;
- Poppler PDF thumbnail and page rendering;
- ImageMagick and `dcraw`/`dcraw_emu` RAW fallbacks;
- optional `vips` scaled JPEG/standard HEIF rendering inside the same sandbox;
- `ffmpegthumbnailer` media thumbnails;
- `bsdtar` archive listings and DOCX/ODT XML extraction; and
- Poppler PDF text extraction for opt-in content search.

Image previews are normalized to PNG by the helper. Full-length local video uses a separate persistent
GStreamer decoder process: the selected original is never passed to the GUI's native media parser.
Only strictly validated raw RGBA frames and finite/clipped F32 stereo PCM reach the GUI/audio output.
Plain-text previews remain in-process and are limited to 1 MB; they do not invoke a native format parser.

## Isolation and limits

HEIF thumbnails and full previews use distinct browser operations **20** and **21**, including
the existing identity-checked, byte-budgeted cache and changed-during-render/cancellation checks.
Thumbnail decoding considers at most 32 embedded thumbnail IDs, picks the smallest adequate
matching-aspect candidate, applies HEIF container transforms through libheif, validates RGBA
dimensions/stride, preserves an available ICC profile, and encodes a small PNG. Direct decoded
buffers stay within the existing 32 MiB frame budget; missing/inadequate/broken thumbnails or
larger primary images fall back to sandboxed ImageMagick. Full previews decode the primary image
at 1400 pixels through libheif for bounded 8-bit images with supported NCLX profiles; oversized,
high-bit-depth, wide-gamut/HDR or failed native decodes retain the ImageMagick fallback. Both the
pooled and one-shot routes use the same renderer. No HEIF native parser runs in GTK, and a thumbnail
cache hit cannot substitute its smaller image for a full preview. The build requires libheif >=1.17;
no embedded native library or unsafe Rust implementation was added to Hermes.

Selected JPEG previews can use codec-scaled decoding for bounded 8-bit three-component images,
with EXIF orientation and ICC propagation. Grayscale, CMYK, high-precision, oversized or failed
native decodes use the existing fallback. These original-file parsers stay inside the sandbox.

Eligible large JPEG and standard 8-bit HEIF selected previews now try the optional `vips` executable
before ImageMagick. Input is a seekable stdin descriptor, not the original basename or directory;
arguments are fixed, with two processing threads, operation caching disabled and no upscaling.
Encoded input is capped at128 MiB; source pixel ceilings and PNG output/dimension budgets remain.
The command inherits the existing sandbox deadlines, address-space and CPU limits. Native host
decoded-frame budgets are not increased. Absent/failing tools or invalid outputs fall back safely.
HEIF eligibility inspects at most two top-level IDs and32 thumbnail IDs; HDR/high precision,
unsupported NCLX profiles, multi-image containers and potentially selectable large embedded
thumbnails keep the existing fallback, preventing a thumbnail from silently replacing the primary.
This path preserves the preview bounding box/orientation but uses a different resampling filter;
it is not bit-identical to ImageMagick or verified against every camera/ICC encoding.

Video drops excess display-rate frames before rotation/scaling and scales before RGBA conversion,
avoiding full-resolution color conversion for frames that will not be displayed. Frame limits remain
1280×720 at up to30fps, with full-length playback, audio, rotation, seek epochs and sandbox isolation
unchanged. This does not skip codec reference decoding, enable new GPU device access, or remove the
first GStreamer registry scan.

The host's persistent thumbnail cache stores bounded raw sRGB RGBA, not decoder-ready files from
untrusted cache storage. Records validate dimensions, checksum and strong source identities before
encoding a fresh PNG. Disk reads/writes occur off GTK in private storage with bounded queue,
entry and byte limits. This cache does not extend original-file parser permissions or substitute
a thumbnail for the full selected preview. Native video decoding uses a separate isolated worker;
host-side initialization and audio output are distinct from decoding user media.

Strata starts its own executable in a bubblewrap sandbox. The sandbox has:

- a new user, mount, PID, IPC, UTS, cgroup, and network namespace;
- read-only access to `/usr`, required runtime libraries and font/ImageMagick configuration, the Strata executable, and exactly one canonicalized input file;
- writable access only to private mode-0700 output and temporary directories;
- an empty environment with a nonexistent home directory;
- 1.25 GB address-space, 10-second CPU, 32 MB file-size, and 12-second wall-clock limits.

The parent accepts only a bounded, validated PNG, normalized media stream, or UTF-8 text result.
Cancellation or timeout kills bubblewrap, which is the sandbox PID-namespace init process, and
therefore tears down all helper descendants. A missing bubblewrap installation, renderer crash,
malformed result, timeout, or permission failure is fail-closed and produces the normal fallback
icon, an unavailable search notice, or **Preview unavailable** message.

## Full-length playback sessions

### Selected audio preview

Audio file previews request `PreviewAudioMetadata` (browser wire operation **19**) and receive
bounded JSON in the framed data payload, with an empty auxiliary metadata payload. The one-shot
fallback writes `result.json` through `preview-audio-metadata`. Neither path invokes waveform
generation, transcodes the track, or returns a waveform PNG. Operation 17 / the legacy
`preview-audio-waveform` helper remain for legacy direct helper compatibility only; the file
preview provider no longer requests them. The persistent music controller's separate
`MusicWaveform` analysis and queue behavior are unchanged.

FFprobe reads a seekable, read-only input descriptor inside the existing sandbox, including
embedded tags and audio properties. Missing properties are unknown, not invented defaults.
Tag output is limited to 64 entries, 64-character names and 1024-character values; all control
characters are removed. Metadata and artwork are read-only: these are embedded media tags, not
editable Finder/filesystem tags or a ratings database. Artwork decodes separately in the sandbox,
using the selected track's embedded cover first, then bounded, named adjacent cover candidates.

Selection never autoplays. Explicit Play starts only that selected file through a separate
full-length playback session and invokes the drawer's audio-play callback to pause persistent
music. Starting persistent music pauses only the audio preview through `pause_audio_preview()`.
Closing or replacing the drawer cancels metadata/artwork work and stops its own session; it
never automatically resumes persistent music. Request IDs, weak UI references, cancellation
and source identities prevent stale results from updating a replacement preview.

The player card uses GTK height-for-width measurement/allocation, not a nonexistent `width`
property notification or an idle resize poll. Artwork and controls share a row at widths of at
least500px only when both children's measured widths fit with16px spacing; otherwise they stack.
The minimum width remains the stacked width, so widening does not prevent shrinking again.
Metadata stays below the player in the existing scroller, and wide layout follows text direction.

The vinyl and PCM spectrum own removable frame callbacks with weak widget/stream references.
Unmapping removes animation callbacks immediately; pause permits bounded peak/rotation decay,
then no animation callbacks remain. Both obey Hermes Reduce Motion and GTK animation settings
at startup and on active frames. Preview stop/drop cancels artwork, disconnects playback refresh
and visualizer signal handlers, removes frame callbacks, and stops only its own session. The
visualizer draw function reads the latest frame's bands; repainting does not re-run spectrum
analysis. This does not claim the playback stream's separate polling has zero idle CPU.

Playback has the same namespace isolation, empty environment, read-only selected input, and private
temporary filesystem. A read-only host GStreamer plugin registry may also be mounted to avoid
rescanning installed plugins for each selection; it does not expose the user's cache directory.
The worker has 1.25 GB address-space, 32 MB file-size, and 128-descriptor limits. Unlike short-lived
preview helpers, it has no cumulative CPU or movie-duration cutoff. Libav decoder threads are
limited to two when that property is available. Startup/seek requests have a 12-second deadline;
active playback also times out when packets stop arriving. Paused sessions can remain idle.
The dedicated monitor thread both launches and owns the sandbox until cancellation and reaping.
This keeps bubblewrap's parent-death protection tied to the playback session, not to a temporary
GLib loading thread that can retire after 15 seconds of inactivity.

The binary protocol validates header size (4 KiB), payload size, dimensions (at most 1280×720),
packed RGBA byte counts, nonnegative timestamps, audio alignment and payload size (64 KiB), and
bounded error strings before allocating payloads. No original encoded media reaches the main app.
The GUI retains only the latest frame; the audio queue is limited to 96 KB/16 buffers and drops
old queued buffers instead of growing without bound. Seek generations reject stale packets and
flush audio output. Closing/changing a preview signals cancellation; the monitor kills/reaps
bubblewrap and therefore all decoder descendants off the interface thread.

Host PCM packets carry durations calculated from their48kHz stereo sample counts, including split
packet tails. After the volume element, nonempty zero-gain/muted PCM retains its actual zero bytes
but no longer carries the GStreamer GAP hint. With the output sink running unsynchronized to the
decoder's separate clock, that hint otherwise skips normal PCM submission and resets the sink's
next-sample position. Keeping real silence flowing avoids that zero-volume scheduling transition;
it does not pause/restart decoding, substitute a nearly-zero gain, or change the desktop mixer.
Empty gap markers and all other buffer flags/timestamps remain untouched. Volume requests still
use the bounded, coalesced off-interface writer. Native capture regressions check repeated zero/gain
and mute/unmute transitions, exact samples, duration/timestamps, and shared-buffer safety without
opening an audio device.

Decoded audio buffers need not match the transport packet size. The worker splits aligned F32
stereo buffers into packets of at most64KiB, preserving their order and seek epoch without raising
any receiver or queue limit. Empty decoded buffers produce no audio packet; misaligned buffers and
transport errors still stop playback, with callback-level diagnostic messages. Regression coverage
includes the147440-byte PCM buffer that previously failed a short HEVC camera clip, a generated
160000-byte decoded sample, seeking and full playback/replay. This is a transport correction, not
a codec/GPU permission change. GStreamer permits empty buffers and buffers with multiple memory
blocks; see its [buffer documentation](https://gstreamer.freedesktop.org/documentation/gstreamer/gstbuffer.html).
