# Themes

Hermes styles the interface with nine semantic color tokens. Bundled themes are the fallback on any Linux desktop; Azure Glow is the default.

## Follow Noctalia

Choose **Settings → Theme & appearance → Follow Noctalia** to opt in. Hermes never
auto-enables this option or modifies Noctalia's configuration, templates, or GTK
settings. Built-in/custom themes and Follow Omarchy remain available; selecting
one turns off the other followers without forgetting your selected theme.

This integration uses the **Noctalia v5 GTK 4 export**, verified against the
installed Noctalia 5.2.1 template, rather than assuming a legacy Quickshell
`colors.json` file. Noctalia's `config.toml` selects a palette source; custom
palettes live under `noctalia/palettes/` and can contain dark/light variants.
Hermes reads the already-resolved active colors from:

```text
$XDG_CONFIG_HOME/gtk-4.0/noctalia.css
```

`XDG_CONFIG_HOME` defaults to `~/.config`. If this export is missing, enable **GTK 4**
under **Noctalia Settings → Templates** yourself. Hermes does not run its hooks or
enable templates for you. This also works for wallpaper-generated, built-in, and
community palettes because Noctalia performs the source/mode resolution.

The installed template exports `surface` as `window_bg_color`, `surface_container`
as `card_bg_color`, `on_surface` as `window_fg_color`, `primary` as
`accent_bg_color`, and `error` as `destructive_bg_color`. These become Hermes's
background, surface, text, accent, and danger tokens. Muted, highlight, border,
and dim text are derived from those surfaces/text/accent with the same blends in
both light and dark modes. Icons and syntax previews use the existing live-theme
pipeline, not a separate static stylesheet.
Waveform drawing reads the last applied accent/muted colors through
`ThemeManager::waveform_colors()`, including custom-theme editor previews, without
deprecated GTK style-context color lookup.

Hermes only extracts color declarations; it never loads this external CSS as
executable styling or follows imports. All declarations must have unique names
and resolve to opaque three- or six-digit hex colors. Missing required roles,
broken/cyclic aliases, unfinished declarations/comments, invalid UTF-8, and files
larger than 64 KiB or containing more than 128 color declarations reject the entire
palette update. Unknown non-color stylesheet rules are ignored.

Directory monitoring handles in-place writes, atomic file replacement, removal,
and recreation. Changes settle for 150 ms before applying a complete validated
palette; a two-second reconciliation also recovers missed events/unavailable
monitors. Invalid/missing exports keep the last valid palette **for this session**.
On startup without a valid export, the remembered built-in/custom theme supplies
the fallback while Follow Noctalia stays selected and waits for a valid export.
Theme-editor previews are not overwritten by background updates. Stale exported
colors remain stale if Noctalia stops updating its GTK template; Hermes does not
guess colors from wallpapers or run a second palette generator.

Primary references: [Noctalia palette format](https://docs.noctalia.dev/noctalia/theming/palette/),
[GTK template setup](https://docs.noctalia.dev/noctalia/templates/official/gtk-qt/),
and the [v5.2.1 GTK 4 template](https://github.com/noctalia-dev/noctalia/blob/v5.2.1/assets/templates/gtk/gtk4.css).

## Custom theme files

Custom themes are TOML files in:

```text
~/.config/strata/themes/<theme-id>.toml
```

The settings configurator writes the same format, so generated themes can be edited or shared:

```toml
name = "Ocean Blue"
background = "#0c1a2b"
surface = "#122438"
text = "#c9deed"
accent = "#4fd6ff"
danger = "#e5484d"
muted = "#1e3a52"
highlight = "#244d68"
border = "#315b75"
dim_text = "#6f8da3"
```

Strata discovers valid `.toml` files in this directory on startup and displays them under **Your themes**.

## Omarchy Quattro

On Omarchy Quattro, Strata detects the active theme from:

```text
~/.local/state/omarchy/current/theme.name
~/.local/state/omarchy/current/theme/colors.toml
```

The application maps Quattro's `background`, `foreground`, `accent`, `selection`, and `color8` values into its semantic tokens and monitors the current-theme state for changes. It defaults to following Omarchy on first launch.

The system option is not shown when a valid Quattro current-theme state is unavailable. Legacy Omarchy theme layouts and alacritty-based color extraction are intentionally unsupported.
