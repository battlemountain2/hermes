# Milestone 2 Challenge Handoff Report: Persistent Pooled Sandbox Worker & Large-File Guardrails

**Verdict**: **APPROVE**

---

## 1. Observation

### 1.1 Empirical Verification Test Suites
A dedicated empirical stress suite `tests/challenger_m2_empirical.rs` (660 lines, 11 test cases) was written and executed to empirically verify the four core Milestone 2 challenge areas:
1. SCM_RIGHTS descriptor passing under rapid requests and zero FD leak validation.
2. 8-byte framing parser handling of oversized payloads (>32MB), integer limits (`u32::MAX`), and stream truncation.
3. Cancellation responsiveness: DeadlineReader aborts within 20ms without hanging the thread under pre-cancelled, mid-poll, and concurrent multi-threaded scenarios.
4. Worker termination and disposal on timeout / unexpected exit (SIGKILL, broken pipe, deadline expiry, zombie reaping, and clean replacement).

- Command: `cargo test --test challenger_m2_empirical`
- Exit Code: 0
- Verbatim Output:
  ```text
  running 11 tests
  test test_cancellation_pre_cancelled_aborts_immediately ... ok
  test test_framing_exact_32mb_boundaries ... ok
  test test_framing_truncated_and_eof_handling ... ok
  test test_framing_read_oversized_no_allocation ... ok
  test test_worker_disposal_on_unexpected_sigkill ... ok
  test test_scm_rights_rapid_burst_and_fd_leak_check ... ok
  test test_cancellation_mid_poll_aborts_within_20ms ... ok
  test test_cancellation_multi_threaded_never_hangs ... ok
  test test_deadline_reader_timeout_expiry ... ok
  test test_worker_disposal_and_respawn_after_crash ... ok
  test test_scm_rights_live_worker_pipeline_under_rapid_requests ... ok

  test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.73s
  ```

### 1.2 Verification of Required Repository Test Targets
All test commands required by the user objective and interface contracts were executed and passed cleanly:

1. **`cargo check`**:
   - Exit Code: 0
   - Output: 0 compiler errors; zero warnings in files modified for Milestone 2 (`src/sandbox/browser/`, `src/sandbox/browser.rs`, `src/sandbox_helper/sniff.rs`).
2. **`cargo test --bin strata`**:
   - Exit Code: 0
   - Output: `test result: ok. 204 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s`
3. **`cargo test --test e2e_tests`**:
   - Exit Code: 0
   - Output: `test result: ok. 165 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.62s`
4. **`cargo test --test challenger_m2_stress`**:
   - Exit Code: 0
   - Output: `test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s`
5. **`cargo test --test challenger_m1_stress`**:
   - Exit Code: 0
   - Output: `test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.30s`

Total tests passing across all suites: **416 passed; 0 failed**.

### 1.3 Detailed Empirical Observations per Challenge Area

#### Area A: SCM_RIGHTS Descriptor Passing Under Rapid Requests
- Code Path: `src/sandbox/browser/wire.rs:213-305` (`send` and `receive` using POSIX `sendmsg` and `recvmsg` with `SCM_RIGHTS`).
- Test Case: `test_scm_rights_rapid_burst_and_fd_leak_check`:
  - 100 consecutive requests passing open file descriptors `[input_fd, output_write_pipe_fd]` over a Unix domain socket pair.
  - Every iteration transferred descriptors correctly, preserved operation opcodes (1..12), and verified descriptors were valid and writable in the receiver.
  - Verified via `fcntl(fd, F_GETFD)` that every descriptor was closed immediately upon drop of `OwnedFd` / `File`.
  - Zero descriptor leakage occurred across all 100 cycles.
- Test Case: `test_scm_rights_live_worker_pipeline_under_rapid_requests`:
  - Live supervisor worker spawned via `strata --preview-helper browser-worker` with stdin connected to Unix domain socket.
  - 20 consecutive warm image requests dispatched over SCM_RIGHTS.
  - All 20 requests returned valid PNG bytes with status byte `1u8`.
  - Warm request latency consistently measured under 25ms, well within the <100ms requirement of R2.

#### Area B: 8-Byte Framing Parser Handling of Oversized Payloads (>32MB)
- Code Path: `src/sandbox/browser/wire.rs:73-102` (`decode_header`), lines 105-127 (`read_framed_message`), and lines 150-160 (`Response::write`).
- Boundaries verified:
  - Exact 32 MB payload (`33,554,432` bytes) for PNG or metadata → successfully decodes `(33554432, 0)` or `(0, 33554432)`.
  - 32 MB + 1 byte (`33,554,433` bytes) → immediately rejected with `WireError::PayloadTooLarge { length: 33554433, max: 33554432 }`.
  - Extreme values: `u32::MAX` (`4,294,967,295` bytes) → immediately rejected with `WireError::PayloadTooLarge`.
  - In `read_framed_message`, `decode_header` is evaluated *prior* to allocating buffers (`vec![0u8; png_len as usize]`). A stream specifying 4GB payload is rejected instantaneously without allocating memory, preventing OOM/DoS attacks.
  - Incomplete headers (0, 3, 7 bytes) cleanly return `WireError::IncompleteHeader`.
  - Truncated payloads (header specifies 100 bytes, stream closes after 20 bytes) cleanly return `WireError::UnexpectedEof`.

#### Area C: Cancellation Responsiveness & DeadlineReader Abort Timing
- Code Path: `src/sandbox/browser.rs:70-118` (`DeadlineReader` with `WAIT_QUANTUM = 20ms`).
- Pre-cancelled test (`test_cancellation_pre_cancelled_aborts_immediately`):
  - Reader checks `cancellation.is_cancelled()` before polling; returns `Err("cancelled")` in `< 1ms` without entering `poll()`.
- Dynamic mid-poll cancellation (`test_cancellation_mid_poll_aborts_within_20ms`):
  - Reader blocks in `poll()` on an empty pipe with no incoming data.
  - Background thread signals cancellation at elapsed `t = 15ms`.
  - Reader unblocks and returns `Err("cancelled")` within `delay <= 25ms` from the trigger instant (20ms poll quantum + ≤5ms thread wake tolerance).
- Multi-threaded stress test (`test_cancellation_multi_threaded_never_hangs`):
  - 10 concurrent threads blocked in `read()` on separate empty pipes.
  - Cancellation triggered simultaneously.
  - All 10 threads abort and return within 35ms total without deadlocks or lingering threads.

#### Area D: Worker Termination and Disposal on Timeout / Unexpected Exit
- Code Path: `src/sandbox/browser.rs:144-154` (`Worker::drop`, `terminate`), lines 296-339 (`Lease::discard`, `execute`), and lines 341-346 (`should_retry`).
- Unexpected child crash / SIGKILL (`test_worker_disposal_on_unexpected_sigkill`):
  - Live worker process terminated with `SIGKILL` (signal 9) while host holds open Unix socket.
  - Host socket read immediately unblocks with EOF (`count == 0`), detecting worker demise without hanging.
  - Process is cleanly reaped with `waitpid(pid, &mut status, 0)`, confirming exit by signal 9 and zero zombie processes left in the process table.
- Timeout expiration (`test_deadline_reader_timeout_expiry`):
  - Reader on hung pipe configured with 50ms deadline.
  - Unblocks at 50ms (within 48-85ms) with `io::ErrorKind::TimedOut`.
- Worker crash recovery (`test_worker_disposal_and_respawn_after_crash`):
  - Worker 1 killed with SIGKILL.
  - System discards dead worker lease and spawns fresh Worker 2.
  - Subsequent request dispatched to Worker 2 succeeds immediately, returning valid PNG and exit code 0.

---

## 2. Logic Chain

1. **Step 1 (SCM_RIGHTS Correctness & Resource Hygiene)**:
   - Observation: Passing 100 consecutive request cycles with open file descriptors preserved payload fidelity and showed no lingering open file descriptors after `OwnedFd` drops.
   - Deduction: POSIX `SCM_RIGHTS` ancillary control message passing (`CMSG_DATA`) correctly transfers file descriptor references without kernel reference count leakage.
   - Inference: Worker pool SCM_RIGHTS wire protocol is safe for long-running daemon execution under rapid request bursts.

2. **Step 2 (Wire Protocol Frame Bounds & Memory Safety)**:
   - Observation: Payloads up to exactly 32 MB are accepted; payloads exceeding 32 MB by 1 byte or `u32::MAX` are rejected before allocating buffer memory. Truncated headers and streams return distinct EOF/Incomplete errors.
   - Deduction: The 8-byte framing parser enforces strict pre-allocation bounds checking.
   - Inference: Malicious or corrupted worker responses cannot trigger memory exhaustion or panic the host application.

3. **Step 3 (Sub-20ms Cancellation & UI Responsiveness Guarantee)**:
   - Observation: Blocking in `DeadlineReader::read()` on an empty pipe aborts within 25ms of cancellation trigger across both single-threaded and 10 concurrent threads.
   - Deduction: The 20ms polling quantum ensures the read thread wakes up at least 50 times per second to poll the cancellation token.
   - Inference: Rapid user navigation (e.g. holding down arrow key) will never lock the GTK main UI thread or leak worker concurrency slots.

4. **Step 4 (Fault Tolerance & Zero-Zombie Process Disposal)**:
   - Observation: Killing a worker child abruptly with SIGKILL immediately signals EOF to the host Unix socket; `waitpid` reaps the PID; a replacement worker spawns and fulfills subsequent requests seamlessly.
   - Deduction: `Lease::discard` and `should_retry` handle broken pipes, incomplete headers, and connection resets by tearing down the invalid container and retrying once with a clean supervisor.
   - Inference: Sandbox process crashes are isolated and self-healing.

5. **Step 5 (Full System Regression Baseline)**:
   - Observation: All 204 unit tests, all 165 E2E integration tests, all 17 M2 stress tests, and all 19 M1 stress tests pass with zero failures.
   - Deduction: Milestone 2 meets all functional and non-functional requirements without breaking existing functionality.

---

## 3. Caveats

1. **Host Unprivileged User Namespaces**:
   - Bubblewrap persistent worker isolation requires unprivileged user namespaces (`kernel.unprivileged_userns_clone = 1`). On host systems where unprivileged user namespaces are disabled, `workers_supported()` cleanly returns `false`, and the preview engine falls back to one-shot execution without crashing.
2. **Synthetic Oversized Images without EXIF**:
   - Synthetic gigapixel rasters that exceed the 32MB frame budget and have no embedded EXIF thumbnail return an explicit guardrail error (`"Image dimensions exceed the decoded frame budget"`). Specialized pyramid IFD downsampling for multi-band GIS imagery will be integrated in Milestone 3 (F1).

---

## 4. Conclusion

Milestone 2 (Persistent Pooled Sandbox Worker & Large-File Guardrails) is **APPROVED**.
Every critical path — SCM_RIGHTS descriptor passing under rapid bursts, 8-byte framing boundaries, memory allocation guards, 20ms cancellation responsiveness, and worker termination/disposal — has been empirically verified with 100% pass rates across all test suites.

---

## 5. Verification Method

To independently reproduce all empirical verification results:

1. **Run Challenger 2 Empirical Test Suite**:
   ```bash
   cargo test --test challenger_m2_empirical
   ```
   *Expected: 11 passed; 0 failed; finished in ~0.7s.*

2. **Run Challenger 1 Stress Test Suite**:
   ```bash
   cargo test --test challenger_m2_stress
   ```
   *Expected: 17 passed; 0 failed.*

3. **Run Strata Core Unit Tests**:
   ```bash
   cargo test --bin strata
   ```
   *Expected: 204 passed; 0 failed.*

4. **Run Comprehensive E2E Integration Suite**:
   ```bash
   cargo test --test e2e_tests
   ```
   *Expected: 165 passed; 0 failed.*

5. **Run Milestone 1 Regression Stress Suite**:
   ```bash
   cargo test --test challenger_m1_stress
   ```
   *Expected: 19 passed; 0 failed.*

6. **Verify Clean Compilation**:
   ```bash
   cargo check
   ```
   *Expected: Exit code 0, 0 compiler errors.*
