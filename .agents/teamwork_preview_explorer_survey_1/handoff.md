# Architectural Survey & Handoff Report: Requirement R1 (Full GIS GeoTIFF Inspector & Overview Pipeline)

## 1. Observation

### 1.1 Existing Preview Pipeline Files & Structure
The preview architecture in Hermes consists of seven key components across `src/`:

1. **Format Classification Registry (`src/services/formats.rs`)**:
   - Lines 57-58: `FormatFamily::Image` documentation notes `"Standard raster images (PNG, JPEG, WebP, GIF, BMP, TIFF, AVIF, …)"`.
   - Lines 88-89: `FormatFamily::Image` maps to `Some(PreviewHandler::Image)`.
   - Lines 247-249: `classify_by_name()` maps extensions `"tif" | "tiff"` to `FormatFamily::Image`.
   - Lines 299-301: `thumbnail_handler_for_name()` maps `"tif" | "tiff"` to `ThumbnailHandler::Image`.
   - There is no distinct `FormatFamily::GeoTiff`, `PreviewHandler::GeoTiff`, or geospatial metadata awareness in the format classifier.

2. **Preview Request & Content Abstraction (`src/services/preview.rs`)**:
   - Lines 20-32:
     ```rust
     pub enum PreviewContent {
         Text { content: String, truncated: bool },
         Image,
         Media,
         Rasterized { png: Vec<u8> },
         SandboxedMedia { data: Vec<u8> },
         Pdf { png: Vec<u8>, page: i32, pages: i32 },
         Code { language: String, content: String },
         Markdown { content: String },
         Model3D { format: String, data: Vec<u8> },
         Unsupported,
     }
     ```
   - Only `Rasterized { png }` exists for images; there is no container for geospatial metadata (CRS, EPSG, resolution, bounding box, band statistics).

3. **Preview Adapter (`src/adapters/local_preview.rs`)**:
   - Lines 49-56: Dispatches format classification via `classify_by_mime()` and `classify_by_name()`.
   - Lines 70-71: Maps `PreviewHandler::Image` to `PreviewContent::Image`.
   - Line 78: Maps handler to `ParseOperation` via `preview_operation(PreviewHandler::Image) -> Some(ParseOperation::PreviewImage)`.
   - Lines 90-131: Spawns sandboxed parsing task via `gio::spawn_blocking(move || crate::sandbox::parse(&path, operation, value, &cancellation))`.
   - Line 121: Packages returned PNG data as `PreviewContent::Rasterized { png: output.data }`.

4. **Sandbox Host Execution (`src/sandbox.rs`)**:
   - Lines 20-35: Defines `ParseOperation` enum (`PreviewImage`, `PreviewPdf`, etc.).
   - Lines 57-74: Maps operation to output filename (defaults to `result.png`).
   - Lines 148-180: Checks result file size (`MAX_OUTPUT_BYTES = 32 * 1024 * 1024`), verifies PNG magic bytes `b"\x89PNG\r\n\x1a\n"`, and reads `result.meta` via `read_metadata()`.
   - Lines 242-246: Enforces strict sandboxed process limits via `prlimit`:
     ```bash
     /usr/bin/prlimit --as=1342177280 --cpu=10 --fsize=33554432 -- /app/strata --preview-helper <operation> /input /output/<output_name> <value>
     ```
     Limits: Address Space ceiling `--as=1342177280` (1.25 GB), CPU time `--cpu=10` (10s), Max file size `--fsize=33554432` (32 MB).

5. **Sandbox Helper Implementation (`src/sandbox_helper.rs`)**:
   - Lines 45: `"preview-image" => (render_image(input, 1400)?, None)`.
   - Lines 76-80:
     ```rust
     fn render_image(path: &Path, size: i32) -> Result<Vec<u8>, String> {
         render_imagemagick(path, size).or_else(|_| render_pixbuf(path, size))
     }
     ```
   - Lines 86-101: `render_imagemagick()` executes `magick` / `convert <input> -auto-orient -thumbnail <size>x<size> png:-`.
   - Lines 69-74: `render_pixbuf()` executes `gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true)` and saves to PNG buffer.
   - Lines 306-319: `render_pdf_surface()` creates a `cairo::ImageSurface` and outputs PNG via `surface.write_to_png(&mut png)`.

6. **Preview UI Drawer (`src/ui/preview.rs`)**:
   - Lines 99-108: Renders metadata header box containing three labels: `SIZE`, `MODIFIED`, and `TYPE`.
   - Lines 425-539: Handles `PreviewContent::Rasterized { png }`: creates `gtk::gdk::Texture` from bytes, renders inside `gtk::Picture`, wraps in `gtk::ScrolledWindow` with custom zoom and drag-pan controllers.
   - No UI widgets exist for displaying geospatial projection, bounding coordinates, resolution, band counts, or placement map indicators.

7. **DrawingArea Pattern (`src/ui/settings.rs`)**:
   - Lines 661-685: Demonstrates GTK4 `gtk::DrawingArea` with `set_draw_func` using Cairo (`context.rectangle()`, `context.fill()`, `context.clip()`), providing the exact rendering mechanism for custom placement mini-maps.

### 1.2 Dependencies in Cargo.toml
`Cargo.toml` lines 12-25:
- `cairo-rs = { version = "0.21.5", features = ["pdf", "png"] }`
- `fontconfig-sys = { package = "yeslogic-fontconfig-sys", version = "6.0.1" }`
- `gdk-pixbuf = "0.21.5"`
- `gio = { version = "0.21.5", features = ["v2_72"] }`
- `gtk = { package = "gtk4", version = "0.10.3", features = ["v4_12"] }`
- `ignore = "0.4.23"`
- `poppler = { package = "poppler-rs", version = "0.25.0" }`
- `serde = { version = "1.0", features = ["derive"] }`
- `sourceview5 = { version = "0.10.0", features = ["gtk_v4_12"] }`
- `toml = "0.9"`
- `tracing = "0.1.41"`
- `tracing-subscriber = "0.3.20"`

Strict compile lints in lines 29-45:
- `unsafe_code = "deny"`
- `unused_must_use = "deny"`
- Clippy denies undocumented unsafe, dbg macro, allow attributes without reason.

### 1.3 Available Pure-Rust Crates Verified in Cargo Registry
- **`tiff = "0.11.3"`** (Pure Rust, MIT, maintained by `image-rs`):
  - In-tree at `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tiff-0.11.3/`.
  - Supports multi-IFD navigation: `decoder.dimensions()`, `decoder.seek_to_image(ifd_index)`, `decoder.next_image()`.
  - Supports decoding all sample types: `DecodingResult::U8`, `U16`, `F32`, `F64`, `I16`, `I32`.
  - Supports tag extraction: `get_tag_u16_vec()`, `get_tag_f64_vec()`, `get_tag_ascii_string()`.
  - Supports chunk/strip decoding: `read_chunk()`, `read_chunk_to_buffer()`.
- **`geotiff-core = "0.8.1"`** (Pure Rust, MIT/Apache-2.0):
  - In-tree at `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/geotiff-core-0.8.1/`.
  - **Zero third-party dependencies** (`dependencies` table is empty).
  - Implements `GeoMetadata`, `GeoTransform`, `GeoKeyDirectory`, `CrsInfo`.
  - Extracts EPSG codes (projected and geographic), model tiepoints, pixel scale, 4x4 transformation matrices, and derives bounding boxes.

### 1.4 Test Baseline
- Command: `cargo test`
- Result: 181 passed; 0 failed; 0 ignored; finished in 0.06s.

---

## 2. Logic Chain

### 2.1 Why Current GeoTIFF / COG Rendering Fails
1. **Gigapixel OOM Crash**:
   - Both `gdk_pixbuf::Pixbuf::from_file_at_scale` and ImageMagick `convert <path> -thumbnail` read Directory 0 (the full-resolution layer) by default.
   - For a typical aerial or satellite GeoTIFF (e.g. 40,000 x 40,000 pixels = 1.6 gigapixels), an uncompressed 4-channel pixel buffer requires `40000 * 40000 * 4 = 6.4 GB`.
   - The sandbox enforces `prlimit --as=1342177280` (1.25 GB virtual memory). Any allocation request exceeding 1.25 GB immediately aborts or triggers `ENOMEM`, crashing the helper process and surfacing `"Preview unavailable"`.
2. **SIGXFSZ (File Size Exceeded) Crash**:
   - ImageMagick spills intermediate pixel caches to `/tmp/magick-XXXXXX` when raster size exceeds physical memory thresholds.
   - When the scratch cache exceeds 32 MB (`--fsize=33554432`), the Linux kernel sends signal 25 (`SIGXFSZ`), immediately killing the process.
3. **Terrain Elevation (DEM Float32 / Int16) Distortion & Clipping**:
   - GdkPixbuf does not support floating-point pixels (supports only 8-bit/channel).
   - ImageMagick clips raw float elevations to `0.0..1.0` or truncates without contrast stretching.
   - DEMs contain NoData values (e.g. `-9999.0`, `-32767`, or `-3.4028e+38`). Linear min/max scaling across the full array compresses valid terrain elevations (e.g. 500m to 2000m) into less than 1% of the dynamic range, rendering rasters completely solid black or white with no visible relief.
4. **Multi-Band Optical Distortion**:
   - 4-band imagery (e.g. NAIP, PlanetScope: Red, Green, Blue, Near-Infrared) is misinterpreted by standard 4-channel image loaders as RGBA. The NIR channel is treated as opacity/alpha, causing water bodies, shadows, and low-NIR surfaces to become completely transparent or corrupted.
   - 16-bit satellite reflectance (0..10,000) shifted to 8-bit without percentile stretching renders as near-black.
5. **Missing Geospatial Metadata**:
   - ImageMagick and GdkPixbuf drop TIFF tags 33550 (`ModelPixelScaleTag`), 33922 (`ModelTiepointTag`), 34735 (`GeoKeyDirectoryTag`), and 42113 (`GDAL_NODATA`), leaving the user with zero spatial context.

### 2.2 Why Pure-Rust `tiff` + `geotiff-core` Is the Optimal Architecture
1. **Zero External C Library Dependencies**:
   - System GDAL (`gdalinfo`, `libgdal`) is not installed on the host system.
   - Adding `libgdal` would require massive C++ dependencies, PROJ data tables, and dynamic linking inside the Bubblewrap sandbox mount profile.
   - In contrast, `geotiff-core = "0.8.1"` has zero external dependencies and compiles entirely in pure Rust.
2. **Strict Safety Compliance**:
   - Hermes enforces `#![deny(unsafe_code)]`.
   - `tiff` and `geotiff-core` expose safe Rust APIs for parsing tags, extracting overviews, and computing affine transformations.
3. **Low-Memory Overview Extraction**:
   - COGs and standard pyramidal GeoTIFFs store internal overviews in subsequent IFDs.
   - With `tiff::Decoder`, Hermes can iterate through IFD headers in milliseconds using `decoder.seek_to_image(i)`, inspect `decoder.dimensions()`, and choose an overview level matching the target preview size (e.g., 1000–1600 px).
   - Only the chosen downsampled overview raster (~1–4 MB) is decompressed into RAM, completely bypassing gigapixel full-frame decoding.

---

## 3. Recommended Implementation Architecture

```
                  ┌──────────────────────────────────────────────┐
                  │           Hermes UI (src/ui/preview.rs)      │
                  │  - GeoTIFF Bounding-Box Map Indicator (Cairo)│
                  │  - GIS Metadata Badges (EPSG, Res, Bounds)   │
                  │  - ScrolledWindow Zoom/Pan Texture Display   │
                  └──────────────────────▲───────────────────────┘
                                         │ PreviewEvent::Ready(Preview)
                  ┌──────────────────────┴───────────────────────┐
                  │    LocalPreviewProvider (src/adapters)       │
                  │  - Classify .tif/.tiff -> FormatFamily::GeoTiff│
                  │  - ParseOperation::PreviewGeoTiff            │
                  │  - Parses result.png + result.meta (JSON)    │
                  └──────────────────────▲───────────────────────┘
                                         │ Wire Protocol / bwrap stdout
                  ┌──────────────────────┴───────────────────────┐
                  │    Sandbox Helper (src/sandbox_helper.rs)    │
                  │  1. Header / IFD sniffer (tiff crate)        │
                  │  2. Pyramid Overview Selection (avoid OOM)   │
                  │  3. Multi-Band RGB mapping / DEM stretching  │
                  │  4. GeoTIFF Tag parsing (geotiff-core)       │
                  │  5. Writes result.png + result.meta          │
                  └──────────────────────────────────────────────┘
```

### 3.1 Pipeline Stages & Algorithms

#### Stage 1: Identification & Format Routing
- In `src/services/formats.rs`:
  - Introduce `FormatFamily::GeoTiff` (or specialize `.tif` and `.tiff` handling to probe GeoTIFF tags).
  - Add `PreviewHandler::GeoTiff`.
  - Add `ParseOperation::PreviewGeoTiff` in `src/sandbox.rs`.

#### Stage 2: Pyramid Overview Selection (Memory Protection)
In `src/sandbox_helper.rs`:
1. Open file using `tiff::decoder::Decoder::new(File::open(input)?)`.
2. Inspect IFD 0 dimensions `(w0, h0)`.
3. Loop through available IFDs via `decoder.seek_to_image(index)`:
   - Target preview bounding box: `1400 x 1400`.
   - Find the smallest overview IFD `k` where `max(width, height) >= 1200`, or if all overviews are smaller, the largest available overview.
   - If an overview IFD `k` (where `max(w, h) <= 2048`) exists, select IFD `k`.
   - If no internal overview exists (flat single-layer TIFF):
     - If `w0 * h0 <= 2048 * 2048`: decode IFD 0 directly.
     - If `w0 * h0 > 2048 * 2048`: utilize chunk/strip reading (`decoder.read_chunk()`) with spatial sub-sampling (decoding every $N$-th row/column) to assemble a downsampled 1400px raster without ever allocating $> 16\text{ MB}$.

#### Stage 3: Dynamic Band & Contrast Normalization
1. **Single-Band Float32 / Int16 DEM**:
   - Extract raw elevation buffer via `decoder.read_image()`.
   - Extract `GDAL_NODATA` tag (tag 42113, string) if present; identify NoData values (common: `-9999.0`, `-32767.0`, `f32::NAN`, `f32::INFINITY`, or values $< -9000.0$).
   - Sample valid terrain pixels to calculate the 2nd percentile ($P_2$) and 98th percentile ($P_{98}$).
   - Normalize elevation $z$:
     $$v = \left(\frac{z - P_2}{P_{98} - P_2}\right).clamp(0.0, 1.0)$$
   - Color mapping:
     - Apply terrain color ramp (hypsometric tint: lowlands deep green `#2d6a4f` $\to$ mid elevation khaki/buff `#d4a373` $\to$ mountain brown `#6c584c` $\to$ snow peaks `#f8f9fa`) or high-contrast grayscale.
     - Render NoData pixels as transparent or background tone.
2. **Multi-Band Optical Imagery ($\ge 3$ bands)**:
   - Band mapping: Extract Band 0 as Red, Band 1 as Green, Band 2 as Blue. Discard Band 3+ (NIR / QA bands) for RGB preview.
   - 16-bit to 8-bit dynamic stretching: Compute 2% and 98% percentiles per channel to expand surface reflectance (typically 200–3500) smoothly across 0–255.
3. Write normalized 8-bit RGBA pixel buffer to `result.png` using Cairo (`cairo::ImageSurface::create_for_data` or `cairo::ImageSurface::create` + `surface.write_to_png()`).

#### Stage 4: Geospatial Metadata Extraction
In `src/sandbox_helper.rs`, extract GeoTIFF tags:
1. `ModelPixelScaleTag` (33550): `[scale_x, scale_y, scale_z]`
2. `ModelTiepointTag` (33922): `[i, j, k, x, y, z]`
3. `GeoKeyDirectoryTag` (34735): Parse GeoKeys using `geotiff_core::geokeys::GeoKeyDirectory`:
   - `ProjectedCSTypeGeoKey` (3072): Projected EPSG code (e.g. `32632`, `3857`).
   - `GeographicTypeGeoKey` (2048): Geographic EPSG code (e.g. `4326`, `4269`).
4. Derive geographic bounds using `geotiff_core::GeoTransform::from_tiepoint_and_scale()`:
   - $\text{min\_x} = \text{tiepoint}_X - \text{tiepoint}_I \times \text{scale}_X$
   - $\text{max\_y} = \text{tiepoint}_Y + \text{tiepoint}_J \times \text{scale}_Y$
   - $\text{max\_x} = \text{min\_x} + \text{width} \times \text{scale}_X$
   - $\text{min\_y} = \text{max\_y} - \text{height} \times \text{scale}_Y$
5. Write metadata structure to `result.meta` as JSON/TOML:
   ```json
   {
     "epsg": 32632,
     "crs_name": "WGS 84 / UTM zone 32N",
     "dimensions": [40000, 40000],
     "resolution": [10.0, 10.0],
     "bounds": [500000.0, 5100000.0, 900000.0, 5500000.0],
     "band_count": 4,
     "data_type": "MultiBand",
     "elevation_min": null,
     "elevation_max": null
   }
   ```

#### Stage 5: UI Placement Badge & Map Indicator (`src/ui/preview.rs`)
1. Extend `PreviewContent` in `src/services/preview.rs`:
   ```rust
   pub enum PreviewContent {
       ...
       GeoTiff {
           png: Vec<u8>,
           metadata: GeoTiffMetadata,
       },
   }
   ```
2. In `src/ui/preview.rs`:
   - Add GIS metadata badge row below the existing `SIZE / MODIFIED / TYPE` bar:
     - `PROJ`: e.g. `EPSG:32632` or `EPSG:4326`
     - `RESOLUTION`: e.g. `10.0 m/px`
     - `BOUNDS`: e.g. `[11.23°, 43.51°] - [11.58°, 43.89°]`
     - `BANDS`: e.g. `4 Bands (RGB+NIR)` or `DEM (Float32)`
   - Mini-Map Indicator Widget:
     - Create a `gtk::DrawingArea` (width: 160px, height: 90px).
     - Use `set_draw_func` with Cairo to render a simplified world map outline / coordinate grid box with an accent-colored highlighted rectangle for the bounding box.

---

## 4. Caveats & Edge Cases

1. **Non-Standard & User-Defined Projections**:
   - GeoTIFF files with `ProjectedCSTypeGeoKey == 32767` denote a user-defined projection. In this case, `epsg` will be `None`.
   - *Handling*: Fallback to `GeoAsciiParamsTag` (tag 34737) citation string or display `"Custom / Projected CRS"`.
2. **Projected Coordinates vs Longitude/Latitude**:
   - UTM and Web Mercator bounds are in meters (e.g. `x = 500,000`, `y = 5,200,000`), whereas EPSG:4326 coordinates are in degrees (`-180..180`, `-90..90`).
   - For the placement mini-map, when coordinates are in meters, if standard EPSG is known, convert to approximate lat/lon (or display the bounding box on a localized grid indicator).
3. **Files Without Internal Overviews**:
   - If an un-pyramided GeoTIFF has massive dimensions (e.g. 30,000 x 30,000), decoding full IFD 0 would trigger OOM.
   - The fallback must perform strided strip/tile sampling rather than attempting full raster decompression.
4. **Sandboxed Filesystem Access**:
   - Pure-Rust decoding via `tiff` and `geotiff-core` requires only the single read-only input file already bound at `/input` by `src/sandbox.rs`. It requires zero additional host library bindings.

---

## 5. Conclusion

1. **Root Cause of Existing Deficiencies**:
   The existing pipeline treats all TIFF files identically to standard PNG/JPEG rasters via ImageMagick and GDK Pixbuf, leading to OOM crashes on gigapixel files, clipping on float32 DEMs, NIR channel corruption on multi-band imagery, and complete absence of GIS metadata.
2. **Recommended Dependencies**:
   Add to `Cargo.toml`:
   - `tiff = "0.11"` (pure Rust TIFF decoder with multi-IFD overview seeking and float32 decoding).
   - `geotiff-core = "0.8.1"` (pure Rust zero-dependency GeoTIFF metadata & transform parser).
   - `serde_json = "1.0"` (for serialization of metadata between helper and host via `result.meta`).
3. **Architecture Summary**:
   - Sandboxed helper reads IFD overview levels directly to extract 1400px previews in < 50ms using < 15MB RAM.
   - Dynamic contrast stretching normalizes DEM float32 elevations (with NoData masking) and multi-band optical imagery (mapping optical bands to RGB).
   - `geotiff-core` extracts EPSG, bounding box, resolution, and band details.
   - `src/ui/preview.rs` renders the normalized texture along with GIS metadata badges and a Cairo-rendered geographical placement mini-map indicator.

---

## 6. Verification Method

To independently verify this architecture and subsequent implementations:

1. **Baseline Integrity Check**:
   ```bash
   cargo test
   ```
   Confirm all 181 existing Hermes unit tests continue to pass without error.

2. **Synthetic GeoTIFF Pipeline Test**:
   Create unit tests in `src/adapters/local_preview/tests.rs` (following the existing pattern in lines 24–60):
   - **Test 1 (Pyramid Overview Extraction)**:
     Construct a multi-IFD TIFF file in `temp_dir()` where IFD 0 is 4096x4096 and IFD 1 is 1024x1024. Run `sandbox_helper::run(&["preview-geotiff", path, output, "1400"])`. Verify execution succeeds, memory remains bounded, and output is valid PNG with dimensions $\le 1400$.
   - **Test 2 (Float32 DEM Dynamic Normalization)**:
     Construct a single-band 256x256 Float32 TIFF with elevations between 300.0m and 1800.0m and a `-9999.0` NoData border. Verify generated PNG has non-zero contrast variance (not all black `#000000` or all white `#ffffff`).
   - **Test 3 (Geospatial Metadata Extraction)**:
     Write GeoTIFF tags (`ModelPixelScaleTag`, `ModelTiepointTag`, `GeoKeyDirectoryTag` for EPSG:32632). Verify `result.meta` contains `epsg: 32632`, correct bounding coordinates, and pixel resolution.

3. **Memory Ceiling Verification**:
   Execute the sandboxed helper under `prlimit --as=1342177280 --cpu=10 --fsize=33554432` against a high-resolution GeoTIFF to confirm no OOM or SIGXFSZ occurs.
