# BRIEFING — 2026-10-01T03:30:15Z

## Mission
Investigate file-header sniffing and EXIF thumbnail fallback for Milestone 2 (R2 & F5, F6) in hermes preview pipeline.

## 🔒 My Identity
- Archetype: explorer
- Roles: investigator, synthesizer
- Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m2_2
- Original parent: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Milestone: Milestone 2 (Header Sniffing & EXIF Fallback)

## 🔒 Key Constraints
- Read-only investigation — do NOT implement / modify source code
- Files for content delivery, messages for coordination
- Self-contained 5-component handoff report

## Current Parent
- Conversation ID: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Updated: not yet

## Investigation State
- **Explored paths**:
  - `ORIGINAL_REQUEST.md` (§R2)
  - `.agents/teamwork_preview_orchestrator_1/PROJECT.md` (§F5, §F6, Code Layout §sniff.rs)
  - `.agents/teamwork_preview_explorer_survey_2/handoff.md` (§1.6, §2, §4)
  - Upstream Strata commit `6305099975207423a5a248c719d330cea6c22957` (full commit patch & tests)
  - Upstream Strata commit `015621c00c044adea2eaab948db579ca8aaa0432` (`worker.rs`, `process.rs`)
  - `src/sandbox.rs` (limits: `--fsize=33554432`, `MAX_OUTPUT_BYTES = 32 MiB`)
  - `src/sandbox_helper.rs` (current image rendering and missing sniffers)
  - `src/sandbox_helper/tests.rs`
  - `Cargo.toml`
- **Key findings**:
  - Upstream commit `63050999` solved the crash where images > 134 MP trigger SIGXFSZ due to full-resolution decode exceeding fsize limits.
  - In Hermes, `--fsize=33554432` (32 MiB) means images > 8.39 MP trigger SIGXFSZ if uncompressed raw frames are passed via tmpfs/pipe/memfd; upstream Strata raised fsize to 512 MiB (134 MP).
  - Fast sniffers: JPEG (SOF0-SOF3/SOF5-SOF7/SOF9-SOF11/SOF13-SOF15), PNG (IHDR), GIF (LSD), TIFF (IFD0 tags 0x0100 & 0x0101, supporting Standard & BigTIFF, LE & BE).
  - EXIF thumbnail fallback: Extracts JPEG thumbnail from APP1 / TIFF IFD1 tags `JPEGInterchangeFormat` (0x0201) and `JPEGInterchangeFormatLength` (0x0202) via `kamadak-exif` or direct APP1 stream slicing.
  - Guarding against malicious embedded frames: check `exceeds_decoded_frame_budget` on thumbnail itself before decoding; bound input bytes.
  - Code architecture: Implement in `src/sandbox_helper/sniff.rs` (matching `PROJECT.md`), expose to `src/sandbox_helper.rs` and `src/sandbox.rs`.
- **Unexplored areas**: none (investigation complete).

## Key Decisions Made
- Structure dimension sniffers and EXIF extraction into `src/sandbox_helper/sniff.rs` per `PROJECT.md`.
- Provide complete implementations and code snippets for JPEG, PNG, GIF, and TIFF (Standard + BigTIFF).
- Analyze both 32 MiB (Hermes current) and 512 MiB (Strata) frame budget thresholds.
- Detail unit tests with test fixtures covering oversized images, EXIF fallback, and malicious forged thumbnails.

## Artifact Index
- DISPATCH.md — record of incoming dispatch messages
- progress.md — liveness heartbeat and progress tracking
- BRIEFING.md — persistent working memory
- handoff.md — final 5-component handoff report
