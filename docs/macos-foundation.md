# macOS foundation for issue #6

## Implemented boundary

This first patch adds `src/platform.rs` and adjacent modules. The pinned
`gio 0.21.5` source excludes `DesktopAppInfo` on macOS. All Hermes references to
that type now live in Linux-only desktop-file helpers. The launcher call site
at this base is `src/ui/browser.rs::open_location`.

Linux application discovery retains the existing `/usr/share/applications` and
`~/.local/share/applications` trust locations, executable-bit check, error text
and `DesktopAppInfo::from_filename` parsing. The helper returns `gio::AppInfo`.
The UI retains its launch, working-directory restoration and launch-error flow.
Icon discovery remains independent of permission to launch, with cancellation
and source-identity checks retained by the thumbnail caller.

On macOS and other unsupported targets, desktop-file discovery returns an
explicit unsupported error before launching or consulting launch permissions.
Icon lookup returns `None`, preserving the generic icon. The desktop-entry
branch returns before ordinary URI launching or thumbnail decoder scheduling.
GIO's desktop-entry interface is documented separately from portable AppInfo.
[GIO DesktopAppInfo](https://docs.gtk.org/gio-unix/class.DesktopAppInfo.html)

`platform::preview_isolation()` exposes a compile-target capability contract.

| Target | Value | Implemented backend |
| --- | --- | --- |
| Linux, including Asahi Linux | `LinuxBubblewrap` | Existing Bubblewrap launch paths |
| macOS | `MacXpcPending` | Pending |
| Other targets | `Unsupported` | Pending |

`is_implemented()` describes code availability. Linux still needs working
Bubblewrap, permitted namespaces and the appropriate decoder dependencies.
The private platform module exposes it within the crate for `print_diagnostics`,
which reports the selected backend, implementation availability and pending runtime
readiness checks. Production enum variants are target-specific, with all variants
available to the adjacent capability tests. Linux sandbox launchers invoke Bubblewrap. The Mac Dev candidate rejects preview,
thumbnail and playback admission before decoder dispatch. Its browser-worker module
is a pending-platform stub, so Linux pipe and socket ABIs are excluded from the Mac
binary. Office availability and automatic media warmup honor this boundary. A future
Mac adapter must retain these guards until its isolated service passes acceptance. Every startup, decode,
timeout or protocol failure must fail closed. An in-process or unrestricted
child-process decoder fallback is forbidden.

## Pending physical gates

PR #21 adds an Apple Silicon GitHub Actions build and a Dev bundle builder.
Native build results and physical GTK startup remain separate acceptance records.
Native application discovery, platform integration, distribution signing and all XPC
decoder work remain pending. Stable/Dev profile resolution and Mac migration also need
physical qualification. Mac storage has its own acceptance gate.
Linux-only test fixtures elsewhere in the repository still need a Mac
audit, including the existing thumbnail desktop-icon expectation.

The Swift service, macOS profile migration and native Mac compilation remain
pending. The parent owns the integrated Linux build and test run. Adjacent tests cover
desktop-file permission/parsing/icon behavior, target capability selection and
the unsupported-platform contract. Tests discover generated applications without
launching them or changing file associations.

## First Apple Silicon Dev candidate

`.github/workflows/macos.yml` builds the exact PR head on an ARM64 `macos-15`
runner with Rust 1.99.0. `packaging/macos/Brewfile` declares the native libraries.
The `macos-dev` Cargo feature selects the Dev identity and profile on macOS,
including a Finder launch. Linux retains its environment-selected profile.

`scripts/build-macos-dev.py` creates a new output directory atomically, rejects
non-ARM64 executables and packages the existing winged-H icon. CI signs the app
ad hoc, checks its signature, records linked libraries, refreshes the post-signing
binary checksum and creates `Hermes-Dev-arm64.zip`. This candidate dynamically
links to Homebrew libraries. It is an early development build with pending Mac
file-operation qualification and isolated preview support.

After downloading the `hermes-macos-arm64-dev` artifact from PR #21, extract its
inner ZIP. In the resulting `macos-candidate` directory, run the following.

```sh
brew bundle --file ./Brewfile
./"Hermes Dev.app"/Contents/MacOS/hermes --diagnostics
open "Hermes Dev.app"
```

Verify that diagnostics report `macos-aarch64`, the Dev application ID and four
`hermes-dev` profile roots. Start with generated test folders. Report whether the
first window opens, folder navigation works, text is readable and the window
behaves correctly on the M1 Pro display. Keep startup messages if launch fails.
macOS Gatekeeper acceptance and physical UX remain to be checked on that Mac.
The artifact has ad-hoc signing and has no Apple notarization.

## Physical M1 Dev bootstrap

Use the planned 2021 14-inch M1 Pro with 16 GB and record its actual macOS
version. Use the separate Hermes Dev profile without importing legacy data. An optional
disposable macOS login account can also be used for qualification. GLib's macOS paths must be observed
on-device. Setting XDG variables alone is insufficient evidence of isolation.

1. Record source, architecture, toolchain, SDK and dependency versions. Work from
   the integrated foundation commit and retain the complete log.

   ```sh
   git rev-parse HEAD
   git status --short
   sw_vers
   uname -m
   xcodebuild -version
   xcrun --show-sdk-path
   rustc -Vv
   cargo -V
   pkg-config --modversion gtk4 gtksourceview-5 gio-2.0 poppler-glib libheif \
     gstreamer-1.0 gstreamer-app-1.0 gstreamer-video-1.0 fontconfig
   ```

2. Resolve missing native dependencies in that disposable environment under a
   separately reviewed bootstrap change. Use the pinned Rust toolchain and
   ARM64 libraries. Record dependency prefixes and any pkg-config overrides.
   Run `cargo check --locked --all-targets`, then `cargo build --locked` natively.
   Save the first failing diagnostic if either fails. Address further compile
   blockers in a bounded follow-up before attempting GUI acceptance.

3. Run `HERMES_DEV=1 ./target/debug/hermes --diagnostics` and save the four
   reported profile roots. Confirm each resolves to the disposable account's
   intended `hermes-dev` namespace. Inventory those roots before and after the
   first launch. Require no stable/legacy import or stable/legacy profile writes.
   Run `cargo test --locked --bin hermes platform::` on the native target.
   Record the actual selected tests and outcomes.

4. After profile checks and startup blockers are resolved, launch a local Dev
   candidate against a generated empty folder. Verify the
   `io.github.battlemountain2.Hermes.Dev` application identity and capture the
   first rendered window, logs and observed profile writes. Try generated
   executable and nonexecutable `.desktop` entries. Both must report unsupported
   launching and retain a generic icon. Keep decoder previews unavailable while
   XPC is pending. Image-preview acceptance follows the isolated-service proof.

   ```sh
   fixture_root=$(mktemp -d "${TMPDIR:-/tmp}/hermes-mac-foundation.XXXXXX")
   mkdir "$fixture_root/empty"
   HERMES_DEV=1 ./target/debug/hermes "$fixture_root/empty" \
     >"$fixture_root/dev.log" 2>&1
   ```

## XPC one-image acceptance

The next implementation must first add an independently sandboxed XPC service
inside a development bundle and route one image operation through it. XPC
supports descriptor transport and separate service sandboxing. The service's
entitlements and request protocol need explicit review and physical negative
tests. [Apple XPC service guide](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingXPCServices.html)

Use the following as the acceptance contract for that follow-up. The service
name below is a proposed Dev identity, pending implementation.

1. Assemble a local `Hermes Dev.app` with bundle ID
   `io.github.battlemountain2.Hermes.Dev` and embedded
   `Contents/XPCServices/HermesPreview.xpc`, proposed service ID
   `io.github.battlemountain2.Hermes.Dev.Preview`. Record bundle paths, executable
   hashes and signing identity. Run `codesign --verify --strict --verbose=2`
   against both bundles and `codesign -d --entitlements :-` against the service.
   Verify App Sandbox is enabled and grants exclude network, broad user-folder
   access and temporary sandbox exceptions. Retain the entitlement output.

2. Generate one 64-by-64 opaque sRGB PNG with four distinct colored quadrants
   using a fixture generator in the disposable account. Save its SHA-256 and
   expected corner pixels. Create unrelated sentinel files beside the image
   and elsewhere in the account. The broker opens only the selected image
   read-only and transfers a descriptor with a request ID, operation, output
   bounds and deadline. The service receives no directory or broad bookmark.

   This fixture generator uses Python's standard library. Retain `fixture_root`
   from the bootstrap step and run it from the same shell.

   ```sh
   python3 - "$fixture_root" <<'PY'
   import pathlib
   import struct
   import sys
   import zlib

   root = pathlib.Path(sys.argv[1])
   colors = [bytes((255, 0, 0, 255)), bytes((0, 255, 0, 255)),
             bytes((0, 0, 255, 255)), bytes((255, 255, 0, 255))]
   rows = []
   for y in range(64):
       row = b"".join(colors[2 * (y // 32) + x // 32] for x in range(64))
       rows.append(b"\x00" + row)

   def chunk(kind, payload):
       checksum = zlib.crc32(kind + payload) & 0xffffffff
       return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", checksum)

   png = b"\x89PNG\r\n\x1a\n"
   png += chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 6, 0, 0, 0))
   png += chunk(b"sRGB", b"\x00")
   png += chunk(b"IDAT", zlib.compress(b"".join(rows)))
   png += chunk(b"IEND", b"")
   (root / "quadrants.png").write_bytes(png)
   (root / "adjacent-sentinel").write_text("must remain inaccessible\n")
   (pathlib.Path.home() / "hermes-xpc-sentinel").write_text("disposable account only\n")
   PY
   shasum -a 256 "$fixture_root/quadrants.png"
   ```

3. Require a distinct sandboxed service PID and log the operation/request ID.
   Decode only in the service. Return bounded packed sRGB RGBA with explicit
   width, height and stride. For this fixture require width 64, height 64,
   stride 256 and exactly 16,384 bytes. Validate lengths with checked arithmetic
   before allocation or display. Verify all four quadrant samples and display
   the image in the Dev preview through the normal selection entry point.

4. In a service-side diagnostic mode, prove reading the supplied descriptor
   succeeds, writing it fails, and opening both sentinel paths fails. Attempt
   network connection to a listener owned by the fixture and require denial.
   Record the actual attempted operations, errors and sandbox logs. Entitlement
   inspection alone does not satisfy these negative checks.

5. Select a truncated copy, then inject an oversized/invalid response, kill the
   service during decoding, cancel selection and remove the embedded service
   from a copied candidate bundle. Re-sign that copy and record its identity
   before testing the missing-service case. Every case must produce a bounded
   error or generic icon, preserve GUI responsiveness and reject stale replies.
   Use a 12-second request deadline for this spike and verify service teardown
   and descriptor release after cancellation/timeout. Repeat a valid selection
   after recoverable failures. Verify no original-file decoder runs in the GUI
   or an unrestricted helper, including when XPC is unavailable.

6. Retain source/binary identities, OS/SDK/dependency versions, resolved profile
   roots, entitlement/signature output, fixture hashes, request/service logs,
   failure timings and screenshot. Claim only this one-image path after all
   checks pass. Other formats, resource ceilings beyond this request, persistent
   workers, playback, Office, profile migration, distribution signing and
   notarization remain separate pending gates.
