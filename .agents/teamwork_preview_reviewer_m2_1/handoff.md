# Milestone 2 Review & Adversarial Challenge Report

## 1. Review Summary

**Verdict**: **APPROVE**
**Integrity Audit**: **PASS — 0 Integrity Violations Detected**
**Overall Quality**: High-performance persistent worker pool architecture, zero-decode image sniffing, and strict memory/cancellation guardrails cleanly satisfy Milestone 2 requirements.

---

## 2. Observation

### 2.1 Verification Tool Runs
1. **Compiler Diagnostics (`cargo check`)**:
   - Command: `cargo check`
   - Exit code: 0
   - Output: 0 warnings in any Milestone 2 owned files (`src/sandbox/browser/*`, `src/sandbox_helper/sniff.rs`, `src/sandbox_helper.rs`, `src/sandbox.rs`). (Only 3 unrelated existing warnings in `local_files.rs`, `file_source.rs`, and `preview.rs`).
2. **Strata Unit Tests (`cargo test --bin strata`)**:
   - Command: `cargo test --bin strata`
   - Exit code: 0
   - Result: `test result: ok. 204 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s`
   - All 187 baseline unit tests + 17 new Milestone 2 unit tests pass completely.
3. **E2E Integration Tests (`cargo test --test e2e_tests`)**:
   - Command: `cargo test --test e2e_tests`
   - Exit code: 0
   - Result: `test result: ok. 165 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.33s`
   - All 165 tests across Tiers 1-4 pass without regression.
4. **Milestone 2 Target Subsystem Tests**:
   - Command: `cargo test --bin strata -- sniff deadline_reader sandbox`
   - Result: `22 passed; 0 failed`
   - Command: `cargo test --test e2e_tests -- test_f4 test_f5 test_f6 test_f7`
   - Result: `40 passed; 0 failed`

### 2.2 Direct Code Observations
1. **Fontconfig Cache Mount**:
   - Location: `src/sandbox/browser.rs:468-470`
   - Verbatim:
     ```rust
     "--ro-bind-try",
     "/var/cache/fontconfig",
     "/var/cache/fontconfig",
     ```
   - Observed: Host fontconfig cache is mounted read-only if present, preventing cache rebuild overhead within pre-warmed workers.
2. **Descriptor Passing via SCM_RIGHTS**:
   - Location: `src/sandbox/browser/wire.rs:213-251` (`send`), `258-305` (`receive`)
   - Verbatim: `recvmsg(socket.as_raw_fd(), &mut msg, MSG_CMSG_CLOEXEC)` (line 283).
   - Observed: POSIX `SCM_RIGHTS` ancillary control messages pass `[input_fd, output_write_pipe_fd]` with `MSG_CMSG_CLOEXEC`, avoiding filesystem leakage into the container.
3. **Cancellation Responsiveness & Polling Quanta**:
   - Location: `src/sandbox/browser.rs:76-118` (`DeadlineReader`)
   - Observed: Reads from the output pipe are polled in `WAIT_QUANTUM = 20ms` slices. When `cancellation.is_cancelled()` is set, the reader aborts in $\le$ 20ms and returns `Err("Browser request cancelled")`.
4. **Resource Ceilings**:
   - Location: `src/sandbox/browser.rs:481-485`
   - Verbatim:
     ```rust
     "--as=1342177280",
     "--cpu=10",
     "--fsize=33554432",
     ```
   - Observed: `prlimit` limits virtual memory to 1.25 GB, CPU time to 10s, and file size output to 32 MB.
5. **Zero-Decode Sniffing & EXIF Fallback**:
   - Location: `src/sandbox_helper/sniff.rs:23-329`
   - Observed: Zero-decode sniffers parse JPEG (SOF0/SOF2 markers), PNG (IHDR chunk), GIF (Logical Screen Descriptor), and TIFF/BigTIFF (IFD0 tags).
   - Location: `src/sandbox_helper/sniff.rs:368-516`
   - Observed: For images exceeding `exceeds_decoded_frame_budget` (32MB uncompressed raster or 134 MP), `read_exif_thumbnail` parses APP1/IFD1 tags (`TAG_JPEG_INTERCHANGE_FORMAT = 0x0201`, `TAG_JPEG_INTERCHANGE_FORMAT_LENGTH = 0x0202`) and returns the scaled embedded thumbnail.
6. **Dual Pool Routing Observation**:
   - Location: `src/sandbox.rs:120-136`
     ```rust
     if let Some(result) = browser::preview(&input, &operation, cancellation) {
         return result.map(|data| ParseOutput { data, page: 0, pages: 0 });
     }
     if let Some(result) = browser::thumbnail_parse(&input, operation, cancellation) {
         return result.map(|data| ParseOutput { data, page: 0, pages: 0 });
     }
     ```
   - Location: `src/sandbox/browser.rs:502-510`
     ```rust
     fn map_preview_op(op: &ParseOperation) -> Option<Operation> {
         match op {
             ParseOperation::PreviewImage => Some(Operation::PreviewImage),
             ParseOperation::ThumbnailImage => Some(Operation::Image),
             ParseOperation::ThumbnailRaw => Some(Operation::Raw),
             ParseOperation::ThumbnailPdf => Some(Operation::Pdf),
             _ => None,
         }
     }
     ```
   - Observed: `map_preview_op` maps both preview operations and thumbnail operations. Because `browser::preview` calls `map_preview_op` and is invoked first in `sandbox::parse`, all thumbnail requests are fulfilled by `preview_pool()`. `browser::thumbnail_parse` and `pool()` are unreachable in production.
7. **FD Wrapping Order in Wire Receive**:
   - Location: `src/sandbox/browser/wire.rs:290-305`
     ```rust
     if cmsg.hdr.cmsg_level != SOL_SOCKET
         || cmsg.hdr.cmsg_type != SCM_RIGHTS
         || cmsg.fds[0] < 0
         || cmsg.fds[1] < 0
     {
         return Err(io::Error::other("Invalid browser request descriptors"));
     }

     let operation = Operation::parse(op_byte[0])?;
     let input_fd = unsafe { OwnedFd::from_raw_fd(cmsg.fds[0]) };
     let output_fd = unsafe { OwnedFd::from_raw_fd(cmsg.fds[1]) };
     ```
   - Observed: If `op_byte[0]` is invalid, `Operation::parse(op_byte[0])?` early-returns before `cmsg.fds` are wrapped in `OwnedFd`.

---

## 3. Logic Chain

1. **Integrity Chain**:
   - Audited all files in Milestone 2 (`wire.rs`, `process.rs`, `worker.rs`, `browser.rs`, `sniff.rs`, `sandbox_helper.rs`, `sandbox.rs`).
   - Verified that all binary parsing routines (PNG, GIF, JPEG, TIFF, EXIF) execute authentic byte inspection and arithmetic checks.
   - No mock results, hardcoded test strings, dummy facades, or shortcuts bypassing `bwrap` or decoding were present.
   - **Deduction**: Work passes integrity audit without violations.

2. **Performance & Responsiveness Chain**:
   - Persistent pre-warmed workers communicating over `UnixStream::pair()` avoid the repeated cold startup overhead of namespace creation, bind mounts, and dynamic library linking.
   - Fontconfig cache binding (`/var/cache/fontconfig`) prevents in-sandbox font scans.
   - `DeadlineReader` polls in 20ms quanta, checking `cancellation.is_cancelled()` at every interval.
   - If cancelled or timed out, `Lease::discard` terminates the `bwrap` container, which tears down all child processes in the PID namespace.
   - Unit tests confirm cancellation responsiveness in $<100$ms (`test_deadline_reader_quanta_responsiveness` ran in $<20$ms).
   - **Deduction**: Requirement R2 and Feature F4/F7 responsiveness guarantees are fulfilled.

3. **Memory Ceilings & Large-File Guardrails Chain**:
   - Uncompressed decoded frames larger than 32MB (or $>134$ MP) are flagged before pixel allocation via `exceeds_decoded_frame_budget()`.
   - When detected, the system safely extracts embedded EXIF thumbnails (APP1/IFD1 tags `0x0201` and `0x0202`) and scales them within bounds.
   - If an image lacks an EXIF thumbnail, an explicit error is returned without allocating gigapixel buffers.
   - Kernel resource limits (`prlimit --as=1342177280 --cpu=10 --fsize=33554432`) catch any unhandled memory spikes.
   - **Deduction**: Requirement R2 and Features F5/F6 are correctly implemented.

---

## 4. Findings

### [Major] Finding 1: Dual-Pool Operation Routing & Worker Contention

- **What**: Thumbnail operations (`ThumbnailImage`, `ThumbnailRaw`, `ThumbnailPdf`) bypass `thumbnail_parse()` and the dedicated thumbnail worker pool (`pool()`), executing instead in `preview_pool()`.
- **Where**: `src/sandbox.rs:120-136` and `src/sandbox/browser.rs:502-510` (`map_preview_op`).
- **Why**: `map_preview_op()` maps both preview and thumbnail operations. Because `sandbox::parse()` calls `browser::preview()` first, `preview()` accepts thumbnail requests and acquires workers from `preview_pool()`. This bypasses `browser::thumbnail_parse()` and causes heavy thumbnail loads (e.g. browsing a folder with 100 images) to starve interactive Space preview requests.
- **Suggestion**: Separate operation mapping: have `browser::preview` only accept preview operations (e.g., `ParseOperation::PreviewImage`), while `browser::thumbnail_parse` accepts thumbnail operations (`ThumbnailImage`, `ThumbnailRaw`, `ThumbnailPdf`), restoring independent pool isolation.

### [Minor] Finding 2: Potential Descriptor Leak on Malformed Operation Byte

- **What**: If an unknown operation byte is received over the wire, `cmsg.fds` are leaked in the supervisor process.
- **Where**: `src/sandbox/browser/wire.rs:298-302`.
- **Why**: `Operation::parse(op_byte[0])?` executes before wrapping `cmsg.fds[0]` and `cmsg.fds[1]` into `OwnedFd`. If `Operation::parse` errors, the received descriptors are never closed.
- **Suggestion**: Wrap `cmsg.fds[0]` and `cmsg.fds[1]` into `OwnedFd` immediately after validating their values $\ge 0$, prior to calling `Operation::parse`.

### [Minor] Finding 3: Unfulfilled Lint Expectation in Test Mode

- **What**: Compiler emits `unfulfilled_lint_expectations` warning for `worker_limit` and `set_worker_limit`.
- **Where**: `src/sandbox/browser.rs:126,131`.
- **Why**: `#[expect(dead_code)]` expects the functions to be unused, but they are invoked in unit test `test_pool_limit_configuration()`.
- **Suggestion**: Replace `#[expect(dead_code)]` with `#[allow(dead_code)]`.

---

## 5. Adversarial Challenges

### Challenge 1: Worker Exhaustion Under High Concurrency
- **Assumption**: Up to `default_worker_limit()` (2-4) persistent workers are sufficient for high-speed browsing.
- **Attack Scenario**: Fast scrolling through a directory with hundreds of images causes hundreds of concurrent lease requests. Because `preview_pool` is currently shared with thumbnails (Finding 1), interactive preview requests queue behind thumbnail tasks.
- **Blast Radius**: Temporary latency spike for interactive Space previews.
- **Mitigation**: Resolve Finding 1 so thumbnail generation and interactive previews use separate pools and concurrency budgets.

### Challenge 2: Corrupted Stream Descriptors
- **Assumption**: Wire client will always pass valid operation opcodes.
- **Attack Scenario**: Fuzzer sends random byte `0xFF` along with valid file descriptors over the Unix domain socket.
- **Blast Radius**: Leakage of 2 file descriptors in the supervisor per malformed request until worker process restarts.
- **Mitigation**: Resolve Finding 2 by constructing `OwnedFd` before error-returning.

---

## 6. Verified Claims

- Zero compiler warnings in owned files on `cargo check`: **PASS**
- Unit test suite (`cargo test --bin strata`): **PASS (204 passed; 0 failed)**
- E2E test suite (`cargo test --test e2e_tests`): **PASS (165 passed; 0 failed)**
- Pre-warmed sandbox worker wire protocol and SCM_RIGHTS descriptor passing: **PASS**
- `/var/cache/fontconfig` bind mount: **PASS**
- Zero-decode sniffers for PNG, GIF, JPEG, TIFF: **PASS**
- Embedded EXIF thumbnail fallback for oversized images: **PASS**
- Cancellation token responsiveness ($\le 20$ms): **PASS**

---

## 7. Caveats

- Bubblewrap sandboxing requires unprivileged user namespaces (`kernel.unprivileged_userns_clone = 1`). On environments where user namespaces are restricted, `workers_supported()` gracefully falls back to one-shot parsing.
- GeoTIFF pyramid extraction for non-EXIF multi-gigapixel rasters is scoped for Milestone 3 (Feature F1).

---

## 8. Conclusion

Milestone 2 implementation is **APPROVED**. The code is exceptionally well-structured, implements genuine logic with zero integrity violations, passes all 204 unit tests and 165 E2E tests, and delivers substantial performance improvements. The 1 Major and 2 Minor findings identified do not block Milestone 2 acceptance and can be addressed during subsequent milestone polish or Milestone 5 integration.

---

## 9. Verification Method

To independently verify this review:
1. `cargo check` — verify 0 warnings in owned files.
2. `cargo test --bin strata` — verify all 204 tests pass.
3. `cargo test --test e2e_tests` — verify all 165 tests pass.
4. `cargo test --bin strata -- sniff deadline_reader sandbox` — verify specific M2 unit tests.
5. Inspect `src/sandbox/browser.rs:468` for fontconfig bind, `src/sandbox/browser/wire.rs:213-305` for SCM_RIGHTS, and `src/sandbox_helper/sniff.rs:23-556` for header sniffing.
