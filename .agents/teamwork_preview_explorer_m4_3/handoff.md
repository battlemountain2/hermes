# Handoff Report: Milestone M4.3 — Interactive PDF Text Selection & Clipboard Copying

## 1. Observation

### Reference Commit Anatomy (`e816f85b`)
Commit `e816f85bfa4dea1160cd1b72c4bd37b3ef4f9991`:
- **Author**: Tai Nguyen <87302343+JoeJoeflyn@users.noreply.github.com>
- **Subject**: `feat(preview): select and copy text in PDF quick previews (#1223)`
- **Key Changes Across Subsystems**:
  1. `src/sandbox_helper.rs`: Extended `render_pdf_page` to extract per-page glyph layouts via Poppler C FFI (`poppler_page_get_text_layout`), serializing `PdfTextLayer` to `result.text`.
  2. `src/sandbox.rs`: Added `MAX_TEXT_LAYER_BYTES: u64 = 8 * 1024 * 1024;`, added `text_layer: Option<crate::services::PdfTextLayer>` to `ParseOutput`, and deserialized `result.text` on `ParseOperation::PreviewPdf`.
  3. `src/services/preview.rs`: Defined `PdfTextLayer` struct with fields `width: f32`, `height: f32`, `text: String`, `glyphs: Vec<[f32; 4]>` (bounding box `[x1, y1, x2, y2]` in surface pixel coordinates); updated `PreviewContent::Pdf` to include `text_layer: Option<Arc<PdfTextLayer>>`.
  4. `src/adapters/local_preview.rs`: Updated `PreviewContent::Pdf` instantiation to wrap `output.text_layer.map(std::sync::Arc::new)`, and updated `preview_content_size` cache estimation to include text length and glyph vector size (`glyphs.len() * size_of::<[f32; 4]>()`).
  5. `src/ui/preview/pdf_text.rs` & tests: Pure-functional text geometry module for lines, hit testing (`hit_text`), caret snapping (`caret_at`), descender detection (`has_descender`), selection band trimming, selection runs (`selection_runs`), double-click word ranges (`word_range`), and triple-click line ranges (`line_range`).
  6. `src/ui/preview/pdf_ranges_tests.rs`: Tests covering character/word/line granularities, cross-page drag range generation (`pdf_desired_ranges`), unbind persistence (`pdf_drop_unselected_layer`), and modifier handling (`pdf_shortcut_modifiers`).
  7. `src/ui/preview.rs`: Integrated `gtk::DrawingArea` overlay on `gtk::Picture`, Cairo highlight rendering, drag selection via `gtk::GestureDrag`, hover cursor updates via `gtk::EventControllerMotion`, double/triple click detection, and standard copy keyboard shortcut (`Ctrl+C`) via `gdk::Display::default().clipboard().set_text(...)`.

---

### Exact Poppler FFI Implementation (`src/sandbox_helper.rs`)
In `src/sandbox_helper.rs`, `poppler-rs` does not expose a safe wrapper for `poppler_page_get_text_layout`. The implementation directly calls Poppler's C FFI:

```rust
// poppler-rs does not bind poppler_page_get_text_layout, so the glyph boxes come
// through FFI; the returned array is g_malloc'd and freed here.
#[expect(
    unsafe_code,
    reason = "poppler-rs exposes no safe binding for poppler_page_get_text_layout"
)]
fn pdf_text_layer(page: &poppler::Page, width: i32, height: i32, scale: f64) -> Option<Vec<u8>> {
    use glib::translate::ToGlibPtr;

    let text = page.text()?;
    if text.is_empty() {
        return None;
    }
    let mut rects = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: page is a valid PopplerPage; rects/count are valid out-pointers.
    let ok = unsafe {
        poppler::ffi::poppler_page_get_text_layout(page.to_glib_none().0, &mut rects, &mut count)
    };
    if ok == glib::ffi::GFALSE || rects.is_null() {
        return None;
    }
    // SAFETY: on success poppler returned a g_malloc'd array of count rectangles.
    let layout = unsafe { std::slice::from_raw_parts(rects, count as usize) };
    let glyphs: Vec<[f32; 4]> = layout
        .iter()
        .map(|rect| {
            [
                (rect.x1 * scale) as f32,
                (rect.y1 * scale) as f32,
                (rect.x2 * scale) as f32,
                (rect.y2 * scale) as f32,
            ]
        })
        .collect();
    // SAFETY: rects came from g_malloc and is freed exactly once here.
    unsafe { glib::ffi::g_free(rects.cast()) };
    if glyphs.len() != text.chars().count() {
        return None;
    }
    let layer = crate::services::PdfTextLayer {
        width: width as f32,
        height: height as f32,
        text: text.to_string(),
        glyphs,
    };
    serde_json::to_vec(&layer)
        .ok()
        .filter(|bytes| bytes.len() as u64 <= crate::sandbox::MAX_TEXT_LAYER_BYTES)
}
```

Key observations on FFI and safety rules:
- Crate root enforces `#![deny(unsafe_code)]` and strict clippy lints (`allow_attributes = "deny"`, `undocumented_unsafe_blocks = "deny"`, `multiple_unsafe_ops_per_block = "deny"`).
- `#[expect(unsafe_code, reason = "...")]` must be used instead of `#[allow]`.
- Every unsafe operation is isolated into its own `unsafe { ... }` block with an explicit `// SAFETY:` rationale comment.
- Poppler memory allocation: `poppler_page_get_text_layout` allocates `rectangles` with `g_malloc`; the caller must free it with `glib::ffi::g_free(rects.cast())`.
- Integrity check: `glyphs.len() == text.chars().count()` verifies that character indices match glyph bounding boxes 1:1.

---

### Data Structures & Wire Protocol Serialization

#### Data Structures (`src/services/preview.rs`)
```rust
/// Extracted text and per-character bounds for one rendered PDF page.
/// `glyphs[i]` locates the i-th char of `text` in rendered PNG pixels.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PdfTextLayer {
    pub width: f32,
    pub height: f32,
    pub text: String,
    pub glyphs: Vec<[f32; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PreviewContent {
    ...
    Pdf {
        png: Vec<u8>,
        page: i32,
        pages: i32,
        text_layer: Option<Arc<PdfTextLayer>>,
    },
    ...
}
```

#### IPC / Wire Protocol Interaction
Hermes supports two execution modes for sandboxed preview tasks:
1. **One-shot Sandbox (`parse_one_shot`)**:
   - Helper process executes `preview-pdf <page> <size>` inside Bubblewrap (`bwrap`).
   - Helper generates:
     - `result.png`: rendered PNG page surface.
     - `result.meta`: `"{page} {pages}"` string.
     - `result.text`: serialized JSON `PdfTextLayer`.
   - Host `sandbox.rs` reads `result.text` bounded by `MAX_TEXT_LAYER_BYTES` (8 MB) and parses JSON into `ParseOutput.text_layer`.
2. **Persistent Pooled Sandbox Worker (`src/sandbox/browser/`)**:
   - Request framing: 1-byte opcode (`Operation::Pdf = 3`).
   - Response framing: 8-byte header `[png_len: u32 LE, metadata_len: u32 LE]`.
   - When using persistent workers, `metadata` can carry `{ "page": i32, "pages": i32, "text_layer": Option<PdfTextLayer> }`. In current architecture, `map_preview_op(&ParseOperation::PreviewPdf)` returns `None`, so `PreviewPdf` routes directly to `parse_one_shot`, which already provides full text layer extraction.

---

### Text Segmentation Model (`src/ui/preview/pdf_text.rs`)
The text segmentation module (`pdf_text.rs`) defines the following core functions:

1. **`lines(layer: &PdfTextLayer) -> Vec<PdfLine>`**:
   - Segments `layer.text` on newline characters (`\n`).
   - For each line slice, calculates `top = min(rect[1])` and `bottom = max(rect[3])` across solid glyphs (`rect[2] > rect[0] && rect[3] > rect[1]`).
   - Empty lines inherit height from preceding lines to allow hit-testing on blank lines.

2. **`image_bounds(layer: &PdfTextLayer, width: f64, height: f64) -> (f64, f64, f64)`**:
   - Calculates `scale = (width / layer.width).min(height / layer.height)`.
   - Calculates letterbox offsets `(x, y) = ((width - layer.width * scale) / 2.0, (height - layer.height * scale) / 2.0)`.
   - Matches GTK's `gtk::Picture` layout under `ContentFit::Contain`.

3. **`hit_text(layer: &PdfTextLayer, x: f32, y: f32) -> bool`**:
   - Tests whether a point falls within a line's vertical bounds (`top - height*0.5 .. bottom + height*0.5`) and horizontal bounds (`x1 - height .. x2 + height`).
   - If user clicks in document margins or between paragraphs, `hit_text` returns `false`, allowing the gesture to remain a panning/scroll drag instead of selecting text.

4. **`caret_at(layer: &PdfTextLayer, x: f32, y: f32) -> usize`**:
   - Locates closest line vertically and snaps to the glyph whose midpoint is closest to `x`.

5. **`has_descender(layer: &PdfTextLayer, start: usize, end: usize) -> bool`**:
   - Identifies whether characters have descenders (`'g'`, `'j'`, `'p'`, `'q'`, `'y'`, `'Q'`, `','`, `';'`, `'('`, `')'`, `'['`, `']'`, `'{'`, `'}'`, `'_'`, or non-ASCII characters).
   - If false, the selection highlight band is trimmed to `top + (bottom - top) * 0.82` (82% height) to prevent bottom-heavy highlight rendering on Latin text without descenders.

6. **`selection_runs(layer: &PdfTextLayer, start: usize, end: usize) -> Vec<[f32; 4]>`**:
   - Merges selected glyphs on each line into contiguous rectangular highlight runs `[x1, top, x2, bottom]`.

7. **`word_range` and `line_range`**:
   - Expands selection outward to whitespace boundaries (for double-click) or line boundaries (for triple-click).

---

### UI Gesture Integration & Rendering (`src/ui/preview.rs`)

1. **Widget Structure in ListItem Factory**:
   ```
   gtk::Overlay
   ├── child: gtk::Picture (displays rendered page PNG)
   ├── overlay: gtk::DrawingArea (text selection overlay with custom draw_func)
   └── overlay: gtk::Spinner (loading indicator)
   ```

2. **Cairo Highlight Drawing**:
   - In `text_area.set_draw_func`:
     - Reads selection color from theme accent:
       ```rust
       fn pdf_selection_color() -> Option<gtk::gdk::RGBA> {
           crate::ui::theme::ThemeManager::shared()
               .current_tokens()
               .and_then(|tokens| gtk::gdk::RGBA::parse(&tokens.accent).ok())
       }
       ```
       *(Note: `ThemeManager::current_tokens` in `src/ui/theme.rs` currently requires `pub(crate)` visibility).*
     - Sets Cairo RGBA source with accent color at `alpha = 0.35`.
     - For each run in `selection_runs`, calculates scaled coordinates and invokes `rounded_rect(&cr, rx, ry, rw, rh, 2.0 * s)` followed by `cr.fill()`.

3. **Mouse Gestures (`gtk::GestureDrag`, `gtk::GestureClick`, `gtk::EventControllerMotion`)**:
   - `pan.connect_drag_begin`:
     - Calls `pdf_page_at(&scroll, &visible_pages, &text_layers, x, y)`.
     - If lands on text (`hit_text == true`):
       - Sets `pdf_drag = PdfDrag::Select`.
       - Calculates click count via timestamp/distance (`pdf_press` tracking): 1 = char, 2 = word, 3 = line.
       - If Shift is held: extends selection from existing anchor.
       - Else: sets anchor to `caret_at(x, y)`.
       - Updates ranges via `pdf_desired_ranges` and applies dirty redraws via `pdf_apply_ranges`.
     - If lands outside text (margins):
       - Sets `pdf_drag = PdfDrag::Pan`.
       - Sets cursor to `"grabbing"` and records `drag_origin` for scrolling.
   - `pan.connect_drag_update`:
     - If `PdfDrag::Select`: finds nearest page via `pdf_page_near`, recalculates caret, updates ranges across multiple pages.
     - If `PdfDrag::Pan`: updates `scroll.hadjustment()` and `scroll.vadjustment()`.
   - `pan.connect_drag_end`:
     - Resets cursor and transitions `pdf_drag` to `PdfDrag::Idle`.
   - `EventControllerMotion`:
     - When idle: updates cursor to `"text"` (I-beam) when hovering over glyph ink, or `"grab"` when hovering margins.

4. **Keyboard Controller (`gtk::EventControllerKey`)**:
   - Attached to the list/scroll widget with `PropagationPhase::Capture`.
   - `pdf_shortcut_modifiers(modifiers)`:
     - Allows `CONTROL_MASK` with or without `LOCK_MASK` (Caps Lock / Num Lock support).
     - Explicitly rejects `SHIFT_MASK` and `ALT_MASK`.
   - **Ctrl+C**: Extracts combined text across all selected pages using `pdf_selected_text(&text_layers, &pdf_ranges)`, and copies it to GTK4 clipboard via `gdk::Display::default().clipboard().set_text(&text)`.
   - **Ctrl+A**: Selects all text across all loaded pages.
   - **Escape**: Clears current selection.

5. **Scroll & Unbind Memory Management**:
   - When pages scroll out of view and are unbound in `gtk::ListView`:
     - `pdf_drop_unselected_layer`: If the unbound page is NOT part of an active selection, its `PdfTextLayer` is freed.
     - If the unbound page IS part of an active selection, its layer is retained in memory so `Ctrl+C` can copy across scrolled pages.
     - Stale unbound layers are discarded as soon as the selection changes.

---

### Critical Constraint Verification: No Modal / Vim Navigation Modes
- **Constraint Source**: `ORIGINAL_REQUEST.md §R3` ("excluding Vim/modal keyboard modes"), `PROJECT.md §Interface Contracts §Rich Format Contracts` ("No modal/Vim navigation mode"), and user dispatch ("CRITICAL CONSTRAINT: DO NOT implement modal/Vim keyboard navigation modes").
- **Verification**:
  - The implementation uses standard desktop GUI interaction:
    - Mouse click & drag selection.
    - Double-click word, triple-click line.
    - Shift-click range extension.
    - Standard `Ctrl+C` copy, `Ctrl+A` select all, `Escape` deselect.
  - Zero modal state machines (`Mode::Normal`, `Mode::Visual`, etc.).
  - Zero single-letter Vim navigation bindings (`v`, `y`, `h`, `j`, `k`, `l`, `g`, `G`, `/`, etc.).

---

## 2. Logic Chain

1. **Why Poppler C FFI is Required**:
   - `poppler-rs` crate version 0.25 wraps Poppler glib objects (`poppler::Page`, `poppler::Document`), but omits `poppler_page_get_text_layout`.
   - `poppler::ffi::poppler_page_get_text_layout` exists in the underlying `poppler-sys` bindings.
   - Calling this FFI directly allows extracting per-glyph bounding boxes (`x1, y1, x2, y2`) in document points without spawning external tools or third-party binaries.

2. **Why Glyph Bounding Boxes Must be Scaled by Render Factor**:
   - Poppler points are fixed at 72 DPI.
   - `render_pdf_surface` renders the page to a Cairo image surface of size `width x height` with a computed `scale` factor.
   - Multiplying each `rect` coordinate by `scale` places glyph bounding boxes precisely into pixel coordinates matching the rendered PNG image.

3. **Why Margin Press Detection is Essential for UX**:
   - A PDF viewer in a file manager preview drawer must support both panning (scrolling) and text selection.
   - If any mouse press anywhere on the page starts a text selection, panning becomes impossible.
   - `pdf_text::hit_text` uses glyph bounding boxes plus line height padding to determine whether a click hit actual text ink. Clicks outside text ink transition to `PdfDrag::Pan`, preserving smooth mouse panning.

4. **Why Descender Trimming Improves Visual Polish**:
   - Font bounding boxes span full ascent and descent heights.
   - On lines with only uppercase letters or numbers (no descenders), highlighting the full font box creates an asymmetrical space below the text that appears visually unbalanced.
   - Checking `has_descender` and trimming height to `0.82` creates a clean, tight highlight band identical to native macOS Preview.app.

5. **Why Caps Lock / Lock Modifiers Must be Filtered**:
   - On X11 and Wayland, `gtk::gdk::ModifierType` includes `LOCK_MASK` when Caps Lock is active.
   - Checking `modifiers == ModifierType::CONTROL_MASK` fails when Caps Lock is enabled.
   - `pdf_shortcut_modifiers` explicitly checks `modifiers.contains(CONTROL_MASK) && !modifiers.intersects(SHIFT_MASK | ALT_MASK)`, guaranteeing `Ctrl+C` and `Ctrl+A` work reliably under all keyboard locking states.

6. **Why Virtualized Unbound Pages Must Retain Selected Layers**:
   - In GTK4 `ListView`, rows that scroll out of viewport are recycled and unbound.
   - If unbound pages immediately drop their `PdfTextLayer`, any multi-page selection spanning beyond the viewport loses its text data, causing `Ctrl+C` to copy incomplete text.
   - Keeping selected pages in `text_layers` until selection changes solves this virtualization issue without leaking memory.

---

## 3. Caveats

1. **`ThemeManager::current_tokens` Visibility**:
   - In `src/ui/theme.rs`, line 414: `fn current_tokens(&self) -> Option<ThemeTokens>` is currently private.
   - To invoke `crate::ui::theme::ThemeManager::shared().current_tokens()` from `src/ui/preview.rs`, it must be declared `pub(crate) fn current_tokens(&self) -> Option<ThemeTokens>`.
   - Alternatively, `pdf_selection_color` can fall back to a standard accent color (`#3584e4`) if `current_tokens` is unavailable.

2. **Poppler Ligature & Glyph Count Discrepancies**:
   - In rare PDFs with complex ligature font encodings, Poppler's `poppler_page_get_text_layout` rectangle count can diverge from `page.text()?.chars().count()`.
   - When this happens, `pdf_text_layer` safely returns `None`, allowing the visual page to render normally while disabling text selection on that single malformed page.

3. **Scanned Documents (Images without Text)**:
   - For scanned PDFs lacking an embedded text stream, `page.text()` returns an empty string. `pdf_text_layer` returns `None`, and the preview pane displays the rendered raster with normal pan/zoom gestures.

---

## 4. Conclusion

Milestone M4.3 architecture is thoroughly designed, proven by reference commit `e816f85b`, and fully aligned with Hermes coding standards and project constraints:

1. **Sandbox Helper & Poppler FFI**:
   - Add `pdf_text_layer` to `src/sandbox_helper.rs` with `#[expect(unsafe_code, reason = "poppler-rs exposes no safe binding for poppler_page_get_text_layout")]`.
   - Each unsafe block isolated with explicit `// SAFETY:` rationale.
   - Output serialized `PdfTextLayer` to `result.text`.
2. **IPC & Wire Protocol**:
   - Define `PdfTextLayer` in `src/services/preview.rs` (deriving `serde::Serialize, serde::Deserialize`).
   - Add `text_layer: Option<Arc<PdfTextLayer>>` to `PreviewContent::Pdf`.
   - Update `ParseOutput` in `src/sandbox.rs` with `MAX_TEXT_LAYER_BYTES = 8MB` guardrail.
   - Update `src/adapters/local_preview.rs` to propagate `text_layer` and account for layer memory in cache size calculation.
3. **Segmentation Model**:
   - Create `src/ui/preview/pdf_text.rs` and `src/ui/preview/pdf_text/tests.rs` implementing `lines`, `image_bounds`, `hit_text`, `caret_at`, `has_descender`, `selection_runs`, `word_range`, `line_range`, and `selection_text`.
4. **UI Preview Drawer & Cairo Rendering**:
   - In `src/ui/preview.rs`, overlay `gtk::DrawingArea` onto `gtk::Picture`.
   - Implement Cairo drawing for rounded highlight boxes using theme accent.
   - Wire `GestureDrag` with hit-testing (pan vs select), `GestureClick` for multi-click word/line selection, `EventControllerMotion` for I-beam cursor, and `EventControllerKey` for `Ctrl+C` clipboard copy via `gdk::Display::default().clipboard().set_text(...)`.
   - Manage virtualized scrolling by retaining selected unbound layers and discarding unselected unbound layers.
5. **Vim Constraint Compliance**:
   - Strictly no Vim/modal navigation modes. Standard mouse drag selection and desktop keyboard shortcuts only.

---

## 5. Verification Method

### Step-by-Step Verification Commands
Once implemented by the worker agent:

1. **Verify Unit Tests**:
   ```bash
   cargo test --lib ui::preview::pdf_text
   cargo test --lib ui::preview::pdf_ranges_tests
   ```
   Ensures all line segmentation, caret snapping, hit testing, multi-page range generation, and descender trimming tests pass.

2. **Verify Clippy & Strict Lint Compliance**:
   ```bash
   cargo clippy --all-targets -- -D warnings
   ```
   Ensures no clippy warnings, no undocumented unsafe blocks, and proper `#[expect(unsafe_code, reason = "...")]` usage.

3. **Verify Baseline Regression Suite**:
   ```bash
   cargo test --lib
   ```
   Ensures all 181 existing tests continue to pass without regression.

4. **Verify E2E PDF Tests**:
   ```bash
   cargo test --test e2e -- test_f12
   ```
   Runs E2E test scenarios validating PDF text extraction, page preview rendering, thumbnail generation, and bounding-box intersection.

5. **Runtime Interactive Invalidation Conditions**:
   - Dragging across text must draw accent highlight rectangles over glyphs.
   - Dragging in page margins must pan/scroll the document.
   - Double-clicking text must select the whole word; triple-clicking must select the whole line.
   - Pressing `Ctrl+C` while text is selected must copy text to the system clipboard (pasteable into other applications).
   - Pressing `Escape` must clear the active selection.
   - Scrolling long PDFs must preserve selected text across offscreen pages.
