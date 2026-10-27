# Hermes 1.0 Transition Blueprint & Migration Guide

## 1. Executive Summary

As Hermes reaches feature maturity and establishes its own design identity—combining rich previews, high-performance search, an integrated music library, and native Wayland/Linux desktop integration—this document specifies the architectural and operational blueprint for transitioning the project identity from **Strata** to **Hermes 1.0**.

The transition guarantees:
- **Zero user disruption**: Automatic, transparent migration of user configuration, sessions, playlists, and ratings on first launch.
- **Data safety & isolation**: Transactional atomic file copies with a recorded migration journal (`migration-v1.json`). Legacy Strata files are preserved untouched and never overwritten or deleted.
- **Zero symlink traps**: No symlinks are created between legacy Strata and modern Hermes data directories to prevent recursive loops, stale references, and permission anomalies.
- **Clean upstream identity**: Distinct application ID (`io.github.battlemountain2.Hermes`), D-Bus service names, and clean package naming for distribution packaging (PKGBUILD, AUR, Homebrew).

---

## 2. Naming & Identifiers

| Component | Legacy (v0.2.x) | 1.0 Target (Hermes) | Compatibility Strategy |
| :--- | :--- | :--- | :--- |
| **Cargo Crate Name** | `hermes` | `hermes` | Primary crate and binary |
| **Primary Binary** | `strata` | `hermes` | `strata` symlink to `hermes` in distribution packages |
| **Application ID** | `org.strata.Strata` | `io.github.battlemountain2.Hermes` | Desktop entry & D-Bus service identifier |
| **Window Class** | `strata` | `hermes` | WM rule matching `^(hermes\|strata)$` |
| **D-Bus Bus Name** | `org.strata.Strata` | `io.github.battlemountain2.Hermes` | Single-instance bus acquisition |
| **Config Directory** | `~/.config/strata` | `~/.config/hermes` | Centralized in `crate::paths::config_dir()`; dev mode uses `hermes-dev` |
| **Data Directory** | `~/.local/share/strata` | `~/.local/share/hermes` | Centralized in `crate::paths::data_dir()` |
| **State Directory** | `~/.local/state/strata` | `~/.local/state/hermes` | Centralized in `crate::paths::state_dir()` |
| **Cache Directory** | `~/.cache/strata` | `~/.cache/hermes` | Isolated per-app cache root in `crate::paths::cache_dir()` |
| **Log Env Var** | `STRATA_LOG` | `HERMES_LOG` | `HERMES_LOG` inspected first; falls back to `STRATA_LOG` |
| **Dev Mode Env Var** | `STRATA_DEV` | `HERMES_DEV` | `HERMES_DEV=1` isolates configuration to `hermes-dev` |

---

## 3. Directory & State Migration Strategy

### 3.1 Migration Life-Cycle on Startup

When Hermes initializes, `paths::migrate_legacy_data_if_needed()` runs immediately before any service or UI component reads state:

```
                  ┌────────────────────────┐
                  │ Hermes Startup Process │
                  └───────────┬────────────┘
                              │
               Does migration-v1.json exist in state?
                     ├── Yes ──> Already migrated / initialized; skip
                     │
                     └── No ───> Check legacy Strata roots:
                                   ├── No legacy files ──> Skip migration
                                   │
                                   └── Found legacy files ──> Transactional Copy:
                                        1. Atomic copy legacy config -> ~/.config/hermes/
                                        2. Atomic copy legacy data -> ~/.local/share/hermes/
                                        3. Atomic copy legacy state -> ~/.local/state/hermes/
                                        4. Atomic copy custom themes -> ~/.config/hermes/themes/
                                        5. Write migration journal: migration-v1.json
                                        6. Leave legacy Strata files completely untouched
```

### 3.2 Migrated State Artifacts

1. **User Settings (`settings.toml`):**
   - Startup directory preferences, hidden file defaults, layout density, and view modes.
2. **Folder Trails & Tabs (`trails.toml`):**
   - Active and background tabs, navigation history, and split-pane configurations.
3. **Folder Views (`folder_views.json`):**
   - Per-directory custom sort columns, directions, and view preferences.
4. **Sidebar Order (`sidebar-order`):**
   - User-defined sidebar places and bookmark order.
5. **Recent Folders (`recent-folders.toml`):**
   - Recently accessed locations for jump lists and dialogs.
6. **Custom Themes (`themes/*.toml`):**
   - Custom user theme definition files.
7. **Music Collections (`music-collections-v1.json`):**
   - Playlists, favorite tracks, albums, and artists.
8. **Track Ratings (`ratings-v1.json`):**
   - 1-to-5 star user ratings for music files.
9. **Saved Searches (`saved-searches/queries-v1.json`):**
   - User queries, filters, and smart folder definitions.
10. **Music Playback Session (`music-queue-v1.json`):**
    - Current track, playlist queue, repeat/shuffle status, and volume.

### 3.3 Idempotency & Failure Safety

- **Atomic staging**: Files are copied to a temporary file (`<dst>.hermes-migrating-tmp`) before being renamed into place via `fs::rename`.
- **No clobbering**: If a destination file already exists in Hermes, the migration engine skips it, preserving user modifications.
- **Non-destructive**: Legacy Strata files are **never deleted or modified** during migration. They remain intact in `~/.config/strata`, `~/.local/share/strata`, and `~/.local/state/strata`.
- **No symlinks**: No symlinks are created between Strata and Hermes roots.
- **Migration Journal**: A structured record is written to `~/.local/state/hermes/migration-v1.json` documenting exact sources, destinations, byte counts, and migration status. Once the journal exists, future launches skip migration checks.

---

## 4. Desktop & Window Manager Integration

### 4.1 Desktop Entry (`io.github.battlemountain2.Hermes.desktop`)

```ini
[Desktop Entry]
Name=Hermes
GenericName=File Manager
Comment=A fast, polished, personal file manager for Linux
Exec=hermes %U
Terminal=false
Type=Application
Icon=io.github.battlemountain2.Hermes
Categories=System;FileTools;FileManager;Utility;
MimeType=inode/directory;
StartupNotify=true
StartupWMClass=hermes
Actions=NewWindow;Home;Music;

[Desktop Action NewWindow]
Name=New Window
Exec=hermes --new-window

[Desktop Action Home]
Name=Open Home
Exec=hermes ~

[Desktop Action Music]
Name=Open Music
Exec=hermes --workspace=music
```

### 4.2 Window Manager Rules (Hyprland / Omarchy)

To ensure window positioning, floating rules, opacity, and decorations function identically regardless of whether the binary was invoked as `hermes` or `strata`:

```lua
-- ~/.config/hypr/hyprland.lua
-- Match both new and legacy window classes
windowrulev2 = "float, class:^(hermes|strata)$, title:^(Settings|Properties)$"
windowrulev2 = "size 800 600, class:^(hermes|strata)$, title:^(Settings)$"
windowrulev2 = "center, class:^(hermes|strata)$, title:^(Settings)$"
windowrulev2 = "opacity 0.96 0.92, class:^(hermes|strata)$"
```

---

## 5. Build System & Packaging Blueprint

### 5.1 Cargo Configuration

```toml
[package]
name = "hermes"
version = "0.2.0" # Bumping to 1.0.0 for release milestone
edition = "2024"
description = "A fast, polished, personal file manager for Linux"
authors = ["battlemountain2"]
license = "GPL-3.0-or-later"
repository = "https://github.com/battlemountain2/hermes"
keywords = ["file-manager", "gtk4", "linux", "wayland"]
categories = ["filesystem", "gui"]

[[bin]]
name = "hermes"
path = "src/main.rs"
```

### 5.2 Packaging (PKGBUILD for Arch Linux / Omarchy)

```bash
pkgname=hermes-git
pkgver=1.0.0.r0
pkgrel=1
pkgdesc="A fast, polished, personal file manager for Linux"
arch=('x86_64' 'aarch64')
url="https://github.com/battlemountain2/hermes"
license=('GPL-3.0-or-later')
depends=('gtk4' 'cairo' 'glib2' 'gstreamer' 'gst-plugins-base' 'poppler-glib')
makedepends=('cargo' 'git')
provides=('strata' 'hermes')
conflicts=('strata')

package() {
    cd "$srcdir/hermes"
    install -Dm755 "target/release/hermes" "$pkgdir/usr/bin/hermes"
    ln -s "hermes" "$pkgdir/usr/bin/strata"
    install -Dm644 "data/io.github.battlemountain2.Hermes.desktop" "$pkgdir/usr/share/applications/io.github.battlemountain2.Hermes.desktop"
    ln -s "io.github.battlemountain2.Hermes.desktop" "$pkgdir/usr/share/applications/strata.desktop"
}
```

---

## 6. Implementation Status for 1.0 Release

- [x] Foundation: Centralized, platform-aware paths module (`src/paths.rs`) with `hermes-dev` support.
- [x] Foundation: Non-destructive, transactional migration engine with temporary file atomic rename and `migration-v1.json` journaling.
- [x] Foundation: Clean component wiring across settings, trails, folder views, sidebar, recent folders, ratings, collections, saved searches, queue, waveforms, lyrics, and thumbnails.
- [ ] Packaging: Native PKGBUILD recipes for x86_64 and Asahi aarch64.
- [ ] Pipeline: Clean GitHub Actions release matrix for Arch and macOS Homebrew tap.
