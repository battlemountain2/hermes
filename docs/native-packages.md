# Native Linux test packages

These are qualification artifacts, not stable releases. The recipe supports
native x86_64 and aarch64 builds; ARM64 still needs a real Asahi build, install and
desktop acceptance. macOS requires its own platform work and Darwin artifact.

Use a clean checkout at an approved full commit SHA. Output must be a new directory
outside that checkout:

```sh
python3 scripts/build-arch-package.py --sha FULL_40_CHARACTER_COMMIT --output /absolute/new/output
```

The command never installs Hermes or installs missing system dependencies. It
generates a local Git source archive, checksum-bound PKGBUILD, `.SRCINFO` and
`BUILD-INFO.json`, then invokes makepkg as the current unprivileged user. Use
`--prepare-only` to inspect the recipe first. `--target-dir` accepts a separate
Cargo cache; do not share a writable target directory with another active build.
The committed version is preserved. No tag URL or checksum bypass is used.

The package declares required native libraries, includes the approved icon,
Stable/Dev launchers, source identity, Rust dependency/license inventory and the
reversible integration helper. It has no install hook, Strata executable alias or
default-manager/picker takeover. Optional decoder, thumbnail and Office helpers
are declared as optional dependencies; missing capabilities must remain visible.

After building, `scripts/test_arch_package.py` inspects archive boundaries,
required files, source identity and package metadata without installing the app.
`SHA256SUMS` records the package digest. A successful build on CachyOS is only a
host-built candidate: it is not proof of a clean Arch install or portability.

The **Native Arch package** workflow builds x86_64 in a disposable Arch container
and uploads the package, recipe, source identity and checksums as CI artifacts.
A separate fresh runtime job checks offline dependency refusal, installation,
installed-binary CLI/private-display startup, same-version reinstall, removal and
data/default fixture retention. Its downloadable evidence records remaining gates.
See [package qualification](package-qualification.md) for the downloaded-artifact
checkpoint and reproduction. Previous-release upgrades, installed-app migration,
browser picker opt-in/restore, common previews and physical desktop acceptance
remain required.

The immutable release workflow currently publishes GNU/Linux binary bundles and
source, not these native packages. Stable native-package publication and package
promotion remain open work. Dynamically linked bundles are not dependency-free
portable installs. No Mac stability is implied by an aarch64 Linux build.
