# Hermes 1.0 Transition Blueprint & Migration Guide

## 1. Executive Summary

As Hermes reaches feature maturity and establishes its own design identity—combining rich previews, high-performance search, an integrated music library, and native Wayland/Linux desktop integration—this document specifies the architectural and operational blueprint for transitioning the project identity from **Strata** to **Hermes 1.0**.

The transition guarantees:
- **Zero user disruption**: Automatic, transparent migration of user configuration, sessions, playlists, and caches on first launch.
- **Backwards compatibility**: Symlinks and fallback paths so existing terminal scripts, shell aliases, desktop environments, and window manager rules (such as Hyprland/Omarchy configs) continue functioning uninterrupted.
- **Clean upstream separation**: Distinct application ID, D-Bus service name, and package naming for distribution packaging (AUR, Flatpak, Nix).

---

## 2. Naming & Identifiers

| Component | Legacy (v0.2.x) | 1.0 Target (Hermes) | Compatibility Strategy |
| :--- | :--- | :--- | :--- |
| **Cargo Crate Name** | `strata` | `hermes` | Binary alias `strata` installed alongside `hermes` |
| **Primary Binary** | `strata` | `hermes` | `strata` -> symlink to `hermes` in `/usr/bin` |
| **Application ID** | `org.strata.Strata` | `io.github.lgse.Hermes` | Desktop entry alias & WM_CLASS compatibility |
| **Window Class** | `strata` | `hermes` | WM rule matching `^(hermes\|strata)$` |
| **D-Bus Bus Name** | `org.strata.Strata` | `io.github.lgse.Hermes` | Single-instance bus acquisition with legacy fallback |
| **Config Directory** | `~/.config/strata` | `~/.config/hermes` | Auto-migration on first start with fallback detection |
| **Cache Directory** | `~/.cache/strata` | `~/.cache/hermes` | Transparent directory migration or fresh cache root |
| **Data Directory** | `~/.local/share/strata` | `~/.local/share/hermes` | Database & playlist migration on startup |
| **Log Env Var** | `STRATA_LOG` | `HERMES_LOG` | `HERMES_LOG` inspected first; falls back to `STRATA_LOG` |

---

## 3. Directory & State Migration Strategy

### 3.1 Migration Life-Cycle on Startup

When Hermes initializes, the filesystem adapter executes a safe, non-destructive migration hook before reading settings or initializing SQLite databases:

```
                  ┌────────────────────────┐
                  │ Hermes Startup Process │
                  └───────────┬────────────┘
                              │
               Does ~/.config/hermes exist?
                     ├── Yes ──> Continue normal startup
                     │
                     └── No ───> Does legacy ~/.config/strata exist?
                                       ├── No ───> Initialize fresh ~/.config/hermes
                                       │
                                       └── Yes ──> Execute Migration Procedure:
                                                    1. Copy ~/.config/strata -> ~/.config/hermes
                                                    2. Create compatibility symlink if safe
                                                    3. Migrate ~/.local/share/strata -> ~/.local/share/hermes
                                                    4. Log: "Migrated legacy configuration to ~/.config/hermes"
```

### 3.2 Migrated State Artifacts

1. **User Settings (`settings.toml`):**
   - Startup directory preferences, hidden file defaults, layout density, and view modes.
2. **Tab Sessions (`sessions.json`):**
   - Active and background tabs, navigation history, and split-pane configurations.
3. **Favorites & Pinned Places (`pinned.json` / GTK Bookmarks):**
   - User-defined sidebar bookmarks and quick-access locations.
4. **Saved Searches (`saved_searches.json`):**
   - User queries, filters, and smart folder definitions.
5. **Music Library (`~/.local/share/strata/music/`):**
   - SQLite database (`catalog.db`), custom playlists (`*.m3u`), and track metadata cache.

### 3.3 Idempotency & Failure Safety

- All file copy operations are transactional: write to a temporary location (`.hermes-migrating.*`) and atomically rename (`renameat2` / `fs::rename`).
- Legacy files are never deleted automatically during migration; they remain in `~/.config/strata` as a safety net unless explicitly cleaned up by the user.

---

## 4. Desktop & Window Manager Integration

### 4.1 Desktop Entry (`io.github.lgse.Hermes.desktop`)

```ini
[Desktop Entry]
Name=Hermes
GenericName=File Manager
Comment=A fast, polished, personal file manager for Linux
Exec=hermes %U
Terminal=false
Type=Application
Icon=io.github.lgse.Hermes
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

### 5.1 Cargo.toml Reconfiguration

```toml
[package]
name = "hermes"
version = "1.0.0"
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

[[bin]]
name = "strata"
path = "src/compat_strata.rs" # Transparent alias calling hermes main
```

### 5.2 Packaging (PKGBUILD for Arch Linux / AUR)

```bash
pkgname=hermes-git
pkgver=1.0.0.r0
pkgrel=1
pkgdesc="A fast, polished, personal file manager for Linux"
arch=('x86_64')
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
    install -Dm644 "data/io.github.lgse.Hermes.desktop" "$pkgdir/usr/share/applications/io.github.lgse.Hermes.desktop"
    ln -s "io.github.lgse.Hermes.desktop" "$pkgdir/usr/share/applications/strata.desktop"
}
```

---

## 6. Implementation Checklist for 1.0 Release

- [x] Phase 1: Sniffing decoders, compiler hygiene, SQLite lock resilience, non-modal navigation.
- [ ] Phase 2: Per-folder view memory store (path -> ViewConfig).
- [ ] Phase 3: Home Dashboard Hub (recent files, pinned locations, recent music).
- [ ] Phase 4: Unified Omnibar & search consolidation (command palette & smart pills).
- [ ] Phase 5: Executable rename (`hermes` bin + `strata` wrapper) and `Cargo.toml` package bump to 1.0.0.
- [ ] Phase 6: Automatic configuration migration hook in `src/adapters/local_files.rs` and `src/storage.rs`.
