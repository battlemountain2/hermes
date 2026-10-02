# Project Context — Hermes Preview & UI Overhaul (Phase 1)

## System & Environment
- Target OS: Linux
- Workspace: /home/bry/.gemini/antigravity/scratch/hermes
- Core Language: Rust (Cargo project)
- Security: Bubblewrap sandbox isolation (`bwrap`), Linux `prlimit`

## Requirements Summary
- R1: Full GIS GeoTIFF Inspector & Overview Pipeline (pyramids without gigapixel decompress, dynamic band/contrast stretching for float32 DEM & multi-band RGB, geospatial metadata & bounding box placement badge).
- R2: Persistent Pooled Sandbox Worker & Large-File Guardrails (pre-warmed sandboxed workers with <100ms response over wire protocol, file-header sniffing, EXIF thumbnail fallback for oversized images, prlimit memory ceilings and timeouts).
- R3: Rich Format Previews (No Modal Navigation: 3D models STL/3MF, eBooks/comics EPUB/CBZ/CBR, spreadsheets ODS/XLS/XLSX, audio waveforms FLAC/MP3/WAV/OGG, interactive PDF selection copying/highlighting).
- R4: Status Bar Completion & Disk Utilization (wire update_free_space to active directory, eliminate dead code warnings, dynamic updates on create/delete/cross-mount).
- Baseline: 181 existing tests must pass throughout.

## Architecture Guidelines
- Strict dispatch-only orchestrator: all source edits, tests, and builds must be run by subagents.
- Zero-tolerance forensic integrity: authentic logic only, no facades, no stubs.
