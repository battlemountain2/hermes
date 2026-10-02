# Next agent: Fix Previews, Dynamic Column Sizing, and Sidebar Drag & Drop

## Handoff status

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
