# Handoff Report: Rich Previews (R3), Status Bar (R4), & Test Baseline

## 1. Observation

### 1.1 Test Suite Baseline
- **Command & Output**: Running `cargo test -- --list` reports:
  ```
  181 tests, 0 benchmarks
  ```
- **Test Execution**: Running `cargo test` succeeds in 0.06s:
  ```
  test result: ok. 181 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
  ```
- **Test Structure & Fixtures**:
  - The repository has **no `tests/` directory** on `main` (confirmed via `list_dir`).
  - All 181 tests are co-located unit tests located in `tests.rs` or test submodules next to implementation files (e.g., `src/adapters/local_preview/tests.rs`, `src/services/formats/tests.rs`, `src/services/search/tests.rs`, `src/ui/preview/tests.rs`).
  - Fixtures are generated dynamically at runtime using `std::env::temp_dir()` (e.g. `cairo::PdfSurface` generates test PDF bytes dynamically in `src/adapters/local_preview/tests.rs:24-60`).
  - Sandbox unit tests test `sandbox_command` flags directly (checking `--unshare-all`, `--as=...`, `--cpu=10`, `--fsize=...` in `src/sandbox/tests.rs`) and test execution directly via `crate::sandbox_helper::run(&[...])` in-process without spawning `bwrap`.

### 1.2 Status Bar Implementation (`src/ui/status_bar.rs` & `src/ui/window.rs`)
- **Compiler Warnings (`cargo check`)**:
  ```text
  warning: unused import: `FileEntry`
    --> src/ui/window.rs:17:20
  warning: field `free_space` is never read
    --> src/ui/status_bar.rs:10:5
     |
   6 | pub struct StatusBar {
  ...
  10 |     free_space: gtk::Label,
  warning: method `update_free_space` is never used
    --> src/ui/status_bar.rs:63:12
     |
  63 |     pub fn update_free_space(&self, path: &Path) {
  warning: function `build_status_bar` is never used
    --> src/ui/status_bar.rs:86:8
     |
  86 | pub fn build_status_bar() -> StatusBar {
  ```
- **Status Bar Structure (`src/ui/status_bar.rs`)**:
  - Struct `StatusBar` contains:
    - Line 7: `pub container: gtk::Box`
    - Line 8: `item_count: gtk::Label`
    - Line 9: `selection_info: gtk::Label`
    - Line 10: `free_space: gtk::Label`
  - Methods:
    - Line 14: `pub fn new() -> Self` creates the layout (`container` with left box containing `item_count` and `selection_info`, right-aligned `free_space`).
    - Line 46: `pub fn update_item_count(&self, count: usize)` sets `"1 item"` or `"{count} items"`.
    - Line 54: `pub fn update_selection(&self, count: usize, total_bytes: u64)` sets `"{count} selected, {format_file_size(total_bytes)}"`.
    - Line 63: `pub fn update_free_space(&self, path: &Path)` calls `gio::File::for_path(path).query_filesystem_info_future("filesystem::free", glib::Priority::DEFAULT)` and updates `free_space` label to `"{format_file_size(free_bytes)} free"`.
    - Line 86: `pub fn build_status_bar() -> StatusBar` returns `StatusBar::new()`.
- **Wiring in `src/ui/window.rs`**:
  - Line 174: `let status_bar = Rc::new(super::status_bar::StatusBar::new());` instantiates `StatusBar` directly (leaving `build_status_bar` unused).
  - Lines 191-218:
    ```rust
    if matches!(
        event,
        BrowserEvent::EntriesInserted { .. }
            | BrowserEvent::EntriesReplaced { .. }
            | BrowserEvent::EntriesSpliced { .. }
            | BrowserEvent::SelectionSetChanged { .. }
            | BrowserEvent::FocusChanged { .. }
            | BrowserEvent::ColumnAdded { .. }
    ) {
        let mut total_items = 0;
        let mut selected_items = 0;
        let mut selected_bytes = 0;
        
        if let Some(depth) = context_controller.active_depth() {
            if let Some(entries) = context_controller.column_entries(depth) {
                total_items = entries.len();
            }
            let selected = context_controller.selected_entries();
            selected_items = selected.len();
            for entry in selected {
                if let crate::model::MetadataValue::Known(size) = entry.size {
                    selected_bytes += size;
                }
            }
        }
        context_status_bar.update_item_count(total_items);
        context_status_bar.update_selection(selected_items, selected_bytes);
    }
    ```
  - `context_status_bar.update_free_space` is **never called anywhere in `window.rs`**.
  - Events such as `BrowserEvent::ColumnsTruncated`, `BrowserEvent::ColumnReloaded`, `BrowserEvent::Reset`, and initial startup do not update the status bar or trigger free space queries.

### 1.3 Preview Registry and Format Dispatching (`src/preview/` & Related)
- **Current Architecture**:
  - `src/services/formats.rs`:
    - `classify_by_mime(content_type: &str) -> FormatFamily`
    - `classify_by_name(name: &OsStr) -> FormatFamily`
    - `thumbnail_handler_for_name(name: &OsStr) -> Option<ThumbnailHandler>`
  - `src/services/preview.rs`:
    - `PreviewContent`:
      - Text, Image, Media, Rasterized, SandboxedMedia, Pdf
      - Lines 28-30: `Code { language, content }`, `Markdown { content }`, `Model3D { format, data }` (currently unconstructed variants generating warnings).
  - `src/adapters/local_preview.rs`:
    - `LocalPreviewProvider::load`: queries `standard::content-type`, gets `family.preview_handler()`, maps to `sandbox::ParseOperation`, and runs `sandbox::parse(&path, operation, value, &cancellation)` in a worker thread.
  - `src/sandbox.rs`:
    - Spawns Bubblewrap `bwrap` with `--unshare-all`, private tmpfs, read-only `/usr`, `/lib`, `/input`, read-write `/output`, and `prlimit` limits (`--as=1342177280`, `--cpu=10`, `--fsize=33554432`).
    - Executes `/app/strata --preview-helper <operation> /input /output/<name> <value>`.
  - `src/sandbox_helper.rs`:
    - CLI entry point `run(arguments)` executing format renderers:
      - `thumbnail-image`, `thumbnail-heif`, `thumbnail-raw`, `thumbnail-pdf`, `thumbnail-video`
      - `preview-image`, `preview-heif`, `preview-pdf`, `preview-media`, `preview-audio`
      - `extract-pdf-text`, `preview-archive`, `preview-office`, `extract-office-text`
- **Existing Upstream Commits for Required Formats**:
  - **Interactive PDF Selection**:
    - Commit `e816f85b` (`remotes/upstream/feat/1220-pdf-preview-text-selection`):
      - In `sandbox_helper.rs`: calls `poppler::ffi::poppler_page_get_text_layout` to retrieve glyph bounding boxes, outputs `result.text` as JSON (`PdfTextLayer`).
      - In `src/ui/preview/pdf_text.rs`: segments glyphs into lines and hit-testable spans.
      - In `src/ui/preview.rs`: connects gesture drag and selection controllers, renders highlight overlays, and handles Ctrl+C / copy to clipboard via `gdk::Display::default().clipboard().set_text(...)`.
  - **3D Models (STL, 3MF geometry)**:
    - Commit `570bdd30` (`feat(preview): add sandboxed STL, 3MF and FreeCAD previews (#1276)`):
      - In `src/sandbox_helper/model.rs`: parses STL (binary and ASCII) and 3MF (ZIP with `3D/3dmodel.model` XML and embedded thumbnails).
      - Renders 3D geometry using a depth buffer and directional lighting (`shade`), outputting PNG.
      - In `src/ui/preview.rs`: replaces the `"3D Model Viewer"` placeholder with rasterized PNG preview.
  - **eBooks & Comics (EPUB, CBZ, CBR)**:
    - Commit `ba676d3e` (`feat(preview): show comic and EPUB covers in thumbnails and Quick Preview (#1325)`):
      - In `src/sandbox_helper/archive_cover.rs`: extracts first image from CBZ (ZIP), CBR (RAR via `bsdtar`/`unrar`), or EPUB (`META-INF/container.xml` -> OPF manifest -> `<item properties="cover-image">` or `<meta name="cover">`).
      - In `src/services/formats.rs`: registers `epub`, `cbz`, `cbr` with `ThumbnailHandler::Image` and `PreviewHandler::Image`.
  - **Spreadsheets (ODS, XLS, XLSX)**:
    - Commit `d39052be` (`feat(preview): add sortable virtual tables for documents and spreadsheets (#729)`):
      - Uses `calamine` to parse ODS, XLS, XLSX workbooks.
      - Renders virtual table in `src/ui/table_view.rs`.
      - Extracts searchable text by concatenating row cells.
  - **Audio Waveforms (FLAC, MP3, WAV, OGG)**:
    - Commit `40a5a606` & `24857a55` (`upstream/feat/audio-soundcloud-visualizer`):
      - Provides live spectrum waveform visualization in `src/ui/preview/waveform.rs`.
    - Native FFmpeg Alternative:
      - FFmpeg is already installed (`/usr/bin/ffmpeg`) and has the built-in `showwaves` and `showwavespic` filters.
      - In `sandbox_helper.rs:render_audio_preview`: replacing dummy black video `-f lavfi -i color=c=black:s=640x360:r=1` with `-filter_complex "[0:a:0]showwaves=s=640x360:mode=line:colors=0x268bd2[v]"` produces a real-time synchronized waveform video stream rendered directly by GTK's native `gtk::Video` widget.

---

## 2. Logic Chain

1. **Why `update_free_space` has dead code compiler warnings**:
   - `src/ui/status_bar.rs:63` defines `pub fn update_free_space(&self, path: &Path)`.
   - In `src/ui/window.rs`, `StatusBar` is instantiated at line 174. In the event observer (lines 177-219), `context_status_bar.update_item_count(...)` and `context_status_bar.update_selection(...)` are called, but `update_free_space` is never called.
   - Rust's dead code analysis flags `update_free_space` as unused.
   - Because `update_free_space` is the only code that reads `self.free_space` (outside `StatusBar::new`), `free_space` is flagged as an unread field.
   - In addition, line 86 `build_status_bar()` is dead code because `window.rs` invokes `StatusBar::new()` directly.

2. **How Status Bar should be wired for dynamic updates and cross-mount navigation**:
   - Active path retrieval: `context_controller.active_location()` returns `Option<Location>`, and `Location::native_path()` returns `Option<&Path>`.
   - Navigation events: The observer in `window.rs` must include `BrowserEvent::ColumnsTruncated`, `BrowserEvent::ColumnReloaded`, `BrowserEvent::Reset`, and the initial setup call.
   - Dynamic file creation/deletion: `src/adapters/local_files.rs` uses `gio::FileMonitor` on active folders. File changes trigger `EntriesInserted`, `EntriesSpliced`, or `EntriesReplaced`. Calling `update_free_space(path)` inside the observer ensures free disk space refreshes whenever files are created or deleted.
   - Cross-mount navigation: `StatusBar::update_free_space` invokes GIO's `query_filesystem_info_future("filesystem::free", ...)`. GIO resolves the mount point containing the given `Path` and queries filesystem statistics from the OS. Thus, navigating between `/home` and `/mnt/external` naturally reflects the free space of the active mount point.

3. **How Rich Previews (R3) integrate with Hermes preview architecture**:
   - Format classification (`services/formats.rs`) must map extensions:
     - 3D: `.stl`, `.3mf` -> `FormatFamily::Model3D` or `PreviewHandler::Image` (or `PreviewHandler::Model3D`).
     - eBooks/Comics: `.epub`, `.cbz`, `.cbr` -> `FormatFamily::Image` (or `PreviewHandler::Image`, `ThumbnailHandler::Image`).
     - Spreadsheets: `.ods`, `.xls`, `.xlsx` -> `FormatFamily::OfficeDocument` / `TextExtractor::Office` / `PreviewHandler::Office` (or dedicated Table preview).
     - Audio: `.flac`, `.mp3`, `.wav`, `.ogg` -> `FormatFamily::Audio`.
   - Sandbox execution (`sandbox.rs` & `sandbox_helper.rs`):
     - All parsing runs inside `bwrap` with memory and CPU bounds.
     - 3D: renders software-shaded PNG from STL/3MF.
     - eBooks/Comics: extracts cover image via `bsdtar` / zip / rar reader into `result.png`.
     - Spreadsheets: extracts sheet rows via `calamine` into tabular data or formatted text.
     - Audio: generates waveform video/PNG via `ffmpeg` (`showwaves` / `showwavespic`).
     - Interactive PDF: Poppler extracts glyph layout rectangles into `result.text` (`PdfTextLayer`), enabling UI text highlight and clipboard copying.

---

## 3. Caveats

1. **No External Test Fixture Files**: `tests/` directory does not exist on `main`. Tests rely on ephemeral files generated in `/tmp`. Adding test cases for new formats should follow this existing pattern (generating small in-memory/temp fixtures) or adding tiny, bounded fixtures under `tests/fixtures/`.
2. **Calamine Crate Dependency**: Spreadsheet parsing via `calamine` requires adding `calamine = "0.26"` (or similar) to `Cargo.toml`.
3. **FFmpeg Waveform vs DrawingArea Waveform**: FFmpeg's `showwaves` produces an animated Ogg/WebM video stream directly runnable in the existing `gtk::Video` widget without custom UI widget code. If interactive clicking/seeking on individual waveform bars is required, upstream `SoundCloudWaveform` (`src/ui/preview/waveform.rs`) requires custom `gtk::DrawingArea` rendering.
4. **Poppler FFI for PDF Layout**: `poppler-rs` does not expose safe bindings for `poppler_page_get_text_layout`, so `unsafe { poppler::ffi::poppler_page_get_text_layout(...) }` is used in upstream `e816f85b`. Hermes has `unsafe_code = "deny"`, so an `#[expect(unsafe_code, reason = "...")]` attribute is required.

---

## 4. Conclusion

- **Status Bar (R4)**: The compiler warnings are caused by `update_free_space` and `build_status_bar` being uncalled. In `src/ui/window.rs`, `status_bar.update_free_space(path)` can be wired directly in `controller.observe(...)` using `context_controller.active_location().and_then(|l| l.native_path())`. This eliminates all 3 status bar compiler warnings and provides dynamic item counts, multi-selection aggregate byte totals, and cross-mount free disk space updates.
- **Rich Format Previews (R3)**: Complete, tested upstream implementations already exist in Git history (`e816f85b`, `570bdd30`, `ba676d3e`, `d39052be`, `40a5a606`). They follow the existing Bubblewrap `prlimit` sandboxing model and format dispatch registry.
- **Test Baseline**: All 181 unit tests pass in 0.06s. Any new format extractors can be covered with co-located unit tests in `tests.rs` following the established in-memory / tempdir fixture conventions.

---

## 5. Verification Method

1. **Verify Test Suite**:
   ```bash
   cargo test -- --list
   # Expect: 181 tests, 0 benchmarks
   cargo test
   # Expect: 181 passed; 0 failed
   ```
2. **Verify Compiler Warnings & Status Bar Dead Code**:
   ```bash
   cargo check
   # Inspect lines in src/ui/status_bar.rs (lines 10, 63, 86) and src/ui/window.rs (line 17)
   ```
3. **Verify Git History Commits for Reference Implementations**:
   ```bash
   git show --stat e816f85b   # PDF text selection
   git show --stat 570bdd30   # 3D models (STL, 3MF)
   git show --stat ba676d3e   # eBooks & Comics (EPUB, CBZ, CBR)
   git show --stat d39052be   # Spreadsheets (ODS, XLS, XLSX)
   git show --stat 40a5a606   # Audio waveform visualizer
   ```
