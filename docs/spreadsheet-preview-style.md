# Spreadsheet preview styling

The read-only XLSX/ODS table uses the same live interface font and semantic theme
tokens as Hermes. It has no independent font preference, fixed monospace family,
music-title treatment, dimension pill, card shadow, rounded controls or sheet fade.
All rules are confined to `.spreadsheet-*` selectors in `src/style.css`.

The preview drawer additionally offers square, semantic Layout/Data controls. Layout is a separate
read-only printed-document reader; Data is this sortable table. Controls remain present while
loading a workbook so switching to Data cancels a slow layout, and a failed conversion falls back
to Data explicitly. Layout may omit charts/active objects under the unchanged protected profile.

## Layout and states

- Both Data and Layout use persistent, non-overlay horizontal/vertical scrollbars. Grid/Layout
  handles have10px cross-axis thickness and32px minimum length, with semantic hover/active colors.
  Layout leaves pointer input over the bars to GTK instead of entering pan or text selection.
- Data builds only the first sheet table initially. Tabs build the requested sheet once and retain
  it for subsequent visits; the non-homogeneous stack avoids measuring every hidden sheet. Sheet
  data is shared between tab callbacks instead of copied into each tab. Parsing limits are unchanged.
- A two-line metadata block keeps filename and dimensions together beside the
  square fullscreen button. Both labels ellipsize independently; tooltips preserve
  their full values. Dimensions and sheet position still update on tab activation.
- Sheet names ellipsize at 20 characters of natural width, with complete tooltips.
  The horizontal tab scroller contains wide/many sheets rather than expanding the
  preview pane. Keyboard focus can reach off-screen tabs through GTK scrolling.
- Tabs keep identical border widths and font weight in every state. The current
  sheet has an accent underline; hover, pressed and keyboard-focus states use
  semantic tokens without layout shifts or transitions.
- Column headings and compact cells inherit the interface font. Grid lines target
  GTK's actual cell borders and listview row separators. Headers, rows and square
  scrollbars have semantic hover/pressed/focus styling; defensive selected-row
  styling uses `@theme_on_accent` on `@theme_accent`.
- Truncation copy wraps inside the pane. Sheet/grid scrolling disables kinetic
  motion. GTK stack transitions are disabled regardless of global motion settings.

The existing `NoSelection` model, bounded data, per-column sorting/resizing and
fullscreen reparent/restore/cleanup logic are unchanged. Selected-row rules do not
enable selection or turn the preview into an editor. Chart rendering, workbook
formatting, formula calculation and conversion are outside this change.

## Validation and remaining checks

Adjacent tests in `src/ui/table_view/tests.rs` include a display-free stylesheet
contract and an optional bounded GTK fixture with long titles, 16 long sheet names,
large dimensions, wrapped truncation text, two live palette/font sizes (13/22px),
240px minimum-width checks, fullscreen-action bounds, tab changes and numeric sort
preservation. The existing repeated-fullscreen lifecycle test follows the new
metadata/scroller hierarchy.

Coordinated validation passed the regular table tests and both exact private-GTK
cases: narrow geometry, live Sans/Monospace changes at 13/22px, theme colors,
sheet changes, numeric sorting and repeated fullscreen restoration. The first live
font check observed the title before newly realized cells had processed GTK's
style update; the regression now waits for two frame ticks and verifies both the
widget and Pango-layout contexts of every mapped label. No font assertions were
removed. No Hermes/user window, audio or preferences were touched.

For the parent's coordinated run, use `cargo test ui::table_view::tests::` for the
regular tests. On a private isolated GTK display only, run the two ignored
`desktop_spreadsheet_` tests individually with `--ignored --exact --test-threads=1`.
Physical compositor appearance, keyboard scrolling and hover/focus feel still
need a later user-controlled review; a private Broadway test is not that sign-off.

GTK node references: [ColumnView CSS nodes](https://docs.gtk.org/gtk4/class.ColumnView.html#css-nodes)
and [GTK 4.12 theme separator rules](https://github.com/GNOME/gtk/blob/4.12.5/gtk/theme/Default/_common.scss).
