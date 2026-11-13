# Hermes review — October 15, 2026

> Historical review of commit `7425bc4`, preserved for reference. Findings and test counts below
> describe that snapshot, not the current build. See [the roadmap](docs/1.0-roadmap.md) and
> [the current handoff](NEXT_AGENT.md) for subsequent fixes, verification and remaining work.

Reviewed local commit `7425bc4` against the agreed search/media behavior. Repository was clean
before and after review. No application source, user preferences, GitHub state or live Hermes
window was changed. Review artifacts/probes live outside the repository.

## Fix first

### P1 — Default-hidden Smart Folders trigger continual sidebar rebuilds

`/home/bry/hermes/src/ui/window.rs:1325` conditionally skips `append_smart_folders()`, but that
method contains the only saved-revision acknowledgement at
`/home/bry/hermes/src/ui/window/saved_searches.rs:40`.
The timer at `/home/bry/hermes/src/ui/window.rs:2430` still compares revisions every150ms.
With the default setting false, saved_revision stays0 while the initialized store has revision1+.
It therefore removes/recreates sidebar rows on every tick, even with no saved searches.
This is a concrete source-level explanation for avoidable UI churn and unstable sidebar focus;
the impact on the user's particular machine was not profiled.

Fix: acknowledge processed revisions independently of whether the section is displayed, and add
a default-hidden regression that verifies unchanged row identities across idle polling ticks.

### P1 — Dismissing an inferred filter can panic on valid Unicode input

`/home/bry/hermes/src/ui/search.rs:880` finds byte offsets in `current.to_lowercase()` and then
slices the original string using those offsets. Lowercasing can change UTF-8 byte length.
Repro: enter `İ songs` and dismiss Audio. Original length8; lowercased match starts4 and ends9.
The original-string slice is out of bounds. A separate native Rust probe confirms those slices
are invalid; the running user's Hermes was not used to trigger a crash.

Fix: retain original token spans or translate offsets correctly; do not slice original text using
normalized-string offsets. Test Unicode expansion, escaping and repeated/protected tokens.

### P1 — Natural-language inference suppresses real song and album names

`/home/bry/hermes/src/services/search.rs:818` recognizes any unquoted `video` token as a hard
video-type restriction, before a literal title can match. The parser probe shows `Video Games`
becoming terms `Games`, category `video`; an audio track actually titled Video Games is excluded.
Similarly, `/home/bry/hermes/src/services/search.rs:704` parses `Lost in the Dream` as terms
`Lost Dream` constrained to a path containing `the`, excluding normally located matching albums.

Fix: preserve literal candidates or use confidence-aware alternatives instead of discarding them
with inferred hard constraints. Add ordinary title/album names to the acceptance corpus.

## Audio preview issues

### P2 — Visualizer retains the stream after the preview is discarded

At `/home/bry/hermes/src/ui/preview/audio/visualizer.rs:83`, the stream owns a signal callback
capturing `WeakVisualizer`. Despite its name, that helper stores a STRONG stream clone at
line143, creating stream -> callback -> stream ownership. It is not disconnected on stop.
A private GTK probe importing the actual visualizer, with a silent dummy MediaStream, confirms
the stream remains alive after every caller reference and visualizer are dropped.

Tick callbacks additionally capture their DrawingArea strongly, and stop() only changes a bool,
not the installed callback. Cleanup needs actual callback ownership/removal and weak object refs.
The existing AudioPreview.stop() does release the playback Session, so this is not evidence that
closed previews continue decoding or playing audio; retained objects/widgets can accumulate.

### P2 — Wide-pane audio layout never receives its resize notification

`/home/bry/hermes/src/ui/preview/audio.rs:231` listens for `notify::width` on a GtkBox. GTK's
allocated width is not a GObject property; both GI and native probes report no `width` property.
Thus this callback does not implement the advertised switch at500px; the player starts vertical
and has no working allocation-based switch to horizontal.

Fix: use a real allocation/layout mechanism, then test narrow/wide transitions with long metadata
and larger interface fonts. Do not just rename the notification to width-request.

### P2 — Vinyl animation ignores Hermes's Reduce Motion preference

`/home/bry/hermes/src/ui/preview/audio/vinyl.rs:88` and the duplicate handle implementation check
only gtk-enable-animations, not the app's `ui::motion::animations_enabled()` helper. A private
probe using the actual vinyl/motion modules observes angle changes while Hermes's helper returns
false for reduced motion. Runtime preference changes also are not checked by active ticks.

Fix: use the common motion policy and stop existing animation when that policy changes.

The Previous/Next labels are also misleading: Previous seeks to0; Next seeks to the current
track's duration (`/home/bry/hermes/src/ui/preview/audio.rs:260`). Neither selects another track.
For a deliberately single-track preview, label the real actions or disable unavailable navigation.

## Additional search correctness gaps

- Explicit precedence: `type:folder videos` produces directory=true AND category=video, which
  cannot match any entry because the scorer rejects directories under category filters.
  See `/home/bry/hermes/src/services/search.rs:819` and `:202`.
- `songs by Cameron Winter` leaves the literal term `songs` after artist extraction. The parser
  only consumes a trailing songs token when at least two tokens remain (`:765`). Most tracks by
  that artist do not contain the word songs, so this common request misses them. The documented
  FLAC-prefixed example happens to take a different branch and hides the problem.
- Quotes are discarded before matching (`/home/bry/hermes/src/services/search.rs:565`), allowing
  quoted terms to be typo-expanded. Quoted `"ext:flac"` is even parsed as an extension operator
  rather than literal text; the standalone parser probe confirms it.
- Applying Filters reconstructs free terms without quotes
  (`/home/bry/hermes/src/ui/search.rs:771`). Applying unchanged filters to `"Video Games"` can
  therefore remove its protection and activate the hard video restriction.
- Location inference checks only root-relative paths: with This folder already at Downloads,
  `PDFs in Downloads` excludes a.pdf because its relative path no longer contains Downloads.
  See `/home/bry/hermes/src/services/search.rs:209` and `:704`.
- Chip dismissal replaces the first text occurrence, not the inferred token's actual span.
  For `"songs" songs`, dismissing Audio targets the protected first occurrence and can leave
  the actual inferred trailing songs token active (`/home/bry/hermes/src/ui/search.rs:880`).
- Album/artist entity-label matching still uses a different fuzzy strategy from Songs; a typo
  such as Cameron Wonter can find songs but lose the corresponding artist/album entity.
  See `/home/bry/hermes/src/services/search/groups.rs:410`.

## Layout/keyboard follow-ups

Inferred labels at `/home/bry/hermes/src/ui/search.rs:852` are unrestricted and non-ellipsized in
a single horizontal row: a long artist phrase can impose its full width on the dialog.
Notice and interpretation slots are also hidden/shown (`:844`, `:978`, `:1101`), contrary to the
reserved-space plan. Existing geometry tests passed for their synthetic scenarios; they do NOT
prove stable geometry for long interpreted queries or actual indexing/count transitions. Add those
specific cases before claiming no resizing. No live-user stutter was attributed to these by measurement.

The existing capture-phase Enter handler at `:523` opens a selected result regardless of focus,
interfering with keyboard activation of the new chip buttons. This handler predates the update,
but the new controls need a focus-aware behavior check.

## Validation performed

- Test compilation and all-target check pass, with six pre-existing binary-test lint warnings.
- Unmodified full headless suite exits139 (segmentation fault). Excluding the new non-ignored
  GTK-initializing dimensions test gives746 passed/105 ignored/1 filtered out. That dimensions
  test passes when run separately on a private GTK display. This is a test-isolation issue,
  NOT evidence that the normal application has the same segmentation fault.
- Three existing isolated search checks pass: geometry/row reuse, pagination, grouped entities.
- The newly claimed interpretation-slot test FAILS in private GTK at
  `/home/bry/hermes/src/ui/search/tests.rs:1489`: it changes the field while the dialog is hidden,
  so begin_query does not render its chips. Fix the fixture, then test actual geometry and dismissal.
- Prior native zero-volume/mute restoration regressions pass; the silence/GAP fix remains present.
- Real visualizer/vinyl source was exercised with a silent dummy stream on private Broadway;
  no personal audio/video was opened and no hardware sink or desktop mixer was changed.
- The font/theme wiring is present in source. Computed menu typography and pixel-level visual
  polish were not exhaustively verified. No release rebuild was needed for this review.

The notes' lock-free spectrum claim is inaccurate: Session::spectrum_bands() takes a mutex and
executes analysis on its caller, currently the GTK draw path. This alone does not establish an
audible or visible performance regression; benchmark or move analysis before making that claim.

## Artifacts and next step

Logs: `/tmp/hermes-review-20261015-3IP5VK/` (compile.log, unit.log, unit-no-gtk-init.log,
gtk-review.log, audio-probe.log, widget-isolated.log, check.log).
Source-only parser replay: `/tmp/hermes-search-review-probe.rs`.
GTK lifecycle/motion/property probe: `/tmp/hermes-audio-review-probe.rs`.
Both probes import or replay current source without modifying the repository; parser replay does
not substitute for a full catalog integration test.

Both review agents are closed and all owned Broadway servers stopped. Keep the existing working
tree intact. Recommended fix order: sidebar polling + Unicode crash, literal/explicit search
semantics, preview lifecycle/layout/motion, then fuller UI/performance regression coverage.
