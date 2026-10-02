# BRIEFING — 2026-10-01T13:42:30Z

## Mission
Investigate and design Geospatial Metadata extraction (via geotiff-core 0.8.1) and GTK4 UI integration (badge row + Cairo placement map indicator) for Requirement R1 (F3) in Hermes.

## 🔒 My Identity
- Archetype: explorer
- Roles: Geospatial Metadata & GTK4 UI Explorer (M3.3)
- Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m3_3
- Original parent: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Milestone: M3.3 (Geospatial Metadata & GTK4 UI)

## 🔒 Key Constraints
- Read-only investigation — do NOT implement / modify source code
- Produce 5-component handoff report at `.agents/teamwork_preview_explorer_m3_3/handoff.md`
- Maintain progress.md with "Last visited: [timestamp]"
- Exact struct definitions, UI layout, Cairo drawing specs, and file changes

## Current Parent
- Conversation ID: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Updated: not yet

## Investigation State
- **Explored paths**:
  - `ORIGINAL_REQUEST.md`, `PROJECT.md`, survey `handoff.md`
  - `src/services/formats.rs`, `src/services/formats/tests.rs`
  - `src/services/preview.rs`, `src/adapters/local_preview.rs`
  - `src/sandbox.rs`, `src/sandbox/browser.rs`, `src/sandbox/browser/wire.rs`, `src/sandbox/browser/worker.rs`
  - `src/sandbox_helper.rs`, `src/sandbox_helper/sniff.rs`
  - `src/ui/preview.rs`, `src/ui/settings.rs`, `src/ui/theme.rs`, `src/style.css`
  - `tests/fixtures/geotiff.rs`, `tests/e2e/tier1_isolated.rs`, `tests/e2e/tier2_boundaries.rs`, `tests/e2e/tier3_pairwise.rs`, `tests/e2e/tier4_scenarios.rs`
- **Key findings**:
  - GeoTIFF tags (33550 pixel scale, 33922 tiepoint, 34735 GeoKey directory) can be extracted into structured JSON metadata (`epsg`, `crs_name`, `dimensions`, `resolution`, `bounds`, `band_count`, `data_type`, `elevation_min`, `elevation_max`).
  - Wire protocol in M2 already passes `metadata: Vec<u8>` framed with PNG. Extending `ParseOutput` to retain metadata allows zero-copy JSON passing from both worker pool and one-shot sandbox.
  - Cairo 160x90 `DrawingArea` renders rounded background, world graticule, continental silhouettes, and coordinate-projected footprint highlight with division-by-zero, non-finite, and minimum-size (4x4) guards.
  - UI layout combines GIS badge rows with the placement map in a dedicated header above the pannable/zoomable texture viewport.
- **Unexplored areas**: None. All components fully mapped.

## Key Decisions Made
- `FormatFamily::GeoTiff` and `PreviewHandler::GeoTiff` route `.tif` and `.tiff` through `ParseOperation::PreviewGeoTiff`.
- `PreviewContent::GeoTiff { png: Vec<u8>, metadata: GeoTiffMetadata }` added to `PreviewContent`.
- Coordinate reprojection for UTM zones (32601..32660, 32701..32760) and Web Mercator (3857) implemented via pure-Rust closed-form spherical/ellipsoidal approximations for the Cairo map footprint.

## Artifact Index
- DISPATCH.md — Task assignment
- BRIEFING.md — Situational awareness
- progress.md — Liveness & status tracking
- handoff.md — Final structured report
