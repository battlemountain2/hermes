# BRIEFING — 2026-10-01T03:11:15Z

## Mission
Investigate Hermes codebase for Requirement R2: Sandbox Worker (bwrap invocation, lifecycle/overhead, persistent pool design) and Large-File Guardrails (classification/sniffing, EXIF fallback, resource limits/timeouts/cancellation).

## 🔒 My Identity
- Archetype: explorer
- Roles: investigation, synthesis
- Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_survey_2
- Original parent: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Milestone: R2 Sandbox Worker & Guardrails Survey

## 🔒 Key Constraints
- Read-only investigation — do NOT implement
- Write only to /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_survey_2/
- Produce 5-component handoff.md
- Send message to parent agent when complete

## Current Parent
- Conversation ID: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Updated: not yet

## Investigation State
- **Explored paths**:
  - `src/sandbox.rs`: Current `bwrap` invocation and `sandbox_command`.
  - `src/sandbox_helper.rs`: In-container dispatch and render functions.
  - `src/adapters/local_preview.rs`: Host preview dispatch via `gio::spawn_blocking`.
  - `src/ui/thumbnail.rs`: Thumbnail generation pipeline.
  - Upstream Strata commits `015621c0`, `028ff1b2`, `63050999`, and `e816f85b`.
  - `src/sandbox/browser.rs`, `src/sandbox/browser/wire.rs`, `src/sandbox/browser/worker.rs`, `src/sandbox/browser/process.rs`.
- **Key findings**:
  - One-shot 1-2s latency is caused by namespace creation, dynamic linking of 40MB+ binary and 50+ libs, un-cached fontconfig scans, and ImageMagick/pixbuf subprocess cascade.
  - Persistent worker pool architecture in Strata `015621c0` uses pre-warmed single-threaded supervisors inside `bwrap`, communicating over a Unix socket.
  - Uses `SCM_RIGHTS` to pass file descriptors (`input_fd`, `output_pipe_fd`), bypassing container filesystem permissions.
  - Uses disposable `fork()` with Landlock/seccomp per job to eliminate memory leaks and cross-job contamination.
  - Response framing uses 8-byte header (`png_len` + `metadata_len`) over a pipe. Latency drops to ~20-42ms.
  - Strata `63050999` implements fast dimension sniffers (JPEG SOF0, PNG IHDR, GIF) and falls back to EXIF thumbnails via `kamadak-exif` for images exceeding decoded frame budget (`width * height * 4 > FILE_SIZE_LIMIT_BYTES`), preventing SIGXFSZ/OOM.
  - `DeadlineReader` enforces 20ms poll quanta to instantly terminate cancelled workers without stalling the UI thread.
- **Unexplored areas**: None remaining for R2 survey.

## Key Decisions Made
- Fully documented all 5 survey objectives with line numbers and upstream commit references in `handoff.md`.

## Artifact Index
- DISPATCH.md — Recorded dispatch message
- BRIEFING.md — Persistent context & memory
- progress.md — Liveness heartbeat
- handoff.md — 5-component handoff report for R2
