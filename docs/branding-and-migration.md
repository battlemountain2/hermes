# Hermes identity and legacy-data import

Current implementation: November 4, 2026, on `fix/6-release-safety`.
This supersedes the earlier automatic-migration blueprint. Implemented does not
mean qualified for a stable release; see [release roadmap](1.0-roadmap.md).

## Identity and isolated storage

The stable application ID is `io.github.battlemountain2.Hermes`. The approved
winged-H assets and upstream attribution remain. Necessary internal `strata-`
icon names and compatibility environment variables are not branding defects.
Do not install writable aliases to upstream Strata stores or take over its
binary/desktop entry.

`src/paths.rs` resolves config, data, state and cache roots through GLib's
platform directories. On Linux these normally use the XDG locations:

| Profile | Config | Data | State | Cache |
| --- | --- | --- | --- | --- |
| Stable | `~/.config/hermes` | `~/.local/share/hermes` | `~/.local/state/hermes` | `~/.cache/hermes` |
| Development | `~/.config/hermes-dev` | `~/.local/share/hermes-dev` | `~/.local/state/hermes-dev` | `~/.cache/hermes-dev` |

Presence of `HERMES_DEV` or compatibility `STRATA_DEV` selects Development
(including a value of `0`; unset both to select Stable). Development never
inspects/imports legacy stores and cannot enable or restore desktop picker
defaults. Different application IDs alone are not data isolation.

## Explicit import and startup order

Stable startup checks legacy `strata` config/data/state roots in a worker.
If importable files or an unfinished import exist, a setup window asks the user
to import/resume, continue with existing Hermes data, or retry the check.
Controllers and browsing windows initialize after that decision succeeds.
No user-data copies happen before consent.

The import allowlist is:

- Config: `settings.toml`, `trails.toml`, `folder_views.json`,
  `sidebar-order`, `recent-folders.toml`, and `themes/*.toml`.
- Data: `music-collections-v1.json`, `ratings-v1.json`, and
  `saved-searches/queries-v1.json`.
- State: `music-queue-v1.json`.

Ratings remain local photo/video/document ratings; hearts are separate music
favorites. Importing a saved-search file does not introduce a Smart Folder UI.

Existing destination files are preserved, including files changed during
recovery. Conflicting collections are not merged. Originals are never removed,
and caches, desktop-provider settings and arbitrary legacy files are excluded.
Caches rebuild independently.

## Recovery records and bounds

The Linux implementation in `src/paths/legacy_import.rs` uses:

- A private `legacy-import-v2/` transaction directory under Hermes's state root.
- Immutable `journal-NNNN.json` generations written before publication.
- Verified, private `backups/NNN.backup` files for entries that need copying.
- Anonymous temporary files on supporting Linux filesystems, with private named
  reservations as a compatibility path; exclusive linking, file/directory
  synchronization and descriptor-anchored traversal rejecting symbolic links.
- A process-released directory lock; simultaneous imports fail visibly.
- Limits of 16 MiB per file, 128 MiB total, 128 files, 128 KiB per journal and
  1,024 journal generations. Larger collections require manual review.

A completed v1 journal remains compatible. An unfinished v1 journal prompts
again; its bytes are backed up, but its stored paths are never executed.
A resumed v2 import can recover a backed-up entry after its original disappears.
Choosing Continue records that decision; it does not remove unfinished backups.
Retry syncs reused publications before advancing their checkpoint. Anonymous
staging reclaims partial unpublished bytes after a crash. Older unknown stages
and unsupported-filesystem crash remnants retain their ownership-review gate.
See [staging and storage failures](migration-staging.md).

On an invalid journal, unsafe path, changed backup, permission failure or full
storage, startup shows an error and retains originals and recovery records.
Fix the underlying storage problem and retry. Do not delete recovery records or
copy a backup over a newer Hermes file without reviewing both.

This implementation requires Linux descriptor paths and hard-link support.
The macOS import adapter is not implemented; do not reuse Linux flag constants
or promise equivalent Mac recovery based only on a successful compilation.

## Desktop integration is a separate opt-in

Changing identity or importing data never changes default handlers. Folder
defaults and the Open/Save portal are separate integrations. Picker setup in
Settings embeds the same restoration/journal owner as
`scripts/desktop-integration.py` and requires Python 3. It restores the recorded
previous provider rather than assuming KDE. Unknown legacy state or intervening
external edits fail visibly and preserve recovery records. It does not restart
portal services or interrupt screen sharing.

See [desktop integration](desktop-integration.md) for supported launch/setup
commands. The former blueprint's proposed flags and package aliases were not
evidence that those behaviors existed.

## Evidence and remaining gates

Automated fixtures cover explicit consent, Development isolation, safe existing
destinations, symlink/ancestor changes, interruption before/after backup and
publication, checksum failures, v1 recovery, concurrent import and size limits.
A private GTK test exercises the first-launch choice before import; it uses
generated files and isolated XDG roots.

The November 7 [installed migration qualification](installed-migration-qualification.md)
adds six native UI cases using the downloaded package and generated profiles.
Package install/reinstall/removal retention also has separate userspace evidence.
Positive previous-release upgrades, private crash-stage cleanup review, broader
storage-pressure behavior and physical desktop acceptance remain release gates.
