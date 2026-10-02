## 2026-10-01T14:06:47Z
<USER_REQUEST>
You are Challenger M3.1: GeoTIFF Pyramid & DEM Challenger.
Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_challenger_m3_1
Project root: /home/bry/.gemini/antigravity/scratch/hermes
Original request: /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md
Scope document: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/PROJECT.md
Worker handoff: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_worker_m3_1/handoff.md

MANDATORY FIRST STEP: Read /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md.
Maintain progress.md in your working directory with a "Last visited: [timestamp]" header.

OBJECTIVE:
Empirically stress test the GeoTIFF pyramid overview extraction and DEM/optical normalization pipeline:
1. Write and execute empirical stress tests in your working directory (or using cargo test harnesses):
   - Multi-IFD COGs with diverse pyramid levels (1024x1024, 2048x2048, 4096x4096): verify the selected overview matches the specification (smallest in [1200, 2048] or largest <= 2048).
   - Flat single-layer gigapixel rasters (e.g. 5000x5000): verify subsampling stride and verify memory remains bounded <= 10MB without triggering sandbox OOM or SIGXFSZ.
   - Float32 DEMs with NoData sentinels (-9999.0, NaN, Inf, < -9000.0): verify generated PNG has non-zero contrast variance (> 10) and NoData pixels are transparent RGBA [0,0,0,0].
   - Flat terrain DEMs where min == max (P2 == P98): verify no division-by-zero or NaN pixel outputs.
   - 4-band optical rasters with NIR = 0: verify NIR is discarded and alpha = 255 across all pixels.
2. Confirm all 216 unit tests and 165 E2E tests pass.
3. Write structured handoff report at `/home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_challenger_m3_1/handoff.md` with explicit verdict: `APPROVE` or `REQUEST_CHANGES`. Notify orchestrator via send_message.
</USER_REQUEST>
