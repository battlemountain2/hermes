# Original User Request

## Initial Request — 2026-10-01T03:01:23Z

Implement Phase 1 of the Hermes preview and UI overhaul: build a Full GIS GeoTIFF Inspector, a high-performance pooled sandbox worker, an expanded suite of rich file previews (3D models, comics/eBooks, spreadsheets, audio waveforms, interactive PDF selection), and complete the Status Bar disk capacity and item count integration.

Working directory: /home/bry/.gemini/antigravity/scratch/hermes
Integrity mode: development

## Requirements

### R1. Full GIS GeoTIFF Inspector & Overview Pipeline
Implement a dedicated sandboxed preview pipeline for GeoTIFF (`.tif`, `.tiff`) and Cloud Optimized GeoTIFF (COG) files:
- **Pyramid Overview Extraction:** Read internal downsampled overview levels (or generate downscaled raster views) without decompressing full-resolution gigapixel layers into memory.
- **Dynamic Band & Contrast Normalization:** Handle multi-band imagery (mapping optical bands to standard RGB) and single-band elevation models (DEM / float32) with automatic contrast stretching so raster data renders visibly instead of clipping.
- **Geospatial Metadata & Placement Badge:** Extract CRS/projection (EPSG codes), coordinate bounding box, spatial resolution, and band count into the preview header, with an embedded bounding-box map indicator showing geographical placement.

### R2. Persistent Pooled Sandbox Worker & Large-File Guardrails
Replace the one-shot `bwrap` process execution model with a high-performance persistent worker pool:
- Maintain a pool of pre-warmed sandboxed workers communicating over a local wire protocol, dropping preview response latency from ~1-2s to under 100ms.
- Implement file-header dimension and size sniffing before decode. For oversized images exceeding memory thresholds, extract and scale embedded EXIF thumbnails (adapting upstream Strata's `63050999` architecture).
- Enforce strict memory ceilings (`prlimit`), timeouts, and cancellation token responsiveness to guarantee the UI thread never locks or crashes on malformed files.

### R3. Rich Format Previews (No Modal Navigation)
Expand format classification and sandboxed preview extractors (excluding Vim/modal keyboard modes):
- **3D Models:** Render preview thumbnails for STL and 3MF geometry files.
- **eBooks & Comics:** Extract cover art and previews for EPUB, CBZ, and CBR archives.
- **Spreadsheets:** Render tabular previews and extract searchable text for ODS, XLS, and XLSX sheets.
- **Audio Waveforms:** Generate waveform visualizers for audio formats (FLAC, MP3, WAV, OGG).
- **Interactive PDF Selection:** Enable text highlighting and clipboard copying directly from the PDF preview drawer (adapting upstream `e816f85b`).

### R4. Status Bar Completion & Disk Utilization
Complete and polish the bottom status bar (`src/ui/status_bar.rs`):
- Wire `update_free_space` to active directory changes and navigation events in `src/ui/window.rs`, eliminating dead code compiler warnings.
- Display directory item counts, multi-selection aggregate sizes, and remaining filesystem capacity.
- Ensure the status bar dynamically updates on file creation, deletion, and cross-mount navigation.

## Acceptance Criteria

### GIS & Image Capabilities
- [ ] GeoTIFF files render visual overview previews without decompressing full-resolution rasters or triggering OOM/SIGXFSZ.
- [ ] Single-band DEM / float32 TIFFs display normalized terrain gradients instead of all-black or all-white rasters.
- [ ] GeoTIFF metadata (projection/CRS, dimensions, resolution, band count) and bounding-box location appear in the preview pane.

### Performance & Sandbox Security
- [ ] Warm preview renders from the pooled worker respond in under 100ms.
- [ ] Memory limit protections successfully catch oversized images and fall back to EXIF thumbnails.
- [ ] All external format parsers run strictly confined within Bubblewrap isolation.

### Extended Previews & Core Stability
- [ ] Valid STL/3MF files, EPUB/CBZ covers, spreadsheet tables, and audio waveforms render in their respective preview views.
- [ ] PDF preview allows highlighting text and copying it to the system clipboard.
- [ ] Status bar displays real-time free disk space for the active filesystem without compiler warnings.
- [ ] All 181 existing Hermes unit tests continue to pass (`cargo test`).

## Follow-up — 2026-10-01T13:16:40Z

The quota has been reset. Please resume work and complete the remaining milestones for Phase 1.
