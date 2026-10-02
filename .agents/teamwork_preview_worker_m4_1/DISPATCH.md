## 2026-10-01T19:02:02Z
You are Worker M4.1 (Rich Format Previews Builder).
Your working directory is: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_worker_m4_1
Project root: /home/bry/.gemini/antigravity/scratch/hermes
Original request: /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md
Scope document: /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_2/PROJECT.md

MANDATORY INTEGRITY WARNING:
DO NOT CHEAT. All implementations must be genuine. DO NOT hardcode test results, create dummy/facade implementations, or circumvent the intended task. A teamwork_preview_auditor will independently verify your work. Integrity violations WILL be detected and your work WILL be rejected.

MANDATORY FIRST STEPS:
1. You MUST read /home/bry/.gemini/antigravity/scratch/hermes/ORIGINAL_REQUEST.md in full before starting work.
2. Read the scope document /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_orchestrator_2/PROJECT.md.
3. Read the three Explorer handoff reports:
   - /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m4_1/handoff.md (3D Models & Comic/eBook Previews)
   - /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m4_2/handoff.md (Spreadsheets & Audio Waveforms)
   - /home/bry/.gemini/antigravity/scratch/hermes/.agents/teamwork_preview_explorer_m4_3/handoff.md (Interactive PDF Text Selection & Clipboard Copying)
4. Initialize and update progress.md regularly with your liveness heartbeat.

IMPLEMENTATION MANDATE:
Implement Milestone 4 (R3: Rich Format Previews):
1. **3D Models (STL & 3MF Previews)**:
   - Implement geometry parsing for ASCII STL, binary STL, and 3MF OPC ZIP packages (reading `3D/3dmodel.model` XML with `quick-xml`).
   - Implement software rasterizer with isometric projection, normal lighting, 2D barycentric Z-buffer, work bounds (`MAX_MODEL_RASTER_WORK = 100_000_000`), generating PNG.
   - Reference commit: `570bdd30`.
2. **eBooks & Comics (EPUB, CBZ, CBR Cover Art Extraction)**:
   - EPUB: read `META-INF/container.xml` -> parse OPF package document (`quick-xml`) for EPUB 3 `cover-image` item or EPUB 2 `<meta name="cover">` -> extract cover image.
   - CBZ: list ZIP archive entries -> natural alphanumeric sort (`compare_names`) -> extract earliest cover image.
   - CBR: invoke `bsdtar` in sandbox to list and extract earliest cover image.
   - Scale extracted cover (max 16 MP guard) to PNG.
   - Reference commit: `ba676d3e`.
3. **Spreadsheets (ODS, XLS, XLSX Virtual Tables & Text Extraction)**:
   - Add `calamine = "0.36.1"` and `csv = "1.4.0"` to `Cargo.toml`.
   - Parse ODS, XLS, XLSX workbooks via `calamine`, enforcing limits (20MB workbook, 200 preview rows, 256 cols, 100,000 cells).
   - In-sandbox text extraction for search indexer: concatenate cell strings separated by spaces/newlines.
   - UI: GTK4 `gtk::ColumnView` virtual table view (`src/ui/table_view.rs`) with `gtk::SortListModel`, `gtk::CustomSorter`, and column resizing.
   - Reference commit: `d39052be`.
4. **Audio Waveforms (FLAC, MP3, WAV, OGG Visualizers)**:
   - In sandbox worker, invoke FFmpeg `showwavespic` filter:
     `ffmpeg -nostdin -v error -threads 2 -i <input> -filter_complex "showwavespic=s=800x240:colors=0x7aa2f7:filter=peak:scale=cbrt" -frames:v 1 -c:v png -f image2 -y <output.png>`
   - Concurrently extract audio metadata (format, duration, sample rate, channels).
   - UI: `PreviewContent::AudioWaveform { png, metadata }` displaying waveform and transport/metadata in preview drawer.
   - Reference commits: `40a5a606`, `24857a55`.
5. **Interactive PDF Selection & Clipboard Copying**:
   - In `src/sandbox_helper.rs`, call Poppler C FFI `poppler_page_get_text_layout` with `#[expect(unsafe_code, reason = "poppler-rs exposes no safe binding for poppler_page_get_text_layout")]` and dedicated per-operation `// SAFETY:` blocks, freeing memory via `glib::ffi::g_free`.
   - Serialize `PdfTextLayer` to `result.text` (max 8MB).
   - Pure-functional text segmentation model in `src/ui/preview/pdf_text.rs` (`lines`, `image_bounds`, `hit_text`, `caret_at`, `has_descender`, `selection_runs`, `word_range`, `line_range`).
   - In `src/ui/preview.rs`: Cairo highlight overlay on `gtk::DrawingArea`, mouse drag selection (`GestureDrag`), click counts (`GestureClick`), I-beam cursor (`EventControllerMotion`), standard `Ctrl+C` copy to GTK clipboard (`gdk::Display::default().clipboard().set_text(...)`).
   - Maintain selected unbound layers across virtualized scrolling.
   - CRITICAL CONSTRAINT: DO NOT implement modal/Vim keyboard navigation modes (explicitly forbidden). Standard mouse drag selection and standard copy keyboard shortcut only.
   - Reference commit: `e816f85b`.
6. **Integration & Wire Protocol**:
   - `src/sandbox/browser/wire.rs`:
     `PreviewModel = 14`, `PreviewArchiveCover = 15`, `PreviewSpreadsheet = 16`, `PreviewAudioWaveform = 17`.
   - Update `src/services/formats.rs`, `src/services/preview.rs`, `src/adapters/local_preview.rs`, `src/sandbox.rs`, `src/sandbox/browser/worker.rs`, `src/sandbox_helper.rs`.
   - Ensure 0 compiler warnings in all modified files.

VERIFICATION REQUIREMENTS:
1. `cargo check` must succeed with ZERO warnings in all modified files.
2. `cargo test --bin strata` must pass 100% of unit tests.
3. `cargo test --test e2e_tests` must pass 100% of tests (all 165 tests across Tiers 1-4).
4. Co-locate unit tests for the new parsers, text geometry, and wire protocol.

OUTPUT:
Write your structured findings, changes made, and test outputs to handoff.md in your working directory. Send a message to the orchestrator when complete.
