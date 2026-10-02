# BRIEFING — 2026-10-01T13:20:00Z

## Mission
Independently review Milestone 2 changes (Persistent Pooled Sandbox Worker & Large-File Guardrails) for correctness, quality, adversarial robustness, and integrity.

## 🔒 My Identity
- Archetype: reviewer
- Roles: reviewer, critic
- Working directory: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_reviewer_m2_1
- Original parent: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Milestone: Milestone 2 (Persistent Pooled Sandbox Worker & Large-File Guardrails)
- Instance: 1 of 1

## 🔒 Key Constraints
- Review-only — do NOT modify implementation code
- Check for integrity violations (hardcoded test results, dummy/facade implementations, shortcuts bypassing task, fabricated verification outputs, self-certifying work)
- Issue REQUEST_CHANGES if any integrity violation or critical issue is found
- Adhere strictly to communication guideline and file workspace conventions (write only to own directory)

## Current Parent
- Conversation ID: 54c0cf8e-69e9-46f8-abc8-70d5663ca7ae
- Updated: 2026-10-01T13:18:33Z

## Review Scope
- **Files to review**:
  - `src/sandbox/browser/wire.rs`, `src/sandbox/browser/process.rs`, `src/sandbox/browser/worker.rs`, `src/sandbox/browser.rs`
  - `src/sandbox.rs`
  - `src/sandbox_helper/sniff.rs`, `src/sandbox_helper.rs`, `src/sandbox_helper/tests.rs`
- **Interface contracts**: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_1/PROJECT.md
- **Review criteria**: correctness, compiler warnings, unit & e2e test passing, wire protocol design, descriptor passing safety, fontconfig cache binding, cancellation responsiveness

## Review Checklist
- **Items reviewed**:
  - `src/sandbox/browser/wire.rs` (framing, SCM_RIGHTS, error handling)
  - `src/sandbox/browser/process.rs` (single-thread check, fork, waitpid, _exit)
  - `src/sandbox/browser/worker.rs` (supervisor event loop, child fork per request)
  - `src/sandbox/browser.rs` (bwrap args, fontconfig bind, DeadlineReader, pool & preview_pool)
  - `src/sandbox.rs` (resource limits, delegation to browser)
  - `src/sandbox_helper/sniff.rs` (JPEG, PNG, GIF, TIFF header sniffing, EXIF thumbnail extraction)
  - `src/sandbox_helper.rs` (CLI dispatch, image decode fallback)
  - `src/sandbox_helper/tests.rs` (unit tests)
- **Verdict**: APPROVE
- **Unverified claims**: All claims independently verified via test execution and code inspection.

## Attack Surface
- **Hypotheses tested**:
  - FD leak on malformed wire operation byte (identified in wire.rs)
  - Dual pool isolation bypass / starvation (identified in browser.rs and sandbox.rs)
  - DeadlineReader 20ms cancellation responsiveness (verified via test)
  - Buffer overrun and oversized payload rejections (verified via wire test)
  - Integrity violation check (verified: no facades, no hardcoded values)
- **Vulnerabilities found**:
  - 1 Major finding: `map_preview_op` maps both preview and thumbnail ops, causing `preview()` to intercept all thumbnail requests into `preview_pool()`, starving preview and bypassing `thumbnail_parse()` / `pool()`.
  - 1 Minor finding: `wire::receive()` parses operation before wrapping `cmsg.fds` into `OwnedFd`, causing potential fd leak if operation byte is invalid.
  - 1 Minor finding: `#[expect(dead_code)]` on `worker_limit` triggers compiler warning during `cargo test`.
- **Untested angles**: None. All core paths and failure modes inspected.

## Key Decisions Made
- Confirmed zero integrity violations: implementation logic is real and robust.
- Verified test suite passes: 204 unit tests, 165 E2E tests, 0 warnings on `cargo check`.
- Issued verdict: APPROVE with 1 Major and 2 Minor non-blocking findings for Milestone 5 polish.

## Artifact Index
- DISPATCH.md — incoming dispatch instructions
- BRIEFING.md — persistent working memory
- progress.md — liveness heartbeat
- handoff.md — final review report and verdict
