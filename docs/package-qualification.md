# Native package qualification

## November 7 x86_64 userspace checkpoint

The package from [foundation CI run 42402404072](https://github.com/battlemountain2/hermes/actions/runs/42402404072)
passed isolated Arch userspace installation checks using the signed November 1
Arch bootstrap. The signing fingerprint matched the Arch release key in the
installed distribution keyring. The test root shares the host kernel and has its
own filesystem, package database, D-Bus session, GTK display and generated profiles.

| Identity | Recorded value |
| --- | --- |
| GitHub artifact | `13586471268`, `hermes-arch-x86_64-test-candidate` |
| Built source commit | `037072bfa7981853e4451f070a9aaedfdeac19db` |
| Package | `hermes-file-manager-0.2.0-1-x86_64.pkg.tar.zst` |
| Package SHA256 | `e696cd3fd72898ba2f5d782d8617ab3fecb665f2c7506d789b5686a461cd2e02` |
| Download ZIP SHA256 | `576f1b374721498648fec0ef697b8fdb001bfd29f6255a0f989408a1ffce4a53` |

PR builds record GitHub's synthetic merge commit. Always read the artifact's
`BUILD-INFO.json` and installed `SOURCE_COMMIT` when identifying the built source.
The branch-head commit and package source can consequently differ.

Automatically exercised checks cover the following workflows.

- Offline installation refuses missing dependencies before changing fixtures.
- Signed distribution packages supply declared runtime dependencies.
- Pacman installs the downloaded Hermes artifact and checks its installed files.
- The installed executable reports its exact version/source, runtime capabilities
  and separate Stable/Dev directories.
- The installed GUI presents and renders a generated folder through private
  Broadway and D-Bus instances, with fatal GTK criticals and CSS error detection.
- Same-version reinstall preserves settings, session, playlist, favorite, rating,
  folder-preference, development-profile and legacy-profile fixture bytes.
- Removal clears package-owned files and preserves those data fixtures plus the
  recorded MIME and portal-default fixtures.

The data checks cover package-transaction retention. The follow-up runtime stage
also exercises native installed-app consent and recovery with separate profiles.
See [installed migration qualification](installed-migration-qualification.md) for
the six scenarios, crash-stage review and immutable-baseline upgrade arguments.
An actual upgrade from an earlier supported release remains open. Same-version
reinstall has its own result in the evidence.

The first standalone GUI smoke recorded a window at 147 ms and the first rendered
folder batch at 159 ms. This single generated-folder sample describes startup
only. Physical Wayland rendering, real browser Open/Save, audible playback,
optional-helper preview corpus, Asahi ARM64 and macOS have separate pending gates.

Local evidence lives under `/home/bry/hermes-validation/2026-11-07/`, including
`final-lifecycle/RESULTS.json`, per-command logs, baseline/runtime package inventories
and bootstrap signature verification. These paths are development evidence on
Bry's desktop. Hosted runs retain their own downloadable runtime evidence.

## Repeatable disposable-container check

The native-package workflow has separate build and runtime jobs. The runtime job
starts from `archlinux:base`, installs Python/Git observer tools, downloads that
run's package and binds qualification to the workflow's source SHA. Dependencies
come from package metadata. All package-manager changes stay in the fresh job
container. Its result participates in the existing aggregate release gate.
The official container image suppresses documentation through a `NoExtract`
rule. The runtime fixture removes that documentation rule before installing
Hermes so file-integrity checks cover its full payload. Other package-manager
policies remain in force. The first hosted run exposed this difference from the
bootstrap userspace. [Arch container configuration](https://github.com/archlinux/archlinux-docker/blob/master/pacman-conf.d-noextract.conf)

For a local reproduction, prepare an extracted candidate directory with
`BUILD-INFO.json`, `SHA256SUMS` and its single package. Use fresh absolute source
and evidence paths. The following example requires an available Docker daemon.

```sh
mkdir /absolute/new/evidence
docker run --rm \
  --mount type=bind,source=/absolute/candidate,target=/candidate,readonly \
  --mount type=bind,source=/absolute/hermes/scripts,target=/checks,readonly \
  --mount type=bind,source=/absolute/new/evidence,target=/evidence \
  --env EXPECTED_SOURCE_COMMIT=FULL_40_CHARACTER_PACKAGE_COMMIT \
  archlinux:base /bin/bash -euc '
    pacman -Syu --needed --noconfirm python
    printf "hermes-package-qualification-v1\n" > /.hermes-disposable-root
    sed -i "/^NoExtract[[:space:]]*=.*usr\/share\/doc\//d" /etc/pacman.conf
    python /checks/qualify_arch_package.py \
      --candidate /candidate --source-commit "$EXPECTED_SOURCE_COMMIT" \
      --output /evidence/run
  '
```

The runner requires mapped root in a private user namespace or a Docker container,
the explicit disposable-root marker, Arch identity and a fresh output directory.
Use a new root for every complete qualification run. Already-installed runtime
libraries would prevent the missing-dependency check from exercising its fixture.
A failure retains its command log and a partial report with remaining gates.

The marker is an operator safety check. Container/rootfs creation and its mounts
define the isolation boundary. Keep real Home, desktop sockets, host package
databases and live application data outside that boundary. Bubblewrap reproductions
need a subordinate UID/GID mapping for pacman's sandbox downloader and package
ownership. The recorded run retained pacman's standard signature/sandbox policy.
