# Installed migration and upgrade qualification

## November 7 checkpoint

The downloaded x86_64 package built from
`6ccf986190cd5be4e96005e627d19cb6cc55342b` passed six native UI scenarios in a
private Arch userspace. Its package SHA256 is
`936ea3362f2e566f1c18890ea9536de017d87f4a228857862058f79696cc52ed`.
The source tree matches foundation PR #9 head `c21e296c`.

The observer launches the installed `/usr/bin/hermes`, a private Xvfb display,
private D-Bus and accessibility buses, and generated XDG profiles. It locates
controls by their native accessible names and activates their standard actions.
Normal browsing windows close through the actual Close window control. These
checks exercise the packaged executable's startup and UI entry points.

| Scenario | Automatically checked behavior |
| --- | --- |
| Import consent | Browsing/store initialization waits for the decision; Import preserves originals, newer destination ratings, playlists, favorites, tabs and queue data; verified backups are private; normal restart enters browsing |
| Continue | The decision persists across restart; the declined collection stays in the legacy store |
| Durable recovery checkpoint | A generated valid journal and verified backup recover a missing original through the Resume control; earlier journal generations remain intact |
| Corrupt backup | The real error message appears before destination publication; repairing the generated backup and choosing Retry, then Resume, finishes safely |
| Abrupt interruption | After the setup window appears, the observer bounds the app's file-size limit to 1 MiB; a padded valid collection triggers SIGXFSZ mid-copy; restart resumes and verifies the complete collection and originals |
| Development | Dev enters browsing with a pending Stable journal present; legacy originals and Stable recovery records retain their bytes and completion state |

The durable-checkpoint scenarios use explicitly prepared recovery fixtures. The
interruption scenario terminates an actual importing process through its resource
limit. Its report records that signal separately. The artificial collection uses
valid JSON with whitespace padding. Applying the limit after setup isolates the
copy interruption from renderer/font-cache initialization.

The interrupted copy left one private `.import-stage-*` directory with a 1 MiB
partial file. Recovery completed using verified backups and source data. The
observer records retained stages and carries **private crash-stage retention and
cleanup review** into the pending release gates. Safe cleanup requires a reviewed
ownership/recovery protocol before removing crash leftovers. Backups and immutable
journals also remain available for recovery review.

The subsequent [staging lifecycle batch](migration-staging.md) adds anonymous
temporary files and strengthens the installed fixture to three interruptions
with zero newly created named stages. Its PR records the new executable/artifact
results. The evidence above describes the earlier package and its observed
leftover; older/fallback stages retain their explicit ownership review.

Local evidence is under
`/home/bry/hermes-validation/2026-11-07/migration/installed-run-3/` and the combined
`full-lifecycle/` run. Each includes a JSON result and generated profiles/logs.
Hosted results belong in the batch's PR and runtime-evidence artifact. The CI
artifact excludes disposable font/renderer caches and live IPC directories.

## Repeat the installed UI checks

Inside the marked disposable Arch root used by the package qualification runner,
install the observer dependencies after the clean runtime dependency checks.

```sh
pacman -S --needed --noconfirm python-gobject xorg-server-xvfb
dbus-run-session -- python3 scripts/qualify_installed_migration.py \
  --source-commit FULL_40_CHARACTER_PACKAGE_COMMIT \
  --output /absolute/new/migration-evidence
```

The main [package qualification runner](package-qualification.md) executes this
stage automatically before reinstall/removal. Its original retention fixtures
have separate XDG roots. A failed UI scenario fails the runtime CI job and retains
its partial report. Physical Wayland parenting, accessibility with a real assistive
tool, browser portals, storage-pressure hardware behavior, Asahi and macOS each
have additional acceptance gates.

## Genuine version upgrades

The public Hermes release list was empty at this checkpoint. Available native
candidates both identify as `0.2.0-1`. Their equal-version reinstall is qualified
separately. A previous-release upgrade needs an approved, previously qualified
older native package artifact. [Hermes releases](https://github.com/battlemountain2/hermes/releases)

The runner accepts that baseline through two explicit arguments. Mount its
extracted artifact directory read-only alongside the candidate and provide both
full source identities.

```sh
python3 scripts/qualify_arch_package.py \
  --candidate /candidate --source-commit FULL_CANDIDATE_SOURCE_COMMIT \
  --baseline /baseline --baseline-source-commit FULL_BASELINE_SOURCE_COMMIT \
  --output /absolute/new/upgrade-evidence
```

The baseline must pass the same checksum, package-layout and source checks. Hermes
package names and architectures must match. Pacman's `vercmp` must report a
strictly older baseline. Equal versions and downgrades fail before installation.
The runner then verifies the installed baseline executable, installs the
candidate, verifies its installed package version/source and checks fixture
retention. Both versions, source identity and baseline digest enter the report.

The current equal-version artifacts exercised the rejection path successfully.
The positive previous-release upgrade gate remains pending until a qualified
older package and an approved newer candidate are available.
