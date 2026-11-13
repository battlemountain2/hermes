# Release safety and interface checkpoint

November 4, 2026. Issue #6; branch `fix/6-release-safety`, based on `6d2d0299`.
This is an isolated implementation batch, not a stable release or installed update.

## Changes and evidence

Recursive transfers now request decisions at the actual conflict. Folder merges
preserve unrelated destination files and ask separately about nested file
conflicts. File choices are Skip, Keep both, Replace and Cancel; applying a choice
to remaining conflicts is scoped by conflict type and the current operation.
Changing destinations require another decision. Same-filesystem new-folder moves
retain their inode rather than copying every child. Cancellation retains completed
work and remaining originals; it is not a rollback of the entire operation.

The previous synchronous recursive size walk and UI preflight were removed.
Unknown folder totals stay indeterminate. Completed-leaf bytes are counted, but
continuous byte callbacks, accurate speed/ETA on long files and broader failure
qualification remain open. Do not describe the current HUD as fully verified
real-time telemetry.

Stable startup gates store/controller initialization behind an explicit legacy
import choice. Private, verified backups and immutable journal generations enable
safe retry without replacing existing Hermes files. Dev never imports shared
Strata data. See [migration behavior](branding-and-migration.md) for allowlists,
limits, v1 compatibility and Linux-only implementation details.

Picker setup in Settings embeds the existing desktop-setup owner, runs off the
GTK thread and restores the recorded provider instead of guessing KDE. It refuses
unknown or externally changed configuration, retains recovery records and does
not restart portals. Python 3 is declared as an optional setup dependency.

The picker has a labelled New folder action and Ctrl+Shift+N. Creation preserves
the Save filename and validates names; existing items are never replaced. Main
and picker use the same saved theme/font profile. Directory monitoring handles
atomic preference replacements and custom-theme additions/edits. Menus and
disabled actions use semantic theme colors; two unsupported GTK stylesheet
properties were removed. User preference: Follow Noctalia, with picker sync on.

Home uses compact horizontal folder shortcuts and a Personal folder header
action. The reported six standard folders plus four pins form two equal rows
at the tested size. Other counts wrap naturally. Recent Music and Recent Files
retain their existing behavior and order.

## Reproduce automated checks

Build the unit-test executable:

```sh
cargo test --locked --bin hermes --no-run -j 2
cargo check --locked --all-targets -j 2
python3 scripts/check-release-safety.py --test-binary /absolute/path/to/printed/test-executable
```

The runner requires Python 3, `gtk4-broadwayd` and `dbus-run-session`. It runs
each module/GUI case in a separate process with private XDG roots, generated
documents, a private display/bus and critical warnings treated as failures.
It does not launch the installed Hermes, play music, install packages, change
desktop defaults or reuse a source directory as a fixture. Logs and generated
Home/picker screenshots remain in the temporary directory it prints.

Current result: **130 passed** across eight module groups and seven explicit
GTK cases. The seven ignored tests are dependency-bound, not permanently
excluded: the runner executes chooser/filter/choice/cancellation, overwrite,
New folder, live theme/custom-theme/font updates, transfer conflict controls,
Home layout, and first-launch consent. Both compilation checks passed without
warnings. This does not certify the full repository test suite or green CI.

The existing Python picker-setup suite also passes (11 cases). Clippy is now
installed locally; strict all-target linting still fails on existing integration
test policy violations (unexplained allow attributes, unwraps and unsafe blocks).
The production-bin lint pass also exposes substantial pre-existing UI/controller
lint debt. These are open foundation tasks, not waived gates or passing checks.

The overwrite test's prior GLib critical came from GTK's recent-file serializer
with a missing test process program name, confirmed with a debugger backtrace.
The test now initializes its program name as the application launcher does.
It passes with criticals fatal; no warning handler was suppressed. Broadway
can still report parentless fixture dialogs and frame-timing warnings; these
are separate from physical Wayland parenting and frame pacing acceptance.

## Remaining release work

- Transfer descriptor-relative path/race hardening, continuous byte progress,
  storage failures, cross-filesystem/device removal and physical clipboard/drag
  interoperability. Reference relocation is attempted for successful moves;
  persistence failures are reported in logs, without a durable recovery journal.
- Archive-process isolation/bounds, complete paged Home search and pane
  lifecycle/routing findings from the audit. This batch does not close them.
- Repository-wide formatting/lint/dependency gates, immutable source-based
  publication, installed native package checks and release-candidate soak time.
- Real Noctalia/browser/Wayland/scaling, keyboard/accessibility, audio-under-load,
  Asahi ARM64 and Mac platform proof. Offscreen snapshots do not prove these.

The main checkout, installed binary, actual user data and desktop providers were
left unchanged. Follow-up installation/restart is a separate, user-controlled step.
