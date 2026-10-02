# BRIEFING — 2026-10-01T03:35:45Z

## Mission
Investigate persistent pooled sandbox worker and wire protocol architecture for Milestone 2 (R2 & F4), analyzing upstream commits, worker process pool inside bwrap, wire protocol design (UnixStream, SCM_RIGHTS, framing), and sandbox::parse delegation.

## 🔒 My Identity
- Archetype: explorer
- Roles: investigation, synthesis
- Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m2_1
- Original parent: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Milestone: Milestone 2 (Worker Pool & Wire Protocol Explorer)

## 🔒 Key Constraints
- Read-only investigation — do NOT implement / modify source code
- Write only to working directory (/home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m2_1/)

## Current Parent
- Conversation ID: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Updated: not yet

## Investigation State
- **Explored paths**:
  - Upstream Strata commits `015621c00c044adea2eaab948db579ca8aaa0432`, `028ff1b22031dfc35821b8a3009707fc27316921`, `9c60c0b2`, and `63050999`
  - `src/sandbox.rs`, `src/sandbox_helper.rs`, `src/main.rs`, `src/adapters/local_preview.rs`, `src/ui/thumbnail.rs`
  - `src/sandbox/browser.rs`, `src/sandbox/browser/wire.rs`, `src/sandbox/browser/worker.rs`, `src/sandbox/browser/process.rs`
  - `tests/common/wire_protocol.rs`, `tests/e2e/tier2_boundaries.rs`, `tests/e2e/tier3_pairwise.rs`
- **Key findings**:
  - Upstream Strata solved the 1-2s preview latency bottleneck by pre-spawning bwrap containers, communicating over local UnixStream with SCM_RIGHTS descriptor passing, and using a single-threaded supervisor that forks disposable children in private PID namespaces.
  - Review commit `028ff1b2` hardened cancellation in `DeadlineReader` (polling in 20ms quanta, instantly terminating worker and freeing pool slot) and isolated caches per pool (`pool.cache`).
  - Wire protocol uses 1-byte Operation enum request with SCM_RIGHTS `[input_fd, output_write_pipe_fd]` and 8-byte LE header `[png_len: u32, metadata_len: u32]` followed by payloads, bounded by 32 MiB.
  - Delegating in `sandbox::parse` accelerates both preview drawer and thumbnails (<45ms warm latency vs 800-2300ms one-shot), while preserving one-shot bwrap as safe fallback.
- **Unexplored areas**:
  - None (Milestone 2 Worker Pool & Wire Protocol investigation complete).

## Key Decisions Made
- Confirmed design matches Strata PR #1222 architecture and E2E test suite contract.
- Recommended adding dependencies (`rustix`, `landlock`, `seccompiler`, `libc`, `serde_json`, `kamadak-exif`) in Cargo.toml.
- Recommended handling `--browser-worker` in `src/main.rs` and routing via `src/sandbox.rs::parse`.

## Artifact Index
- DISPATCH.md — Dispatch log
- BRIEFING.md — Working memory
- progress.md — Liveness heartbeat and step tracking
- handoff.md — 5-component final handoff report
