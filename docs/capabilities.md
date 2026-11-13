# Capability and qualification matrix

November 13 source checkpoint. **Implemented** describes routing/code, **verified**
names specific evidence, **experimental** needs integration or physical acceptance,
and **deferred** is outside the 1.0 scope. No format-extension list guarantees every
codec, camera profile or malformed document works.

| Capability | Implementation / dependency | Evidence and limits |
| --- | --- | --- |
| Linux browsing, local operations, collections | GTK4/GIO, shared isolated data roots | Generated ordinary/safety tests; physical cross-device, full-disk and real upgrades remain open |
| Standard photos / thumbnails | Bounded image helpers; optional ImageMagick/libvips routes | Generated image/cache/cancellation tests; camera/ICC/HDR corpus and physical renderer review still required |
| HEIF / RAW | libheif and available isolated conversion helpers | Implemented; decoder availability and camera variants vary; no complete camera qualification claim |
| Audio / video | Isolated FFmpeg/GStreamer paths, installed codecs; optional ffmpegthumbnailer | Synthetic transport/focus/poster tests; audible playback, hardware/Asahi and broad phone-codec corpus remain separate |
| PDF | Poppler-backed document reader | Implemented; generated parsing and private widget tests do not establish all malformed-PDF or large-document behavior |
| DOCX / ODT / PPTX layouts | Optional local LibreOffice to PDF, bounded cache/posters and text fallback | Layout first remains the product policy; conversion timeout/failure is not a completed layout; complex layout/font corpus and cold-open latency remain open |
| XLSX / ODS / XLS | Bounded workbook parsing and interactive sheet view; optional layout/thumbnail helpers | Generated workbook and private view tests; no general Excel formula recalculation or full-fidelity promise |
| Archives / EPUB / comics / models | Bounded registered helper routes, optional libarchive tools | Generated parsing fixtures; archive-worker and extraction-boundary audit remains open |
| Home unified search | Shared existing catalog, fuzzy/accent/NL queries, contents opt-in | Implemented; complete large-library coverage, paging and budget/recovery performance gates remain open |
| Default manager and XDG picker | Separate reversible Linux opt-ins | Private setup/GTK fixtures; real browser/Wayland/Flatpak acceptance remains experimental |
| Native x86_64 pacman package | Source-bound recipe, declared runtime/optional helpers | Host-built artifact inspected; clean-machine install/upgrade/uninstall and RC qualification remain open |
| Linux aarch64 / Asahi | Native recipe target | Experimental until built/installed/tested on the actual reference device |
| Apple Silicon macOS | Experimental native Dev build/bundle in PR #21, Homebrew runtime dependencies, separate Dev profile | First-window physical testing pending; decoder previews/playback and Linux descriptor-backed caches are unavailable until Mac adapters pass qualification; XPC proof and broader integration precede stable |

The live routing source is `src/services/formats.rs`; earlier measurements and
format descriptions in [the preview audit](preview-format-audit.md) are dated
evidence, not a newly regenerated all-format qualification. The test runner's
`coverage.json` records which checks actually ran and which remain excluded with
their dependency/platform/corpus reasons. The native package's `.PKGINFO` lists
required and optional dependencies, while `DEPENDENCIES.json` inventories the
resolved Rust graph and copied license notices.

Preserve the approved winged-H identity and upstream attribution. Main app and
picker follow the chosen Noctalia settings. No automatic installation, updates,
default takeover, renderer forcing or telemetry is added by this batch.
