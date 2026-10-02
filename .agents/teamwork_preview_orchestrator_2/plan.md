# Execution Plan — Project Orchestrator Gen 2 (Milestone 4 & Final Integration)

## Objective
Deliver Milestone 4 (R3: Rich Format Previews — 3D Models, eBooks/Comics, Spreadsheets, Audio Waveforms, Interactive PDF Selection), complete M4 quality gate, execute Phase 2 integration and forensic audit, and report project completion to Sentinel.

---

## Phase 1: Orchestrator Gen 2 Initialization & M3 Gate Verification
- [x] Create `DISPATCH.md` with parent prompt.
- [x] Create `BRIEFING.md` with persistent memory and configuration.
- [x] Start heartbeat cron task (`schedule`).
- [x] Adapt `PROJECT.md` from Gen 1.
- [x] Create `GATE_STATUS.md` recording M1, M2, and M3 gate closure.
- [x] Create `plan.md`, `progress.md`, and `context.md`.

---

## Phase 2: Milestone 4 (Rich Format Previews — R3)
### Features in Scope:
1. **3D Models**: STL (ASCII/binary) & 3MF software rasterizer thumbnail preview (`src/sandbox_helper/model.rs`).
2. **eBooks & Comics**: Cover art extraction for EPUB, CBZ, CBR (`src/sandbox_helper/archive_cover.rs`).
3. **Spreadsheets**: ODS, XLS, XLSX tabular parsing via `calamine`, virtual table view and searchable text extraction (`src/sandbox_helper/table.rs`, `src/ui/table_view.rs`).
4. **Audio Waveforms**: Waveform visualizer generation for FLAC, MP3, WAV, OGG.
5. **Interactive PDF Selection**: Poppler glyph layout extraction, highlight overlays, and clipboard copying (adapting commit `e816f85b`). (Note: No modal/Vim keyboard navigation).

### Iteration Loop:
1. **Exploration**:
   - Spawn 3 Explorers (`teamwork_preview_explorer`) to investigate:
     - Explorer M4.1: 3D Models (STL/3MF) & eBooks/Comics (EPUB/CBZ/CBR) architecture and reference commits `570bdd30`, `ba676d3e`.
     - Explorer M4.2: Spreadsheets (`calamine` ODS/XLS/XLSX) & Audio Waveforms (FLAC/MP3/WAV/OGG) and reference commits `d39052be`, `40a5a606`.
     - Explorer M4.3: Interactive PDF text selection & clipboard copying (reference commit `e816f85b`) and UI drawer integration.
2. **Implementation**:
   - Spawn Worker M4 (`teamwork_preview_worker`) with Explorer findings, reference commits, and MANDATORY INTEGRITY WARNING.
   - Worker implements features, updates Cargo.toml dependencies, and verifies `cargo check` and unit tests.
3. **Review & Challenge**:
   - Spawn 2 Reviewers (`teamwork_preview_reviewer`) independently.
   - Spawn 2 Challengers (`teamwork_preview_challenger`) with empirical stress testing suites.
4. **Forensic Audit**:
   - Spawn Forensic Auditor (`teamwork_preview_auditor`) for M4 code.
5. **Gate 4 Evaluation**:
   - Check all criteria (Pass build/tests, 2x APPROVE, 2x APPROVE, 1x CLEAN). Update `GATE_STATUS.md`.

---

## Phase 3: Final Integration & Hardening (Phase 2 Milestone)
- [ ] Run full test suite across all milestones (181 baseline tests, all strata unit tests, 165 E2E tests, challenger stress suites).
- [ ] Verify 0 compiler warnings.
- [ ] Run final project-wide forensic audit (`teamwork_preview_auditor`).
- [ ] Document final results and metrics.

---

## Phase 4: Victory Report to Sentinel
- [ ] Synthesize all accomplishments, test counts, performance metrics, and audit verdicts.
- [ ] Send victory message via `send_message` to Sentinel (`de5db9f9-e8e0-4efe-98aa-deaf92fdf84f`).
