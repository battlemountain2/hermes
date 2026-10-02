# Project: Hermes Preview & UI Overhaul (Phase 1 & Phase 2)

## Architecture
Hermes is a high-performance GTK4 file manager in Rust. Phase 1 overhauls the preview subsystem and status bar.
- **Preview Isolation**: Bubblewrap (`bwrap`) container isolation with Linux `prlimit` bounds (AS ceiling, CPU limit, FSIZE limits, tmpfs isolation).
- **Worker Execution Model**: Persistent pooled sandbox workers communicating over Unix domain sockets with SCM_RIGHTS file descriptor passing and 8-byte framed wire protocol (`[png_len, metadata_len]`), achieving <100ms response.
- **Large-File Guardrails**: File header sniffing (JPEG, PNG, GIF, TIFF) without pixel decompression; decoded frame budget checks; EXIF thumbnail fallback via `kamadak-exif`.
- **GIS GeoTIFF Pipeline**: Pure-Rust `tiff` (0.11) and `geotiff-core` (0.8.1). Pyramid overview IFD extraction without gigapixel decompression; float32 DEM contrast stretching (NoData filtering, 2-98% percentile stretching, terrain ramp); multi-band optical RGB mapping; geospatial metadata extraction (EPSG/CRS, resolution, bounds); Cairo DrawingArea placement map indicator.
- **Rich Format Previews**:
  - 3D Models: STL (ASCII/binary) & 3MF geometry software rasterizer to PNG.
  - eBooks & Comics: EPUB, CBZ, CBR cover art extraction.
  - Spreadsheets: ODS, XLS, XLSX workbook parsing (`calamine`), virtual table view, searchable text extraction.
  - Audio Waveforms: FLAC, MP3, WAV, OGG waveform visualizer generation.
  - Interactive PDF: Poppler glyph layout extraction, highlight overlay, clipboard copying (adapting upstream e816f85b, without modal/Vim navigation).
- **Status Bar Integration**: Free disk space (`update_free_space`), directory item counts, multi-selection aggregate sizes, dynamic updates on file creation/deletion/cross-mount navigation, elimination of compiler warnings.

## Feature Inventory
| # | Feature | Description | Milestone | Source |
|---|---------|-------------|-----------|--------|
| F1 | GeoTIFF Pyramid Overview Extraction | Read downsampled IFD overview levels without gigapixel decompression; sub-sampling fallback | M3 | ORIGINAL_REQUEST §R1 |
| F2 | Dynamic Band & Contrast Normalization | Float32 DEM contrast stretching (NoData filtering, 2-98% percentiles, terrain ramp); multi-band optical RGB mapping | M3 | ORIGINAL_REQUEST §R1 |
| F3 | Geospatial Metadata & Placement Badge | Extract EPSG/CRS, dimensions, resolution, bounding box; render Cairo DrawingArea placement mini-map | M3 | ORIGINAL_REQUEST §R1 |
| F4 | Persistent Pooled Sandbox Worker | Pre-warmed sandbox workers communicating over local wire protocol with <100ms response | M2 | ORIGINAL_REQUEST §R2 |
| F5 | File-Header Sniffing & Dimension Guardrails | Fast dimension sniffing (JPEG, PNG, GIF, TIFF); decoded frame budget calculation | M2 | ORIGINAL_REQUEST §R2 |
| F6 | EXIF Thumbnail Fallback | Fall back to embedded EXIF thumbnails for oversized images exceeding decoded frame budget | M2 | ORIGINAL_REQUEST §R2 |
| F7 | Memory Ceilings, Timeouts & Cancellation | Strict `prlimit` bounds, deadline reader polling (20ms), instant cancellation responsiveness | M2 | ORIGINAL_REQUEST §R2 |
| F8 | 3D Model Previews (STL, 3MF) | Parse STL and 3MF geometry, render shaded depth-buffered thumbnail preview | M4 | ORIGINAL_REQUEST §R3 |
| F9 | eBook & Comic Cover Previews | Extract cover art from EPUB (container.xml/OPF), CBZ (ZIP), CBR (RAR) | M4 | ORIGINAL_REQUEST §R3 |
| F10 | Spreadsheet Previews (ODS, XLS, XLSX) | Parse workbooks with `calamine`, render virtual table view, extract searchable text | M4 | ORIGINAL_REQUEST §R3 |
| F11 | Audio Waveform Visualizers | Generate waveform visualizations for FLAC, MP3, WAV, OGG | M4 | ORIGINAL_REQUEST §R3 |
| F12 | Interactive PDF Text Selection & Copy | Poppler glyph layout extraction, highlight overlay in preview drawer, clipboard copy | M4 | ORIGINAL_REQUEST §R3 |
| F13 | Status Bar Free Disk Space Wiring | Wire `update_free_space` to active directory changes, eliminate compiler dead code warnings | M1 | ORIGINAL_REQUEST §R4 |
| F14 | Dynamic Status Bar Updates & Multi-Mount | Update item counts, multi-selection aggregate sizes, dynamic updates on file create/delete/cross-mount | M1 | ORIGINAL_REQUEST §R4 |
| F15 | Regression Baseline Preservation | Ensure all 181 existing tests continue to pass | M1-M5, E2E | ORIGINAL_REQUEST §Criteria |
| F16 | E2E Test Suite (Tiers 1-4) | Comprehensive opaque-box test suite covering all features with Category-Partition, BVA, Pairwise | E2E Track | ORIGINAL_REQUEST §Criteria |
| F17 | Adversarial Hardening (Tier 5) | White-box stress testing, memory edge cases, corrupted/malformed files | M5 | Project Pattern §Phase 2 |

## Milestones
| # | Name | Scope | Dependencies | Status |
|---|------|-------|-------------|--------|
| E2E | E2E Test Suite Track | Build test runner, fixtures, and Tiers 1-4 test cases; publish TEST_READY.md | none | DONE |
| M1 | Status Bar Completion & Warning Elimination | Wire update_free_space in window.rs, eliminate unused warnings in status_bar.rs, dynamic updates | none | DONE |
| M2 | Persistent Pooled Sandbox Worker & Guardrails | Pre-warmed sandbox worker pool, wire protocol, header sniffing, EXIF fallback, prlimit & cancellation | none | DONE |
| M3 | Full GIS GeoTIFF Inspector & Overview Pipeline | Pure-Rust tiff/geotiff-core overview extractor, DEM & RGB contrast stretch, metadata & placement badge | M2 | DONE |
| M4 | Rich Format Previews (No Modal Navigation) | 3D models (STL/3MF), eBooks/comics (EPUB/CBZ/CBR), spreadsheets, audio waveforms, interactive PDF selection | M2, M3 | IN_PROGRESS |
| M5 | Final Integration, 100% E2E Pass & Tier 5 Hardening | Verify 100% E2E test pass, 181 baseline tests, Tier 5 challenger hardening, forensic integrity audit | E2E, M1, M2, M3, M4 | PLANNED |

## Interface Contracts

### Status Bar ↔ Window
- `StatusBar::update_free_space(&self, path: &Path)`: Queries GIO filesystem free space for `path` asynchronously and updates `free_space` label (`"{format_file_size(free_bytes)} free"`).
- `StatusBar::update_item_count(&self, count: usize)`: Updates item count label (`"1 item"` or `"{count} items"`).
- `StatusBar::update_selection(&self, count: usize, total_bytes: u64)`: Updates selection label (`"{count} selected, {format_file_size(total_bytes)}"`).
- `Window::setup_event_observer`: Calls `update_free_space(path)` whenever active location changes or directory events fire (`EntriesInserted`, `EntriesSpliced`, `ColumnAdded`, `ColumnsTruncated`, `ColumnReloaded`, `Reset`).

### Host Sandbox ↔ Worker Wire Protocol
- Request Transport: Unix Domain Socket (`UnixStream::pair()`).
- File Descriptors passed via SCM_RIGHTS: `[input_file_fd, output_write_pipe_fd]`.
- Request Header: 1-byte `Operation` code.
- Response Header: 8-byte little-endian framing: `[png_len: u32, metadata_len: u32]`, followed by PNG payload and metadata JSON payload.
- Error handling: Non-zero exit or broken pipe maps to `SandboxError::ProcessFailed` with graceful worker replacement.

### GeoTIFF Metadata Contract
- Metadata Payload (JSON in `result.meta` / wire protocol):
  ```json
  {
    "epsg": 32632,
    "crs_name": "WGS 84 / UTM zone 32N",
    "dimensions": [40000, 40000],
    "resolution": [10.0, 10.0],
    "bounds": [500000.0, 5100000.0, 900000.0, 5500000.0],
    "band_count": 4,
    "data_type": "MultiBand",
    "elevation_min": null,
    "elevation_max": null
  }
  ```
- Preview Drawer Integration: `PreviewContent::GeoTiff { png: Vec<u8>, metadata: GeoTiffMetadata }`.

### Rich Format Contracts (M4)
- **3D Models (STL, 3MF)**:
  - Wire Op: `Operation::PreviewModel` (or `PreviewStl`, `Preview3mf`).
  - Output: PNG rasterized thumbnail (depth-buffered isometric shaded projection, e.g. 512x512).
  - Preview variant: `PreviewContent::Rasterized(png_bytes)`.
- **eBooks & Comics (EPUB, CBZ, CBR)**:
  - Wire Op: `Operation::PreviewArchiveCover`.
  - Output: Extracted cover image (PNG or converted to PNG).
  - Preview variant: `PreviewContent::Rasterized(png_bytes)`.
- **Spreadsheets (ODS, XLS, XLSX)**:
  - Wire Op: `Operation::PreviewSpreadsheet`.
  - Output: JSON tabular structure (sheets, headers, row preview, row count) + searchable text.
  - Preview variant: `PreviewContent::Spreadsheet { table: SpreadsheetData }`.
- **Audio Waveforms (FLAC, MP3, WAV, OGG)**:
  - Wire Op: `Operation::PreviewAudioWaveform`.
  - Output: Rendered waveform PNG (e.g. amplitude peaks over time, dark theme with accent bars) + metadata (duration, sample rate, channels).
  - Preview variant: `PreviewContent::AudioWaveform { png: Vec<u8>, metadata: AudioMetadata }`.
- **Interactive PDF Selection (Upstream e816f85b)**:
  - Poppler extraction: Extract glyph bounding boxes and character text per page.
  - UI Drawer: Overlay selection rectangles, drag-to-select, Ctrl+C / context menu copy to clipboard.
  - No modal/Vim navigation mode.

### Code Layout
- `src/services/formats.rs`: Format classification (`classify_by_mime`, `classify_by_name`, `thumbnail_handler_for_name`).
- `src/services/preview.rs`: `PreviewContent` enum variants.
- `src/adapters/local_preview.rs`: Local preview provider, operation dispatching.
- `src/sandbox.rs`: Sandbox process runner, worker pool client.
- `src/sandbox/browser/`: Persistent worker pool, wire protocol, process management.
- `src/sandbox_helper.rs`: In-sandbox helper CLI and format extractors:
  - `src/sandbox_helper/geotiff.rs`: GeoTIFF overview extraction, contrast stretch, metadata extraction.
  - `src/sandbox_helper/model.rs`: STL & 3MF software rasterizer.
  - `src/sandbox_helper/archive_cover.rs`: EPUB, CBZ, CBR cover extractor.
  - `src/sandbox_helper/table.rs`: Calamine spreadsheet parser and text extractor.
  - `src/sandbox_helper/audio.rs`: Audio decoder and waveform visualizer.
  - `src/sandbox_helper/sniff.rs`: Fast image header sniffing & EXIF thumbnail fallback.
- `src/ui/status_bar.rs`: Status bar widget.
- `src/ui/window.rs`: Main window event wiring.
- `src/ui/preview.rs`: Preview drawer, GeoTIFF placement map DrawingArea, interactive PDF selection overlay.
- `src/ui/preview/pdf_text.rs`: PDF glyph layout and text selection model.
