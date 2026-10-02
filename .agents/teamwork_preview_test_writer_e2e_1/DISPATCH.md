## 2026-10-01T03:12:06Z
You are the E2E Test Suite Designer (E2E Testing Track).
Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_test_writer_e2e_1
Project root: /home/bry/.gemini/antigravity/scratch/hermes
Original request: /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md
Scope document: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/PROJECT.md

MANDATORY FIRST STEP: Read ORIGINAL_REQUEST.md and PROJECT.md.
Maintain progress.md in your working directory with a "Last visited: [timestamp]" header.

OBJECTIVE:
Design and build the comprehensive opaque-box E2E test suite for Hermes Phase 1:
1. Create TEST_INFRA.md at project root:
   - Document test philosophy (opaque-box, requirement-driven, derived from user requirements).
   - Enumerate all 14 features (F1 through F14) from PROJECT.md Feature Inventory.
   - Define test methodology: Category-Partition, Boundary Value Analysis, Pairwise Combinations, Real-World Workloads.
   - Define minimum coverage thresholds:
     - Tier 1: >=5 test cases per feature (happy path isolation).
     - Tier 2: >=5 test cases per feature (boundaries, malformed/corrupted files, empty files, size limits).
     - Tier 3: pairwise combinations of interacting features.
     - Tier 4: >=5 realistic application scenarios.
2. Implement the automated E2E test suite in tests/ (e.g. `tests/e2e_tests.rs` or modular files in `tests/e2e/`):
   - Generate test fixtures dynamically (or in `tests/fixtures/`) for:
     - GeoTIFF / COG (pyramidal IFDs, float32 DEM elevation rasters, multi-band optical RGB rasters, geospatial tags).
     - Sandbox worker (wire protocol framing, header sniffing, EXIF thumbnail fallback for >32MB/134MP, prlimit memory ceilings).
     - Rich previews (STL/3MF 3D models, EPUB/CBZ/CBR comic covers, ODS/XLS/XLSX spreadsheets, FLAC/MP3/WAV/OGG audio waveforms, interactive PDF glyph layout).
     - Status bar (free disk space query, item counts, selection totals, dynamic directory updates).
   - Ensure tests are independent of internal private functions and use entry points/APIs.
3. Ensure existing 181 unit tests are not broken.
4. When test suite infrastructure is complete, publish TEST_READY.md at project root summarizing runner commands and coverage matrix.
5. Deliver handoff.md in your working directory and notify the orchestrator.

SCOPE BOUNDARIES:
- DO NOT modify existing implementation code in src/. You only create tests in tests/, test fixtures, TEST_INFRA.md, and TEST_READY.md.
