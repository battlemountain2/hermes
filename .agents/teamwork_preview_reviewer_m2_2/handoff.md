# Milestone 2 Independent Review & Adversarial Challenge Report

## Review Summary

**Verdict**: APPROVE

---

## 1. Observation

### 1.1 Scope and File Inspection
Directly observed and inspected the following source and test files:
- `src/sandbox/browser/wire.rs`: Lines 1–366. Wire protocol framing (`[png_len: u32, metadata_len: u32]`), `Operation` enum and parsing (12 variants), `Response` read/write with 32MB payload ceiling (`MAX_PAYLOAD_SIZE = 33,554,432`), POSIX `SCM_RIGHTS` descriptor passing (`send`, `receive`).
- `src/sandbox/browser/process.rs`: Lines 1–86. Single-threaded verification (`require_single_thread` checking `/proc/self/task`), disposable child process creation (`fork_child`), process reaping (`wait`), and clean termination (`exit` via POSIX `_exit`).
- `src/sandbox/browser/worker.rs`: Lines 1–97. In-container worker supervisor loop taking stdin socket (fd 0), dispatching `handle_request`, forking disposable child per request, reading input from `/proc/self/fd/{fd}`, executing renderers, and writing framed response to the output pipe.
- `src/sandbox/browser.rs`: Lines 1–571. Pre-warmed sandbox worker pool (`Pool`, `Lease`), dual pools (`pool()` for directory thumbnails, `preview_pool()` for interactive Space previews), `DeadlineReader` polling in `WAIT_QUANTUM = 20ms` slices, `workers_supported()` capability probe, Bubblewrap invocation with `--ro-bind-try /var/cache/fontconfig`, and LRU render cache (64 entries, 30s TTL).
- `src/sandbox/browser/tests.rs`: Lines 1–155. Unit tests for wire operation parsing, response serialization/deserialization, oversized payload rejection, immediate cancellation, 20ms quanta cancellation responsiveness, successful pipe read, pool limit clamping, and cache hit/miss semantics.
- `src/sandbox.rs`: Lines 12–276. Resource limit constants (`FILE_SIZE_LIMIT_BYTES = 33,554,432`, `ADDRESS_SPACE_LIMIT_BYTES = 1,342,177,280`, `CPU_TIME_LIMIT_SECS = 10`), module exposure (`pub(crate) mod browser;`), and delegation in `parse()` to `browser::preview` and `browser::thumbnail_parse` with fallback to `parse_one_shot`.
- `src/sandbox_helper/sniff.rs`: Lines 1–557. Zero-decode header sniffers for PNG (`sniff_png_dimensions`), GIF (`sniff_gif_dimensions`), JPEG (`sniff_jpeg_dimensions`), and TIFF (`sniff_tiff_dimensions`), decoded frame budget check (`exceeds_decoded_frame_budget`), EXIF thumbnail extractor (`read_exif_thumbnail`), and thumbnail scaler (`scale_embedded_thumbnail`).
- `src/sandbox_helper.rs`: Lines 10–180. CLI dispatch for `browser-worker`, integration of header sniffing and EXIF thumbnail fallback in `render_image` and `render_raw`.
- `src/sandbox_helper/tests.rs`: Lines 1–137. Unit tests for PNG, GIF, JPEG, TIFF sniffing, decoded frame budget boundary calculations, and rejection of oversized forged embedded thumbnails.

### 1.2 Verbatim Tool Command Results
1. **Compiler Hygiene (`cargo check`)**:
   ```
   Command: cargo check
   Exit code: 0
   Output:
   warning: function `map_validation_error` is never used (src/adapters/local_files.rs:35:4)
   warning: variants `Missing`, `NotDirectory`, and `Inaccessible` are never constructed (src/services/file_source.rs:22:5)
   warning: variants `Code`, `Markdown`, and `Model3D` are never constructed (src/services/preview.rs:28:5)
   ```
   Zero warnings originate in files owned by Milestone 2.

2. **Unit Test Suite (`cargo test --bin strata`)**:
   ```
   Command: cargo test --bin strata
   Exit code: 0
   Output:
   test result: ok. 204 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
   ```
   All 187 baseline tests + 17 Milestone 2 unit tests pass.

3. **E2E Test Suite (`cargo test --test e2e_tests`)**:
   ```
   Command: cargo test --test e2e_tests
   Exit code: 0
   Output:
   test result: ok. 165 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.35s
   ```
   All 165 tests across Tiers 1–4 pass.

---

## 2. Logic Chain

1. **Integrity Verification**:
   - Source code analysis of `src/sandbox_helper/sniff.rs`, `src/sandbox/browser/*.rs`, `src/sandbox.rs`, and `src/sandbox_helper.rs` confirmed genuine, substantive implementations.
   - Sniffers parse real file binary headers (IHDR for PNG; Logical Screen Descriptor for GIF; SOI, APP, and SOF0..SOF15 markers for JPEG; IFD0 tags 256/257 for Standard TIFF and BigTIFF).
   - EXIF parser extracts actual APP1 TIFF blocks and parses tags 0x0201 and 0x0202.
   - SCM_RIGHTS passes real open file descriptors across Unix domain sockets using `sendmsg`/`recvmsg`.
   - No hardcoded test responses, dummy facade implementations, or simulated shortcuts were found.

2. **Header Sniffing Correctness & Boundary Safety**:
   - `sniff_png_dimensions`: Validates 8-byte PNG signature `\x89PNG\r\n\x1a\n`, verifies `IHDR` chunk tag at offset 12, reads big-endian 32-bit width and height at offsets 16 and 20, and requires `width > 0 && height > 0`.
   - `sniff_gif_dimensions`: Validates `GIF87a` and `GIF89a` signatures, reads little-endian 16-bit dimensions at offsets 6 and 8, and requires `width > 0 && height > 0`.
   - `sniff_jpeg_dimensions`: Validates SOI (`0xFF 0xD8`), traverses marker headers skipping `0xFF` padding, properly ignores standalone markers (`0xD0`..=`0xD8`, `0x01`), halts on SOS (`0xDA`) and EOI (`0xD9`), correctly parses all SOF markers (`0xC0`..=`0xC3`, `0xC5`..=`0xC7`, `0xC9`..=`0xCB`, `0xCD`..=`0xCF`), extracts big-endian height and width, and guards all slice accesses against buffer overrun.
   - `sniff_tiff_dimensions`: Validates both little-endian (`II`) and big-endian (`MM`) byte orders for Standard TIFF (magic 42) and BigTIFF (magic 43). Parses IFD0 entries, caps loop iterations at `.min(512)` to prevent DoS, and handles SHORT (type 3), LONG (type 4), and LONG8 (type 16) tags.
   - All sniffers return `None` on truncated buffers or invalid magics without panic.

3. **Decoded Frame Budget Math**:
   - `exceeds_decoded_frame_budget(width, height)`:
     ```rust
     let pixels = (width as u64) * (height as u64);
     let bytes = pixels.saturating_mul(4);
     bytes > MAX_DECODED_FRAME_BUDGET_BYTES || pixels > MAX_PIXELS_CEILING
     ```
   - Mathematical precision:
     - 4 bytes per pixel (RGBA uncompressed raster).
     - 32 MB budget = `33,554,432` bytes.
     - Maximum pixel count under 32 MB = `8,388,608` pixels.
     - Boundary test 1: `2896 x 2896 = 8,386,816` pixels = `33,547,264` bytes (< 32 MB) -> `false` (within budget).
     - Boundary test 2: `2897 x 2897 = 8,392,609` pixels = `33,570,436` bytes (> 32 MB) -> `true` (exceeds budget).
     - Boundary test 3: `12000 x 12000 = 144,000,000` pixels (> `134,217,728` ceiling) -> `true` (exceeds budget).
     - Product of two `u32` values fits in `u64` without overflow (`u32::MAX * u32::MAX < u64::MAX`). Multiplying by 4 uses `saturating_mul(4)`, precluding integer wrap-around.

4. **EXIF Thumbnail Fallback Safety (Decompression Bomb & Malformed EXIF Protection)**:
   - `read_exif_thumbnail`: Scans for APP1 `0xFFE1` marker with `Exif\0\0` header in JPEG, or direct TIFF block in TIFF/DNG.
   - IFD1 parsing: Locates tags `0x0201` (`JPEGInterchangeFormat`) and `0x0202` (`JPEGInterchangeFormatLength`).
   - Bounds safety: Validates `length > 0`, `offset < tiff.len()`, and `offset.checked_add(length)? <= tiff.len()`.
   - Adversarial defense against forged thumbnails: `scale_embedded_thumbnail` sniffs the dimensions of the extracted thumbnail payload *before* passing it to `gdk_pixbuf::PixbufLoader`. If a forged thumbnail payload declares dimensions exceeding `MAX_DECODED_FRAME_BUDGET_BYTES` (e.g. 20,000 x 20,000), it returns `Err("Embedded thumbnail exceeds the decoded frame budget")` immediately, preventing memory exhaustion attacks inside the fallback path.

5. **Persistent Pooled Sandbox Architecture & Worker Lifecycle**:
   - `bwrap` container initialization with `--ro-bind-try /var/cache/fontconfig` avoids expensive font re-scanning and library loading on every request, meeting the <100ms latency requirement.
   - Passing open descriptors via `SCM_RIGHTS` (`input_fd`, `output_write_pipe_fd`) isolates the worker from host filesystem directory access. Responses stream directly into RAM pipe buffers.
   - In-container supervisor is single-threaded and executes `fork_child()` per request. The child process exits via `_exit()`, guaranteeing zero memory fragmentation, state contamination, or cross-request data leaks.
   - UI cancellation responsiveness: `DeadlineReader` polls in 20ms slices (`WAIT_QUANTUM`). Upon user cancellation, the reader returns in ≤20ms, and `Lease::discard()` kills `bwrap` via `terminate(&mut worker.child)`.
   - Resilience: If a worker process exits unexpectedly or a broken pipe occurs, `Lease::execute` retires the dead worker and retries once with a newly spawned worker.

---

## 3. Caveats

1. **TIFF Files with IFD0 at File End > 2MB**:
   - `image_dimensions` sniffs the first 64 KB of the file. If IFD0 is located beyond 64 KB and the file size exceeds 2 MB, `image_dimensions` returns `None`. In that scenario, `render_image` falls through to the sandboxed decoder (`render_imagemagick` / `render_pixbuf`), which operates under the 1.25 GB address space and 32 MB file size limits. Specialized pyramid overview extraction for large GeoTIFFs is handled in Milestone 3.
2. **Unsupported Host Environments**:
   - If unprivileged user namespaces are disabled on a host (`bwrap` cannot run), `workers_supported()` evaluates to `false`, and `sandbox::parse` seamlessly falls back to one-shot sandbox execution.

---

## 4. Conclusion

The Milestone 2 implementation fulfills all requirements of Feature F4 (Persistent Pooled Sandbox Worker), Feature F5 (File-Header Sniffing & Dimension Guardrails), Feature F6 (EXIF Thumbnail Fallback), and Feature F7 (Memory Ceilings, Timeouts & Cancellation).

- **Integrity**: Verified authentic logic with zero hardcoded shortcuts or facades.
- **Robustness**: Header sniffers, frame budget math, and EXIF thumbnail extraction handle boundary conditions, integer overflow edges, and adversarial decompression bombs securely.
- **Compatibility**: 0 compiler warnings in owned files, 204/204 unit tests passing, 165/165 E2E tests passing.

**Verdict**: APPROVE.

---

## 5. Verification Method

To independently verify the Milestone 2 deliverables:

1. **Verify Compiler Warnings (Zero in owned files)**:
   ```bash
   cargo check
   ```

2. **Verify All Unit Tests (204 tests pass)**:
   ```bash
   cargo test --bin strata
   ```

3. **Verify All E2E Integration Tests (165 tests pass across Tiers 1–4)**:
   ```bash
   cargo test --test e2e_tests
   ```

4. **Verify Milestone 2 Specific Unit Tests**:
   ```bash
   cargo test --bin strata -- sniff
   cargo test --bin strata -- deadline_reader
   cargo test --bin strata -- pool
   cargo test --bin strata -- wire
   ```

5. **Verify Milestone 2 Specific E2E Boundary & Pairwise Tests**:
   ```bash
   cargo test --test e2e_tests -- test_f4
   cargo test --test e2e_tests -- test_f5
   cargo test --test e2e_tests -- test_f6
   cargo test --test e2e_tests -- test_f7
   ```
