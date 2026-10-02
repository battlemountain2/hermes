# Handoff Report: Spreadsheet (ODS, XLS, XLSX) & Audio Waveform (FLAC, MP3, WAV, OGG) Architecture & Implementation Strategy

## 1. Observation

### 1.1 Git Historical Reference Commits
1. **Commit `d39052be`** (`feat(preview): add sortable virtual tables for documents and spreadsheets (#729)`):
   - Modified 51 files, adding 2393 insertions.
   - Added dependencies: `calamine = "0.36.1"` and `csv = "1.4.0"`.
   - Introduced `src/services/table.rs` with:
     - `WORKBOOK_BYTE_LIMIT = 20 * 1024 * 1024` (20 MiB ceiling)
     - `TABLE_TEXT_LIMIT = 4 * 1024 * 1024` (4 MiB text ceiling)
     - `TABLE_VALUE_LIMIT = 100_000` (100,000 cells ceiling)
     - `TABLE_COLUMN_LIMIT = 256` (256 columns ceiling)
     - `read_workbook(path: &Path) -> Result<TableData, String>` calling `calamine::open_workbook_auto(path)`.
   - Introduced `src/ui/table_view.rs` (444 lines) and `src/ui/table_view/selection.rs` (476 lines):
     - Uses GTK4 `gtk::ColumnView` + `gtk::NoSelection` + `gtk::SortListModel` + `gtk::CustomSorter`.
     - Sorting is driven by `SortKey`: numeric `f64` comparison via `total_cmp`, text comparison alphabetically, with `SortKey::Empty` sorting last.
     - Cell layout via `gtk::SignalListItemFactory`, column auto-sizing sampling up to 64 rows, clamped between 72px and 640px.
   - Wire protocol addition: `ParseOperation::PreviewWorkbook`, output name `result.json`, verified with `TableData::from_json(data).is_ok()`.

2. **Commits `40a5a606` and `24857a55`** (`feat(preview): render a live spectrum waveform for audio files` & `fix(preview): source the audio spectrum from decoded media only`):
   - Located on branch `upstream/feat/audio-soundcloud-visualizer`.
   - Added `src/ui/preview/waveform.rs` with `SoundCloudWaveform`:
     - Constant layout parameters: `NUM_BARS: usize = 84`, `NUM_BANDS: usize = 24`, `HORIZON_GAP: f64 = 1.2`, `BAR_GAP: f64 = 1.4`, `BASELINE_RATIO: f64 = 0.66`, `BOT_REFLECTION_RATIO: f64 = 0.28`.
     - Cairo rendering: Top gradient with color stops from `tip` (`tokens.highlight`) to `accent` (`tokens.accent`); bottom reflection gradient with 0.55 to 0.15 alpha attenuation.
     - 1024-sample FFT with Hann window in `src/ui/media.rs` calculating 24 logarithmic frequency bands (20Hz–16000Hz).
     - Beat drop kick detection from sub-bass jump (`sub_bass > 0.40 && delta > 0.15 && elapsed_ms > 260`).
     - Tick callback driving 60 FPS redraws during playback with smooth decay `decay * 0.88 - 0.02`.

### 1.2 Native FFmpeg Waveform Capabilities
- Running `ffmpeg -filters | grep showwaves` on the system confirmed:
  ```
  .. showwaves         A->V  Convert input audio to a video output.
  .. showwavespic      A->V  Convert input audio to a video output single picture.
  ```
- Running `ffmpeg -h filter=showwavespic` verified parameter support:
  - `s` / `size`: Output raster dimensions (e.g. `800x240`).
  - `colors`: Color specifications (e.g. `0x7aa2f7` Tokyonight blue).
  - `filter`: `average` or `peak` (`peak` yields crisp, discrete vertical waveform spikes).
  - `scale`: `lin`, `log`, `sqrt`, `cbrt` (`cbrt` provides balanced dynamic range for quiet and loud passages).
  - Output format: Generates single-frame 8-bit RGBA PNG with alpha transparency (`file /tmp/waveform.png` produced `PNG image data, 600 x 160, 8-bit/color RGBA, non-interlaced`).
- Tested command on generated 16-bit PCM RIFF WAV audio:
  ```bash
  ffmpeg -y -i /tmp/test.wav -filter_complex "showwavespic=s=600x160:colors=0x5c7aff:filter=peak:scale=cbrt" -frames:v 1 /tmp/waveform.png
  ```
  Result: Returned exit code 0 in ~40ms, producing a 2.2 KB transparent RGBA PNG.

### 1.3 Existing Hermes Baseline & E2E Test Suite Status
- Running `cargo test --bin strata`: 216 tests passed in 0.08s.
- Running `cargo test --test e2e_tests`: 165 tests passed in 0.41s.
- `tests/e2e/tier1_isolated.rs`: Contains test fixtures `sample_ods_spreadsheet()`, `sample_xlsx_spreadsheet()`, `sample_xls_spreadsheet()`, `sample_wav()`, `sample_flac()`, `sample_mp3()`, `sample_ogg()`.
- `tests/e2e/tier2_boundaries.rs`: Verifies rejection of missing `content.xml`, missing `workbook.xml`, corrupted OLE magic, truncated RIFF headers, zero sample rates, zero channels, and truncated OggS BOS pages.
- `tests/e2e/tier3_pairwise.rs`:
  - `test_p9_spreadsheet_ods_and_wire_response_framing`: validates 8-byte framing with spreadsheet metadata.
  - `test_p10_audio_waveform_and_frame_budget_sniffing`: validates standard `800x200` waveform output within frame budget.
  - `test_p20_audio_flac_stream_and_wire_framing_limit`: validates FLAC stream wire framing.
- `tests/e2e/tier4_scenarios.rs`: Scenario 4 tests financial spreadsheet ingestion, `extract-office-text`, and `preview-archive`.

### 1.4 Wire Protocol & Sandbox State
- In `src/sandbox/browser/wire.rs`:
  - Wire header framing: 8-byte little-endian: `[png_len: u32, metadata_len: u32]`.
  - Max payload size: `MAX_PAYLOAD_SIZE = 32 * 1024 * 1024` (32 MiB).
  - Existing operations: `Image = 1`, `Raw = 2`, `Pdf = 3`, `Video = 4`, `ImageMetadata = 5`, `MediaMetadata = 6`, `PreviewImage = 7`, `DocumentMermaid = 8`, `DocumentMath = 9`, `DocumentMathInline = 10`, `ThreeMfThumbnail = 11`, `FreeCadThumbnail = 12`, `PreviewGeoTiff = 13`.
  - Next operation codes available for M4: `PreviewSpreadsheet = 14`, `PreviewAudioWaveform = 15`.
- In `src/sandbox/browser/worker.rs`:
  - Requests are handled by `execute_job(operation, input_fd, output_fd)` via `/proc/self/fd/<fd>`.
- In `src/sandbox/browser.rs`:
  - Sandbox limits enforced by `prlimit`: `--as=1342177280` (1.25 GB), `--cpu=10` (10s), `--fsize=33554432` (32 MiB).
  - Bubblewrap command: `--unshare-all`, `--clearenv`, `--ro-bind /usr /usr`. `/usr/bin/ffmpeg` is mounted and available.

---

## 2. Logic Chain

### 2.1 Spreadsheets: Architecture, Parsing, Virtual Table & Text Search
1. **Format Scope & Crate Selection**:
   - The user request requires tabular preview and text extraction for ODS, XLS, and XLSX.
   - ODS is a ZIP archive containing `content.xml`.
   - XLSX is an OpenXML ZIP archive containing `xl/workbook.xml`, `xl/worksheets/sheet*.xml`, and `xl/sharedStrings.xml`.
   - XLS is a legacy binary Compound File Binary (CFB) format containing BIFF8 records.
   - The pure-Rust crate `calamine = "0.36.1"` supports all three formats (plus XLSB) with zero C library dependencies and zero unsafe blocks.
   - Adding `calamine = "0.36.1"` and `csv = "1.4.0"` to `Cargo.toml` satisfies all requirements without external runtime requirements.

2. **Calamine Parsing & Memory Budgeting**:
   - Calamine's `open_workbook_auto(path)` automatically sniffs the container (ZIP vs OLE) and instantiates the proper reader (`Xlsx`, `Xls`, `Ods`).
   - Prior to opening, the file size must be checked against `WORKBOOK_BYTE_LIMIT = 20 * 1024 * 1024` (20 MiB) to prevent decompression bomb denial-of-service.
   - The parser reads the first worksheet range (`workbook.worksheet_range_at(0)`).
   - Iteration over `range.rows()` must enforce hard limits:
     - `TABLE_COLUMN_LIMIT = 256` columns max.
     - `TABLE_ROW_LIMIT = 200` rows max for preview.
     - `TABLE_VALUE_LIMIT = 100_000` cells max.
     - `TABLE_TEXT_LIMIT = 4 * 1024 * 1024` (4 MiB) text max.
   - If any limit is reached, `truncated` is marked `true` and iteration halts.

3. **Wire Protocol Schema for Spreadsheets**:
   - Operation: `Operation::PreviewSpreadsheet = 14`.
   - In-sandbox execution reads `/proc/self/fd/<fd>` using `calamine`, extracts headers and rows, and serializes `SpreadsheetData` to JSON.
   - Response framing: `png_len = 0`, `metadata_len = json_bytes.len()`, followed by `json_bytes`.
   - Data structure:
     ```rust
     #[derive(Debug, Clone, Serialize, Deserialize)]
     pub struct SpreadsheetData {
         pub sheet_names: Vec<String>,
         pub active_sheet: usize,
         pub headers: Vec<String>,
         pub rows: Vec<Vec<String>>,
         pub total_rows: usize,
         pub total_cols: usize,
         pub truncated: bool,
     }
     ```

4. **Searchable Text Extraction for Search Indexer**:
   - In `src/services/formats.rs`:
     - MIME types (`spreadsheetml.sheet`, `vnd.ms-excel`, `opendocument.spreadsheet`) and extensions (`xlsx`, `xls`, `ods`) map to `FormatFamily::Spreadsheet`.
     - Capabilities: `preview: Some(PreviewHandler::Spreadsheet)`, `text_extractor: Some(TextExtractor::Spreadsheet)` (or `TextExtractor::Office`).
   - In `src/sandbox_helper.rs`:
     - `extract_office_text` is extended to support `.ods` by reading `content.xml`.
     - In-sandbox `extract_spreadsheet_text` uses `calamine` to iterate over all cell values across all sheets, joining cells with spaces and rows with newlines up to `byte_limit`.
     - Output is validated UTF-8 and passed back to `services::search::LocalTextExtractionProvider`.

5. **GTK4 Virtual Table View (`src/ui/table_view.rs`)**:
   - Loading 200 rows with dozens of columns using standard GTK widgets would allocate thousands of individual widgets, causing UI lag.
   - GTK4 `gtk::ColumnView` with `gtk::SortListModel` and `gtk::NoSelection` provides widget recycling: only visible viewport rows are instantiated.
   - Sorting: Column headers attach `gtk::CustomSorter` with `SortKey` (numeric `f64` comparison via `total_cmp`, string comparison alphabetically).
   - Resizing: `gtk::ColumnViewColumn::set_resizable(true)` with auto-measured initial widths (72px to 640px).
   - Display: Displays headers, alternating rows, and a bottom warning banner if `truncated` is true.

---

### 2.2 Audio Waveforms: Architecture, Sandbox Generation, Aesthetics & UI
1. **Waveform Generation Architecture**:
   - Generating waveforms on the host thread or in-process could block GTK or crash on malformed audio decodes.
   - The user request requires sandboxed waveform generation inside the Bubblewrap sandbox.
   - FFmpeg (`ffmpeg n9.0.2`) is pre-installed on the host and bind-mounted at `/usr/bin/ffmpeg` inside the container.
   - FFmpeg's `showwavespic` filter decodes any supported audio container (FLAC, MP3, WAV, OGG, Opus, AAC) and directly outputs a single RGBA PNG image with transparency in a single sub-60ms pipeline.
   - This eliminates the need for heavyweight Rust audio decoding crates (`symphonia`, `rodio`) or complex multi-codec bindings.

2. **Sandbox Invocation Model**:
   - Operation: `Operation::PreviewAudioWaveform = 15`.
   - Sandbox worker executes:
     ```bash
     ffmpeg -nostdin -v error -threads 2 \
       -i /proc/self/fd/<fd> \
       -filter_complex "showwavespic=s=800x240:colors=0x7aa2f7:filter=peak:scale=cbrt" \
       -frames:v 1 -c:v png -f image2 -y /tmp/waveform.png
     ```
   - Concurrently extracts audio metadata (format, duration, channels, sample rate) into `AudioMetadata`:
     ```rust
     #[derive(Debug, Clone, Serialize, Deserialize)]
     pub struct AudioMetadata {
         pub format: String,
         pub duration_seconds: f64,
         pub sample_rate: u32,
         pub channels: u16,
         pub bitrate: Option<u64>,
     }
     ```
   - Wire framing: `png_len = png_bytes.len()`, `metadata_len = meta_json.len()`, followed by `png_bytes` and `meta_json`.

3. **Visual Aesthetics & Theme Alignment**:
   - Background: Alpha transparency (`RGBA`), enabling automatic blending with Hermes' dark background (`#1a1b26` / `#1e1e2e`).
   - Waveform color: Tokyo Night / Hermes accent blue `0x7aa2f7` (or dynamically supplied from `ThemeManager::shared().appearance_tokens().accent`).
   - Filter mode: `filter=peak` provides sharp amplitude spikes matching modern audio visualizers (SoundCloud / Ableton) rather than blurred blocks (`average`).
   - Amplitude scaling: `scale=cbrt` (cubic root) ensures that low-volume intro sections remain clearly visible while dynamic peak crescendos do not clip.
   - Output resolution: `800x240` (matches standard preview drawer width and provides 1:3.3 aspect ratio).

4. **UI Integration in `src/ui/preview.rs`**:
   - `PreviewContent::AudioWaveform { png, metadata }`:
     - Top metadata bar: format badge (`FLAC 24-bit 96kHz`), duration (`03:42`), sample rate (`44.1 kHz`), channels (`Stereo`).
     - Center display: `gtk::Picture` rendering the transparent waveform over a subtly textured dark container.
     - Bottom playback bar: Play/Pause button, timeline seek slider, volume control.
     - Optional live enhancement: When playback is started, Hermes can activate the live 84-bar dancing spectrum from `SoundCloudWaveform` (`24857a55`) or sweep a playback cursor across the static waveform.

---

### 2.3 Error Handling & Memory Bounds
1. **Corrupted & Malicious Spreadsheets**:
   - File size ceiling: Reject files `> 20 MiB` immediately with `"Workbook is too large to preview safely"`.
   - Truncated/corrupted ZIP archives (ODS/XLSX): `calamine` returns `CalamineError::Zip` -> map to `"Invalid or corrupted spreadsheet archive"`.
   - Truncated OLE compound binary (XLS): `calamine` returns `CalamineError::InvalidOle` -> map to `"Invalid Excel binary workbook header"`.
   - Missing worksheets / empty workbook: Check `workbook.sheet_names().is_empty()` -> map to `"Workbook contains no worksheets"`.
   - Decompression bomb guard: Per-row iteration limits cap total memory to `< 4 MiB` allocated strings.
   - Sandbox resource limit: `prlimit --as=1342177280 --cpu=10 --fsize=33554432` protects the host system.

2. **Corrupted & Malicious Audio Files**:
   - Truncated RIFF WAV, invalid FLAC stream marker, zero sample rate, zero channels: FFmpeg errors out with non-zero exit code.
   - The worker catches non-zero exit and returns `Err("Audio file is corrupted or unsupported")`.
   - Wall-time timeout (12s) and cancellation token responsiveness prevent infinite hangs on `/dev/urandom` or streaming FIFOs.
   - Wire framing size check rejects payloads `> 32 MiB`.

---

## 3. Caveats
1. **Chart & Macro Extraction**:
   - Embedded VBA macros, conditional formatting rules, and vector charts inside Excel/ODS sheets are not rendered in tabular preview (only text and numeric cell data are extracted). This is standard for file manager quick previews.
2. **Formula Calculations**:
   - `calamine` extracts the cached evaluated values stored by Excel/LibreOffice. It does not evaluate dynamic formula recalculations for sheets saved without cached results.
3. **Audio Waveform Color Token Passing**:
   - If the sandbox worker runs in a separate process, it cannot directly inspect GTK's live in-memory CSS theme state unless the accent hex color string (e.g. `7aa2f7`) is passed in the request header or configuration. The default Hermes dark theme accent `#7aa2f7` should be used as the standard fallback.
4. **Live Playback vs Static Waveform**:
   - Static waveform generation via `showwavespic` provides instant, pre-rendered visualization without starting playback. Live FFT spectrum visualization requires an active playback pipeline (`gtk::MediaStream` / GStreamer). The recommended architecture uses the static waveform as the base preview, with interactive playback layered on top.

---

## 4. Conclusion & Concrete Implementation Recommendations

### 4.1 Required Cargo Dependencies (`Cargo.toml`)
```toml
[dependencies]
calamine = "0.36.1"
csv = "1.4.0"
```
No other C libraries or crates are required.

### 4.2 Format Registry Updates (`src/services/formats.rs`)
```rust
// Add to FormatFamily:
pub enum FormatFamily {
    ...
    Spreadsheet,
}

// In classify_by_mime:
"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
| "application/vnd.ms-excel"
| "application/vnd.oasis.opendocument.spreadsheet" => FormatFamily::Spreadsheet,

// In classify_by_name:
"xlsx" | "xls" | "ods" => FormatFamily::Spreadsheet,

// In capabilities:
FormatFamily::Spreadsheet => FormatCapabilities {
    family: FormatFamily::Spreadsheet,
    preview: Some(PreviewHandler::Spreadsheet),
    thumbnail: None,
    text_extractor: Some(TextExtractor::Office), // or TextExtractor::Spreadsheet
}
```

### 4.3 Wire Protocol & Sandbox Operations
- `src/sandbox/browser/wire.rs`:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  #[repr(u8)]
  pub(crate) enum Operation {
      ...
      PreviewGeoTiff = 13,
      PreviewSpreadsheet = 14,
      PreviewAudioWaveform = 15,
  }
  ```
- `src/sandbox.rs`:
  ```rust
  pub(crate) enum ParseOperation {
      ...
      PreviewSpreadsheet,
      PreviewAudioWaveform,
  }
  ```
- `src/sandbox_helper.rs`:
  - Delegate `preview-spreadsheet` to `src/sandbox_helper/table.rs`.
  - Delegate `preview-audio-waveform` to `src/sandbox_helper/audio.rs`.

### 4.4 New Source Files Recommended for Implementation
1. `src/services/table.rs`: Calamine workbook parsing, `SpreadsheetData` struct, row/col budget limits, JSON serialization.
2. `src/sandbox_helper/table.rs`: In-sandbox helper for spreadsheet parsing and text extraction.
3. `src/sandbox_helper/audio.rs`: In-sandbox helper calling FFmpeg `showwavespic` and extracting audio metadata.
4. `src/ui/table_view.rs`: GTK4 `gtk::ColumnView` virtualized table, `SortListModel`, `CustomSorter`, and column resizing.
5. `src/ui/table_view/selection.rs`: Multi-cell text selection support.
6. `src/ui/preview/waveform.rs`: Waveform visualizer container with playback transport controls.

### 4.5 Preview Content & Drawer Integration
- `src/services/preview.rs`:
  ```rust
  pub enum PreviewContent {
      ...
      Spreadsheet { table: SpreadsheetData },
      AudioWaveform { png: Vec<u8>, metadata: AudioMetadata },
  }
  ```
- `src/ui/preview.rs`:
  - Add branches in `render(preview)`:
    - `PreviewContent::Spreadsheet { table } => self.render_spreadsheet_viewer(table)`
    - `PreviewContent::AudioWaveform { png, metadata } => self.render_audio_waveform(png, metadata)`

---

## 5. Verification Method

### 5.1 Compilation & Existing Baseline Regression Verification
Execute from the project root `/home/bry/.gemini/antigravity/scratch/hermes`:
```bash
# Verify unit test baseline remains 100% passing (216 tests)
cargo test --bin strata

# Verify E2E suite remains 100% passing (165 tests across Tiers 1-4)
cargo test --test e2e_tests
```

### 5.2 Specific E2E Spreadsheet & Audio Tests
```bash
# Run spreadsheet isolated and boundary tests
cargo test --test e2e_tests test_f10
cargo test --test e2e_tests test_scenario_4

# Run audio isolated and boundary tests
cargo test --test e2e_tests test_f11
cargo test --test e2e_tests test_p10_audio
cargo test --test e2e_tests test_p20_audio
```

### 5.3 Manual Subprocess Waveform Verification Command
To verify FFmpeg waveform output directly on sample audio streams:
```bash
# Generate sample test WAV
python3 -c '
import struct, math
sr = 44100; n = int(sr * 0.2)
with open("/tmp/verify.wav", "wb") as f:
    f.write(b"RIFF" + struct.pack("<I", 36 + n*2) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 1, sr, sr*2, 2, 16) + b"data" + struct.pack("<I", n*2))
    for i in range(n): f.write(struct.pack("<h", int(math.sin(2*math.pi*440*i/sr)*16000)))
'

# Render waveform via FFmpeg showwavespic
ffmpeg -y -i /tmp/verify.wav -filter_complex "showwavespic=s=800x240:colors=0x7aa2f7:filter=peak:scale=cbrt" -frames:v 1 /tmp/verify.png

# Confirm valid 800x240 RGBA PNG
file /tmp/verify.png
```

### 5.4 Invalidation Conditions
This investigation's recommendations are invalidated if:
1. `calamine` v0.36 fails to compile on Linux x86_64 under Rust 2024 edition (verified working).
2. The host environment lacks `ffmpeg` with `showwavespic` compiled in (verified present in FFmpeg n9.0.2).
3. The Bubblewrap sandbox configuration drops `/usr/bin` from its read-only mounts (verified present).
