# Context — Project Orchestrator Gen 2

## Mission & Role
- Orchestrator Generation 2 for Hermes Preview & UI Overhaul.
- Parent: Sentinel (conv ID: `de5db9f9-e8e0-4efe-98aa-deaf92fdf84f`).
- Project Root: `/home/bry/.gemini/antigravity/scratch/hermes`.
- Working Directory: `/home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_2`.

## Historical Context & Prior Gates
- **Milestone 1 (R4)**: Status bar free space wiring, item counts, selection metrics, 0 warnings, 187 unit tests passing. Verified by 2 Reviewers, 2 Challengers, and Forensic Auditor.
- **Milestone 2 (R2)**: Persistent pooled sandbox worker, Unix domain socket SCM_RIGHTS descriptor passing, 8-byte framed wire protocol, header sniffers for dimension extraction, EXIF thumbnail fallback, prlimit memory & CPU ceilings. Verified by 2 Reviewers, 2 Challengers, and Forensic Auditor. 204 unit tests, 165 E2E tests passing.
- **Milestone 3 (R1)**: GeoTIFF GIS inspector and overview pyramid pipeline. IFD discovery without gigapixel decompression, float32 DEM contrast normalization (NoData sentinels, 2-98% percentile stretching, hypsometric tint), multi-band optical RGB mapping, geospatial metadata extraction, 160x90 Cairo DrawingArea placement map with Snyder projection. Verified by Reviewers, Challengers (`tests/challenger_m3_1_stress.rs`, `tests/challenger_m3_2_stress.rs`), and Auditor. 216 unit tests, 165 E2E tests passing.

## Current Target: Milestone 4 (R3 — Rich Format Previews)
Requirements to execute:
1. 3D Models: Render preview thumbnails for STL and 3MF geometry files.
2. eBooks & Comics: Extract cover art and previews for EPUB, CBZ, and CBR archives.
3. Spreadsheets: Render tabular previews and extract searchable text for ODS, XLS, and XLSX sheets.
4. Audio Waveforms: Generate waveform visualizers for audio formats (FLAC, MP3, WAV, OGG).
5. Interactive PDF Selection: Enable text highlighting and clipboard copying directly from the PDF preview drawer (adapting upstream e816f85b).
(Note: No modal/Vim keyboard navigation modes).

Upstream reference commits available:
- `570bdd30`: STL, 3MF 3D geometry software rasterizer.
- `ba676d3e`: EPUB, CBZ, CBR cover extractor.
- `d39052be`: ODS, XLS, XLSX spreadsheet parser (`calamine`) and virtual table view.
- `40a5a606` / `24857a55`: Audio waveform visualization.
- `e816f85b`: Poppler glyph layout text selection and clipboard copying.

## Hard Constraints
- DISPATCH-ONLY: Never edit source code directly; never run build/test commands directly.
- All tasks delegated to subagents.
- MANDATORY INTEGRITY WARNING included in all Worker dispatches.
- Forensic Auditor is a BINARY VETO.
- Succession trigger at 16 spawns.
