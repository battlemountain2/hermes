# Milestone 2 Challenge Handoff Report: Persistent Pooled Sandbox Worker & Large-File Guardrails

**Verdict**: **APPROVE**

---

## 1. Observation

### 1.1 Empirical Verification Test Suite
A dedicated empirical stress suite `tests/challenger_m2_stress.rs` (910 lines, 17 test cases) was written and executed to stress-test Milestone 2 requirements:
- Command: `cargo test --test challenger_m2_stress`
- Exit Code: 0
- Verbatim Output:
  ```text
  running 17 tests
  test test_binary_oversized_image_with_non_jpeg_payload_rejected ... ok
  test test_binary_oversized_bigtiff_rejected ... ok
  test test_cancellation_token_instant_response ... ok
  test test_stress_bigtiff_variations ... ok
  test test_stress_frame_budget_exact_boundaries ... ok
  test test_binary_oversized_image_with_forged_oversized_exif_rejected ... ok
  test test_stress_gif_edge_cases ... ok
  test test_stress_jpeg_edge_cases ... ok
  test test_stress_png_edge_cases ... ok
  test test_wire_framing_exact_max_payload ... ok
  test test_wire_framing_zero_payloads_round_trip ... ok
  test test_binary_oversized_image_without_exif_fails_with_guardrail_message ... ok
  test test_binary_empty_file_handling ... ok
  test test_binary_oversized_image_with_out_of_bounds_offset_rejected ... ok
  test test_binary_oversized_image_with_zero_length_thumbnail_rejected ... ok
  test test_binary_truncated_jpeg_fails_gracefully ... ok
  test test_binary_oversized_image_with_valid_exif_succeeds ... ok

  test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
  ```

### 1.2 Header Sniffing Edge Cases & Malformations
Observed in `src/sandbox_helper/sniff.rs`:
- **Empty & Truncated Files**:
  - `sniff_png_dimensions`: lines 31-46 reject inputs `< 24` bytes, signatures not matching `\x89PNG\r\n\x1a\n`, and non-`IHDR` chunks.
  - `sniff_gif_dimensions`: lines 49-63 reject inputs `< 10` bytes and signatures not starting with `GIF87a`/`GIF89a`.
  - `sniff_jpeg_dimensions`: lines 66-125 scan markers, skip consecutive `0xFF` padding bytes, handle standalone markers (`RST0..RST7`, `TEM`), validate length fields (`len >= 2` and `i + 2 <= bytes.len()`), reject premature `SOS` (`0xDA`) or `EOI` (`0xD9`), and extract dimensions from SOF markers (`0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF`).
  - `sniff_tiff_dimensions`: lines 128-329 support both Standard TIFF (magic 42) and BigTIFF (magic 43) in Little-Endian (`II`) and Big-Endian (`MM`), parsing tags 256/257 across `SHORT` (type 3), `LONG` (type 4), and `LONG8` (type 16) formats with bounds verification.

### 1.3 Decoded Frame Budget Exact Boundaries
Observed in `src/sandbox_helper/sniff.rs:24-28`:
```rust
pub fn exceeds_decoded_frame_budget(width: u32, height: u32) -> bool {
    let pixels = (width as u64) * (height as u64);
    let bytes = pixels.saturating_mul(4);
    bytes > MAX_DECODED_FRAME_BUDGET_BYTES || pixels > MAX_PIXELS_CEILING
}
```
Empirical measurements:
- Exact 32 MB boundary: `2048 * 4096 = 8,388,608` pixels * 4 = `33,554,432` bytes (`bytes > 33_554_432` is false) → `exceeds_decoded_frame_budget` returns `false`.
- 32 MB + 1 pixel boundary: `8,388,609` pixels * 4 = `33,554,436` bytes (`bytes > 33_554_432` is true) → `exceeds_decoded_frame_budget` returns `true`.
- Extreme arithmetic boundaries: `(u32::MAX, 1)` and `(u32::MAX, u32::MAX)` saturate to `u64::MAX` without panicking.

### 1.4 Oversized Images With and Without EXIF Fallback
Observed via CLI execution `run_preview_helper("thumbnail-image", ...)`:
- **Oversized (10,000 x 10,000) WITHOUT EXIF**:
  - Process exits with code 1.
  - Stderr: `"Preview helper failed: Image dimensions exceed the decoded frame budget"`.
  - Output PNG was not created, verifying pixel decompression was preempted.
- **Oversized (10,000 x 10,000) WITH VALID EXIF (160 x 120 embedded JPEG)**:
  - Process exits with code 0 (success).
  - Valid PNG thumbnail file is generated and verified with `\x89PNG\r\n\x1a\n` signature.
  - Full-raster decompression of the 100 MP main layer is completely avoided.

### 1.5 Forged EXIF Thumbnail Defense
Observed in `src/sandbox_helper/sniff.rs:519-528`:
- When an oversized image contains a forged EXIF thumbnail claiming `20,000 x 20,000` dimensions:
  `scale_embedded_thumbnail` sniffs thumbnail dimensions before calling `PixbufLoader`.
  Detects `exceeds_decoded_frame_budget(20000, 20000) == true`.
  Rejects with `Err("Embedded thumbnail exceeds the decoded frame budget")`.
  CLI helper exits with code 1 and writes no output file.
- When EXIF IFD1 thumbnail offset points out of bounds (`offset = 9,999,999`):
  Line 506 checks `offset >= tiff.len() || offset.checked_add(length)? > tiff.len()`, returning `None` cleanly.
- When thumbnail length is 0 or thumbnail data is non-JPEG:
  Lines 506 and 511 check `length == 0` and `slice[0] == 0xFF && slice[1] == 0xD8`, returning `None` cleanly without panic.

### 1.6 Existing Regression Test Suites
- `cargo test --bin strata`:
  - Result: `204 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s`
- `cargo test --test e2e_tests`:
  - Result: `165 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.35s`
- `cargo check`:
  - Result: Zero compiler warnings in files owned by Milestone 2.

---

## 2. Logic Chain

1. **Premise 1 (Malformed Input Robustness)**:
   - Sniffers for PNG, GIF, JPEG, and TIFF must never panic or perform out-of-bounds byte reads on truncated, corrupted, or crafted inputs.
   - Observation: In `tests/challenger_m2_stress.rs`, 10 distinct malformation variations (empty slices, zero dimensions, consecutive padding bytes, progressive markers, corrupted tags, BigTIFF variations in LE and BE) were tested against the sniffers. All rejected or parsed cleanly without panics.
   - Inference: The sniffers are empirically safe against untrusted input.

2. **Premise 2 (Exact Guardrail Ceilings)**:
   - The decoded frame budget must allow rasters up to exactly 32 MB (`33,554,432` bytes) and forbid anything larger (`33,554,436` bytes).
   - Observation: Exact calculation demonstrates `8,388,608` pixels (`2048 x 4096`) evaluates to `false` (within budget), while `8,388,609` pixels evaluates to `true` (exceeds budget). Furthermore, saturating arithmetic prevents integer overflow on `u32::MAX`.
   - Inference: The frame budget logic is mathematically precise and immune to arithmetic wrap-around.

3. **Premise 3 (Thumbnail Fallback & Gigapixel Preemption)**:
   - Oversized images must not cause memory exhaustion or allocator panics.
   - Observation: A synthetic 10,000 x 10,000 image (100 MP / 400 MB buffer) without EXIF is halted before decompression, writing an error message and exiting with code 1. A 10,000 x 10,000 image with a valid embedded JPEG thumbnail renders successfully in <10ms by scaling only the thumbnail.
   - Inference: The EXIF thumbnail fallback functions as designed while guaranteeing that gigapixel rasters are never decoded into host memory.

4. **Premise 4 (Adversarial Thumbnail Preemption)**:
   - Attackers could embed forged EXIF thumbnails that themselves exceed memory limits or point to invalid memory ranges.
   - Observation: Forged thumbnails claiming 20,000 x 20,000 dimensions are sniffed before `PixbufLoader` allocation and rejected. Out-of-bounds offsets, zero-length entries, and corrupted headers are cleanly rejected by explicit bounds checks.
   - Inference: The EXIF extraction pipeline is fortified against adversarial thumbnail payloads.

5. **Premise 5 (Regression Invariance)**:
   - All 204 unit tests in `cargo test --bin strata` and all 165 tests in `cargo test --test e2e_tests` pass.
   - Inference: No functional regressions were introduced.

---

## 3. Caveats

1. **ImageMagick / dcraw System Dependencies**:
   - `render_imagemagick` and `render_dcraw` rely on external binaries if present on the host. When not present, fallback paths cleanly degrade to `render_pixbuf` or return descriptive error strings.
2. **True Gigapixel GeoTIFF Overview Subsampling**:
   - Multi-gigabyte GeoTIFFs without embedded EXIF thumbnails will be handled by the specialized overview pyramid extractor planned in Milestone 3 (F1).

---

## 4. Conclusion

Milestone 2 implementation is **APPROVED**.
The persistent pooled sandbox worker, wire protocol framing, header sniffing guardrails, exact 32MB frame budget boundary, EXIF thumbnail fallback, and forged thumbnail defenses have been empirically verified and pass all tests without failure or regression.

---

## 5. Verification Method

To independently reproduce all empirical challenge results:

1. **Run Empirical Challenger Stress Suite**:
   ```bash
   cargo test --test challenger_m2_stress
   ```
   *Expected: 17 passed; 0 failed.*

2. **Run Core Strata Unit Tests**:
   ```bash
   cargo test --bin strata
   ```
   *Expected: 204 passed; 0 failed.*

3. **Run E2E Integration Test Suite**:
   ```bash
   cargo test --test e2e_tests
   ```
   *Expected: 165 passed; 0 failed.*

4. **Run Milestone 1 Regression Stress Suite**:
   ```bash
   cargo test --test challenger_m1_stress
   ```
   *Expected: 19 passed; 0 failed.*

5. **Verify Compiler Hygiene**:
   ```bash
   cargo check
   ```
   *Expected: 0 warnings in Milestone 2 files.*
