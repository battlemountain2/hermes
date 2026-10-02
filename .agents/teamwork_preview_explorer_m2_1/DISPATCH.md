## 2026-10-01T03:29:53Z

<USER_REQUEST>
You are Explorer M2.1: Worker Pool & Wire Protocol Explorer.
Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m2_1
Project root: /home/bry/.gemini/antigravity/scratch/hermes
Original request: /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md
Scope document: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/PROJECT.md
Survey handoff: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_survey_2/handoff.md

MANDATORY FIRST STEP: Read ORIGINAL_REQUEST.md and PROJECT.md.
Maintain progress.md in your working directory with a "Last visited: [timestamp]" header.

OBJECTIVE:
Investigate the persistent pooled sandbox worker and wire protocol architecture for Milestone 2 (R2 & F4):
1. Examine git history for upstream Strata commit `015621c00c044adea2eaab948db579ca8aaa0432` and review commit `028ff1b2`.
2. Inspect how a persistent pre-warmed worker process pool is structured inside Bubblewrap (`bwrap`).
3. Detail the wire protocol design (`src/sandbox/browser/wire.rs`):
   - Unix Domain Socket transport (`UnixStream::pair()`).
   - SCM_RIGHTS descriptor passing (`[input_fd, output_write_pipe_fd]`).
   - Request framing (1-byte operation enum).
   - Response framing (8-byte header `[png_len: u32, metadata_len: u32]`, bounded by MAX_OUTPUT_BYTES).
4. Detail how `sandbox::parse` in `src/sandbox.rs` and `src/adapters/local_preview.rs` delegates requests to the worker pool to achieve <100ms warm latency.
5. Produce handoff.md in your working directory and notify the orchestrator.

SCOPE BOUNDARIES:
- Read-only exploration. DO NOT modify any source code files.
</USER_REQUEST>
