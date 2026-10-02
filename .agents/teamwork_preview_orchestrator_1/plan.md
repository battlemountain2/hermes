# Concrete Plan — Hermes Preview & UI Overhaul (Phase 1)

## Overview
Hermes is a terminal/graphical file manager in Rust. Phase 1 enhances preview capabilities (GeoTIFF GIS inspector, pooled sandbox worker, rich preview suite, status bar completion).

## Phases & Steps

### Phase 0: Survey & Assessment
- [ ] Dispatch 3 parallel Explorers:
  - Explorer 1 (GIS & Sandbox Architecture): Inspect current preview pipeline (`src/preview/`, bubblewrap integration, worker process model, image decode, TIFF handling).
  - Explorer 2 (Rich Previews & PDF Interaction): Inspect existing format detectors, rendering widgets, PDF preview drawer, selection/clipboard handling, 3D, comic/epub, spreadsheet, audio waveform infrastructure.
  - Explorer 3 (Status Bar & Core Stability & Test Suite): Inspect `src/ui/status_bar.rs`, `src/ui/window.rs`, `update_free_space`, compiler warnings, and examine existing 181 unit tests (`cargo test`).
- [ ] Synthesize findings into `PROJECT.md` (Feature Inventory, Architecture, Code Layout, Interfaces, Milestone Definitions).

### Phase 1: Dual Track Launch
- [ ] Track A (E2E Testing Track): Dispatch E2E Testing Orchestrator / Test Writer to establish `TEST_INFRA.md` and design test suites (Tier 1-4).
- [ ] Track B (Implementation Track):
  - Milestone 1 (M1): Persistent Pooled Sandbox Worker & Large-File Guardrails (R2)
  - Milestone 2 (M2): Full GIS GeoTIFF Inspector & Overview Pipeline (R1)
  - Milestone 3 (M3): Rich Format Previews (No Modal Navigation: 3D STL/3MF, eBooks/comics EPUB/CBZ/CBR, spreadsheets ODS/XLS/XLSX, audio waveforms FLAC/MP3/WAV/OGG, interactive PDF selection) (R3)
  - Milestone 4 (M4): Status Bar Completion & Disk Utilization (R4)

### Phase 2: Final Milestone (Integration, E2E Pass & Hardening)
- [ ] Verify 100% of E2E tests pass (Tiers 1-4).
- [ ] Ensure all 181 existing tests pass without regressions.
- [ ] Adversarial Coverage Hardening (Tier 5): Challenger initiates coverage audit and stress tests.
- [ ] Forensic Integrity Audit (teamwork_preview_auditor) — check against cheats, stubs, facades, and verify authentic logic.

### Phase 3: Reporting & Completion
- [ ] Prepare final human report and completion notification to Sentinel.
