## 2026-10-01T13:33:45Z
You are Explorer M3.3: Geospatial Metadata & GTK4 UI Explorer.
Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m3_3
Project root: /home/bry/.gemini/antigravity/scratch/hermes
Original request: /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md
Scope document: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/PROJECT.md
Survey handoff: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_survey_1/handoff.md

MANDATORY FIRST STEP: Read /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md.
Maintain progress.md in your working directory with a "Last visited: [timestamp]" header.

OBJECTIVE:
Investigate and design the Geospatial Metadata extraction and GTK4 UI integration for Requirement R1 (F3):
1. Geospatial metadata extraction via `geotiff-core = "0.8.1"`:
   - Extract `ModelPixelScaleTag` (33550), `ModelTiepointTag` (33922), `GeoKeyDirectoryTag` (34735).
   - Parse ProjectedCSTypeGeoKey (3072) / GeographicTypeGeoKey (2048) into EPSG codes.
   - Calculate bounding box [min_x, min_y, max_x, max_y] and pixel resolution [res_x, res_y].
   - Serialize into `result.meta` JSON matching the GeoTIFF Metadata Contract in PROJECT.md.
2. Preview subsystem integration:
   - `src/services/formats.rs`: how `.tif` / `.tiff` are classified and routed.
   - `src/services/preview.rs`: define `GeoTiffMetadata` struct and add `PreviewContent::GeoTiff { png: Vec<u8>, metadata: GeoTiffMetadata }`.
   - `src/adapters/local_preview.rs`: parse `result.meta` JSON and emit `PreviewContent::GeoTiff`.
3. GTK4 UI in `src/ui/preview.rs`:
   - Display GIS metadata badge row: CRS/EPSG, Resolution, Bounding Box, Band Type.
   - Render Cairo `gtk::DrawingArea` placement map indicator: 160x90 widget with world/grid outline and highlighted bounding box showing spatial footprint.
4. Detail exact struct definitions, UI layout, and file changes.

SCOPE BOUNDARIES:
- Read-only exploration. DO NOT modify any source code files.
- Produce a structured handoff report at: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m3_3/handoff.md following standard handoff structure (Observation, Logic Chain, Caveats, Conclusion, Verification Method).
- When finished, send a message to orchestrator with summary and report path.
