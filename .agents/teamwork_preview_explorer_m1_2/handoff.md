# Handoff Report: Window Event Wiring (`src/ui/window.rs`)

**Agent**: Explorer M1.2 (Window Event Wiring Explorer)  
**Milestone**: Milestone 1 (Status Bar Completion & Warning Elimination)  
**Project Root**: `/home/bry/.gemini/antigravity/scratch/hermes`  

---

## 1. Observation

### 1.1 `StatusBar` Instantiation & Window Layout in `src/ui/window.rs`
- **Instantiation** (`src/ui/window.rs:174-176`):
  ```rust
  let status_bar = Rc::new(super::status_bar::StatusBar::new());
  let context_status_bar = status_bar.clone();
  let context_controller = controller.clone();
  ```
- **Layout Placement** (`src/ui/window.rs:288-293`):
  ```rust
  let browser_vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
  browser_vbox.append(&browser.widget());
  browser_vbox.append(&status_bar.container);
  
  tree_paned.set_end_child(Some(&browser_vbox));
  ```
  `StatusBar` is packed into `browser_vbox` directly underneath the browser widget.
- **Unused Builder in `src/ui/status_bar.rs:86-88`**:
  ```rust
  pub fn build_status_bar() -> StatusBar {
      StatusBar::new()
  }
  ```
  Because `window.rs:174` calls `StatusBar::new()` directly, `build_status_bar` triggers `warning: function build_status_bar is never used`.

### 1.2 Current Event Observer in `src/ui/window.rs:177-219`
The observer currently has two event-handling blocks:
```rust
controller.observe(move |event| {
    if matches!(
        event,
        BrowserEvent::ColumnAdded { .. }
            | BrowserEvent::ColumnsTruncated { .. }
            | BrowserEvent::SortingFinished { .. }
            | BrowserEvent::PreviewRequested { .. }
            | BrowserEvent::FocusChanged { .. }
    ) {
        if let Err(error) = context_trails.update_active_view(context_view()) {
            tracing::warn!(%error, "unable to save Trail view state");
        }
        context_tab_bar.refresh();
    }
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
});
```

### 1.3 Missing Free Space Calls and Compiler Warnings
Running `cargo check` outputs 4 warnings directly tied to `window.rs` and `status_bar.rs`:
```text
warning: unused import: `FileEntry`
  --> src/ui/window.rs:17:20
   |
17 |         EntryKind, FileEntry, Location, MetadataValue, Trail, TrailBrowserDensity,
   |                    ^^^^^^^^^

warning: field `free_space` is never read
  --> src/ui/status_bar.rs:10:5
   |
10 |     free_space: gtk::Label,
   |     ^^^^^^^^^^

warning: method `update_free_space` is never used
  --> src/ui/status_bar.rs:63:12
   |
63 |     pub fn update_free_space(&self, path: &Path) {
   |            ^^^^^^^^^^^^^^^^^

warning: function `build_status_bar` is never used
  --> src/ui/status_bar.rs:86:8
   |
86 | pub fn build_status_bar() -> StatusBar {
   |        ^^^^^^^^^^^^^^^^
```
- In `src/ui/window.rs:17`, `FileEntry` is imported but at line 361 the code uses `crate::model::FileEntry(...)` fully-qualified, leaving the import unused.
- `context_status_bar.update_free_space` is **never called anywhere in `window.rs`** (nor anywhere else in the codebase).
- `free_space` label in `src/ui/status_bar.rs:10` is constructed, added to the UI, but never updated.

### 1.4 Active Location and Navigation Types
- `src/app/browser.rs:219-221`:
  ```rust
  pub fn active_location(&self) -> Option<Location> {
      self.state.borrow().active_location()
  }
  ```
- `src/app/navigation.rs:437-440`:
  ```rust
  pub fn active_location(&self) -> Option<Location> {
      let depth = self.active_column?;
      Some(self.columns.get(depth)?.location.clone())
  }
  ```
- `src/model/mod.rs:60-65`:
  ```rust
  pub fn native_path(&self) -> Option<&std::path::Path> {
      match &self.kind {
          LocationKind::Native(path) => Some(path),
          LocationKind::Uri(_) => None,
      }
  }
  ```
- `src/ui/status_bar.rs:63-65`:
  ```rust
  pub fn update_free_space(&self, path: &Path) {
      let file = gio::File::for_path(path);
      ...
  ```

---

## 2. Logic Chain

### 2.1 Active Location Filesystem Path Resolution
1. `context_controller.active_location()` returns `Option<Location>`.
2. When the browser has an active column (`self.active_column`), `Location` corresponds to the directory of that active column.
3. `Location::native_path(&self)` returns `Option<&Path>`. If the location is a local filesystem path (`LocationKind::Native`), it borrows `&Path`.
4. Therefore, the canonical extraction chain is:
   ```rust
   if let Some(path) = context_controller.active_location().as_ref().and_then(|l| l.native_path()) {
       context_status_bar.update_free_space(path);
   }
   ```
5. **Virtual / Non-Native Locations (e.g. `trash:///`)**:
   When navigating to a URI location like `trash:///`, `native_path()` returns `None`. To avoid displaying stale free space from the previous folder, `StatusBar` should clear the label (`set_text("")`) when `native_path()` is `None`. Adding `clear_free_space(&self)` to `StatusBar` or allowing `update_free_space(&self, path: Option<&Path>)` handles this edge case cleanly.

### 2.2 Complete Browser Event Matrix for `update_free_space`
The status bar's free disk space must stay synchronized across three categories of events:

| Event | Category | Trigger / Origin | Why `update_free_space` is Required |
|---|---|---|---|
| **Startup / Window Setup** | Initial | `window.rs:489` (`browser.navigate(location)`) | Populates initial free space when the window opens. `navigate` emits `Reset`, `ColumnAdded`, and `FocusChanged`, so the observer catches it automatically. |
| **`BrowserEvent::ColumnAdded`** | Navigation | Opening a directory in Miller columns or new path navigation | New directory column may reside on a separate mount point / filesystem. Active path changes to the new column. |
| **`BrowserEvent::ColumnsTruncated`** | Navigation | Navigating left / closing child columns (`escape`, `close_column`) | Truncates child columns; active depth falls back to a parent or shallower directory, potentially on a different mount point. *(Currently missing from Block 2 in `window.rs`)*. |
| **`BrowserEvent::ColumnReloaded`** | Navigation / Sync | User presses Reload (F5) or `DirectoryChange::Rescan` fires | External processes may have modified disk capacity; explicit refresh must update free space. *(Currently missing from Block 2 in `window.rs`)*. |
| **`BrowserEvent::Reset`** | Navigation | Fresh navigation begins via sidebar, tab switch, breadcrumb, or search | Column stack resets before new directory loads. Sets baseline status. *(Currently missing from Block 2 in `window.rs`)*. |
| **`BrowserEvent::FocusChanged`** | Navigation | Cursor / focus switches columns in Miller columns view | Shifting column focus changes `active_depth()`, which may shift between distinct filesystem mounts (e.g. `/` vs `/mnt/storage`). |
| **`BrowserEvent::EntriesInserted`** | File Modification | Directory enumeration batches arrive from disk | Initial enumeration of directory contents; confirms active directory contents and verifies capacity. |
| **`BrowserEvent::EntriesReplaced`** | File Modification | Sorting finishes or batch replacement occurs | Directory entries are refreshed in sorted order. |
| **`BrowserEvent::EntriesSpliced`** | File Modification | `gio::FileMonitor` detects create/delete/rename in watched directory | **Key real-time trigger:** When a file is created or deleted on disk, `EntriesSpliced` fires. Calling `update_free_space(path)` immediately reflects the reclaimed or consumed disk capacity! |

### 2.3 Why `SelectionSetChanged` Should NOT Call `update_free_space`
- `BrowserEvent::SelectionSetChanged { depth, positions, focused }` is emitted whenever the user modifies the selection (e.g., arrow keys with Shift, Ctrl+A, rubberband dragging).
- Changing selection modifies `selected_items` and `selected_bytes`, but has zero effect on disk capacity.
- Bypassing `update_free_space` during `SelectionSetChanged` prevents spawning redundant GIO async filesystem queries on every keystroke or mouse drag.

### 2.4 Synchronization of Multi-Selection Byte Totals, Item Counts, and Free Space
1. **Directory Item Count**:
   `context_controller.column_entries(depth)` returns the full vector of entries in the active column.
   `total_items = entries.len()` is passed to `context_status_bar.update_item_count(total_items)`.
   Displays `"0 items"`, `"1 item"`, or `"{count} items"`.
2. **Multi-Selection Aggregate Bytes**:
   `context_controller.selected_entries()` returns all entries currently selected in the active column.
   Iterating over selected items sums `size` only when `entry.size` is `MetadataValue::Known(size)` (ignoring directories or pending files where size is `Unknown`).
   `context_status_bar.update_selection(selected_items, selected_bytes)` formats the label:
   - If `count == 0` -> empty string `""`.
   - If `count > 0` -> `"{count} selected, {format_file_size(total_bytes)}"`.
3. **Free Space**:
   `update_free_space(path)` queries GIO asynchronously (`query_filesystem_info_future("filesystem::free")`).
   Formatted as `"{format_file_size(free_bytes)} free"`.
4. **Layout Alignment**:
   `StatusBar::new()` places `item_count` and `selection_info` together in a start-aligned horizontal box, and `free_space` right-aligned with `hexpand: true`.
   All three labels update synchronously and never collide or overlap.

---

## 3. Caveats

1. **Non-Native Locations (URIs)**:
   When `context_controller.active_location()` is a URI scheme without a local mount point (e.g., `trash:///`), `native_path()` returns `None`. If `StatusBar` does not clear `free_space`, the label might retain the previous folder's free space string. Adding `clear_free_space(&self)` to `StatusBar` ensures the label is blanked when `native_path()` is `None`.
2. **Asynchronous GIO Query Ordering**:
   `query_filesystem_info_future` spawns onto `glib::MainContext::default()`. Because GIO filesystem stat queries complete in sub-millisecond time on local disks, race conditions between rapid folder changes are negligible, but if the user rapidly switches mounts, the label reflects the latest resolved future.
3. **Compiler Warnings in `window.rs`**:
   The unused import `FileEntry` in `src/ui/window.rs:17` can be resolved either by removing `FileEntry` from line 17 or by using `FileEntry(...)` instead of `crate::model::FileEntry(...)` at line 361.
4. **`build_status_bar` vs `StatusBar::new`**:
   `build_status_bar` in `src/ui/status_bar.rs:86` should either be called at `src/ui/window.rs:174` (`let status_bar = Rc::new(super::status_bar::build_status_bar());`) or removed from `status_bar.rs` to eliminate the unused function warning.

---

## 4. Conclusion & Proposed Implementation

### 4.1 Recommended Changes to `src/ui/window.rs`

#### Edit 1: Fix Unused Import `FileEntry` (`src/ui/window.rs:17`)
Change lines 16-20 from:
```rust
    model::{
        EntryKind, FileEntry, Location, MetadataValue, Trail, TrailBrowserDensity,
        TrailBrowserMode, TrailViewState,
    },
```
to use `FileEntry` at line 361:
```rust
// In search activate callback (line 361):
search_preview.show(FileEntry(std::rc::Rc::new(crate::model::FileEntryInner { location,
```
*(Or alternatively remove `FileEntry` from line 17).*

#### Edit 2: Use `build_status_bar()` (`src/ui/window.rs:174`)
Change line 174 from:
```rust
    let status_bar = Rc::new(super::status_bar::StatusBar::new());
```
to:
```rust
    let status_bar = Rc::new(super::status_bar::build_status_bar());
```
*(This eliminates the compiler warning on `build_status_bar`).*

#### Edit 3: Comprehensive Observer Wiring (`src/ui/window.rs:191-218`)
Expand the event matching to include `ColumnsTruncated`, `ColumnReloaded`, and `Reset`, and call `update_free_space`:
```rust
        if matches!(
            event,
            BrowserEvent::EntriesInserted { .. }
                | BrowserEvent::EntriesReplaced { .. }
                | BrowserEvent::EntriesSpliced { .. }
                | BrowserEvent::SelectionSetChanged { .. }
                | BrowserEvent::FocusChanged { .. }
                | BrowserEvent::ColumnAdded { .. }
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::ColumnReloaded { .. }
                | BrowserEvent::Reset
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

            if !matches!(event, BrowserEvent::SelectionSetChanged { .. }) {
                if let Some(path) = context_controller
                    .active_location()
                    .as_ref()
                    .and_then(|l| l.native_path())
                {
                    context_status_bar.update_free_space(path);
                } else {
                    context_status_bar.clear_free_space();
                }
            }
        }
```

### 4.2 Recommended Addition to `src/ui/status_bar.rs`
Add `clear_free_space` method:
```rust
impl StatusBar {
    ...
    pub fn clear_free_space(&self) {
        self.free_space.set_text("");
    }
}
```

---

## 5. Verification Method

1. **Verify Compiler Warnings Elimination**:
   ```bash
   cargo check
   ```
   **Expected Outcome**:
   - `warning: field free_space is never read` -> **ELIMINATED**
   - `warning: method update_free_space is never used` -> **ELIMINATED**
   - `warning: function build_status_bar is never used` -> **ELIMINATED**
   - `warning: unused import: FileEntry` -> **ELIMINATED**

2. **Verify Regression Baseline**:
   ```bash
   cargo test
   ```
   **Expected Outcome**: All 181 unit tests pass (`test result: ok. 181 passed; 0 failed`).

3. **Behavioral Invalidation Conditions**:
   - If navigating between columns on different disks does not update the free space string, verify `FocusChanged` is included in the observer matches.
   - If deleting a file does not refresh free space, verify `EntriesSpliced` calls `update_free_space`.
   - If selection drag causes UI stutter, verify `SelectionSetChanged` is excluded from calling `update_free_space`.
