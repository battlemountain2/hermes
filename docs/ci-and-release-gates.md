# Automated release gates

The November 6 follow-up implements these checks; a local pass is not a green
GitHub run or a stable-release decision. The workflow must land and pass before
repository administrators make **Release gates** a required branch-protection
check. Existing rules have not been changed by these source edits.

| Gate | What it enforces |
| --- | --- |
| Quality gates | Rust formatting, workflow lint, app identity, reversible setup fixtures, locked compilation, strict all-target/all-feature Clippy, isolated tests, offline release-control tests |
| Dependency policy | Locked RustSec advisories, allowed licenses, dependency bans and source policy; no new advisory exceptions |
| Spelling | Pinned typos, with explicit technical vocabulary |
| Arch package | Native x86_64 build plus a separate clean runtime job exercising downloaded-artifact installation, CLI/private-display startup, reinstall, removal and fixture retention |
| Release gates | Aggregate failure if any of the preceding jobs fails or is skipped |

`rust-toolchain.toml` pins Rust 1.99.0. `check-tools.toml` pins cargo-deny,
typos and actionlint. A missing tool is an error rather than a skipped check.

Install checking tools into a private directory, explicitly, then add its `bin`
directory to PATH for `bash scripts/check.sh`:

```sh
python3 scripts/check-tools.py --install --root /absolute/private/tool-directory
```

No application installation, desktop-provider change, release or restart occurs
in that command. Rust native development libraries and the isolated test tools
are prerequisites; see the CI dependency list and [test isolation](test-isolation.md).

The dependency refresh includes calamine 0.38 and ZIP 9 to resolve advisory
findings in the old Office/archive graph. The license inventory includes actual
license files; metadata warnings still require review rather than implying legal
certification. Software/security advisories may change after the recorded pass.

The release workflow separately requires an exact default-branch source commit,
successful exact-commit CI and a recorded human approval. See
[release process](release-process.md). Clean-machine installs, physical desktop
acceptance and seven-day reference-machine RC testing remain separate gates.
