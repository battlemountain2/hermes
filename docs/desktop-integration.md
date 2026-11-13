# Optional Linux desktop integration

October24,2026: the first integration increment supports folder opening and
`org.freedesktop.FileManager1` reveal/Properties requests. It is implemented locally, **off by
default**, and independent of the system Open/Save picker. There is no Settings checkbox yet.
The separate experimental picker backend is described below; neither component enables itself.

## Before enabling

Build and install the new Hermes binary and launcher using the existing local installer. Updating
an installed build requires an explicit `--replace`; building alone does not update the installed
app or an already running window. The setup script checks a capability marker in the executable
without running it, and requires the installed launcher to use that binary for `inode/directory`.
It will refuse the older installed binary rather than launch it with an unrecognized flag.

Preview the folder setup without writing anything:

```sh
python3 scripts/desktop-integration.py status
python3 scripts/desktop-integration.py enable
```

Only after deliberately choosing to change folder defaults:

```sh
python3 scripts/desktop-integration.py enable --apply
```

Use `--binary /absolute/path/to/hermes` for a different user-local installation whose installed
launcher points to that exact executable. XDG configuration/data/state overrides must be absolute.
This is a Linux user-local setup, not a system-wide installer or macOS integration.

The folder opt-in changes exactly:

- `inode/directory` in `$XDG_CONFIG_HOME/<desktop>-mimeapps.list`, using the first current desktop
  name in lowercase; without `XDG_CURRENT_DESKTOP`, it uses `mimeapps.list`. Other MIME keys remain.
- `$XDG_DATA_HOME/dbus-1/services/org.freedesktop.FileManager1.service`, pointing to the chosen
  binary's windowless `--gapplication-service` mode.
- `$XDG_CONFIG_HOME/hermes/folder-integration.json`, the explicit versioned permission to own the
  reveal service. Missing, malformed, oversized or symlinked marker files do not enable it.

It also stores a private recovery journal under
`$XDG_STATE_HOME/hermes/folder-integration-transaction.json`. Standard fallbacks are `~/.config`,
`~/.local/share` and `~/.local/state`. The journal is written before configuration changes and
contains the previous bytes/permissions, including whether a file did not previously exist.
Setup refuses another user activation file already declaring the same bus name under a different
filename, rather than leave ambiguous providers.

The explicit apply/restore command asks the existing session bus to reload activation metadata.
This fixes first setup when its service directory did not exist at bus startup; it neither restarts
the bus nor launches a provider. If the refresh is unavailable, the script explains that a
user-controlled logout/login is needed. No portal configuration, unrelated MIME defaults, system
files, originals, playlists or upstream Strata installation are changed.

## Ownership and existing windows

Hermes uses `DO_NOT_QUEUE` without replacement flags. It never evicts Dolphin or another running
`FileManager1` owner and does not silently acquire the name later when that owner exits. You may
need to close the previous owner yourself and reopen Hermes before testing reveal. Status reports
configuration, not proof of live ownership. The setup script never closes or restarts either app.

Stable Hermes windows can observe changes to the opt-in marker. Removing/restoring the marker
releases Hermes's bus name and exported object without closing its windows or stopping music.
Reopening is still recommended after enabling, particularly for an older running build or a newly
created configuration directory. Development windows never claim this desktop-wide name.

D-Bus activation waits without opening a window; a request opens the relevant folder. An unused
service-mode instance expires after30seconds. Ordinary app startup still opens its normal window.
The existing GUI, MPRIS and helper modes are not replaced by a separate long-running daemon.

## Folder/reveal behavior

- `ShowFolders` opens each distinct requested folder. Normal launcher Open requests now preserve
  multiple locations and explicit GIO URI locations instead of discarding all but the first path.
- `ShowItems` groups by parent folder, retains URI order and deduplicates selection. It waits for
  directory loading to finish before selecting all available requested rows. It never opens a
  file, plays audio or forces a preview merely because another app asked to reveal it.
- `ShowItemProperties` opens the existing Properties view for each requested item, using a
  separate window per item rather than silently ignoring all but the first. A combined multi-file
  Properties summary is not implemented. Root-folder Properties are supported.
- Hidden requested items are included in that new window's visibility; global hidden defaults
  are not changed. A departed folder's late directory batches cannot reapply the request there.
- Native filenames retain URI escaping and non-UTF-8 bytes. Supported filesystem schemes use
  GIO; actual remote availability/authentication still depends on the existing installed backends.
  HTTP URLs, relative locations, queries/fragments, remote-authority `file://` paths and malformed
  requests are rejected before opening any window.
- Calls are bounded to64URIs/64KiB total and at most16windows. GTK receives a bounded startup ID;
  physical Wayland focus/activation still needs real-compositor acceptance testing.

## Restore or recover

```sh
python3 scripts/desktop-integration.py restore
python3 scripts/desktop-integration.py restore --apply
```

Restoration preflights all managed files, restores exact prior bytes/permissions and removes files
that did not exist before. It will **refuse** to overwrite any independently edited file or changed
permissions; review that conflict rather than deleting the journal or forcing a whole-file restore.
An interrupted setup retains its journal. `restore --apply` accepts the recorded before/after
states and can unwind a partially completed setup; another enable refuses an interrupted journal.
A private file lock prevents cooperating setup processes from writing concurrently.

Symlinks, shared hardlinks, special files, unsafe parent components and unbounded configuration
snapshots are refused. These defenses do not protect against another program deliberately racing
as the same Unix user; they prevent accidental overwrite and preserve a recoverable setup path.

## Verification

Safe unit/script checks:

```sh
python3 scripts/test_desktop_integration.py
cargo test --locked --bin hermes file_manager::tests::
```

Explicit private-bus check, without connecting to the actual desktop:

```sh
cargo test --locked --bin hermes file_manager::tests::private_bus_dispatches_requests_keeps_existing_owner_and_releases_registrations -- --ignored --exact
```

With a built binary, `gtk4-broadwayd`, `dbus-run-session`, `gdbus` and `gio` installed:

```sh
python3 scripts/check-desktop-integration.py --binary target/debug/hermes
```

The runtime check generates separate HOME/XDG roots, its own session bus whose only activatable
provider is the generated Hermes service, a private Broadway display and synthetic text files. It
tests actual service activation, no automatic startup window, grouped reveal, Properties routing,
invalid-URI rejection, live opt-out and previous-default restoration. It retains logs in the
reported temporary directory, never installs into real user paths, and terminates only its fixture
app/display. Separate ignored private-GTK tests under `ui::window::reveal::tests` verify delayed
selection/stale navigation and actual hidden-file Properties widgets. No sound is played.

Local validation passed13script cases, the private-bus regression, both private-GTK cases and the
real-binary private activation check. These are not physical Wayland/compositor or every remote
filesystem acceptance tests. Actual defaults and the installed binary remain unchanged until
separately authorized installation and activation.

## Experimental Open/Save picker

The candidate implements `org.freedesktop.impl.portal.FileChooser` on the separate
`org.freedesktop.impl.portal.desktop.hermes` bus name. `--desktop-integration-info` reports
`picker:true` and `HERMES_FILE_CHOOSER_V1`; this is code capability, not an enabled setting or a
claim of complete compositor/application acceptance. Older installed binaries may still report
`picker:false` and are deliberately refused by setup. Installation requires its own authorization.

The dedicated `--portal-file-chooser` process starts only with the versioned
`~/.config/hermes/picker-integration.json` marker. It does not start the browser, restore tabs or
initialize music. Only the current desktop portal frontend may open requests; unrelated callers
cannot show dialogs or close another caller's request. Requests are bounded and tracked through
completion, cancellation, caller disconnect and service shutdown. Removing the marker stops this
separate service without closing the file-manager window or music player.

Its activation entry disables GTK portal use only inside the backend process, with
`GDK_DEBUG=no-portals` and `GTK_USE_PORTAL=0`. Otherwise Wayland GTK startup can synchronously wait
for Registry/Settings/Inhibit while the frontend is itself waiting for the backend to register.
The private activation check forces inherited portal use to cover this bootstrap cycle. These
flags do not change the browser, main Hermes environment, GPU renderer or other portal providers.

Implemented locally:

- Open file/folder, multiple selection, SaveFile and SaveFiles destination selection.
- Suggested folder/name/file, glob/MIME filters, selected filter, boolean/enumerated choices,
  accept labels and activation tokens. Native filename bytes are preserved in `file://` results.
- Hermes fonts and semantic theme colors on a square-edged, direct GTK chooser. It intentionally
  uses the in-process chooser, not FileChooserNative/FileDialog, to avoid calling its own portal.
- Picker text inherits Hermes's saved interface font and scale, including live font changes;
  no separate picker-only font is forced. Sidebar rows, breadcrumbs, semantic surfaces, primary
  actions and preview headers follow the main app. The selected-file preview uses the same
  filename header and SIZE/TYPE metadata components. There is no "In this folder" gallery or
  gallery enumeration/decoding. The preview pane is hidden for folders/new save destinations,
  leaving the full file area available until an existing file is selected. Nothing autoplays;
  stale selections and closed dialogs cancel preview work.
- File-list/grid thumbnails reuse Hermes's actual entry icons and cached/sandboxed thumbnail
  pipeline instead of Breeze's large folder/file icons. Sidebar symbols use bundled Lucide
  assets. Native selection, activation, filters and inline new-folder editing stay GTK-owned.
  An adapter observes native cell creation/recycling and handles guarded `file-info`/`info`
  properties on GtkFileThumbnail. These are private GTK details, tested on GTK4.24.1; run the
  generated grid/list fixture before claiming support for a different GTK version. Missing
  properties retain the native fallback instead of calling private C APIs.
- The chooser object is registered before the backend bus name is acquired. Previously the
  first call could reach an advertised backend with no object and fail with UnknownMethod,
  forcing the user to try again. The private checker now makes eight cold, first-call property
  requests without retries before checking real frontend forwarding. Its readiness timings
  measure backend activation/export, not complete browser-to-visible-dialog latency.
- File-type selection is a normal semantic dropdown, without the dark nested-stack background
  or an extra colored parent plate. Focus rings are scoped to controls, not whole panes.
- GTK overwrite confirmation before returning an existing SaveFile target. Canceling/closing a
  request also destroys associated confirmation windows rather than leaving an orphan prompt.
- SaveFiles constructs non-conflicting names, including collisions with broken symlinks and
  other names in the same request. It does not reserve/create files; the caller must still handle
  changes between choosing a name and writing. Hermes never writes document contents here.
- Wayland exported-parent attachment through GDK; unsuccessful/unsupported attachment is kept
  non-modal rather than blocking an unrelated app. X11 foreign parenting is not implemented.

Only native/local or mounted paths are returned. Unmounted remote locations show an inline error;
this is not new GVfs/network browsing support. Requests allow at most8 active dialogs,64KiB of
serialized parameters,256 selections,64 SaveFiles names,32 filters/256 total rules and16 choices.
Globs are bounded to two wildcards per rule. Slow validation has a3second deadline. Unknown
optional fields are ignored; malformed known fields are rejected before showing a window.

### Independent setup and restoration

Inspect or preview without changing anything:

```sh
python3 scripts/desktop-integration.py status --component picker
python3 scripts/desktop-integration.py enable --component picker --binary /absolute/path/to/new/hermes
```

After separately authorized installation and real-desktop acceptance, setup is explicit:

```sh
python3 scripts/desktop-integration.py enable --component picker --apply
python3 scripts/desktop-integration.py restore --component picker
python3 scripts/desktop-integration.py restore --component picker --apply
```

The picker has its own journal, marker, backend activation file and user-local `hermes.portal`
manifest. Setup copies the effective existing portal configuration when needed, overrides only
`org.freedesktop.impl.portal.FileChooser=hermes` in the desktop-specific user configuration, and
preserves the defaults for ScreenCast, Settings, OpenURI and every other interface. Folder MIME
defaults/FileManager1 are unchanged. Provider configuration is switched last on enable and
restored first on opt-out. Existing bytes and permissions are backed up; interrupted operations
remain recoverable. External edits, conflicting activation files, unsafe files and custom
`XDG_DESKTOP_PORTAL_DIR` overrides are refused rather than overwritten or guessed around.

User-local manifest discovery was tested against this system's xdg-desktop-portal1.22.1. Older
frontends may not discover these manifests; do not enable blindly on them. The script does not
restart portals or apps. Portal discovery generally needs a user-controlled logout/login after
setup/restoration; avoid restarting portal services during screen sharing or active dialogs.
Apps bypassing XDG portals keep their own dialogs; selecting this backend cannot force every app
to use it. The setup tool does not verify a running provider or supply a Settings checkbox yet.

### Private validation

```sh
python3 scripts/test_picker_integration.py
cargo test --locked --bin hermes --no-run
python3 scripts/check-picker-integration.py --binary target/debug/hermes --test-binary /absolute/path/to/the/printed/unit-test-executable
```

The runtime harness creates fresh HOME/XDG roots, a restricted private bus and loopback-only
Broadway display. Private GTK tests use generated files for filter/choice roundtrips, successful
selection, cancellation and overwrite blocking/child cleanup. Backend tests verify caller
authentication, duplicate-handle rejection, ownership-protected Close, selected URI replies,
disconnect cleanup and unregistration. It then starts the actual system portal frontend on that
private bus to verify user-local provider discovery, Hermes activation, OpenFile/SaveFile/SaveFiles forwarding,
Request.Close, live marker removal and exact prior-config restoration. No actual portal, installed
binary, user window or desktop default is changed; no audio is played. Logs remain in the printed
temporary directory. Unit tests separately cover native bytes, invalid types/bounds, folder/file
validation and SaveFiles conflicts without creating output files.

The harness also checks the separate Open With menu using generated MIME associations: suggested
handlers precede unrelated applications, the list starts at the suggestions and cancel preserves
the current default. This menu chooses an app for an existing file; it is not the portal Open/Save
picker that chooses a file or destination for another app.

Physical Wayland parent/focus behavior, X11 parenting, Flatpak document-permission handoff,
Firefox/other-app interaction and accessibility review are remaining acceptance gates before
recommending real-desktop opt-in. The current local implementation is experimental, not a1.0
or cross-platform readiness claim.

Protocol/design references: [Strata's FileManager1 adapter](https://github.com/lgse/strata/blob/main/src/adapters/file_manager1.rs),
[MIME default associations](https://specifications.freedesktop.org/mime-apps/latest/default.html),
[D-Bus activation](https://dbus.freedesktop.org/doc/dbus-daemon.1.html), and the distinct
[FileChooser backend contract](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.FileChooser.html).
