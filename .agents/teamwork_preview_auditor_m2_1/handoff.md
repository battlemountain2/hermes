# Forensic Integrity Audit Report: Milestone 2 (Persistent Pooled Sandbox Worker & Large-File Guardrails)

## Forensic Audit Summary
- **Work Product**: Milestone 2: `src/sandbox/browser/wire.rs`, `src/sandbox/browser/process.rs`, `src/sandbox/browser/worker.rs`, `src/sandbox/browser.rs`, `src/sandbox.rs`, `src/sandbox_helper/sniff.rs`, `src/sandbox_helper.rs`
- **Profile**: General Project
- **Integrity Mode**: Development (as specified in `ORIGINAL_REQUEST.md`)
- **Verdict**: **CLEAN**

---

## 1. Observation

### 1.1 Source Inspection
Direct source review of Milestone 2 code confirmed genuine implementations across all target components:

1. **SCM_RIGHTS Descriptor Passing & Wire Framing (`src/sandbox/browser/wire.rs`)**:
   - Lines 58–70 (`encode_frame`): Serializes an 8-byte header `[png_len: u32, metadata_len: u32]` in little-endian byte order, prepending payload buffers.
   - Lines 73–102 (`decode_header`): Validates header size (minimum 8 bytes) and bounds both `png_len` and `meta_len` strictly against `MAX_PAYLOAD_SIZE = 32 * 1024 * 1024` (32 MB) prior to any memory allocation.
   - Lines 189–251 (`send`): Real Linux POSIX socket ancillary message passing constructing `ScmRights2Buffer` with `cmsg_level = SOL_SOCKET`, `cmsg_type = SCM_RIGHTS`, and passing `fds = [input.as_raw_fd(), output.as_raw_fd()]` via `sendmsg` syscall with `MSG_NOSIGNAL`.
   - Lines 258–305 (`receive`): Invocates `recvmsg` syscall with `MSG_CMSG_CLOEXEC`, verifies ancillary message headers (`cmsg_level == SOL_SOCKET && cmsg_type == SCM_RIGHTS && fds >= 0`), safely constructing `OwnedFd::from_raw_fd(cmsg.fds[0])` and `OwnedFd::from_raw_fd(cmsg.fds[1])`.

2. **Single-Threaded Supervisor & Disposable Child Fork (`src/sandbox/browser/process.rs` & `worker.rs`)**:
   - `process.rs` Lines 18–28 (`require_single_thread`): Reads `/proc/self/task` to ensure the supervisor is strictly single-threaded before calling POSIX `fork()`.
   - `process.rs` Lines 53–69 (`wait`): Handles `waitpid` with EINTR loop, checking clean termination and exit code.
   - `process.rs` Lines 75–85 (`exit`): Calls `_exit(status)` to avoid running atexit handlers in the child.
   - `worker.rs` Lines 24–42 (`run`): Reuses inherited FD 0 as `UnixStream`, receiving requests in an event loop.
   - `worker.rs` Lines 47–64 (`handle_request`): Dispatches requests to a disposable child process via `process::fork_child()`. Child executes the render job and calls `process::exit()`; parent reaps child with `process::wait()` and sends a 1-byte status back across the socket.
   - `worker.rs` Line 94: Correctly invokes `std::mem::forget(writer)` to prevent premature closing of borrowed file descriptors.

3. **Bubblewrap Sandbox Pool & Deadline Polling (`src/sandbox/browser.rs`)**:
   - Lines 70–118 (`DeadlineReader`): Uses POSIX `poll` syscall with `WAIT_QUANTUM = 20ms`. Checks `cancellation.is_cancelled()` each slice, guaranteeing prompt UI aborts within ≤20ms.
   - Lines 423–500 (`spawn_worker`): Invokes `bwrap` with `--unshare-all`, `--die-with-parent`, `--new-session`, `--clearenv`, bind-mounts including `--ro-bind-try /var/cache/fontconfig /var/cache/fontconfig`, and wraps execution under `/usr/bin/prlimit --as=1342177280 --cpu=10 --fsize=33554432 -- /app/strata --preview-helper browser-worker`.
   - Lines 512–566 (`preview` and `thumbnail_parse`): Implements dual pooling (`pool()` and `preview_pool()`), query caching with mtime/size invalidation, and automatic worker replacement upon crash or broken pipe.

4. **Zero-Decode Dimension Sniffing & EXIF Fallback (`src/sandbox_helper/sniff.rs`)**:
   - Lines 23–28 (`exceeds_decoded_frame_budget`): Validates `width * height * 4 <= 33,554,432` (32 MB) and `pixels <= 134,217,728` (~134 MP).
   - Lines 30–125: Authentic zero-decode dimension sniffers:
     - PNG: Validates 8-byte signature `\x89PNG\r\n\x1a\n`, verifies `IHDR` chunk at offset 12..16, parses BE u32 dimensions.
     - GIF: Validates `GIF87a`/`GIF89a`, parses LE u16 dimensions from Logical Screen Descriptor.
     - JPEG: Scans markers from SOI, skips padding and variable segments, extracts dimensions from SOF0/SOF2 markers.
     - TIFF: Parses Standard TIFF (magic 42) and BigTIFF (magic 43) in both LE and BE, locating IFD0 and reading ImageWidth (tag 256) and ImageLength (tag 257) with SHORT, LONG, and LONG8 types.
   - Lines 368–516 (`read_exif_thumbnail`): Parses JPEG APP1 `Exif\0\0` and TIFF headers, navigates IFD0 -> IFD1, extracts JPEGInterchangeFormat (0x0201) and JPEGInterchangeFormatLength (0x0202).
   - Lines 519–556 (`scale_embedded_thumbnail`): Sniffs embedded thumbnail dimensions and rejects oversized thumbnails *before* invoking `gdk_pixbuf::PixbufLoader`, neutralizing forged EXIF thumbnail dimension exploits.

5. **Sandbox Delegation Wiring (`src/sandbox.rs`)**:
   - Lines 120–135: `parse()` delegates requests to `browser::preview` and `browser::thumbnail_parse`, with seamless fallback to `parse_one_shot` if pooled workers are not supported.

### 1.2 Prohibited Patterns Scan
- **Hardcoded test results**: 0 occurrences. Grep for test strings, canned buffers, or fixed results returned empty.
- **Facade implementations**: 0 occurrences. All functions execute complete, authentic logic.
- **Fabricated verification outputs**: 0 occurrences. Workspace contains no pre-existing `.log`, fake results, or fabricated attestation artifacts.
- **Self-certifying tests**: 0 occurrences. Tests evaluate authentic round-trip framing, real POSIX socket descriptor passing, real pipe I/O, and valid image data.

---

## 2. Logic Chain

1. **Authenticity of Process Isolation**:
   - `spawn_worker()` in `src/sandbox/browser.rs:427` directly spawns `/usr/bin/bwrap` with `--unshare-all`, `--die-with-parent`, tmpfs, read-only system binds, and Linux `prlimit` bounds (`--as=1342177280`, `--cpu=10`, `--fsize=33554432`).
   - The supervisor forks a disposable child process per request in `src/sandbox/browser/worker.rs:47`. This guarantees that memory allocations and codec state are discarded by the kernel upon child termination, preventing state leakage.
   - Therefore, process isolation is genuine and strictly enforced.

2. **Authenticity of IPC & Transport**:
   - `wire.rs:243` and `wire.rs:283` invoke `sendmsg` and `recvmsg` syscalls with `SCM_RIGHTS` ancillary control messages.
   - The worker accesses the input file via `/proc/self/fd/<fd>` and writes directly to the passed output pipe descriptor in memory.
   - The host decodes the response via `read_framed_message()` checking the 8-byte length prefix.
   - Therefore, IPC is real POSIX SCM_RIGHTS descriptor passing over Unix sockets, satisfying R2 and F4.

3. **Authenticity of Guardrails**:
   - `sniff.rs` parses raw binary headers for PNG, GIF, JPEG, and TIFF formats without decompressing pixels.
   - `exceeds_decoded_frame_budget()` checks both pixel count and RGBA byte size against the 32MB buffer ceiling.
   - When an oversized image is detected, `render_image()` and `render_raw()` call `sniff::read_exif_thumbnail()`, falling back to the scaled embedded thumbnail or returning a bounded error.
   - `scale_embedded_thumbnail()` re-sniffs the thumbnail dimensions before decoding, preventing malicious thumbnail decompression.
   - Therefore, large-file guardrails and EXIF fallbacks are genuine and complete, satisfying R2, F5, and F6.

4. **Authenticity of Cancellation & Timeouts**:
   - `DeadlineReader` in `src/sandbox/browser.rs:76` uses POSIX `poll()` in 20ms slices (`WAIT_QUANTUM`).
   - Immediate cancellation or cancellation triggered mid-read aborts in ≤20ms.
   - When cancelled, `Lease::discard()` sends `SIGKILL` to `bwrap`, which terminates all sandboxed processes immediately.
   - Therefore, timeouts and UI responsiveness guardrails are authentic and fully functional, satisfying R2 and F7.

---

## 3. Caveats

- **Host Environment Namespace Permissions**: Bubblewrap worker execution relies on Linux user namespaces (`kernel.unprivileged_userns_clone = 1`). In restricted environments where `bwrap` is unavailable, `workers_supported()` returns `false`, causing the engine to gracefully fall back to one-shot execution.
- **Images Lacking EXIF Thumbnails**: Synthetically generated oversized images (e.g. 50,000x50,000 PNGs) do not contain EXIF thumbnails. For these files, `sniff::read_exif_thumbnail()` returns `None`, and the preview pipeline cleanly yields an explicit error without allocating gigapixel buffers.

---

## 4. Conclusion

The Milestone 2 implementation strictly conforms to all requirements outlined in `ORIGINAL_REQUEST.md` (R2) and `PROJECT.md` (Features F4, F5, F6, F7).
- No hardcoded test outputs, facades, stubs, or shortcuts were found.
- Bubblewrap isolation, SCM_RIGHTS file descriptor passing, 8-byte framed wire protocol, zero-decode header sniffing, EXIF thumbnail fallback, and 20ms cancellation polling are fully genuine and verified.

**Final Forensic Verdict**: **CLEAN**.

---

## 5. Verification Method

To independently verify the implementation:

1. **Inspect Target Files**:
   - `src/sandbox/browser/wire.rs`
   - `src/sandbox/browser/process.rs`
   - `src/sandbox/browser/worker.rs`
   - `src/sandbox/browser.rs`
   - `src/sandbox.rs`
   - `src/sandbox_helper/sniff.rs`

2. **Execute Unit Tests**:
   ```bash
   cargo test --bin strata -- sniff
   cargo test --bin strata -- deadline_reader
   cargo test --bin strata -- wire
   ```
   *Expected: All unit tests pass.*

3. **Execute E2E Integration and Challenger Stress Tests**:
   ```bash
   cargo test --test e2e_tests -- test_f4
   cargo test --test e2e_tests -- test_f5
   cargo test --test e2e_tests -- test_f6
   cargo test --test e2e_tests -- test_f7
   cargo test --test challenger_m2_stress
   ```
   *Expected: All 25 stress scenarios pass with zero file descriptor leaks and zero zombie processes.*
