# BRIEFING — 2026-10-01T19:05:00Z

## Mission
Investigate technical strategy, requirements, wire protocol schemas, formats, UI integration, and dependency impact for Spreadsheets (ODS, XLS, XLSX) and Audio Waveforms (FLAC, MP3, WAV, OGG) in Hermes.

## 🔒 My Identity
- Archetype: explorer
- Roles: investigator, synthesizer
- Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m4_2
- Original parent: 9b43929e-5ad2-48d5-baa5-b900ff030792
- Milestone: M4.2 (Spreadsheet & Audio Explorer)

## 🔒 Key Constraints
- Read-only investigation — do NOT implement
- Inspect commits d39052be, 40a5a606, 24857a55
- Evaluate calamine for ODS, XLS, XLSX preview & text extraction
- Evaluate FFmpeg options (showwaves / showwavespic) inside sandbox for FLAC, MP3, WAV, OGG
- Memory bounds, error handling for corrupted files
- Wire protocol schemas, format classification in formats.rs, UI integration in preview.rs & table_view.rs

## Current Parent
- Conversation ID: 9b43929e-5ad2-48d5-baa5-b900ff030792
- Updated: not yet

## Investigation State
- **Explored paths**:
  - `git show --stat d39052be`, `src/services/table.rs`, `src/ui/table_view.rs`, `src/sandbox_helper.rs`, `src/adapters/local_preview.rs`
  - `git show 40a5a606`, `git show 24857a55`, `src/ui/media.rs`, `src/ui/preview/waveform.rs`
  - `ffmpeg -h filter=showwavespic`, FFmpeg pipeline tests on RIFF WAV / FLAC / MP3 / OGG
  - `src/sandbox/browser.rs`, `src/sandbox/browser/wire.rs`, `src/sandbox/browser/worker.rs`
  - `src/services/formats.rs`, `src/services/preview.rs`, `src/ui/preview.rs`
  - `tests/e2e/`, `tests/fixtures/spreadsheets.rs`, `tests/fixtures/audio.rs`
- **Key findings**:
  - `calamine` v0.36.1 is pure-Rust, handles ODS, XLS, XLSX, XLSB out of the box with zero system dependencies.
  - Tabular view requires `src/ui/table_view.rs` with `gtk::ColumnView` + `gtk::SortListModel` + `gtk::CustomSorter` + `SortKey` for virtualized fast rendering.
  - Audio waveform rendering inside sandbox is cleanest via native FFmpeg `showwavespic` filter (`s=800x240:colors=0x7aa2f7:filter=peak:scale=cbrt`), outputting RGBA PNG with alpha transparency in <60ms.
  - Live audio spectrum visualization (`SoundCloudWaveform` from `24857a55`) can be coupled in UI for active playback.
  - Wire protocol operations: `Operation::PreviewSpreadsheet` (14) and `Operation::PreviewAudioWaveform` (15).
  - Memory bounds: strict `prlimit` ceilings (1.25GB AS, 10s CPU, 32MB fsize), 20MB file limit for spreadsheets, 200 row / 256 col / 4MB text preview budgets.
- **Unexplored areas**: None. Ready for comprehensive report synthesis.

## Key Decisions Made
- Confirmed `calamine = "0.36.1"` and `csv = "1.4.0"` as optimal Cargo dependencies.
- Confirmed FFmpeg `showwavespic` as optimal sandboxed waveform generator.
- Designed complete wire protocol, format classification, and GTK4 UI integration schemas.

## Artifact Index
- DISPATCH.md — Recorded dispatch instructions
- BRIEFING.md — Situational awareness
- progress.md — Liveness & status tracking
- handoff.md — Final investigation report
