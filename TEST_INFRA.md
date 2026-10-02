# Hermes Test Infrastructure & Methodology Specification (Phase 1)

## 1. Test Philosophy

The Hermes Phase 1 test infrastructure is built upon an **opaque-box, requirement-driven testing philosophy**.

### Principles
1. **Opaque-Box Independence**: Tests interact exclusively with system boundaries, public entry points, binary CLI arguments, Inter-Process Communication (IPC) wire protocols, and standardized filesystem/metadata contracts. Tests are strictly decoupled from private function signatures, internal struct layouts, and volatile implementation details.
2. **Authoritative Specification Derivation**: Every test case's expected behavior is derived directly from user requirements documented in `ORIGINAL_REQUEST.md` and architectural interface contracts defined in `PROJECT.md`.
3. **Progressive Testability**: Tests verify features that are implemented without breaking across development milestones. Features with pending dependencies probe capabilities gracefully, asserting contract invariants and dynamic fixture integrity during development, and executing end-to-end binary execution once merged.
4. **Self-Contained Determinism**: Tests generate isolated runtime fixtures dynamically in dedicated temporary directories (`tempfile::tempdir`), without external internet access or pre-baked binary blobs. Tests clean up resources deterministically upon completion.
5. **Regression Baseline Protection**: The existing 181 unit tests in Hermes form an immutable regression boundary that must remain 100% green across all builds.

---

## 2. Feature Inventory (F1 through F14)

Hermes Phase 1 encompasses 14 distinct functional features grouped into four core architectural pillars:

### Pillar 1: Full GIS GeoTIFF Inspector & Overview Pipeline (R1)
- **F1: GeoTIFF Pyramid Overview Extraction**
  - *Requirement*: Read internal downsampled overview levels (or generate downscaled raster views) without decompressing full-resolution gigapixel layers into memory.
  - *Contract*: Sub-sampling down to preview resolution (<1400px), multi-IFD traversal (IFD0, IFD1, ...), zero out-of-memory or SIGXFSZ faults.
- **F2: Dynamic Band & Contrast Normalization**
  - *Requirement*: Handle multi-band imagery (mapping optical bands to standard RGB) and single-band elevation models (DEM / float32) with automatic contrast stretching so raster data renders visibly instead of clipping.
  - *Contract*: 2%–98% percentile contrast stretching, NoData pixel exclusion, elevation colormap ramp (hypsometric gradient) for DEMs.
- **F3: Geospatial Metadata & Placement Badge**
  - *Requirement*: Extract CRS/projection (EPSG codes), coordinate bounding box, spatial resolution, and band count into the preview header, with an embedded bounding-box map indicator showing geographical placement.
  - *Contract*: JSON metadata payload containing `epsg`, `crs_name`, `dimensions`, `resolution`, `bounds`, and `band_count`. Cairo DrawingArea mini-map placement indicator.

### Pillar 2: Persistent Pooled Sandbox Worker & Guardrails (R2)
- **F4: Persistent Pooled Sandbox Worker**
  - *Requirement*: Maintain a pool of pre-warmed sandboxed workers communicating over a local wire protocol, dropping preview response latency from ~1-2s to under 100ms.
  - *Contract*: Unix domain socket transport (`UnixStream`), SCM_RIGHTS file descriptor passing (`[input_fd, output_fd]`), 8-byte framed wire protocol `[png_len: u32, metadata_len: u32]`.
- **F5: File-Header Sniffing & Dimension Guardrails**
  - *Requirement*: Fast dimension and size sniffing from raw file headers before decode.
  - *Contract*: Extract dimensions for JPEG (SOF0/SOF2), PNG (IHDR), GIF (LSD), and TIFF (IFD tags) within initial file bytes without decoding pixel buffers. Decoded frame budget check: `width * height * 4 > 33,554,432 bytes`.
- **F6: EXIF Thumbnail Fallback**
  - *Requirement*: For oversized images exceeding memory thresholds (>32MB / 134MP), extract and scale embedded EXIF thumbnails without allocating gigapixel frame buffers.
  - *Contract*: Parse TIFF/EXIF APP1 segment; extract IFD1 thumbnail offset and byte count; emit thumbnail preview.
- **F7: Memory Ceilings, Timeouts & Cancellation**
  - *Requirement*: Enforce strict memory ceilings (`prlimit`), timeouts, and cancellation token responsiveness to guarantee the UI thread never locks or crashes on malformed files.
  - *Contract*: `--as=1342177280` (1.25 GB), `--cpu=10` (10s), `--fsize=33554432` (32 MB). Atomic cancellation token drops child processes immediately.

### Pillar 3: Rich Format Previews (R3)
- **F8: 3D Model Previews (STL, 3MF)**
  - *Requirement*: Render preview thumbnails for STL (ASCII and binary) and 3MF geometry files.
  - *Contract*: 80-byte header binary STL parsing, ASCII `facet normal ... endfacet` parsing, 3MF ZIP archive `3D/3dmodel.model` mesh extraction; rasterization to valid PNG.
- **F9: eBook & Comic Cover Previews (EPUB, CBZ, CBR)**
  - *Requirement*: Extract cover art and previews for EPUB, CBZ, and CBR archives.
  - *Contract*: EPUB `META-INF/container.xml` -> OPF manifest cover item; CBZ first alphabetical image entry; CBR RAR container inspection.
- **F10: Spreadsheet Previews (ODS, XLS, XLSX)**
  - *Requirement*: Render tabular previews and extract searchable text for ODS, XLS, and XLSX sheets.
  - *Contract*: Calamine workbook parsing; virtual table rendering; text extraction delimiter formatted with tabs and newlines.
- **F11: Audio Waveform Visualizers (FLAC, MP3, WAV, OGG)**
  - *Requirement*: Generate waveform visualizers for audio formats.
  - *Contract*: Decode audio headers (RIFF PCM, FLAC STREAMINFO, MP3 frame sync, Ogg Vorbis/Opus); compute root-mean-square (RMS) amplitude buckets; render waveform PNG.
- **F12: Interactive PDF Text Selection & Copy**
  - *Requirement*: Enable text highlighting and clipboard copying directly from the PDF preview drawer.
  - *Contract*: Poppler glyph layout extraction (bounding boxes `[x, y, w, h]`, unicode text); selection bounding box intersection; UTF-8 clipboard copy.

### Pillar 4: Status Bar Completion & Disk Utilization (R4)
- **F13: Status Bar Free Disk Space Wiring**
  - *Requirement*: Wire `update_free_space` to active directory changes and navigation events in `src/ui/window.rs`, eliminating dead code compiler warnings.
  - *Contract*: Asynchronous GIO `query_filesystem_info_future("filesystem::free", ...)` invocation; label format `"{size} free"` with compact decimal formatting (`1.2 GB free`).
- **F14: Dynamic Status Bar Updates & Multi-Mount**
  - *Requirement*: Display directory item counts, multi-selection aggregate sizes, and remaining filesystem capacity dynamically across file creation, deletion, and cross-mount navigation.
  - *Contract*: Zero item (`"0 items"`), single item (`"1 item"`), plural (`"{N} items"`); multi-selection aggregate (`"{N} selected, {size}"`); cross-mount updates on filesystem boundaries (`/` vs `/tmp`).

---

## 3. Test Methodology

The E2E test suite applies four formal black-box software engineering testing disciplines:

### 3.1 Category-Partition Method (TSL)
For each feature, the functional domain is decomposed into independent parameters and environment conditions:
- **Inputs**: File types, internal container encodings, metadata tags, argument counts, parameter ranges.
- **Partitions**: Valid standard, valid oversized, valid multi-page/multi-band, empty, corrupted headers, truncated payloads, missing markers, permission-denied.
- **Constraints**: Pairing constraints between format families, memory budgets, and timeout thresholds.

### 3.2 Boundary Value Analysis (BVA)
Tests probe exact thresholds and boundary transitions:
- Dimension boundaries: 0x0 (invalid), 1x1 (minimal valid), 1399x1399 (standard preview), 1400x1400 (preview scale point), 11585x11585 (>134MP limit).
- Output file size limits: 0 bytes, 1 byte, 32MB - 1 byte, 32MB (`MAX_OUTPUT_BYTES`), 32MB + 1 byte (SIGXFSZ trigger).
- Wire protocol framing: 0-byte payload, 8-byte empty frame `[0, 0]`, 32MB maximum frame, partial 4-byte header truncation.
- Selection counts: 0 items, 1 item, 2 items, 1,000 items; 0 bytes, 1 byte, 1024 bytes (1.0 KB), 1,048,576 bytes (1.0 MB), 1,073,741,824 bytes (1.0 GB).

### 3.3 Pairwise Combinations (Orthogonal Testing)
Pairwise test matrices exercise interactions across modular subsystems:
- High-resolution GeoTIFF + Persistent Worker Pool + Memory Limits.
- Malformed EPUB Archive + Header Sniffer + Sandbox Worker Timeout.
- Large Multi-Selection + Cross-Mount Directory Navigation + Rapid Status Bar Updates.
- Oversized Image + EXIF Thumbnail Fallback + Wire Protocol Framing.
- Non-UTF8 File Paths + Special Characters + Sandbox Argument Passing.

### 3.4 Real-World Workloads
Realistic user workflows representing complete user sessions:
- **Scenario 1: GIS Data Ingestion Workflow**: User browses a folder containing multi-spectral Landsat TIFFs and single-band SRTM DEMs. Checks overview render, verifies CRS badge, inspects coordinate bounds.
- **Scenario 2: CAD / 3D Asset Management**: User navigates directory containing mixed binary STL, ASCII STL, and zipped 3MF models. Inspects generated previews under sandbox constraints.
- **Scenario 3: Media Archive & Comic Ingestion**: User views folders with CBR/CBZ comics, EPUB books, and audio files. Generates covers and audio waveforms in rapid succession.
- **Scenario 4: Office Spreadsheet Analysis**: User opens and previews ODS, XLS, and XLSX workbooks, extracts searchable text content, and verifies tabular structure.
- **Scenario 5: Batch File Management & Filesystem Navigation**: User selects 50 mixed files, copies them across different filesystem mounts, checks live status bar capacity updates, item count updates, and aggregate selection sizes.

---

## 4. Minimum Coverage Thresholds

| Test Tier | Scope & Focus | Minimum Threshold |
|-----------|---------------|-------------------|
| **Tier 1** | Happy Path Isolation | $\ge 5$ test cases per feature ($14 \times 5 = \ge 70$ tests) |
| **Tier 2** | Boundaries, Malformed & Edge Cases | $\ge 5$ test cases per feature ($14 \times 5 = \ge 70$ tests) |
| **Tier 3** | Pairwise Interacting Combinations | $\ge 20$ interaction combinations across features |
| **Tier 4** | Real-World Application Scenarios | $\ge 5$ comprehensive end-to-end user workflows |
| **Baseline**| Existing Regression Suite | 181 / 181 unit tests passing |

---

## 5. Test Suite Architecture in `tests/`

```
tests/
├── common/
│   ├── mod.rs             # Subprocess runner, CLI invocation harness, temporary directories
│   ├── wire_protocol.rs   # Wire protocol encoder, 8-byte framing parser, stream mocks
│   ├── sniffer.rs         # Fast image dimension sniffers, frame budget evaluator
│   └── status_bar.rs      # Status bar string formatting & GIO filesystem probe helpers
├── fixtures/
│   ├── mod.rs             # Fixture factory entry points
│   ├── geotiff.rs         # Dynamic GeoTIFF, COG pyramidal IFD, Float32 DEM generators
│   ├── models.rs          # ASCII STL, Binary STL, 3MF ZIP archive generators
│   ├── archives.rs        # EPUB (container.xml/OPF), CBZ, CBR archive generators
│   ├── spreadsheets.rs    # ODS, XLSX, XLS spreadsheet workbook generators
│   ├── audio.rs           # WAV (PCM), FLAC, MP3, OGG audio stream generators
│   └── pdf.rs             # Multi-glyph text PDF stream generators
├── tier1_isolated.rs      # Tier 1 tests: F1-F14 happy paths (70 tests)
├── tier2_boundaries.rs    # Tier 2 tests: F1-F14 boundary & adversarial cases (70 tests)
├── tier3_pairwise.rs      # Tier 3 tests: Interacting pairwise combinations (20 tests)
├── tier4_scenarios.rs     # Tier 4 tests: Real-world end-to-end scenarios (5 tests)
└── e2e_tests.rs           # Root integration test crate unifying all tiers
```

---

## 6. Execution Instructions

Run full E2E test suite:
```bash
cargo test --test e2e_tests
```

Run specific tiers:
```bash
cargo test --test e2e_tests tier1
cargo test --test e2e_tests tier2
cargo test --test e2e_tests tier3
cargo test --test e2e_tests tier4
```

Verify regression baseline:
```bash
cargo test --bin strata
```
