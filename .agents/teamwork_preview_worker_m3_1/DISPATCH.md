## 2026-10-01T13:43:26Z
You are Worker M3: GeoTIFF Pipeline Worker.
Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_worker_m3_1
Project root: /home/bry/.gemini/antigravity/scratch/hermes
Original request: /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md
Scope document: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/PROJECT.md

MANDATORY FIRST STEP: Read /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md.
Maintain progress.md in your working directory with a "Last visited: [timestamp]" header.

MANDATORY INTEGRITY WARNING:
DO NOT CHEAT. All implementations must be genuine. DO NOT hardcode test results, create dummy/facade implementations, or circumvent the intended task. A teamwork_preview_auditor will independently verify your work. Integrity violations WILL be detected and your work WILL be rejected.

BLUEPRINT SPECIFICATIONS (READ ALL THREE EXPLORER REPORTS FIRST):
1. /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m3_1/handoff.md (Pyramid Overview & Subsampling Engine)
2. /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m3_2/handoff.md (Dynamic Band & DEM Contrast Normalization)
3. /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m3_3/handoff.md (Geospatial Metadata & GTK4 UI Integration)

OBJECTIVE & REQUIREMENTS (R1, Features F1, F2, F3):
Implement the complete GIS GeoTIFF preview pipeline:
1. `Cargo.toml`:
   - Add pure-Rust dependencies:
     ```toml
     tiff = "0.11"
     geotiff-core = "0.8.1"
     serde_json = "1.0"
     ```
2. `src/sandbox_helper/geotiff.rs` (new module):
   - Overview extraction engine:
     - Scan IFD headers via `decoder.seek_to_image(i)` without allocating pixel data.
     - Select smallest overview in [1200, 2048] or largest overview <= 2048.
     - If flat raster <= 2048, decode IFD 0 directly.
     - If flat raster > 2048 (gigapixel), implement strided subsampling (stride S = ceil(max(W,H)/1400.0)), decoding only chunks/strips intersecting the grid. Total memory allocation must remain <= 10MB (under 32MB / 1.25GB sandbox limits).
   - DEM Dynamic Contrast Normalization:
     - Detect NoData via tag 42113 (GDAL_NODATA) and sentinels (-9999.0, -32767.0, NaN, Inf, < -9000.0).
     - Compute 2nd and 98th percentiles (P2, P98) across valid elevation pixels using uniform strided sampling.
     - Contrast stretching: clamp((z - P2)/(P98 - P2), 0.0, 1.0).
     - Apply hypsometric terrain color ramp (#2d6a4f -> #d4a373 -> #6c584c -> #f8f9fa) or grayscale.
     - Set NoData pixels to transparent RGBA(0, 0, 0, 0).
   - Multi-Band Optical Imagery:
     - Map Band 0 -> Red, Band 1 -> Green, Band 2 -> Blue. Discard Band 3+ (NIR/QA) so NIR is not treated as alpha.
     - Per-channel 2%-98% percentile stretching to expand 16-bit satellite reflectance to 8-bit RGB without clipping.
     - Set Alpha = 255 for all valid optical pixels.
   - Safe Cairo ImageSurface PNG encoding:
     - Use `cairo::ImageSurface::create_for_data` with owned `Vec<u8>` (100% safe Rust, `#![deny(unsafe_code)]`).
     - ARgb32 native-endian word packing via `(A << 24) | (R << 16) | (G << 8) | B` and `u32::to_ne_bytes()`.
   - Geospatial Metadata Extraction:
     - Extract `ModelPixelScaleTag` (33550), `ModelTiepointTag` (33922), `GeoKeyDirectoryTag` (34735).
     - Parse ProjectedCSTypeGeoKey (3072) / GeographicTypeGeoKey (2048) into EPSG codes. Map to standard CRS names.
     - Compute bounds [min_x, min_y, max_x, max_y] with min/max normalization against inverted bounds.
     - Compute resolution [res_x, res_y] with zero-division fallback (default [1.0, 1.0]).
     - Serialize to `result.meta` JSON conforming to GeoTIFF Metadata Contract in PROJECT.md.
   - Include standalone unit tests at the bottom of `geotiff.rs`.
3. `src/sandbox_helper.rs`:
   - Register `pub(crate) mod geotiff;`
   - Wire `"preview-geotiff"` operation calling `geotiff::render_geotiff(input, value.max(1))?` and writing PNG + `result.meta`.
4. `src/services/formats.rs`:
   - Add `FormatFamily::GeoTiff` and `PreviewHandler::GeoTiff`.
   - Map `.tif` / `.tiff` and `image/tiff` MIME to `FormatFamily::GeoTiff`.
   - Ensure existing format classification tests continue to pass.
5. `src/services/preview.rs`:
   - Define `GeoTiffMetadata` struct (derived with Serialize, Deserialize, Clone, Debug, PartialEq).
   - Add `PreviewContent::GeoTiff { png: Vec<u8>, metadata: GeoTiffMetadata }`.
6. `src/sandbox.rs`, `src/sandbox/browser.rs`, `src/sandbox/browser/wire.rs`, `src/sandbox/browser/worker.rs`:
   - Add `ParseOperation::PreviewGeoTiff` and `Operation::PreviewGeoTiff`.
   - Extend `ParseOutput` to hold `metadata: Option<Vec<u8>>`.
   - Wire worker pool response metadata through to `ParseOutput`.
7. `src/adapters/local_preview.rs`:
   - Map `PreviewHandler::GeoTiff` to `ParseOperation::PreviewGeoTiff`.
   - On `ParseOperation::PreviewGeoTiff`, parse `output.metadata` with `serde_json` into `GeoTiffMetadata` and emit `PreviewContent::GeoTiff` (or fallback to `Rasterized`).
8. `src/ui/preview.rs`:
   - Render `PreviewContent::GeoTiff`:
     - 2-row GIS metadata badges (CRS/EPSG, Resolution, Data Type, Bounds, Dimensions, Elevation Range).
     - 160x90 Cairo `DrawingArea` placement map indicator with dark theme card, graticule, continental outlines, coordinate reprojection (UTM/Web Mercator/WGS84), and highlighted bounding box with `.max(4.0)` zero-division / zero-area guards.
     - Zoomable and pannable image view for raster texture.

EXCLUSIVE FILE OWNERSHIP:
- Cargo.toml
- src/sandbox_helper/geotiff.rs
- src/sandbox_helper.rs
- src/services/formats.rs
- src/services/formats/tests.rs
- src/services/preview.rs
- src/sandbox.rs
- src/sandbox/browser/wire.rs
- src/sandbox/browser.rs
- src/sandbox/browser/worker.rs
- src/adapters/local_preview.rs
- src/ui/preview.rs

VERIFICATION CRITERIA (run all and document in handoff.md):
1. `cargo check` (0 warnings in owned files)
2. `cargo test --bin strata` (all 204 existing strata unit tests + new tests must pass)
3. `cargo test --test e2e_tests` (all 165 E2E tests must pass, especially `test_f1`, `test_f2`, `test_f3`)
4. Output structured handoff report to: `/home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_worker_m3_1/handoff.md` and send completion message to orchestrator.
