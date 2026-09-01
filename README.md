# Strata

**Navigate every layer.**

Strata is an experimental, keyboard-first file manager for Linux. It is designed primarily for Omarchy while remaining portable to other modern Linux environments.

## Browser modes

### List

![Strata List mode showing Miller-column navigation, the places sidebar, and a source-code preview](docs/assets/strata-screenshot.png)

### Explorer

![Strata Explorer mode showing a detailed, sortable file listing](docs/assets/strata-explorer.png)

### Grid

![Strata Grid mode showing files and folders as icon tiles](docs/assets/strata-grid.png)

## Vision

- Miller-column navigation
- Folder peeking on hover
- Ultra-fast search
- Rich file previews
- Collapsible sidebar
- Compact and airy density modes
- List, Explorer, and Grid views
- Persistent tabs with independent browsing context
- Omarchy and system theming
- Complete keyboard navigation

## Documentation

- [Product requirements](docs/prd.md) — product specification
- [Roadmap](docs/roadmap.md) — milestone sequence and exit criteria
- [Work breakdown](docs/todo.md) — actionable project checklist
- [Architecture principles](docs/architecture.md) — boundaries and customization strategy
- [Prototype design reference](docs/design-reference.md) — visual tokens, motion, and interaction baseline
- [Themes](docs/themes.md) — custom theme schema and Omarchy Quattro integration
- [Unsafe code policy](docs/unsafe-code.md) — exception requirements and current inventory
- [Initial technical direction](docs/technical-direction.md) — original technical assessment

## Technology

- Rust
- GTK4
- GIO
- Native Wayland support

## Install a precompiled release

Strata is not yet available through Arch's package repositories. Download the archive and matching `.sha256` file for your CPU from the [latest release](https://github.com/LGSE/strata/releases/latest):

- `x86_64-unknown-linux-gnu` for Intel and AMD PCs
- `aarch64-unknown-linux-gnu` for ARM64 PCs

Install the runtime libraries and optional video preview tools on Arch or Omarchy:

```bash
sudo pacman -S --needed bubblewrap ffmpeg ffmpegthumbnailer fontconfig gst-plugins-good gtk4 gtksourceview5 poppler-glib
```

Then verify, extract, and install the downloaded archive (replace the filename with the release you downloaded). The `gh attestation` check verifies the archive's signed GitHub Actions provenance:

```bash
cd ~/Downloads
sha256sum --check strata-<version>-<target>.tar.gz.sha256
gh attestation verify strata-<version>-<target>.tar.gz --repo LGSE/strata
tar -xzf strata-<version>-<target>.tar.gz
install -Dm755 strata-<version>-<target>/strata ~/.local/bin/strata
```

Each archive also contains `SOURCE_COMMIT`, which records the exact commit used for the build.

Ensure `~/.local/bin` is on `PATH`, then run `strata`. Image thumbnails work without `ffmpegthumbnailer`; when `ffmpegthumbnailer` or `ffmpeg` is unavailable, video files fall back to their video icon or an unavailable preview. Bubblewrap is required: preview parsing fails closed rather than running untrusted native parsers without a sandbox. See [Preview sandbox](docs/preview-sandbox.md) for the providers, permissions, and resource limits.

Media playback requires the GStreamer auto-detection elements provided by
`gst-plugins-good`. Without them, GTK cannot select an audio output and media
previews remain at `0:00`.

FLAC files with embedded cover art display album thumbnails when `ffmpegthumbnailer` is installed.
The preview drawer can play the first 30 seconds through the sandboxed `ffmpeg` media pipeline.

#### Optional HEIC/HEIF and RAW photo previews

HEIC and HEIF thumbnails and previews use an installed GDK Pixbuf loader when available, then
fall back to ImageMagick. On Arch or Omarchy, install ImageMagick and its HEIF decoder:

```bash
sudo pacman -S --needed imagemagick libheif
```

Strata recognizes common camera RAW formats, including DNG, CR2/CR3, NEF, ARW, RAF, ORF, RW2, PEF, and X3F. RAW decoding is provided by tools already installed on the host rather than bundled into Strata. It tries, in order:

1. an installed GDK Pixbuf loader;
2. ImageMagick (`magick` or `convert`); and
3. the LibRaw-compatible `dcraw_emu` or `dcraw` thumbnail extractor.

On Arch or Omarchy, the recommended RAW setup is:

```bash
sudo pacman -S --needed imagemagick libraw
```

Available formats depend on how those host packages were built and on whether a particular camera model is supported. Unsupported or malformed RAW files continue to display the normal image icon.

### Desktop integration

Create a per-user desktop entry so launchers and `xdg-open` can discover Strata:

```bash
mkdir -p ~/.local/share/applications
cat > ~/.local/share/applications/io.github.lgse.Strata.desktop <<EOF
[Desktop Entry]
Name=Strata
Comment=Navigate every layer
Exec=$HOME/.local/bin/strata %U
Icon=system-file-manager
Terminal=false
Type=Application
Categories=Utility;FileManager;
MimeType=inode/directory;
StartupNotify=true
EOF
update-desktop-database ~/.local/share/applications
xdg-mime default io.github.lgse.Strata.desktop inode/directory
```

Confirm the association with:

```bash
xdg-mime query default inode/directory
```

It should print `io.github.lgse.Strata.desktop`.

### Make Strata the Omarchy file manager

The XDG association above makes folders opened by desktop applications use Strata. To also replace Omarchy's <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> shortcut, use the instructions for your Omarchy generation.

#### Omarchy Quattro

Add these overrides to `~/.config/hypr/bindings.lua`:

```lua
hl.unbind("SUPER + SHIFT + F")
hl.unbind("SUPER + ALT + SHIFT + F")
o.bind("SUPER + SHIFT + F", "File manager", "uwsm app -- strata")
o.bind("SUPER + ALT + SHIFT + F", "File manager (cwd)",
  "uwsm app -- strata \"$(omarchy-cmd-terminal-cwd)\"")
```

Apply and validate the configuration:

```bash
hyprctl reload
hyprctl configerrors
```

#### Omarchy 3

In `~/.config/hypr/bindings.conf`, replace the existing Nautilus file-manager binding with:

```ini
bindd = SUPER SHIFT, F, File manager, exec, uwsm app -- strata
```

Optionally add a shortcut that opens the active terminal's working directory:

```ini
bindd = SUPER SHIFT ALT, F, File manager (cwd), exec, uwsm app -- strata "$(omarchy-cmd-terminal-cwd)"
```

Then run `hyprctl reload` and `hyprctl configerrors`.

## Technical highlights

Strata is built around a small application model rather than placing filesystem logic in GTK widgets:

- **Native paths stay native.** Invalid UTF-8 names retain their original Linux path bytes and are converted only for display.
- **Navigation and peeking are separate.** Committed Miller columns participate in history; temporary hover peeks never mutate it.
- **Filesystem work is cancellable.** Directory requests carry generations, stream bounded batches, and reject stale results after rapid navigation.
- **Large directories stay virtualized.** Rows render through GTK list models and are exercised against deterministic fixtures containing up to 100,000 entries.
- **Monitoring is incremental.** Coalesced create, remove, move, and metadata events update sorted columns in place while ambiguous events safely fall back to a rescan.
- **Selection survives change.** Sorting, monitoring, and reloads preserve selection by native location rather than fragile row index.
- **Motion avoids layout churn.** Columns reserve their final width before animating, and horizontal reveal targets remain stable during deep navigation.
- **Failure is explicit.** Loading, empty, unavailable, and error states are distinct, with retry support that does not rewrite navigation history.

The architectural boundaries and performance workflow are documented in [`docs/architecture.md`](docs/architecture.md) and [`docs/performance-baseline.md`](docs/performance-baseline.md).

## Development

### AI development handoff

This checkout is the standalone, private **Hermes** project derived from Strata. The repository is
`battlemountain2/hermes`, and active development currently happens on
`feat/ui-updates`. The Rust package, executable, application ID, resource paths, cache paths,
and some user-facing strings intentionally still use `strata`; perform that rename later as one
coordinated release task rather than changing identifiers piecemeal.

The current `feat/ui-updates` checkpoint is verified with:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test -q
```

The last completed baseline is **173 passing tests** with strict Clippy clean. To run the current development build rather
than an older desktop-installed copy:

```bash
cd ~/Projects/hermes
cargo build
./target/debug/strata
```

Major completed Hermes work includes persistent tabs with a new-tab button, back/forward mouse
buttons, drive mounting and styled eject confirmation, movable Trash/sidebar places, everyday file
operations and Open With, clipboard status/details, folder-state and album-art thumbnails, trusted
`.desktop` icons, optional Today/Yesterday views, recursive fuzzy search, reveal-in-context,
structured and mouse-configurable search filters, opt-in bounded content search, broader text
previews, and selectable Maple Mono/JetBrains Mono fonts. Search content reads run off the GTK
thread, skip symlinks and binary-looking files, and cap direct text reads at 1 MiB.
We also recently added an optional folder tree panel (configurable in Settings), image preview
click-and-drag panning, and image auto-fit zoom scaling. The buggy list-row info buttons were
removed to simplify the UI. Today/Yesterday sidebar searches now use a cancellable routed search
source and publish an exact final result set after background indexing.

The **Advanced Preview Registry** is complete in `src/services/formats.rs`. It centrally
reports format family plus independent preview, thumbnail, and text-extractor capabilities. Local
preview loading, quick-preview eligibility, thumbnails, preview labels/unavailable messages, and
content-search eligibility now consume that registry. Sandbox operation selection remains outside
the service layer. Opt-in content search supports PDF text through the existing Bubblewrap/Poppler
sandbox. PDF work is cancellable and bounded to 32 documents per query, 32 MiB per input, and
1 MiB of extracted text; the search UI reports extraction or limit notices without blocking GTK.

Next, extend the proven extractor boundary to office documents with the same cancellation and
resource-limit guarantees, or move to archive/custom actions before returning for office support.

Preserve these decisions while continuing:

- untrusted image, media, and PDF parsing remains sandboxed through Bubblewrap;
- ImageMagick remains the first JPEG/image renderer while the glycin shared-memory crash workaround
  is needed;
- content search remains opt-in, cancellable, bounded, and off the GTK thread;
- missing optional host tools degrade to icons or explicit unavailable states;
- do not add dual-pane mode, an embedded terminal, or remote providers yet;
- the optional folder-tree panel belongs in Settings, not permanently in the main layout;
- preserve user changes in a dirty worktree and use `apply_patch` for source edits;
- commit focused changes to `feat/ui-updates` (or branch as appropriate) and push to the private Hermes remote.

After the preview registry: add archive/custom actions, remaining polish,
then perform the coordinated Strata-to-Hermes application/package rename and release preparation.

### Requirements

- The latest stable Rust release
- GTK 4.12 or newer
- GtkSourceView 5
- Poppler GLib
- Fontconfig
- A C toolchain and `pkg-config`

On Arch Linux:

```bash
sudo pacman -S --needed base-devel rust bubblewrap fontconfig gtk4 gtksourceview5 poppler-glib
```

Run Strata:

```bash
cargo run
```

For development, run Strata in auto-reload mode. The app rebuilds and restarts
when code or bundled assets change. On Arch, Debian/Ubuntu, and Fedora,
`start-dev` installs missing native dependencies (prompting for `sudo`) and
installs `cargo-watch` automatically when needed:

```bash
make start-dev
```

Run the standard quality checks:

```bash
./scripts/check.sh
```

The script always runs formatting, compilation, Clippy, and tests. It also runs dependency-policy and spelling checks when `cargo-deny` and `typos` are installed. CI runs the complete suite on the latest stable Rust release.

## Creating a release

Maintainers can run the **Release** workflow from GitHub's Actions tab on the default branch and choose a `patch`, `minor`, or `major` version bump. After both Linux targets build successfully, the workflow:

- commits the new version to `Cargo.toml` and `Cargo.lock`;
- creates an annotated `vX.Y.Z` tag; and
- publishes x86-64 and ARM64 archives, SHA-256 checksum files, signed build-provenance attestations, and generated release notes.

The final publishing job uses the protected `release` GitHub environment and is the only job granted write permissions. Repository administrators must configure that environment with required reviewers; prevent self-review when a separate maintainer is available to approve releases. The release workflow stops without publishing if the default branch changes while binaries are building. Run it again from the new head in that case.

## Bundled assets

Strata/Hermes includes a curated Lucide icon subset plus the regular JetBrains Mono and Maple Mono
Normal variable fonts. The interface font is selectable in Settings. See the bundled license files
and [third-party notices](THIRD_PARTY_LICENSES.md) for attribution.

## Status

Strata is at the technical-spike stage. The first objective is to validate responsive Miller columns, cancellable hover peeking, incremental directory enumeration, and previews in very large directories.

## License

Strata is licensed under the [GNU General Public License v3.0 or later](LICENSE).
