# BRIEFING — 2026-10-01T03:11:00Z

## Mission
Investigate the existing Hermes codebase for Requirement R1 (Full GIS GeoTIFF Inspector & Overview Pipeline) and produce a comprehensive architecture handoff report.

## 🔒 My Identity
- Archetype: explorer
- Roles: [explorer, synthesis]
- Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_survey_1
- Original parent: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Milestone: Requirement R1 (Full GIS GeoTIFF Inspector & Overview Pipeline) Architecture Survey

## 🔒 Key Constraints
- Read-only investigation — do NOT implement
- Do NOT modify any source code files
- Communication Guideline: files for content delivery, messages for coordination
- Handoff Report with 5 components (Observation, Logic Chain, Caveats, Conclusion, Verification Method)

## Current Parent
- Conversation ID: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Updated: 2026-10-01T03:11:00Z

## Investigation State
- **Explored paths**:
  - `ORIGINAL_REQUEST.md` (R1 requirements & acceptance criteria)
  - `src/sandbox.rs` (bwrap execution, limits: 1.25GB AS, 32MB fsize, ParseOperation)
  - `src/sandbox_helper.rs` (render_image, render_imagemagick, render_pixbuf)
  - `src/services/formats.rs` (format registry, FormatFamily::Image, PreviewHandler::Image)
  - `src/services/preview.rs` (PreviewRequest, PreviewContent, PreviewProvider)
  - `src/adapters/local_preview.rs` (LocalPreviewProvider, gio::spawn_blocking)
  - `src/ui/preview.rs` (PreviewDrawer, ScrolledWindow, Picture, pan/zoom)
  - `src/ui/settings.rs` (gtk::DrawingArea Cairo drawing reference)
  - `Cargo.toml` (crate dependencies and strict deny lints)
  - `~/.cargo/registry/src/index.crates.io-*/geotiff-core-0.8.1/` (pure Rust zero-dep GeoTIFF metadata & transform parser)
  - `~/.cargo/registry/src/index.crates.io-*/tiff-0.11.3/` (pure Rust multi-IFD overview seeking & tag reader)
- **Key findings**:
  - Currently TIFF files are treated as standard raster images via ImageMagick/Pixbuf in sandbox helper.
  - Full gigapixel GeoTIFFs fail with OOM (1.25GB address space limit) or SIGXFSZ (32MB file size limit) due to full-resolution decompress/temp spilling.
  - Single-band float32 DEMs fail or display all-black/all-white due to lack of contrast stretching and unhandled NoData values (-9999.0).
  - Multi-band satellite imagery (e.g. 4-band RGB+NIR) fails or renders incorrectly because NIR is treated as alpha or 16-bit reflectance is clamped.
  - Pure Rust crates `tiff = "0.11"` and `geotiff-core = "0.8.1"` (zero dependencies) provide complete IFD overview seeking, float32 decoding, and EPSG/bounding-box/resolution calculation without needing external GDAL.
  - Cairo and GTK4 DrawingArea are already present for map indicator and badge rendering.
- **Unexplored areas**: None for R1 survey scope.

## Key Decisions Made
- Recommending `tiff = "0.11"` + `geotiff-core = "0.8.1"` as the lightweight, pure-Rust decoding and metadata pipeline inside the sandbox helper.
- Passing structured GeoTIFF metadata through `result.meta` from sandbox helper to Hermes UI.
- Rendering geographical placement map indicator via Cairo DrawingArea in `src/ui/preview.rs`.

## Artifact Index
- DISPATCH.md — Dispatch log
- progress.md — Liveness heartbeat
- handoff.md — Final handoff report
