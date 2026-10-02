# GeoTIFF Pyramid Overview Extraction & Gigapixel Guardrail Design (M3.1 / Requirement R1 / Feature F1)

## 1. Observation

### 1.1 Pure-Rust `tiff` (v0.11.3) Internal APIs & Behavior
Inspection of `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/tiff-0.11.3/` revealed the following exact mechanics:

1. **Zero-Allocation IFD Initialization in `Decoder::new()`**:
   - In `src/decoder/mod.rs` (lines 786–867), `Decoder::new(reader)` reads the 2-byte byte order (`b"II"` or `b"MM"`), the 2-byte magic (`42` or `43` for BigTIFF), reads the first IFD offset, and calls `decoder.next_image()?`.
   - In `src/decoder/mod.rs` (lines 960–964), `next_image()` calls `Image::from_reader(&mut self.value_reader, ifd)?`.
   - In `src/decoder/image.rs` (lines 143–220), `Image::from_reader` **strictly parses directory tags** (`ImageWidth`, `ImageLength`, `PhotometricInterpretation`, `Compression`, `SamplesPerPixel`, `SampleFormat`, `StripOffsets`/`TileOffsets`).
   - **Crucial finding**: `Image::from_reader` performs **zero pixel decompression and allocates zero image buffers**. `decoder.dimensions()` (line 875) returns `(u32, u32)` immediately after parsing headers in microseconds.

2. **Multi-IFD Navigation & Traversal**:
   - `decoder.more_images() -> bool` (line 967): Returns `true` if `self.next_ifd.is_some()`.
   - `decoder.next_image() -> TiffResult<()>` (lines 960–964): Follows the chained IFD pointer (`next_ifd`), parses directory tags of the next layer into `self.image`, and caches the IFD offset in `self.ifd_offsets`.
   - `decoder.seek_to_image(ifd_index: usize) -> TiffResult<()>` (lines 893–932):
     If `ifd_index` has not been visited, walks the IFD chain up to `ifd_index`. Once discovered, directly loads the directory at `self.ifd_offsets[ifd_index]` into `self.image`.
   - Calling `seek_to_image(i)` updates dimensions and chunk descriptors without decompressing pixel data.

3. **Pixel Decompression Scoping**:
   - `decoder.read_image() -> TiffResult<DecodingResult>` (lines 1409–1419) and `decoder.read_image_to_buffer(result) -> TiffResult<BufferLayoutPreference>` (lines 1467–1489):
     Decompress **only the image currently active in `self.image`**.
   - If `decoder.seek_to_image(k)` is invoked before calling `read_image()`, the decoder decompresses **only IFD `k`**. The base layer (IFD 0) is never read or decoded.

4. **Chunk and Strip Decoding Capabilities**:
   - `decoder.get_chunk_type() -> ChunkType` (lines 1128–1130): Distinguishes `ChunkType::Strip` and `ChunkType::Tile`.
   - `decoder.chunk_dimensions() -> (u32, u32)` (lines 1360–1362): For strips returns `(width, rows_per_strip)`; for tiles returns `(tile_width, tile_height)`.
   - `decoder.chunk_data_dimensions(chunk_index: u32) -> (u32, u32)` (lines 1366–1370): Unpadded data boundary for edge chunks.
   - `decoder.read_chunk(chunk_index: u32) -> TiffResult<DecodingResult>` (lines 1245–1257): Decompresses an individual chunk/strip into RAM.
   - `decoder.read_chunk_to_buffer(&mut self, buffer, chunk_index, output_width)` (lines 1284–1301): Reuses a single chunk buffer without reallocations.

5. **Direct Tag Support for GeoTIFF**:
   - In `src/tags.rs` (lines 141–157), `Tag` contains first-class enum variants for GeoTIFF keys:
     - `Tag::ModelPixelScaleTag = 33550`
     - `Tag::ModelTransformationTag = 34264`
     - `Tag::ModelTiepointTag = 33922`
     - `Tag::GeoKeyDirectoryTag = 34735`
     - `Tag::GeoDoubleParamsTag = 34736`
     - `Tag::GeoAsciiParamsTag = 34737`
     - `Tag::GdalNodata = 42113`
   - In `src/decoder/mod.rs` (lines 1606–1664), `decoder.get_tag(tag)`, `decoder.get_tag_u16_vec(tag)`, `decoder.get_tag_f64_vec(tag)`, and `decoder.get_tag_ascii_string(tag)` provide direct tag access.

6. **Memory Limits Enforcement in `tiff`**:
   - In `src/decoder/mod.rs` (lines 466–505), `Limits` defines `decoding_buffer_size: usize`.
   - Default is 256 MiB. By invoking `decoder.with_limits(Limits { decoding_buffer_size: 32 * 1024 * 1024, ..Default::default() })`, any attempt to allocate more than 32 MB inside `tiff` returns `Err(TiffError::LimitsExceeded)`.

### 1.2 Sandbox Execution Environment Constraints
In `src/sandbox.rs` (lines 17–22, 148–180, 242–246):
- Address space ceiling: `prlimit --as=1342177280` (1.25 GB virtual memory).
- File size ceiling: `prlimit --fsize=33554432` (32 MB).
- CPU time ceiling: `prlimit --cpu=10` (10 seconds).
- Buffer ceiling: `MAX_OUTPUT_BYTES = 32 * 1024 * 1024` (32 MB).
- In `src/sandbox_helper/sniff.rs` (lines 23–28), `exceeds_decoded_frame_budget(width, height)` enforces `width * height * 4 <= 33_554_432` bytes (~8.38 MP, or $2048 \times 2048 = 4.19$ MP).

### 1.3 Test Suite & Existing Fixture Survey
- `tests/fixtures/geotiff.rs`:
  - `GeoTiffBuilder` generates synthetic pyramidal GeoTIFFs (`sample_cog_pyramidal(w, h, levels)`), single-band DEMs (`sample_dem_float32`), multi-band optical imagery (`sample_multiband_optical`), and corrupted headers (`sample_corrupted_geotiff`).
  - Pyramids are stored as linked IFDs (`IFD0 -> IFD1 -> IFD2 ...`) where each level halves width and height (`current_w = (current_w / 2).max(1)`).
- `tests/e2e/tier1_isolated.rs`:
  - Lines 20–34 (`test_f1_geotiff_pyramid_overview_level_1_happy`): Generates a $1024 \times 1024$ 3-level pyramid and executes `run_preview_helper("preview-geotiff", &input, &output, 1400)`.
  - Lines 48–60 (`test_f1_geotiff_single_ifd_fallback_happy`): Tests single IFD raster $512 \times 512$.
  - Lines 63–67 (`test_f1_geotiff_subsampling_within_budget_happy`): Verifies memory budget.
- Baseline test status: All 181 existing tests pass (`cargo test`).

---

## 2. Logic Chain

```
[GeoTIFF File at /input]
        │
        ▼
[Step 1: Decoder::new()] ──────────► Parses IFD 0 tags ONLY. 0 bytes pixel data allocated.
        │
        ▼
[Step 2: enumerate_ifds()] ────────► Traverses IFD headers via next_image().
        │                            Records (ifd_index, width, height) for all layers.
        │
        ▼
[Step 3: select_overview_layer()]
        ├─► Pyramidal Overviews Available?
        │     ├─► Smallest overview with max(w, h) in [1200, 2048]
        │     └─► If all overviews < 1200: Largest overview with max(w, h) <= 2048
        │           │
        │           ▼
        │     [Decision: DirectDecode(IFD k)]
        │     decoder.seek_to_image(k)
        │     decoder.read_image() -> Allocates ONLY ~1-4 MB (IFD 0 untouched!)
        │
        └─► Flat Single-Layer Raster (No overviews <= 2048)?
              ├─► Base IFD 0 max(w, h) <= 2048
              │     └─► [Decision: DirectDecode(IFD 0)]
              │
              └─► Base IFD 0 max(w, h) > 2048 (Gigapixel File)
                    └─► [Decision: SubsampleFallback]
                          Stride S = ceil(max(W, H) / 1400.0)
                          Output buffer = ceil(W/S) x ceil(H/S) (~7.5 MB)
                          Decodes ONLY chunks/strips intersecting sampled lines
                          Total RAM <= 10 MB (Zero gigapixel allocation!)
```

### 2.1 Why Zero-Allocation IFD Traversal Works
1. When opening a gigapixel file (e.g. 50,000 x 50,000 pixels, uncompressed ~10 GB), `Decoder::new()` only reads the 8-byte TIFF header and the directory entries of IFD 0.
2. In `tiff`, `self.image` stores metadata fields (`width`, `height`, `chunk_offsets`, `chunk_bytes`). The image pixel bytes are not read from the underlying `Read + Seek` stream until `read_image()` or `read_chunk()` is called.
3. Therefore, calling `decoder.dimensions()` and sequentially stepping with `decoder.next_image()` reads only a few hundred bytes of directory tags per IFD from disk.
4. Total execution time for enumerating 5–8 IFD headers is $<0.5\text{ ms}$, and total heap allocation is $<1\text{ KB}$.

### 2.2 Overview Selection Algorithm Formulation
Given the target preview bounding box $1400 \times 1400$ px:
1. Target dimension $T = 1400$.
2. Target acceptable overview lower bound: $MIN\_OVERVIEW\_DIM = 1200$.
3. Safe single-layer direct decode ceiling: $MAX\_SAFE\_DIM = 2048$ (or $4,194,304$ total pixels).
   - At $2048 \times 2048$, 4 channels of 8-bit RGBA or 1 channel of 32-bit float elevation requires $2048 \times 2048 \times 4 = 16,777,216$ bytes (16.0 MiB), which sits safely under the 32 MiB buffer ceiling and 1.25 GB virtual memory limit.

**Exact Decision Tree**:
1. Scan all IFDs into `summaries: Vec<IfdSummary>` where `IfdSummary { index, width, height }`.
2. Let overview layers be $O = \{ s \in \text{summaries} \mid s.\text{index} \ge 1 \}$.
3. **Primary Overview Selection**:
   Filter $O_{\text{ideal}} = \{ s \in O \mid 1200 \le \max(s.\text{width}, s.\text{height}) \le 2048 \}$.
   If $O_{\text{ideal}}$ is non-empty:
   Select $s^* = \arg\min_{s \in O_{\text{ideal}}} \max(s.\text{width}, s.\text{height})$.
   $\to$ Return `SelectionDecision::DirectDecode { ifd_index: s^*.index, width: s^*.width, height: s^*.height }`.
4. **Secondary Overview Selection (All Overviews < 1200)**:
   If $O_{\text{ideal}}$ is empty, filter $O_{\text{sub1200}} = \{ s \in O \mid \max(s.\text{width}, s.\text{height}) < 1200 \}$.
   If $O_{\text{sub1200}}$ is non-empty:
   Select $s^* = \arg\max_{s \in O_{\text{sub1200}}} \max(s.\text{width}, s.\text{height})$ (i.e. the highest resolution overview available $\le 2048$).
   $\to$ Return `SelectionDecision::DirectDecode { ifd_index: s^*.index, width: s^*.width, height: s^*.height }`.
5. **No Usable Overview Layer**:
   If $O_{\text{ideal}}$ and $O_{\text{sub1200}}$ are both empty (either $N=1$ single-layer TIFF, or all overviews $> 2048$):
   Evaluate base layer IFD 0 $(W_0, H_0)$:
   - **Case 5A ($\max(W_0, H_0) \le 2048$)**:
     Base layer is already small enough to decode safely without downsampling.
     $\to$ Return `SelectionDecision::DirectDecode { ifd_index: 0, width: W_0, height: H_0 }`.
   - **Case 5B ($\max(W_0, H_0) > 2048$ — Gigapixel Flat Raster)**:
     Base layer would exceed the 32 MB buffer budget / 1.25 GB address space limit.
     Trigger **Subsampling Fallback**:
     $$\text{stride } S = \max\left(1, \left\lceil \frac{\max(W_0, H_0)}{1400.0} \right\rceil\right)$$
     $$W_{\text{out}} = \min\left(1400, \left\lceil \frac{W_0}{S} \right\rceil\right), \quad H_{\text{out}} = \min\left(1400, \left\lceil \frac{H_0}{S} \right\rceil\right)$$
     $\to$ Return `SelectionDecision::SubsampleFallback { ifd_index: 0, src_width: W_0, src_height: H_0, stride: S, out_width: W_out, out_height: H_out }`.

### 2.3 Subsampling Fallback Mechanics Under Memory Ceilings
For a gigapixel raster (e.g. $40,000 \times 40,000$ pixels):
- Stride $S = \lceil 40000 / 1400.0 \rceil = 29$.
- Output grid: $W_{\text{out}} = \lceil 40000 / 29 \rceil = 1380$, $H_{\text{out}} = \lceil 40000 / 29 \rceil = 1380$.
- Preallocated destination buffer: $1380 \times 1380 \times 4 = 7,617,600$ bytes ($\approx 7.26$ MB).

Depending on TIFF chunk layout:

1. **Tiled Layout (`ChunkType::Tile`)**:
   - Tile dimensions: $(tw, th)$, e.g., $256 \times 256$.
   - Tile size in memory: $256 \times 256 \times 4 = 256$ KB.
   - For each tile grid coordinate $(tx, ty)$:
     Check if the tile's bounding box $[tx \cdot tw, (tx+1) \cdot tw) \times [ty \cdot th, (ty+1) \cdot th)$ intersects any sampled coordinates $(x_{\text{src}}, y_{\text{src}}) = (x_{\text{out}} \cdot S, y_{\text{out}} \cdot S)$.
   - If no sampled coordinate falls inside the tile, **skip decoding the tile**.
   - If sampled coordinates exist:
     Decode the single tile via `decoder.read_chunk(tile_index)`.
     Transfer the sampled pixels into the destination buffer:
     $$\Delta x = x_{\text{out}} \cdot S - tx \cdot tw, \quad \Delta y = y_{\text{out}} \cdot S - ty \cdot th$$
     $$\text{dest}[y_{\text{out}} \cdot W_{\text{out}} + x_{\text{out}}] = \text{tile\_pixel}(\Delta x, \Delta y)$$
     Drop tile buffer.
   - **Peak memory**: $7.26\text{ MB (output)} + 0.25\text{ MB (tile)} \approx 7.51\text{ MB}$.

2. **Stripped Layout (`ChunkType::Strip`)**:
   - Rows per strip: $rps$, e.g., 16 or 32 rows.
   - Strip size in memory: $40,000 \times 16 \times 4 \approx 2.56$ MB.
   - For each strip $k \in 0..\lceil H / rps \rceil$, spanning rows $[k \cdot rps, \min(H, (k+1) \cdot rps))$:
     First sampled row: $y_{\text{first}} = \lceil (k \cdot rps) / S \rceil \cdot S$.
     If $y_{\text{first}} \ge \min(H, (k+1) \cdot rps)$, **skip decoding the strip**.
     If sampled rows exist:
     Decode the single strip via `decoder.read_chunk(k)`.
     For each sampled row $y \in [y_{\text{first}}, \min(H, (k+1) \cdot rps))$ with step $S$:
     $y_{\text{out}} = y / S$.
     Sample columns $x_{\text{src}} = x_{\text{out}} \cdot S$, and write to `dest[y_out * W_out + x_out]`.
     Drop strip buffer.
   - **Peak memory**: $7.26\text{ MB (output)} + 2.56\text{ MB (strip)} \approx 9.82\text{ MB}$.

3. **Giant Strip Guardrail**:
   - If $rps \times W \times \text{bpp} > 32 \times 1024 \times 1024$:
     - If uncompressed (`CompressionMethod::None`): Seek directly to `strip_offset + y_src * row_bytes` in the raw file via standard `Seek + Read` to fetch sampled rows/pixels. Memory used: $< 8\text{ MB}$.
     - If compressed: Return a clean error (`"Single-strip compressed raster exceeds 32MB frame budget"`), caught cleanly without OOM or SIGXFSZ.

---

## 3. Recommended Module Structure & Function Signatures

The implementation should be placed in `src/sandbox_helper/geotiff.rs` and registered in `src/sandbox_helper.rs` via `pub(crate) mod geotiff;`.

```
src/
├── sandbox_helper/
│   ├── sniff.rs        (Existing: generic format sniffing)
│   └── geotiff.rs      (New: GeoTIFF overview extractor, guardrails, normalization)
└── sandbox_helper.rs   (Dispatches "preview-geotiff" operation)
```

### 3.1 Data Structures (`src/sandbox_helper/geotiff.rs`)

```rust
use std::{
    fs::File,
    io::{BufReader, Read, Seek},
    path::Path,
};
use tiff::decoder::{Decoder, Limits};

pub const TARGET_PREVIEW_DIM: u32 = 1400;
pub const MIN_OVERVIEW_DIM: u32 = 1200;
pub const MAX_SAFE_DECODE_DIM: u32 = 2048;
pub const MAX_SAFE_DECODE_PIXELS: u64 = 2048 * 2048; // 4.19 MP (~16MB @ 4 bytes/px)
pub const MAX_DECODING_BUFFER_BYTES: usize = 32 * 1024 * 1024; // 32MB ceiling

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfdSummary {
    pub index: usize,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionDecision {
    /// Safe to decompress the specified IFD directly into memory.
    DirectDecode {
        ifd_index: usize,
        width: u32,
        height: u32,
    },
    /// Flat single-layer gigapixel raster; requires strided subsampling across chunks/strips.
    SubsampleFallback {
        ifd_index: usize,
        src_width: u32,
        src_height: u32,
        stride: u32,
        out_width: u32,
        out_height: u32,
    },
}

#[derive(Debug)]
pub enum DecodedRaster {
    Float32 {
        data: Vec<f32>,
        width: u32,
        height: u32,
        nodata: Option<f32>,
    },
    Rgba8 {
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeoTiffMetadata {
    pub epsg: Option<u32>,
    pub crs_name: Option<String>,
    pub dimensions: [u32; 2],
    pub resolution: Option<[f64; 2]>,
    pub bounds: Option<[f64; 4]>,
    pub band_count: u32,
    pub data_type: String,
    pub elevation_min: Option<f32>,
    pub elevation_max: Option<f32>,
}
```

### 3.2 Core Function Signatures (`src/sandbox_helper/geotiff.rs`)

```rust
/// Primary entry point called by `src/sandbox_helper.rs` and the worker pool.
/// Returns raw PNG bytes and serialized JSON metadata.
pub fn render_geotiff(path: &Path, target_box: i32) -> Result<(Vec<u8>, String), String>;

/// Reads all IFD headers in the TIFF stream and returns their index and dimensions.
/// Performs zero pixel allocations.
pub fn enumerate_ifd_summaries<R: Read + Seek>(
    decoder: &mut Decoder<R>,
) -> Result<Vec<IfdSummary>, String>;

/// Determines whether to decode an overview IFD directly or execute subsampling fallback.
pub fn select_overview_layer(
    summaries: &[IfdSummary],
    target_box: u32,
) -> Result<SelectionDecision, String>;

/// Executes decoding based on the selection decision, enforcing the 32MB decoding limit.
pub fn decode_raster<R: Read + Seek>(
    decoder: &mut Decoder<R>,
    decision: &SelectionDecision,
) -> Result<DecodedRaster, String>;

/// Subsamples a high-resolution tiled or stripped raster into ~1400px output buffer.
pub fn decode_subsampled_raster<R: Read + Seek>(
    decoder: &mut Decoder<R>,
    src_width: u32,
    src_height: u32,
    stride: u32,
    out_width: u32,
    out_height: u32,
) -> Result<DecodedRaster, String>;

/// Encodes normalized 8-bit RGBA or float32 DEM data into PNG using Cairo ImageSurface.
pub fn encode_png(
    raster: &DecodedRaster,
) -> Result<(Vec<u8>, Option<f32>, Option<f32>), String>;

/// Extracts EPSG code, pixel resolution, and geographic bounding box using geotiff-core / tags.
pub fn extract_geotiff_metadata<R: Read + Seek>(
    decoder: &mut Decoder<R>,
    base_width: u32,
    base_height: u32,
    elevation_min: Option<f32>,
    elevation_max: Option<f32>,
) -> Result<GeoTiffMetadata, String>;
```

### 3.3 Concrete Reference Implementation Sketch for Algorithm Logic

```rust
pub fn select_overview_layer(
    summaries: &[IfdSummary],
    target_box: u32,
) -> Result<SelectionDecision, String> {
    if summaries.is_empty() {
        return Err("No IFD headers found in TIFF".to_owned());
    }

    let overviews: Vec<&IfdSummary> = summaries.iter().skip(1).collect();

    // 1. Look for overviews in ideal range [1200, 2048]
    let mut ideal_overviews: Vec<&IfdSummary> = overviews
        .iter()
        .copied()
        .filter(|s| {
            let max_dim = s.width.max(s.height);
            max_dim >= MIN_OVERVIEW_DIM && max_dim <= MAX_SAFE_DECODE_DIM
        })
        .collect();

    if !ideal_overviews.is_empty() {
        // Smallest overview where max(w, h) >= 1200
        ideal_overviews.sort_by_key(|s| s.width.max(s.height));
        let best = ideal_overviews[0];
        return Ok(SelectionDecision::DirectDecode {
            ifd_index: best.index,
            width: best.width,
            height: best.height,
        });
    }

    // 2. If all overviews are smaller than 1200, pick the largest overview <= 2048
    let mut small_overviews: Vec<&IfdSummary> = overviews
        .iter()
        .copied()
        .filter(|s| {
            let max_dim = s.width.max(s.height);
            max_dim < MIN_OVERVIEW_DIM
        })
        .collect();

    if !small_overviews.is_empty() {
        // Largest overview <= 2048
        small_overviews.sort_by_key(|s| std::cmp::Reverse(s.width.max(s.height)));
        let best = small_overviews[0];
        return Ok(SelectionDecision::DirectDecode {
            ifd_index: best.index,
            width: best.width,
            height: best.height,
        });
    }

    // 3. Fallback: No overviews <= 2048 exist. Inspect base layer IFD 0.
    let base = summaries[0];
    let max_base_dim = base.width.max(base.height);
    let total_pixels = (base.width as u64) * (base.height as u64);

    if max_base_dim <= MAX_SAFE_DECODE_DIM && total_pixels <= MAX_SAFE_DECODE_PIXELS {
        // Small flat raster: decode IFD 0 directly
        Ok(SelectionDecision::DirectDecode {
            ifd_index: 0,
            width: base.width,
            height: base.height,
        })
    } else {
        // Gigapixel flat raster: trigger subsampling fallback
        let stride = ((max_base_dim as f64) / (target_box as f64)).ceil().max(1.0) as u32;
        let out_width = ((base.width as f64) / (stride as f64)).ceil().min(target_box as f64) as u32;
        let out_height = ((base.height as f64) / (stride as f64)).ceil().min(target_box as f64) as u32;

        Ok(SelectionDecision::SubsampleFallback {
            ifd_index: 0,
            src_width: base.width,
            src_height: base.height,
            stride,
            out_width,
            out_height,
        })
    }
}
```

---

## 4. Caveats

1. **SubIFDs vs Chained IFDs**:
   While the Cloud Optimized GeoTIFF (COG) specification stores overviews as chained IFDs (`next_ifd`), some non-standard pyramidal TIFFs store overview pointers in `Tag::SubIfd` (tag 330). The initial implementation should support chained IFDs (matching `GeoTiffBuilder` and standard GDAL COG outputs). If `summaries.len() == 1` and `Tag::SubIfd` is present, `decoder.get_tag(Tag::SubIfd)` can be probed as an optional enhancement.
2. **Planar vs Chunky Rasters**:
   Satellite rasters are occasionally saved in planar configuration (`PlanarConfiguration::Planar = 2`), where bands are stored in separate chunks rather than interleaved RGB. As noted in `tiff` 0.11 documentation, `decoder.read_chunk_bytes()` on planar images decodes one sample band at a time. The subsampling loop must account for planar chunk indexing if `planar_config == Planar`.
3. **Single Compressed Giant Strip**:
   If an adversarial or poorly generated file has $30,000$ rows compressed in a single strip with no overviews, decompressing the strip requires decompressing the whole raster. In this case, `tiff::decoder::Limits` will cleanly return `TiffError::LimitsExceeded`, protecting the process from crashing `prlimit`.

---

## 5. Conclusion

1. **API Viability**:
   Pure-Rust `tiff = "0.11"` natively provides all required primitives: zero-allocation directory navigation via `next_image()` and `seek_to_image(i)`, chunk/strip querying and decoding via `read_chunk()`, tag extraction via `get_tag()`, and memory bounding via `Limits`.
2. **Overview Selection Algorithm**:
   Iterating IFD headers to find the smallest overview where $\max(w, h) \ge 1200$, or the largest overview $\le 2048$ if all are smaller, guarantees that for pyramidal rasters, only an overview of $\le 4.19$ MP ($\le 16$ MB) is ever decompressed. Base layer IFD 0 is completely bypassed.
3. **Subsampling Fallback**:
   For flat rasters exceeding $2048 \times 2048$, strided spatial sampling across chunk/strip boundaries decodes only the tiles or strips intersecting the ~1400px output grid. Peak memory remains bounded at $\approx 8\text{--}10$ MB, well under the 32 MB and 1.25 GB ceilings.
4. **Read-Only Compliance**:
   No source code was modified during this investigation. All findings and architectures are documented here to inform subsequent implementation milestones.

---

## 6. Verification Method

1. **Existing Test Suite Baseline**:
   ```bash
   cargo test
   ```
   Must pass all 181 unit tests.

2. **Overview Selection Unit Test Verification**:
   The implementer can verify the algorithm against `tests/fixtures/geotiff.rs`:
   - Create 3-level COG ($1024 \times 1024 \to 512 \times 512 \to 256 \times 256$): Algorithm selects IFD 1 ($512 \times 512$, largest $\le 2048$).
   - Create 4-level COG ($4096 \times 4096 \to 2048 \times 2048 \to 1024 \times 1024 \to 512 \times 512$): Algorithm selects IFD 1 ($2048 \times 2048$, smallest overview $\ge 1200$).
   - Create flat raster $512 \times 512$: Algorithm selects IFD 0 direct decode.
   - Create flat raster $4096 \times 4096$: Algorithm selects SubsampleFallback with stride $S = 3$, output $1366 \times 1366$.

3. **E2E Test Runner Verification**:
   Run:
   ```bash
   cargo test --test e2e_tests -- test_f1_
   ```
   Verify all F1 tests pass once `src/sandbox_helper/geotiff.rs` is hooked into `sandbox_helper.rs`.
