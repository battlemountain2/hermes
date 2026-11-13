# File hover previews

Columns, Grid and Explorer use the same optional hover card. Settings → Previews & files →
Rich file hover previews turns it off, retaining a plain filename tooltip. Directory peeks
keep their existing ordered-folder behavior; this is not the removed picker folder gallery.

## Presentation

- Filename, type, size and modification date appear immediately from the current browser entry.
- Songs include embedded title, artist (or album artist) and album when available. A bounded
  metadata probe adds duration and codec; it does not guess tags from filenames or play music.
- Photos show dimensions when the header/parser can provide them; videos add duration,
  dimensions and codec. Documents show basic file facts and an existing cached poster, if any.
- Artwork checks Hermes's existing memory/disk thumbnail cache first. Small image/media artwork
  can be generated with the existing sandbox scheduler; Office/PDF/model conversions are never
  started by hovering. A missing dependency, budget limit or unsupported format keeps the icon.

The card uses semantic theme colors, the interface font, bounded single-line labels, a reserved
104px visual column and a stable minimum footprint. Loading an image cannot push the text aside.
There are no player controls, animation ticks, autoplay, or new folder scans.

## Lifecycle and safety

The browser factories resolve the current entry through their active filtered model, rather than
capturing an old row. Inline renaming and clicks suppress the tooltip. Card widgets are created
only on tooltip request; decoding starts only when the card maps. Pointer exit, unmapping,
recycling or rebinding cancel outstanding work and invalidate its generation.

Quick metadata operations return bounded JSON through the existing sandbox protocol and cache.
The UI enforces a three-second deadline, source identity before/after parsing, the format switches,
size policy, and a current mapped/generation check. Streaming media metadata is not subject to the
old blanket generation-size ceiling; image/artwork generation still is. Originals are unchanged,
and no network service or main-player command is involved.

## Verification and remaining checks

`scripts/check-picker-integration.py --binary <built-hermes> --test-binary <built-test-binary>`
also runs the hover regression in private HOME/XDG/D-Bus/Broadway roots. It uses a generated PNG
and silent WAV, without audio output. It checks image/media metadata transport, lazy loading,
cancellation/rebinding, disabled-preview policy, saved hover preference, stable size/text position,
large-font updates and stylesheet parsing. Snapshots cover dark and Paper themes.

Pure tests cover metadata bounds, attached-art/video distinction, duration formatting and rejection
of expensive cold document work. This is not coverage of every media codec, a physical Wayland
pointer/keyboard-tooltip test, or a claim of richer page/slide metadata. Those remain acceptance
checks; the full preview drawer remains the place for deeper details and playback.
