# BRIEFING — 2026-10-01T13:34:00Z

## Mission
Investigate and design the GeoTIFF Pyramid Overview Extraction and Gigapixel Guardrail engine for Requirement R1 (F1), verifying tiff crate APIs, overview selection algorithm, subsampling fallback, and module architecture.

## 🔒 My Identity
- Archetype: Explorer
- Roles: Teamwork explorer (read-only investigation, synthesis, structured handoff)
- Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m3_1
- Original parent: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Milestone: M3.1

## 🔒 Key Constraints
- Read-only investigation — do NOT implement
- Inspect pure-Rust `tiff = "0.11"` crate (APIs for multi-IFD navigation, dimensions, seeking, decoding)
- Design overview selection algorithm (1400x1400 target preview box, COG/pyramid IFD traversal)
- Design subsampling fallback for flat raster > 2048x2048 under 1.25GB / 32MB sandbox ceilings
- Detail exact function signature and module structure for `src/sandbox_helper/geotiff.rs`

## Current Parent
- Conversation ID: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Updated: 2026-10-01T13:34:00Z

## Investigation State
- **Explored paths**:
  - `~/.cargo/registry/src/.../tiff-0.11.3/`: `src/lib.rs`, `src/decoder/mod.rs`, `src/decoder/image.rs`, `src/tags.rs`, `tests/decode_geotiff_images.rs`
  - `src/sandbox_helper.rs`, `src/sandbox_helper/sniff.rs`, `src/sandbox.rs`, `src/sandbox/browser/worker.rs`, `src/sandbox/browser/wire.rs`
  - `tests/fixtures/geotiff.rs`, `tests/e2e/tier1_isolated.rs`, `tests/e2e/tier2_boundaries.rs`, `tests/e2e/tier3_pairwise.rs`
- **Key findings**:
  - `tiff::decoder::Decoder::new()` and `next_image()` parse only IFD directory tags (`Image::from_reader`), performing zero pixel memory allocations.
  - Multi-IFD navigation via `seek_to_image(i)` allows fast random access across all IFDs.
  - Overview selection chooses smallest overview with `max(w, h) >= 1200`, or largest overview `<= 2048` if all smaller, or direct IFD 0 if `<= 2048`.
  - Subsampling fallback uses strided chunk/strip sampling ($S = \lceil \max(W, H) / 1400.0 \rceil$) with peak memory $\approx 8\text{--}10\text{ MB}$, well within 32MB / 1.25GB limit.
  - `tiff::decoder::Limits` enforces an ironclad 32MB limit on buffer allocation.
- **Unexplored areas**: Downstream M3.2 (dynamic contrast stretch / DEM hypsometric mapping) and M3.3 (EPSG / GeoKey extraction & Cairo mini-map badge).

## Key Decisions Made
- Confirmed pure-Rust `tiff = "0.11"` native support for GeoTIFF tags (`ModelPixelScaleTag`, `ModelTiepointTag`, `GeoKeyDirectoryTag`, `GdalNodata`).
- Overview selection algorithm strictly isolates pixel decompression to selected overview level, never allocating IFD 0 gigapixel rasters.
- Subsampling fallback decodes only chunks intersecting sampled points $(x_{src}, y_{src}) = (x_{out} \cdot S, y_{out} \cdot S)$.
- Designed complete module interface and signatures for `src/sandbox_helper/geotiff.rs`.

## Artifact Index
- DISPATCH.md — Initial dispatch message
- BRIEFING.md — Working memory and context index
- progress.md — Liveness heartbeat and progress tracking
- handoff.md — Final 5-component handoff report
