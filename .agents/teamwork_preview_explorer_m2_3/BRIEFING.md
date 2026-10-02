# BRIEFING — 2026-10-01T03:36:15Z

## Mission
Investigate sandbox resource ceilings, cancellation responsiveness, DeadlineReader polling, and worker pool health/replacement for Milestone 2 (R2 & F7).

## 🔒 My Identity
- Archetype: explorer
- Roles: investigator, synthesis
- Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m2_3
- Original parent: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Milestone: M2.3

## 🔒 Key Constraints
- Read-only investigation — do NOT implement
- Do not modify any source code files
- Focus on R2 & F7: prlimit parameters, gio::Cancellable wiring, DeadlineReader polling in 20ms quanta, worker health check & pool replacement

## Current Parent
- Conversation ID: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Updated: not yet

## Investigation State
- **Explored paths**:
  - `src/sandbox.rs` (one-shot bwrap, prlimit, cancellation, termination)
  - `src/adapters/local_preview.rs` (gio::spawn_blocking, load handle, cancellation token)
  - `src/ui/thumbnail.rs` (cancellation on scroll, task recycling)
  - Upstream commits: `015621c0`, `028ff1b2`, `63050999`, `9c60c0b2`
  - `src/sandbox/browser.rs`, `src/sandbox/browser/wire.rs`, `src/sandbox/browser/worker.rs`, `src/sandbox/browser/process.rs`, `src/sandbox/browser/tests.rs`
  - `tests/e2e/tier1_isolated.rs`, `tests/e2e/tier2_boundaries.rs`, `tests/common/sniffer.rs`, `tests/common/wire_protocol.rs`
- **Key findings**:
  - `prlimit` bounds: 1.25 GB AS (`--as=1342177280`), 10s CPU (`--cpu=10`), 32 MB Fsize (`--fsize=33554432`), 256MB tmpfs (`size=268435456,mode=1777`), `RLIMIT_CORE=0` suppressed, `RLIMIT_NPROC` omitted.
  - `gio::Cancellable` wired to atomic `Cancellation`, observed in 0ms pre-execution queue and ≤20ms during in-flight pipe wait.
  - `DeadlineReader` polling with `WAIT_QUANTUM = 20ms` eliminates GTK UI and worker thread hangs.
  - Worker cancellation immediately kills `bwrap` (PID 1) with `SIGKILL`, terminating all container descendants instantly.
  - Worker health check catches `UnexpectedEof`/`BrokenPipe`/`ConnectionReset` to trigger single-attempt worker replacement; discarded workers decrement `state.count` in `Lease::drop` to prevent pool capacity leaks.
  - Dedicated `"thumbnail-launcher"` thread owns `bwrap` spawning to prevent `PR_SET_PDEATHSIG` kills when GIO thread pool threads terminate.
- **Unexplored areas**: None for M2.3 scope.

## Key Decisions Made
- Confirmed that upstream commits `015621c0` and `028ff1b2` provide the battle-tested blueprint for all R2 & F7 requirements.
- Completed comprehensive 5-component handoff report in `handoff.md`.

## Artifact Index
- `DISPATCH.md` — incoming dispatch message
- `progress.md` — liveness heartbeat
- `BRIEFING.md` — persistent working memory
- `handoff.md` — comprehensive 5-component handoff report
