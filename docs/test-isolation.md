# Test isolation and acceptance evidence

Run `python3 scripts/check-test-suite.py` from the repository. It builds the worker
and all-target/all-feature test artifacts, runs each ordinary test group with one
test thread, then explicitly runs selected generated GTK checks in separate
processes. Process-global GTK state cannot leak between those GUI cases.

Requirements on Linux: Bubblewrap, `gtk4-broadwayd`, `dbus-run-session`, native
development libraries and the pinned Rust toolchain. The runner prints a private
temporary directory containing per-process logs, generated profiles and
`coverage.json`. Copy that directory to durable validation storage when retaining
evidence. A failure names the log and stops the gate. The coverage manifest records
completed groups, the current failing case and excluded dependency/corpus checks.

Each process has a private display, D-Bus session, XDG roots and a mount namespace
masking the real Home directory. The checkout is read-only and generated fixtures
use writable temporary storage. This matters for libraries that consult the passwd
Home path and GIO Trash's filesystem selection; XDG variables alone do not provide
complete isolation. Network access is disabled inside the test process. Real
documents, settings, desktop defaults and installed application state are not test
fixtures. GLib criticals are fatal and GTK stylesheet errors fail the runner.

The disposable Ubuntu GitHub runner needs an explicit CI preparation step for
Bubblewrap user/network namespaces. The workflow checks that it is GitHub-hosted,
adjusts that runner's AppArmor user-namespace restriction and immediately probes
the namespace setup. These commands belong to the hosted CI job. Application
runtime and local checking commands retain their existing host-policy behavior.

Every ignored test must state its platform, helper, decoder, corpus or desktop
requirement. The runner inventories those reasons and explicitly removes cases
from the exclusion list only after executing them successfully. It does not
silently describe all ignored tests as passed. Generated isolated UI tests are
included where their dependency declarations match the available harness; decoder,
personal-media and physical-renderer cases retain separate acceptance requirements.

Broadway demonstrates widget behavior and lifecycle on a private display. It does
not certify Wayland parenting, real browser permission handoff, audible playback,
frame pacing, fractional scaling, accessibility or actual Asahi hardware.

The retained-tab regression also checks that the keyboard context-menu handler is
restored with its tab, and that closing a retained tab releases its file model.

The empirical descriptor-passing worker test performs one cold decode before
timing twenty warm requests. Cold process/decoder startup is logged separately;
the existing 150 ms limit still applies to each of the twenty warm requests.
Hosted qualification exposed the original first-request classification at 216 ms.
