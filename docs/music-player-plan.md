# Hermes music player and desktop integration plan

## Approved next library features — 2026-10-10

After retained-tab performance work, simplify library navigation into Albums, Artists, Songs,
Favorites and Playlists. Add favorites and manually created/named/reordered playlists, with track
and album context-menu actions. Include automatic Most played and Recently added collections.
Do not add Save queue as playlist; queue restart persistence remains a separate existing feature.
Gapless playback is explicitly a later feature, not part of this pass.

These library features are approved plans, not implemented controls. Store favorites, playlist
membership and listening history locally without editing audio files. Recently added should use
when Hermes first indexed a track, not the file's modification time; the first import cannot know
historical arrival dates. Most played should count meaningful main-player listening, not preview
clicks or immediate skips. Define its threshold and renamed/missing-file behavior during implementation.
Keep scanning and persistence off the interface thread, and never autoplay while browsing collections.

The user also reports some MOV and JPG cold loads are slow. Retained tabs avoid reconstructing
unchanged folder presentations; they do not eliminate first-time poster/image decoding. Profile
those separately using representative fixtures before changing decoder budgets or quality.

Implementation update, 2026-10-10: the full-length player, dedicated Music view, interactive
waveforms, live queue popover, metadata track ordering and Noctalia colors are implemented.
Music and music-scoped search now share a persistent, identity-validated local catalog with
folder monitoring. Queue rearrangement/removal and restart persistence (without autoplay) are
implemented too, as is the MPRIS desktop-control bridge. Album double-click plays the complete
ordered album; single-click browses. Music has a hideable queue, a secondary-action menu and
compact shuffle/repeat controls. File audio previews use artwork, single-track playback and
read-only embedded tags/properties instead of waveform generation. Explicit preview Play pauses
main music without replacing its queue; closing never auto-resumes it. Starting main music pauses
the audio preview. Music keeps its real-data waveform, with no empty placeholder.

Music tracks/albums now have right-click and Menu/Shift+F10 context menus for Play, Play next
and Add to queue. Menu targets are captured at opening, so filtering or recycled rows cannot
silently queue a different song. Albums use the complete metadata-ordered album, not just filtered
results; track Play starts the remaining folder queue while track queue actions add just that song.
Queue additions do not start or interrupt playback. Popovers unparent on close/unbind/unmap and
hold only weak view references. A per-window eye toggle hides waveform/status without affecting
playback; it is a presentation option, not a persisted preference or analysis-disable setting.
Shell media-key acceptance, gapless/ReplayGain and remaining cover/performance audits are
follow-ups. See NEXT_AGENT.md
for current validation. The proposed-state observations below are historical, not today's status.

Status: proposed, 2026-10-02. The user wants the earlier built-in player restored, full music
playback with waveforms, dependable covers and media previews, clearer sorting, and Noctalia
colors. Column sizing is deferred. Existing view modes have now been found in the toolbar;
improve their discoverability rather than creating duplicate modes.

## Current state and diagnostic evidence

- The development build automatically rebuilds and restarts under `STRATA_DEV=1`. Sidebar
  transfers and folder pinning are implemented; 248 application tests pass.
- Columns (currently labeled List), Grid, and Explorer already exist. Explorer is the detailed
  table with Name, Size, Type, and Modified columns. Sort order and view mode are separate.
- Sort fields are Name, Size, Modified, and Type, with direction and folders-first controls.
  The user's earlier screenshot has Modified selected. Timestamps do not represent track order.
  Name order is lexical, explaining `1, 10, 11, 2`.
- Audio currently routes to `PreviewAudioWaveform`, which draws a waveform and metadata but
  has no playback controls. The older normalized-media player still exists with a 30-second
  limit. It is unsuitable as a full music playback engine.
- `gst-plugins-good` is missing on this desktop. Inspection found no `autoaudiosink`, `qtdemux`,
  or `flacdec`. Install it before validating playback: `sudo pacman -S --needed gst-plugins-good`.
  This package installation has not been performed; sudo requires the user's password.
- A real MOV converted successfully inside Bubblewrap. A real MP4 failed with the existing
  10-second CPU budget and succeeded with a temporary diagnostic 25-second budget. The app also
  has a 12-second wall limit. Application limits have not been changed.
- `IMG_0696.HEIC` is identified as `image/heif` and rendered successfully by the existing
  one-shot sandbox helper. The user's latest Grid screenshot shows its thumbnail. Reproduce
  the reported drawer failure through the production adapter before assigning a root cause.
- Missing covers are valid JPEGs: BRAT is 3600x3600; Immunity and EUSEXUA are 3000x3000. The
  predecode guard rejects RGBA frames above 32MiB unless an EXIF thumbnail is available.
  Charm's working cover is 1200x1200. The picker also recognizes only a small filename list
  and does not retry another candidate after a chosen cover fails.
- Noctalia is version 5.2.1 here, configured for a wallpaper-derived dark palette. Hermes
  currently supports built-in/custom themes and an explicit Omarchy integration.

## Intended experience

Add a Music workspace inside Hermes: album artwork tiles, artist/album browsing, and a metadata
track list. Keep the file browser available for organizing files and selecting music folders.

Playback belongs to the application, independent of the selected file, drawer, or current tab.
Browsing and closing a preview must not stop a playing track. Selecting a different track
previews its details without changing the playing queue. Playback starts only on an explicit
Play action, never from library scanning or selection alone.

A persistent bottom transport provides cover, title/artist, play/pause, previous/next, seek,
elapsed/remaining time, and volume. An expanded Now Playing view adds a waveform and queue.
Actions include Play album/folder, Play next, and Add to queue. Add repeat/shuffle once the
basic transport and queue are dependable.

## FLAC and waveform design

FLAC is viable. Decode to PCM without a lossy intermediate conversion. Read tags, embedded
artwork, sample rate, channels, bit depth, and total samples separately. Where sample count
is available, duration is total samples divided by sample rate; use a decoder-derived fallback
when duration metadata is absent.

Generate waveforms in the background by streaming decoded samples into min/max peak buckets,
optionally retaining RMS. Cache numeric peaks rather than a fixed colored PNG. GTK/Cairo can
draw them at any size with live theme colors, overlay elapsed progress, and map clicks/drags
to playback time. Waveform generation must not delay playback startup.

Cache identity includes file identity, size, modification time, and analyzer version. Bound
buffers and concurrency; prioritize the playing track and visible albums. Full waveforms
require reading audio but never holding a whole decoded song in memory. Subsequent visits
reuse the cache. Preserve source files and leave ReplayGain opt-in.

Lossless decoding does not guarantee bit-perfect device output: volume and device resampling
remain relevant. Gapless playback needs its own implementation and verification milestone.

## Proposed architecture

| Boundary | Responsibility |
| --- | --- |
| Player service and application controller | Commands, playing track, queue, repeat/shuffle, state transitions |
| Audio playback adapter | Sandboxed decoder session and bounded PCM transport |
| Music library adapter | Cancellable metadata scan and persistent index |
| Waveform service | Peak extraction, cache identity, priorities, cancellation |
| Player UI and Music workspace | Transport, album tiles, track table, waveform, queue |

The first playback spike should prove a sandboxed, seekable decoder session sending PCM chunks
to a host GStreamer `appsrc` audio pipeline. Compressed-media parsing, tag extraction, artwork,
and waveform analysis stay isolated. Define open/play/pause/seek/stop commands, validated sample
format/timestamps, errors, backpressure, and generation IDs to flush stale audio after seeks.
The host handles only validated PCM and desktop audio output.

Do not extend the existing 30-second conversion into a whole-track in-memory lossy file.
Long-playing decoder sessions need resource budgets distinct from short preview jobs. A lifetime
10-second CPU limit is unsuitable. A direct GStreamer source-file backend would change the
current sandbox boundary and must be an explicit architectural decision.

## Staged build and acceptance

### 1. Media and artwork reliability

Add dependency diagnostics and useful errors. Reproduce HEIC in the drawer and establish
media-specific budgets. Provide a bounded poster frame when playable video normalization is
too expensive. Preserve cancellation and fail-closed parsing.

Downsample large JPEG covers inside the sandbox before producing a bounded thumbnail; keep
the memory guard rather than raising it globally. Rank candidates deterministically, retry
failures, and fall back to embedded FLAC art. Support common cover names and a bounded
single-image fallback for album folders. Invalidate caches when artwork changes.

Acceptance: production paths display MOV, MP4, HEIC, embedded FLAC artwork, and the named
3000/3600px covers. Malformed files and resource-limit failures return useful messages.

### 2. Noctalia and sorting clarity

Add Follow Noctalia using a Noctalia v5 user template that exports Hermes's semantic tokens.
Keep the existing configuration namespace until the coordinated rename. Map Material
surface/on-surface/primary/error/outline roles to background, text, accent, danger, border,
muted, and selection colors. Watch the output's parent directory, handle atomic replacement,
debounce changes, and keep the last valid theme when output is absent or incomplete.
Follow Noctalia's application mode, which may differ from its shell mode.

Make the existing toolbar view selector easier to recognize. Consider clearer labels:
Columns, Icons, Details, while retaining compatibility with saved mode names. Keep sort
field and direction visibly distinct, for example Modified: newest first.

Use natural Name sorting with deterministic native-path ties. For music, default to Disc,
Track, then natural filename fallback. Add cached metadata fields for Artist, Album, Album
artist, Year, Disc, Track, Title, and Duration. Grouping by artist/album/year is separate
from sorting. Remember folder/workspace preferences; unavailable dates remain explicit.

Acceptance: natural `1, 2, 10` order, correct multi-disc track order, preserved selection when
switching views, and live Noctalia updates across controls, menus, icons, artwork framing,m

and waveform colors.

### 3. Full-track playback foundation

Prove FLAC/MP3 play, pause, seek, and end-of-track behavior with bounded memory. Add persistent
transport and an in-memory queue. Handle decoder crashes, device loss, and stale seek results.

Acceptance: a complete track plays; repeated seeks and skipping work; browsing another folder
or closing the drawer preserves playback; stopping leaves no decoder processes behind.

### 4. Interactive waveforms and Music workspace

Add cached waveforms with click/drag seeking, album tiles, metadata track tables, queue
reordering, Play album, Play next, repeat/shuffle, and saved library folders. Start with a
folder scan and add an index for fast artist/album queries. Scanning stays local and cancellable.

Acceptance: waveform/playhead timing agrees; cached tracks avoid repeated waveform analysis;
large libraries remain responsive; changed tags/covers update without duplicating tracks.

### 5. Desktop player polish

Add MPRIS for Noctalia's media widget and media keys, queue persistence, gapless album playback,
and optional ReplayGain. Test consecutive FLAC tracks, mixed sample rates, 24-bit material,
missing tags, damaged files, long recordings, and missing output devices.

## Primary references

- [FLAC format overview](https://www.xiph.org/flac/documentation_format_overview.html)
- [FLAC STREAMINFO fields](https://xiph.org/flac/api/structFLAC____StreamMetadata__StreamInfo.html)
- [GStreamer playback and gapless transitions](https://gstreamer.freedesktop.org/documentation/additional/design/playbin.html)
- [GStreamer seeking](https://gstreamer.freedesktop.org/documentation/additional/design/seeking.html)
- [Noctalia v5 app templates](https://docs.noctalia.dev/noctalia/theming/app-theming/)
- [Noctalia template roles](https://docs.noctalia.dev/noctalia/theming/templates/)
