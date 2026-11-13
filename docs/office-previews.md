# Office page and slide previews

October20,2026: optional, local-only layout previews for DOCX/ODT documents and
PPTX/PPSX/POTX presentations, plus XLSX/ODS workbook print layouts. A Layout/Data switch retains
the interactive, bounded spreadsheet table. Neither view edits the source file.

## Reader behavior

- One-shot Office and PDF helpers reuse `/var/cache/fontconfig` read-only when it exists, matching
  the existing pooled helper. This avoids rebuilding system font indexes in each fresh sandbox.
  Missing caches still work; personal font caches and the user's LibreOffice profile stay hidden.
  Full Layout remains first, with saved/cached posters while conversion prepares.
- Workbook Layout and Data views keep horizontal/vertical scrollbars visible, outside the content,
  with larger square drag handles. Layout pan/text-selection gestures leave scrollbar drags alone.
  Horizontal Layout scrolling becomes useful after zooming beyond fit width (Ctrl+scroll).
- Data initially constructs only the first sheet's GTK table. Other sheets are built on activation
  and retained for return visits, preserving their scroll/sort state without eager hidden tables.
- Selected layout conversions take priority over queued Office thumbnail exports. One export
  already running may finish first; work is still serialized and queued requests remain cancellable.
- Full PDF export caps embedded raster images at150dpi for screen previews; text and vector
  objects remain vector content. This affects only temporary/cached PDFs, not the source document.
- Selecting a supported document attempts an isolated LibreOffice PDF export, then uses Hermes's
  existing PDF reader for lazy page images, selectable/copyable text, zoom and fullscreen.
- Saved/cached Office posters are provisional, explicitly labeled and shown while the full layout
  prepares. Saved posters use a separate bounded decoder that never starts LibreOffice. They do
  not complete the request, report a fabricated slide count or replace a newer selection. Closing,
  changing the file or completing the layout cancels/disposes the provisional work.
- Converted PDFs have a private persistent cache in addition to the bounded memory LRU. Unchanged
  documents can reopen without another Office conversion, including after restarting Hermes.
  See [Office cache](office-cache.md) for identity, renderer/font invalidation and storage defenses.
- Supported local workbooks default to Layout when LibreOffice is installed. Layout preserves
  rendered fonts, colors, merged cells and printed content beyond the Data row cap. Print areas
  and pagination follow the workbook; this is not an infinite native Excel canvas. Data retains
  sortable worksheet tabs and bounded rows. The switch remains available during conversion so
  Data can cancel a slow layout; failed layouts fall back to Data with an explanation. Legacy XLS
  remains Data-only. Page requests use the layout path rather than accidentally returning tables.
- Active embedded content remains disabled. The generated XLSX's native chart is omitted by this
  protected rendering profile; the UI explains possible chart/object omissions. Chart fidelity is
  **not** signed off, and the security profile was not loosened to make a visual test pass.
- Presentation slides retain their background, images, shapes and aspect ratio. DOCX/ODT retain
  document page boundaries, images and tables according to the installed LibreOffice importer.
- The header identifies a LibreOffice layout and its page/slide count. Conversion is not promised
  pixel-identical to Microsoft Office: missing fonts, unsupported objects, charts and complex
  layouts can differ. Speaker notes, animations, editing and formula recalculation are not added.
- File icons now request Office thumbnails too: DOCX/ODT, PPTX/PPSX/POTX and XLSX/ODS use a
  bounded embedded PNG/JPEG preview when present, otherwise a first-page-only LibreOffice export.
  Workbook icons crop the first printed page to its visible content instead of tiny cells on an
  almost empty sheet. Icon thumbnails are independent of the selected workbook's Layout/Data mode.
  Legacy binary DOC/PPT/XLS icon rendering is not added.
- Office icons use the existing visible-first scheduler and identity-checked persistent thumbnail
  cache. Cached icons do not need LibreOffice again. Missing/failed icons retain the file icon;
  unsupported embedded thumbnail encodings fall back to conversion rather than disappearing.
- If LibreOffice is missing, conversion fails or its limits are exceeded, the existing bounded
  extracted-text preview remains readable with an explanation. A later page failure stops its
  spinner and explains the failure; it does not display a stale page as successful.
- Content indexing still uses the existing opt-in text extractors, not automatic library-wide
  LibreOffice conversion. No original document, tags, associations or user Office profile is changed.

## Optional dependency

Use a maintained system LibreOffice installation. On Arch/CachyOS:

```sh
sudo pacman -S --needed libreoffice-fresh
```

Hermes does not run an installer. Discovery uses fixed system paths under `/usr` and the optional
`/opt/libreoffice` installation; it does not execute a `soffice` injected through the application's
PATH. Flatpak/Snap launchers and arbitrary custom executables are not integrated in this pass.
The user installed system LibreOffice 26.8.1 after the initial fallback-only check. Production
discovery and strict adapter checks now pass against that installation without the test runtime
override. The optional test-only runtime mount remains useful on machines without a system
renderer; it is never consulted by production discovery.

## Isolation, caching and limits

Export runs off GTK in a fresh bubblewrap PID/network/filesystem namespace with a private Office
profile. Only the selected original is mounted read-only, alongside system runtime/fonts and a
private output directory. There is no user Home, existing Office session, desktop bus or network.
Macros and active embedded content are disabled; link updates are disabled and the profile is
discarded with the namespace. Embedded images remain supported; outside document resources are
not made available. Keep LibreOffice itself updated; isolation is not a claim of parser perfection.

ZIP/XML package preflight retains its50MiB input, expansion/member/event limits. Export alone has
a4GiB virtual-address-space ceiling plus512MiB data-segment limit. The larger address-space budget
is specific to LibreOffice's mapped runtime; it does not reserve4GiB of RAM, nor promise total RSS
below512MiB. Office conversion and icon export now have 30 CPU seconds and 35 wall seconds:
the reported 49-slide presentation reproducibly exhausted the old 10-second CPU limit. The
same file rendered all 49 slides in about 12 seconds in the isolated regression, unchanged.
Other decoders retain their existing 10 CPU seconds, 12 wall seconds and 1.25GiB address-space
ceiling. The 32MiB per-output-file limit is unchanged. Office icon exports and full conversions
share one cancellable admission gate; queue waiting is separately bounded to 35 seconds.
Conversion and subsequent PDF rendering are separate bounded stages, not a single overall deadline.

Converted PDFs are reused in a process-local LRU: at most four entries/64MiB, expiring after five
minutes and keyed by canonical source identity, device/inode, size and nanosecond mtime/ctime.
Conversion reads that canonical source, verifies identity afterwards and rejects canceled/changed
results. Active readers can retain an evicted entry until their request finishes. Each page render
uses a private temporary PDF copy, removed after the request; no unprotected temporary document
cache or personal document fixture is committed. Failed conversions are not
cached as successes. The additional private disk cache is bounded to eight entries/128MiB and
checks source identity, renderer/helper revision and available system fonts; it is not a user
Office profile. Both cache paths still pass their PDF through the isolated reader.

## Verification

`tests/fixtures/office/generate.py` produces original synthetic DOCX/ODT/PPTX fixtures with two
pages/slides, selectable text, images, tables and colored shapes, plus a styled two-sheet XLSX.
It uses python-docx, python-pptx, openpyxl and Pillow only as test-authoring tools; they are not
runtime dependencies of Hermes. Generated DOCX/PPTX fixtures deliberately omit saved thumbnails
so the icon regression exercises first-page conversion rather than a template's stale cover.

The ignored `sandbox::office::tests::generated_office_pages_preserve_layout_selectable_text_and_cached_conversion`
case uses `HERMES_OFFICE_FIXTURE_DIR`, an optional explicit `HERMES_OFFICE_TEST_RUNTIME` directory
and `HERMES_OFFICE_TEST_SHOTS` for generated PNGs. The runtime override is test-only: its read-only
mount still runs through the actual production helper, namespace and resource limits. The case
checks two-page counts, portrait/landscape layout, per-page text/glyphs, changed page pixels,
out-of-range clamping, single conversion across page requests and unchanged original bytes.

Generated conversions were initially checked with LibreOfficeDev26.8.0.0.alpha0 from the bundled
test runtime, then with the installed system LibreOffice 26.8.1. Page and icon PNGs were inspected.
This is not a full real-user Office corpus or validation of every installed font combination.
Private GTK cases cover the adapter, reader identity/labels, fallback, fullscreen, persistent Office
icons and live spreadsheet fonts/themes. Set `HERMES_OFFICE_REQUIRE_LAYOUT=1` in the adapter test
to reject a text fallback. The explicit read-only local sample test accepts `HERMES_OFFICE_TEST_FILE`;
it is ignored by default and never changes or commits the chosen sample.
See NEXT_AGENT.md for final test outcomes and known unrelated test failures.

Implementation references: [LibreOffice command-line options](https://help.libreoffice.org/latest/en-GB/text/shared/guide/start_parameters.html),
[PDF export parameters](https://help.libreoffice.org/latest/en-US/text/shared/guide/pdf_params.html),
and [macro security](https://help.libreoffice.org/latest/en-US/text/shared/optionen/macrosecurity.html).
