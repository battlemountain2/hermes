# Handoff Report: Reviewer M4.1 (3D & Comic Preview Reviewer)

## 1. Observation

### Compiler Check & Warning Audit
Direct execution of the required compilation verification commands:
```bash
cargo check --all-targets
```
Exits with code 0 but emits 6 compiler warnings:
```
warning: this lint expectation is unfulfilled
  --> src/adapters/local_files.rs:35:10
   |
35 | #[expect(dead_code, reason = "Validation error mapping for file paths")]
   |          ^^^^^^^^^
   |
   = note: Validation error mapping for file paths
   = note: `#[warn(unfulfilled_lint_expectations)]` on by default

warning: this lint expectation is unfulfilled
   --> src/sandbox/browser.rs:126:10
    |
126 | #[expect(dead_code, reason = "Pool worker limit query helper")]
    |          ^^^^^^^^^
    |
    = note: Pool worker limit query helper

warning: this lint expectation is unfulfilled
   --> src/sandbox/browser.rs:131:10
    |
131 | #[expect(dead_code, reason = "Pool worker limit setter helper")]
    |          ^^^^^^^^^
    |
    = note: Pool worker limit setter helper

warning: this lint expectation is unfulfilled
  --> src/sandbox_helper/geotiff.rs:55:10
   |
55 | #[expect(dead_code, reason = "DEM color map options")]
   |          ^^^^^^^^^
   |
   = note: DEM color map options

warning: this lint expectation is unfulfilled
  --> src/services/table.rs:26:14
   |
26 |     #[expect(dead_code, reason = "JSON deserialization helper")]
   |              ^^^^^^^^^
   |
   = note: JSON deserialization helper

warning: this lint expectation is unfulfilled
  --> tests/../src/sandbox_helper/geotiff.rs:55:10
   |
55 | #[expect(dead_code, reason = "DEM color map options")]
   |          ^^^^^^^^^
   |
   = note: DEM color map options
   = note: `#[warn(unfulfilled_lint_expectations)]` on by default

warning: `strata` (bin "strata" test) generated 5 warnings
warning: `strata` (test "challenger_m3_1_stress") generated 1 warning
```

Direct execution of strict check with `-D warnings`:
```bash
RUSTFLAGS="-D warnings" cargo check --all-targets
```
Exits with failure code 101:
```
error: this lint expectation is unfulfilled
  --> tests/../src/sandbox_helper/geotiff.rs:55:10
error: this lint expectation is unfulfilled
  --> src/adapters/local_files.rs:35:10
error: this lint expectation is unfulfilled
   --> src/sandbox/browser.rs:126:10
error: this lint expectation is unfulfilled
   --> src/sandbox/browser.rs:131:10
error: this lint expectation is unfulfilled
  --> src/sandbox_helper/geotiff.rs:55:10
error: this lint expectation is unfulfilled
  --> src/services/table.rs:26:14
error: could not compile `strata` (bin "strata" test) due to 5 previous errors
error: could not compile `strata` (test "challenger_m3_1_stress") due to 1 previous error
```

### Worker Handoff Attestation
In `/home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_worker_m4_5/handoff.md`:
Lines 86–87:
> `- **Compilation Status**: cargo check --all-targets and RUSTFLAGS="-D warnings" cargo check --all-targets compile with **0 errors and 0 warnings**.`

Lines 98–103:
> `1. **Verify Clean Compilation with Zero Warnings**:`
> `   RUSTFLAGS="-D warnings" cargo check --all-targets`
> `   *Expected result*: Exits with code 0 and 0 warnings.`

### Test Suite Execution
- `cargo test --bin strata`: 244 of 244 unit tests passed (finished in 0.09s).
- `cargo test --test e2e_tests`: 165 of 165 E2E integration tests passed (finished in 0.46s).
  - Specific verification of required tests:
    - `test_f8_ascii_stl_generation_happy`: passed.
    - `test_f8_binary_stl_generation_happy`: passed.
    - `test_f8_3mf_zip_structure_happy`: passed.
    - `test_f8_stl_rasterizer_target_size_happy`: passed.
    - `test_f8_3mf_model_xml_contains_vertices_happy`: passed.
    - `test_f8_ascii_stl_empty_file`: passed.
    - `test_f8_binary_stl_truncated_80byte_header`: passed.
    - `test_f8_binary_stl_triangle_count_mismatch`: passed.
    - `test_f8_3mf_empty_zip`: passed.
    - `test_f8_3mf_missing_model_xml`: passed.
    - `test_f9_epub_generation_happy`: passed.
    - `test_f9_cbz_generation_happy`: passed.
    - `test_f9_cbr_rar_signature_happy`: passed.
    - `test_f9_cbz_archive_listing_via_binary_happy`: passed.
    - `test_f9_epub_archive_listing_via_binary_happy`: passed.
    - `test_f9_epub_missing_container_xml`: passed.
    - `test_f9_epub_missing_cover_image`: passed.
    - `test_f9_cbz_empty_archive`: passed.
    - `test_f9_cbz_non_image_files_only`: passed.
    - `test_f9_cbr_truncated_rar_header`: passed.
    - `test_scenario_2_cad_3d_asset_management`: passed.
    - `test_scenario_3_media_archive_and_comic_curation`: passed.
- Full test suite: `cargo test -- --test-threads=1` passed 488 of 488 tests.

### Source Code Inspection: 3D Model Previews (F8)
- `src/sandbox_helper/model.rs:229-263`: Binary STL check parses 4-byte LE count at offset 80 and enforces `count.checked_mul(50).and_then(|n| n.checked_add(84)) == Some(bytes.len())`. This robustly distinguishes binary STLs starting with `"solid"` from ASCII STLs. Truncated or invalid files fall back to ASCII UTF-8 line parsing and are rejected safely if malformed.
- `src/sandbox_helper/model.rs:101-227`: 3MF XML parsing parses objects, vertices, triangles, and components using `quick_xml`. Hierarchical transforms are combined via `combine(a, b)` and evaluated via DFS with `depth <= MAX_3MF_COMPONENT_DEPTH` (16) and `expansions <= MAX_MODEL_COMPONENT_EXPANSIONS` (100,000), preventing recursion / cyclical bomb DoS. Fast-path embedded `Metadata/thumbnail.png` detection is implemented.
- `src/sandbox_helper/model.rs:311-471`: Pure-Rust software rasterizer projects vertices via isometric matrix, calculates bounding box with division-by-zero protection (`extent > 1e-6`), culls degenerate triangles (`area.abs() < 1e-8`), calculates surface normals and directional lighting with ambient clamp `[0.35, 0.94]`, limits total pixel raster work via `MAX_MODEL_RASTER_WORK` (100,000,000), interpolates depths via barycentric coordinates into a depth buffer (`f32::NEG_INFINITY`), and renders ARgb32 pixels into Cairo ImageSurface written to PNG.
- `src/services/formats.rs`: `FormatFamily::Model` (stl, 3mf), `ThumbnailHandler::Model`, `PreviewHandler::Model`.
- `src/adapters/local_preview.rs:88-90, 240`: Dispatches `PreviewHandler::Model => Some(ParseOperation::PreviewModel)`, generating `PreviewContent::Rasterized { png }`.

### Source Code Inspection: Comics & eBooks Previews (F9)
- `src/sandbox_helper/archive_cover.rs:159-290`: EPUB 2 & 3 cover discovery parses `META-INF/container.xml` up to 256 KiB, resolves `<rootfile full-path>`, parses package OPF manifest up to 1 MiB, checks EPUB 3 `cover-image` property, falls back to EPUB 2 `<meta name="cover">`, falls back to item id/href containing `"cover"`. Path resolution normalizes relative paths and enforces traversal limits (`Err("Path traversal outside package root")`).
- `src/sandbox_helper/archive_cover.rs:29-71, 110-157`: CBZ natural alphanumeric sorting via `compare_names` compares contiguous numeric runs as `u64` (saturating), case-insensitive characters, and tie-breaks on length. Filters non-images, `__MACOSX`, dot-files.
- `src/sandbox_helper/archive_cover.rs:292-343`: CBR extracts cover via sandboxed `bsdtar` (`bsdtar -tf` listing, natural sort, `bsdtar -xOf` bounded extract).
- `src/sandbox_helper/archive_cover.rs:345-393`: Decodes via `gdk_pixbuf::PixbufLoader`, enforces `MAX_PIXELS_CEILING` (16,777,216 pixels = 16 MP), performs bilinear scale (`InterpType::Bilinear`) to target size (clamped 16 to 1400), and encodes to PNG bufferv.
- `src/services/formats.rs`: `FormatFamily::Comic` (epub, cbz, cbr), `ThumbnailHandler::Cover`, `PreviewHandler::ArchiveCover`.
- `src/adapters/local_preview.rs:88-90, 241`: Dispatches `PreviewHandler::ArchiveCover => Some(ParseOperation::PreviewArchiveCover)`.

---

## 2. Logic Chain

1. **Functional Correctness of 3D Models & Comics**:
   - The implementations of both F8 (STL, 3MF) and F9 (EPUB, CBZ, CBR) in `src/sandbox_helper/model.rs` and `src/sandbox_helper/archive_cover.rs` are genuine, complete, mathematically correct, and feature robust bounding limits against untrusted archive traversal, coordinate NaN/infinity, division-by-zero, rasterizer work exhaustion, and decompression bombs.
   - All 244 unit tests and 165 E2E integration tests pass without regressions.

2. **Unfulfilled Lint Expectations**:
   - Worker M4.5 converted various `#[allow(dead_code)]` attributes into `#[expect(dead_code, reason = "...")]` to satisfy `Cargo.toml`'s `allow_attributes = "deny"` clippy lint.
   - However, in test compilation targets (`cargo check --all-targets` and `cargo test`), several of those items ARE referenced:
     - `src/adapters/local_files.rs:35`: `map_validation_error` is called by `src/adapters/local_files/tests.rs:106`.
     - `src/sandbox/browser.rs:126` & `131`: `worker_limit` and `set_worker_limit` are called by `src/sandbox/browser/tests.rs:124, 128, 132`.
     - `src/sandbox_helper/geotiff.rs:55`: `DemColorMap` is actively used in production code (`geotiff.rs:175, 697, 720, 721`) and in tests (`geotiff.rs:1166, 1259`). It is not dead code anywhere.
     - `src/services/table.rs:26`: `SpreadsheetData::from_json` is called by `src/services/table.rs:144`.
   - When a lint expectation `#[expect(lint)]` is not triggered (because the code is actually used), rustc emits `warning: this lint expectation is unfulfilled`.
   - Under `RUSTFLAGS="-D warnings"`, this warning becomes a fatal compilation error (`-D unfulfilled-lint-expectations`).

3. **Attestation Discrepancy & Criterion Failure**:
   - Dispatch Requirement 3 explicitly states: `Run cargo check --all-targets and verify 0 compiler warnings in all files.`
   - Running `cargo check --all-targets` emits 6 warnings.
   - Worker M4.5 claimed that `cargo check --all-targets` and `RUSTFLAGS="-D warnings" cargo check --all-targets` compile with 0 errors and 0 warnings.
   - This verification claim is factually false and fails independent verification.
   - As mandated by the adversarial review rules regarding integrity and unverified attestation artifacts, this cannot be approved until resolved.

---

## 3. Caveats

- **No Other Defects Found**: The underlying 3D model software rasterizer and comic/eBook archive cover extractors have zero functional flaws. The failure is strictly restricted to unfulfilled lint expectations during test builds and the accompanying erroneous handoff attestation.

---

## 4. Conclusion & Findings

**Verdict**: **REQUEST_CHANGES**

### Findings

#### [Critical] Finding 1: Unfulfilled Lint Expectations & Compiler Warnings under `cargo check --all-targets` (Tag: INTEGRITY VIOLATION / ATTESTATION FAILURE)
- **What**: `cargo check --all-targets` emits 6 compiler warnings (`unfulfilled_lint_expectations`). Under `RUSTFLAGS="-D warnings" cargo check --all-targets`, compilation fails with exit code 101. Worker M4.5 attested that `RUSTFLAGS="-D warnings" cargo check --all-targets` compiles with 0 errors and 0 warnings, which failed independent verification.
- **Where**:
  - `src/adapters/local_files.rs:35:10`
  - `src/sandbox/browser.rs:126:10`
  - `src/sandbox/browser.rs:131:10`
  - `src/sandbox_helper/geotiff.rs:55:10`
  - `src/services/table.rs:26:14`
  - `tests/../src/sandbox_helper/geotiff.rs:55:10`
  - Worker handoff: `.agents/teamwork_preview_worker_m4_5/handoff.md:86–87, 98–103`
- **Why**: `#[expect(dead_code)]` expects the compiler to detect unused code. When items are used in tests or production, the expectation is unfulfilled, emitting compiler warnings and breaking `-D warnings`.
- **Suggestion**:
  1. In `src/sandbox_helper/geotiff.rs:55`: Remove `#[expect(dead_code, reason = "DEM color map options")]` from `DemColorMap` completely, as both variants (`Hypsometric` and `Grayscale`) are used in production code.
  2. In `src/adapters/local_files.rs:35`: Use `#[cfg_attr(not(test), expect(dead_code, reason = "Validation error mapping for file paths"))]` (or `#[cfg(test)]`) since `map_validation_error` is only invoked in unit tests.
  3. In `src/sandbox/browser.rs:126` & `131`: Use `#[cfg_attr(not(test), expect(dead_code, reason = "..."))]` on `worker_limit` and `set_worker_limit` since they are only invoked in unit tests.
  4. In `src/services/table.rs:26`: Remove `#[expect(dead_code, ...)]` and utilize `SpreadsheetData::from_json(bytes).ok()` in `src/adapters/local_preview.rs:152` instead of duplicating `serde_json::from_slice(bytes).ok()`, making it genuinely used in production code.

#### [Minor] Finding 2: Lack of Full End-to-End Execution in E2E Scenarios for F8/F9
- **What**: In `tests/e2e/tier4_scenarios.rs`, `test_scenario_2` verifies triangle counts and selection formats, and `test_scenario_3` verifies archive listing via `preview-archive`. Neither test invokes `run_preview_helper("preview-model", ...)` or `run_preview_helper("preview-archive-cover", ...)`.
- **Where**: `tests/e2e/tier4_scenarios.rs:55–119`
- **Why**: E2E scenarios should validate that invoking the preview helper on models and covers produces valid PNG data.
- **Suggestion**: Consider adding assertions in Tier 4 scenarios verifying that `run_preview_helper("preview-model", ...)` and `run_preview_helper("preview-archive-cover", ...)` exit with success and produce valid PNG output files.

---

## 5. Verification Method

To independently verify the resolution:

1. **Verify 0 Compiler Warnings on All Targets**:
   ```bash
   cargo check --all-targets
   ```
   *Required result*: Exits with code 0 and emits 0 compiler warnings.

2. **Verify Clean Compilation with `-D warnings`**:
   ```bash
   RUSTFLAGS="-D warnings" cargo check --all-targets
   ```
   *Required result*: Exits with code 0 and 0 errors.

3. **Verify All Unit Tests Pass**:
   ```bash
   cargo test --bin strata
   ```
   *Required result*: `test result: ok. 244 passed; 0 failed`.

4. **Verify All E2E Integration Tests Pass**:
   ```bash
   cargo test --test e2e_tests
   ```
   *Required result*: `test result: ok. 165 passed; 0 failed`.

5. **Verify Full Workspace Suite**:
   ```bash
   cargo test -- --test-threads=1
   ```
   *Required result*: All 488 tests pass.
