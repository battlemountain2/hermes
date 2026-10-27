# Hermes identity and app icon

The approved mark is concept B: a flat H with one rising messenger-wing crossbar, no app tile,
glow, bevel, gradient or decorative frame. The SVG is the source of truth. The muted-teal normal
icon is a fixed brand asset, not a hardcoded interface color. The symbolic variant can follow
desktop theme colors. Existing Lucide action icons and their attribution are unchanged.

## Application identity

- Package, executable and release archive prefix: `hermes`.
- Stable application/desktop/icon ID: `io.github.battlemountain2.Hermes`.
- Development application ID: `io.github.battlemountain2.Hermes.Dev`.
- A hidden development desktop entry matches Dev windows to the same icon without adding a
  duplicate app-menu item or registering a second folder handler.
- `HERMES_DEV` selects development mode; `STRATA_DEV` remains a compatibility alias.
- The main window title and application name are Hermes. MPRIS advertises the Hermes desktop entry.
- Build metadata uses `HERMES_BUILD_COMMIT`, accepting `STRATA_BUILD_COMMIT` as an input alias.

The app icon is embedded in a dedicated Hermes resource path. Legacy `strata-` Lucide names,
internal helper mount paths and protocol markers are not blindly renamed. Upstream Strata credits
remain intentionally visible. A separate application ID prevents accidental window/activation
identity collisions; it does not by itself isolate saved settings.

## Saved-data compatibility

This increment does not relocate or rewrite preferences, custom themes, queues, playlists, ratings
or search databases. Several internal storage directories remain named `strata`. Do not delete
those folders during cleanup, or assume Hermes and upstream Strata can safely share them. A later
migration must identify existing Hermes data, back it up, preserve upstream installations and
handle interruption before changing the storage namespace. No running window is restarted.

## Icon assets

`data/icons/scalable/apps/io.github.battlemountain2.Hermes.svg` is the approved mark.
`data/icons/symbolic/apps/io.github.battlemountain2.Hermes-symbolic.svg` has identical geometry.
PNG app icons are generated at16/24/32/48/64/128/256/512/1024px, directly from the SVG rather than
repeated raster resizing. Regenerate with `python3 scripts/generate-app-icons.py` and `rsvg-convert`.

`packaging/macos/Hermes.iconset` contains standard and Retina PNG sizes; `Hermes.icns` contains
the matching PNG representations and table of contents. These are icon assets only, not a working
macOS application bundle or evidence that Hermes builds/runs on macOS.

## User-local Linux installation

After `cargo build --release`, run `python3 scripts/install-local.py`. Use `--replace` to explicitly
update existing regular Hermes files. The installer rejects symlink/non-regular destinations and
does not overwrite an unrelated installation without that flag. It installs only the Hermes
binary, launcher and hicolor icon files; it never runs sudo, launches the GUI, installs a D-Bus
service, changes MIME defaults, edits portal settings or removes upstream Strata.

`--prefix /absolute/test/location --binary /absolute/path/to/hermes` supports isolated validation.
Without `--prefix`, the binary goes to `~/.local/bin/hermes` and assets follow `XDG_DATA_HOME`.
The installed launcher's Exec uses the absolute binary path with desktop-entry escaping, including
spaces and literal percent signs. Ambiguous control/quote/backslash/dollar/backtick paths are
rejected before installation. The template does not claim unsupported D-Bus activation.

Linux folder/reveal integration now has a separate, reversible opt-in setup; the normal installer
still never activates it. See [desktop integration](desktop-integration.md). The system Open/Save
picker has a separate experimental, off-by-default backend and setup, with real-desktop acceptance
still pending. macOS platform adapters and
Asahi ARM64 device testing also remain future work, not implemented by this rename.

## Validation

`python3 scripts/test_app_identity.py` checks source geometry, PNG alpha/dimensions, ICNS records,
isolated installation, launcher syntax, escaping and refusal to replace symlinks/existing files.
Adjacent Rust identity tests check IDs, launchers and resources. The ignored private-GTK icon test
checks normal/symbolic lookup at small sizes without installing desktop metadata or touching the
user's display. Whole-app packaging and physical launcher/dock appearance remain separate checks.
