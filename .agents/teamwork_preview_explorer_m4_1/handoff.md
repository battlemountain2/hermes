# Handoff Report — Explorer M4.1 (3D & Comic Preview Explorer)

## 1. Observation

### Reference Commits
1. **Commit `570bdd30` ("feat(preview): add sandboxed STL, 3MF and FreeCAD previews (#1276)"):**
   - Introduced `src/sandbox_helper/model.rs` (583 lines) and `src/sandbox_helper/model/embedded.rs` (208 lines).
   - Added geometry parsing for STL (ASCII and binary) and 3MF (`3D/3dmodel.model` via XML), plus FreeCAD (`thumbnails/Thumbnail.png`).
   - Implemented a software rasterizer using a fixed-angle isometric projection, directional lighting, 2D barycentric triangle rasterization, depth buffer ($Z$-buffer), and work bounds (`MAX_MODEL_RASTER_WORK = 100_000_000`).
   - Defined strict bounds in `src/services/model_preview.rs`:
     - `MAX_MODEL_INPUT_BYTES = 128 * 1024 * 1024` (128 MiB)
     - `MAX_MODEL_XML_BYTES = 128 * 1024 * 1024` (128 MiB)
     - `MAX_3MF_ARCHIVE_ENTRIES = 256`
     - `MAX_3MF_OBJECTS = 1024`, `MAX_3MF_BUILD_ITEMS = 1024`, `MAX_3MF_COMPONENT_DEPTH = 16`
     - `MAX_MODEL_TRIANGLES = 2_000_000`, `MAX_MODEL_VERTICES = 2_000_000`
     - `MAX_MODEL_COMPONENT_REFERENCES = 100_000`, `MAX_MODEL_COMPONENT_EXPANSIONS = 100_000`
     - `MAX_MODEL_RASTER_WORK = 100_000_000`

2. **Commit `ba676d3e` ("feat(preview): show comic and EPUB covers in thumbnails and Quick Preview (#1325)"):**
   - Introduced `src/sandbox_helper/archive_cover.rs` (294 lines) and `src/sandbox_helper/archive_rar.rs` (126 lines).
   - Implemented EPUB cover discovery:
     - Reads `META-INF/container.xml` (capped at 256 KiB) to locate the rootfile path (e.g. `OEBPS/content.opf`).
     - Reads OPF XML package document to locate the cover image:
       - EPUB 3: `<item properties="... cover-image ..." href="...">`
       - EPUB 2: `<meta name="cover" content="<id>"/>` referencing `<item id="<id>" href="...">`
     - Resolves relative path against OPF directory base, unescaping URI characters (`glib::uri_unescape_string`).
   - Implemented CBZ cover discovery:
     - Inspects ZIP entries (capped at `MAX_ENTRIES = 4096`).
     - Natural alphanumeric sort algorithm (`compare_names`) across image filenames (`.jpg`, `.jpeg`, `.png`, `.webp`, `.gif`) so `page2.png` precedes `page10.png`. Picks the earliest page.
   - Enforced memory limits:
     - `MAX_INPUT_BYTES = 128 MiB`
     - `MAX_IMAGE_BYTES = 8 MiB`
     - Decoded frame dimension ceiling: $W \times H \le 16 \text{ MP}$ ($16,777,216$ pixels) via `gdk_pixbuf::PixbufLoader::connect_size_prepared`.
     - Output scaled to preview edge size (e.g. 256 or 1400) and encoded to PNG.

### Current Hermes Codebase State
1. **Existing Test Baseline (`cargo test`):**
   - Unit tests in `src/main.rs`: 216 passed.
   - Integration tests in `tests/e2e_tests.rs`: 165 passed across Tiers 1–4.
   - Challenger suites: `challenger_m1_stress` (19 passed), `challenger_m3_1_stress` (12 passed), `challenger_m3_2_stress` (15 passed).
   - Fixtures already exist in `tests/fixtures/models.rs` (`sample_ascii_stl`, `sample_binary_stl`, `sample_3mf_model`, `sample_corrupted_stl`) and `tests/fixtures/archives.rs` (`sample_epub`, `sample_cbz`, `sample_cbr`, `sample_corrupted_archive`).
   - Boundary tests in `tests/e2e/tier2_boundaries.rs` explicitly test empty/truncated STL headers, triangle count mismatches, missing `container.xml`, and empty comic archives.

2. **Sandbox & Wire Protocol (`src/sandbox/browser/wire.rs`, `src/sandbox/browser/worker.rs`, `src/sandbox.rs`):**
   - Framing header is 8 bytes: `[png_len: u32, metadata_len: u32]` followed by PNG payload and metadata JSON payload.
   - `Operation` enum in `src/sandbox/browser/wire.rs`:
     - Currently has values 1 to 13: `Image = 1`, `Raw = 2`, `Pdf = 3`, `Video = 4`, `ImageMetadata = 5`, `MediaMetadata = 6`, `PreviewImage = 7`, `DocumentMermaid = 8`, `DocumentMath = 9`, `DocumentMathInline = 10`, `ThreeMfThumbnail = 11`, `FreeCadThumbnail = 12`, `PreviewGeoTiff = 13`.
     - Next operation slots: `PreviewModel = 14`, `PreviewArchiveCover = 15`.
   - `ParseOperation` in `src/sandbox.rs`:
     - Dispatches through persistent worker pool via `browser::preview` / `browser::thumbnail_parse` or falls back to one-shot `bwrap` process execution (`parse_one_shot`).
     - CLI helper entry point `src/sandbox_helper.rs:run(arguments: &[String])` handles `--preview-helper <operation> <input> <output> <value>`.

3. **Format Classification & UI (`src/services/formats.rs`, `src/adapters/local_preview.rs`, `src/ui/preview.rs`):**
   - `src/services/formats.rs`:
     - `classify_by_name`: Currently classifies standard raster images, raw images, office documents, archives (`zip`, `tar`, `rar`, etc.), but does NOT yet classify `.stl`, `.3mf`, `.epub`, `.cbz`, `.cbr`.
     - `thumbnail_handler_for_name`: Currently returns `None` for `.stl`, `.3mf`, `.epub`, `.cbz`, `.cbr`.
   - `src/services/preview.rs`:
     - `PreviewContent` enum already contains `Rasterized { png: Vec<u8> }` and `Model3D { format: String, data: Vec<u8> }`.
   - `src/adapters/local_preview.rs`:
     - Currently maps `PreviewHandler` to `ParseOperation` and dispatches via `sandbox::parse`.
     - Maps rasterized output directly into `PreviewContent::Rasterized { png: output.data }`.
   - `src/ui/preview.rs`:
     - `PreviewContent::Rasterized { png }` is already fully wired to `build_interactive_texture_view(&texture)` (lines 426–435), supporting interactive pan-and-zoom directly in the preview drawer.

4. **Dependencies (`Cargo.toml`):**
   - Current dependencies: `cairo-rs = { version = "0.21.5", features = ["pdf", "png"] }`, `gdk-pixbuf = "0.21.5"`, `geotiff-core = "0.8.1"`, `gio`, `gtk4`, `ignore`, `poppler-rs`, `serde`, `serde_json`, `sourceview5`, `tiff`, `toml`, `tracing`.
   - Missing for M4:
     - `zip = "8.6.0"`: needed for reading 3MF packages, EPUB books, and CBZ comic archives.
     - `quick-xml = "0.41.0"` (or `0.42.0`): needed for parsing 3MF XML geometry and EPUB container/OPF XML.
   - System environment: `/usr/bin/bsdtar`, `/usr/bin/unrar`, `/usr/bin/7z` are installed and bind-mounted read-only inside the Bubblewrap container (`--ro-bind /usr /usr`).

---

## 2. Logic Chain

### 3D Model Rasterization Pipeline (STL & 3MF)
1. **Format Sniffing & STL Parsing:**
   - Binary STLs often contain the word `"solid"` within their 80-byte comment header. A naive `starts_with("solid")` check causes binary STLs to be misread as corrupted ASCII.
   - Therefore, the parser MUST test binary STL structure first:
     ```rust
     if bytes.len() >= 84 {
         let count = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
         if count.checked_mul(50).and_then(|n| n.checked_add(84)) == Some(bytes.len()) {
             // Valid binary STL
         }
     }
     ```
   - If exact length matches, parse binary triangles (50 bytes each: normal vector [12 bytes] + 3 vertices [36 bytes] + attribute byte count [2 bytes]). Validate coordinates with `f32::is_finite()`.
   - If length does not match, parse as ASCII STL: require trimmed start to begin with `"solid"`, parse lines starting with `"vertex"`, group triplets into faces, and verify non-empty geometry.
   - Enforce `MAX_MODEL_TRIANGLES = 2_000_000`.

2. **3MF Model Extraction:**
   - 3MF files are OPC ZIP packages.
   - If package contains an embedded PNG thumbnail (`Metadata/thumbnail.png` or relationships target), fast-path extraction can reuse that PNG without full rasterization if requested.
   - To render geometry: open `3D/3dmodel.model` via `zip::ZipArchive`.
   - Parse XML using `quick-xml::Reader`:
     - Accumulate `<object id="...">` resources containing `<mesh>` vertices and triangles, or `<component objectid="..." transform="...">` references.
     - Accumulate `<build><item objectid="..." transform="..."/></build>` roots.
     - Resolve component tree iteratively using a DFS stack:
       - Combine 3x4 affine transformation matrices: $combine(A, B)$.
       - Enforce component depth limit $\le 16$ and expansion count $\le 100,000$.
       - Transform vertices: $[x', y', z'] = M \cdot [x, y, z, 1]^T$.
       - Enforce vertex limit $\le 2,000,000$ and triangle limit $\le 2,000,000$.

3. **Software Rasterizer & Shading:**
   - Camera: Fixed-angle isometric projection:
     $$\begin{pmatrix} x' \\ y' \\ z' \end{pmatrix} = \begin{pmatrix} -0.83 & 0.55 & 0.0 \\ 0.35 & 0.53 & 0.77 \\ -0.43 & -0.64 & 0.64 \end{pmatrix} \begin{pmatrix} x \\ y \\ z \end{pmatrix}$$
   - Extent & Scale:
     - Compute 2D bounding box $[min_x, max_x] \times [min_y, max_y]$ of projected vertices.
     - Scale factor $S = (0.82 \times \min(W, H)) / \max(max_x - min_x, max_y - min_y)$.
     - Center point $C = [(min_x + max_x)/2, (min_y + max_y)/2]$.
     - Screen coordinates: $X = W/2 + (x' - C_x) S$, $Y = H/2 - (y' - C_y) S$.
   - Directional Shading:
     - Normal vector $N = (P_1 - P_0) \times (P_2 - P_0)$.
     - Light intensity: $light = \text{clamp}(0.58 + 0.36 \frac{|0.3 N_x - 0.5 N_y + 0.8 N_z|}{\|N\|}, 0.35, 0.94)$.
     - Shading color: blend accent and surface theme colors with $light$.
   - $Z$-Buffer Depth Rasterization:
     - Work bound: accumulate $(right - left) \times (bottom - top)$ per triangle. Abort if work exceeds $100,000,000$.
     - Barycentric coordinates via 2D edge function $edge(A, B, P) = (B_x - A_x)(P_y - A_y) - (B_y - A_y)(P_x - A_x)$.
     - Interpolate depth: $Z = \sum w_i z'_i$.
     - If $Z > depth\_buffer[Y \times W + X]$, update depth buffer and pixel buffer.
   - Pure-Rust Cairo/PNG Encoding:
     - Rather than pulling in `tiny-skia` or `resvg`, encode the $W \times H$ ARGB32 buffer into PNG using `cairo::ImageSurface` or `gdk_pixbuf::Pixbuf::from_mut_slice`, both of which are already compiled into Hermes.

---

### eBooks & Comics Cover Art Extraction (EPUB, CBZ, CBR)
1. **CBZ Extraction:**
   - Read ZIP archive entries using `zip::ZipArchive`.
   - Filter entries matching standard image extensions (`jpg`, `jpeg`, `png`, `webp`, `gif`).
   - Use natural numerical sorting (`compare_names`) so that numeric sequences sort correctly (`page1.png`, `page2.png`, `page10.png`).
   - Select the earliest candidate. If no image exists, return error (`"Comic archive has no bounded image"`).
   - Read image payload with size limit `MAX_IMAGE_BYTES = 8 MiB`.

2. **EPUB Extraction:**
   - Open EPUB ZIP archive.
   - Read `META-INF/container.xml` (limit 256 KiB) to find `<rootfile full-path="...">`.
   - Open and parse the package OPF file with `quick-xml`:
     - Match `<item properties="... cover-image ..." href="...">` (EPUB 3).
     - Fallback: match `<meta name="cover" content="<id>"/>` to `<item id="<id>" href="...">` (EPUB 2).
   - Resolve relative path against OPF directory base, unescaping URI percent-encoding (`glib::uri_unescape_string`). Reject directory traversal (`..` escaping package root).
   - Read cover image payload with size limit `MAX_IMAGE_BYTES = 8 MiB`.

3. **CBR Extraction:**
   - CBR archives use the proprietary RAR format (RAR 4.x or RAR 5.x).
   - Two viable architectural paths:
     - **Path A (`bsdtar` in sandbox — Recommended):**
       - The Bubblewrap sandbox already bind-mounts `/usr` read-only and uses `bsdtar` for archive listings (`preview-archive`) and office documents (`src/sandbox_helper.rs:230`).
       - Run `bsdtar -tf <cbr_path>` with a 1 MB output limit to list members.
       - Apply `compare_names` natural sort to select the first image entry.
       - Run `bsdtar -xOf <cbr_path> <selected_entry>` with `MAX_IMAGE_BYTES = 8 MiB` limit to extract the cover bytes.
       - **Benefits:** 100% pure Rust host code, zero C++ `unrar-sys` build dependencies, complies with Hermes's `unsafe_code = "deny"` policy, immune to native C++ RAR parser heap vulnerabilities.
     - **Path B (`unrar` crate):**
       - Wraps the unrar C++ library via `unrar_sys`. Requires unsafe C bindings and C++ compiler toolchain.
     - **Conclusion:** Path A is cleaner, safer, faster to integrate, and fully compatible with existing Hermes sandboxing.

4. **Image Decode & Normalization:**
   - Stream extracted cover bytes into `gdk_pixbuf::PixbufLoader`.
   - Enforce 16 MP image limit (`width * height <= 16_777_216`) inside `connect_size_prepared`.
   - Scale down to requested edge size (e.g. 256 for thumbnails, 1400 for preview) with bilinear interpolation.
   - Save to PNG format via `pixbuf.save_to_bufferv("png", &[("compression", "1")])`.

---

### Wire Protocol & Architectural Integration
1. **Wire Protocol (`src/sandbox/browser/wire.rs`):**
   - Add operations:
     ```rust
     pub(crate) enum Operation {
         ...
         PreviewModel = 14,
         PreviewArchiveCover = 15,
     }
     ```
   - Framing remains standard: 8-byte LE header `[png_len: u32, metadata_len: u32]` followed by PNG bytes and empty/null metadata bytes.

2. **Sandbox Helper Worker Dispatch (`src/sandbox/browser/worker.rs`):**
   - Handle operations in `execute_job`:
     ```rust
     Operation::PreviewModel => {
         let png = crate::sandbox_helper::model::render_model(input_path, 512)?;
         (png, None)
     }
     Operation::PreviewArchiveCover => {
         let png = crate::sandbox_helper::archive_cover::render_cover(input_path, 1400)?;
         (png, None)
     }
     ```

3. **One-shot CLI Fallback (`src/sandbox_helper.rs`, `src/sandbox.rs`):**
   - `ParseOperation` additions: `PreviewModel`, `PreviewArchiveCover`.
   - CLI flags: `"preview-model"`, `"preview-archive-cover"`.
   - Output filename: `"result.png"`.

4. **Format Registry (`src/services/formats.rs`):**
   - Add `FormatFamily::Model` (STL, 3MF) and `FormatFamily::Comic` (EPUB, CBZ, CBR).
   - In `classify_by_name`:
     ```rust
     "stl" | "3mf" => FormatFamily::Model,
     "epub" | "cbz" | "cbr" => FormatFamily::Comic,
     ```
   - In `thumbnail_handler_for_name`:
     ```rust
     "stl" | "3mf" => Some(ThumbnailHandler::Model),
     "epub" | "cbz" | "cbr" => Some(ThumbnailHandler::Cover),
     ```
   - In `PreviewHandler`:
     ```rust
     pub enum PreviewHandler {
         ...
         Model,
         ArchiveCover,
     }
     ```

5. **Adapter & UI Wiring (`src/adapters/local_preview.rs`, `src/ui/preview.rs`):**
   - In `src/adapters/local_preview.rs`:
     - Dispatch `PreviewHandler::Model` $\to$ `ParseOperation::PreviewModel`.
     - Dispatch `PreviewHandler::ArchiveCover` $\to$ `ParseOperation::PreviewArchiveCover`.
     - Both return `output.data`, which maps to `PreviewContent::Rasterized { png: output.data }`.
   - In `src/ui/preview.rs`:
     - `PreviewContent::Rasterized { png }` is already implemented and directly renders via `build_interactive_texture_view(&texture)`!
     - Provides interactive mouse-drag panning, zoom via Ctrl+Scroll, and reset via Ctrl+0 out-of-the-box.

---

## 3. Caveats

1. **FreeCAD (`.fcstd`) Files:**
   - FreeCAD files store a pre-rendered thumbnail at `thumbnails/Thumbnail.png` inside the ZIP archive. If FreeCAD support is desired, it can be handled by `embedded.rs` without any geometry parsing.
2. **Multi-part 3MF Files:**
   - 3MF files referencing external multi-part models (e.g. `path="..."` in component items) cannot be resolved when parsing a single isolated file descriptor without additional file mounts. The reference implementation properly rejects these with `"Multipart model detected. Unable to render preview."`.
3. **CBR Password Protection:**
   - Encrypted RAR archives cannot be extracted without a password; they should fail gracefully reporting that the cover cannot be decoded.
4. **Offline Cargo Cache & Dependencies:**
   - Adding `zip` and `quick-xml` to `Cargo.toml` requires a network fetch or pre-cached crate versions. During our investigation, `cargo add --dry-run zip` and `cargo add --dry-run quick-xml` confirmed crates.io is reachable and resolves `zip v8.6.0` and `quick-xml v0.42.0` cleanly.

---

## 4. Conclusion

- **Feasibility:** 100% feasible with pure-Rust geometry parsing and archive extraction.
- **Cargo Dependencies:**
  - Add `zip = "8.6.0"` (with features `["deflate"]`).
  - Add `quick-xml = "0.41.0"` (or `0.42.0`).
  - No need for `resvg`, `tiny-skia`, or `unrar-sys`; Cairo and `gdk-pixbuf` already handle rasterization output and cover image decoding within existing sandbox boundaries.
- **Security & Sandboxing:**
  - Strict resource limits (`prlimit` AS 1.25 GB, CPU 10s, FSIZE 32 MB).
  - Geometry ceilings (`MAX_MODEL_TRIANGLES = 2,000,000`, `MAX_MODEL_RASTER_WORK = 100,000,000`).
  - Archive ceilings (`MAX_ENTRIES = 4096`, `MAX_IMAGE_BYTES = 8 MiB`, `MAX_XML_BYTES = 256 KiB`).
  - Pixel ceilings (16 MP max for cover decode).
- **Wire Protocol & UI Integration:**
  - Seamlessly integrates with the persistent worker pool via `Operation::PreviewModel` and `Operation::PreviewArchiveCover`.
  - Emits `PreviewContent::Rasterized { png }`, which is already handled by `src/ui/preview.rs` with full interactive pan-and-zoom controls.

---

## 5. Verification Method

### 1. Build Verification
```bash
cargo check
cargo test --no-run
```

### 2. Unit & Integration Test Suite Verification
```bash
cargo test
cargo test --test e2e_tests
```
Verify that:
- `test_f8_ascii_stl_generation_happy` passes.
- `test_f8_binary_stl_generation_happy` passes.
- `test_f8_3mf_zip_structure_happy` passes.
- `test_f9_epub_generation_happy` passes.
- `test_f9_cbz_generation_happy` passes.
- `test_f9_cbr_rar_signature_happy` passes.
- All boundary tests in `tests/e2e/tier2_boundaries.rs` (`test_f8_*`, `test_f9_*`) pass without panicking.
- Scenario tests in `tests/e2e/tier4_scenarios.rs` (`test_scenario_2_cad_3d_asset_management`, `test_scenario_3_media_archive_and_comic_curation`) pass.

### 3. Concrete CLI Sandbox Verification
Run the compiled `strata` binary directly with the sandbox helper flags:
```bash
./target/debug/strata --preview-helper preview-model <path_to_stl_or_3mf> /tmp/model_out.png 512
file /tmp/model_out.png

./target/debug/strata --preview-helper preview-archive-cover <path_to_epub_or_cbz> /tmp/cover_out.png 512
file /tmp/cover_out.png
```
Confirm `/tmp/model_out.png` and `/tmp/cover_out.png` are valid PNG image files with dimensions $\le 512 \times 512$.

### Invalidation Conditions
- Any binary STL starting with `solid` is rejected or misread as ASCII.
- Any model exceeding 2 million triangles allocates excessive memory without failing fast.
- Any corrupted archive causes the persistent worker pool to hang or crash without returning an error frame.
- Any non-zero exit in the sandbox crashes the main GTK UI thread.
