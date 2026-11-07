# Immutable release publication

Implemented workflow controls are not permission to publish. No release was
dispatched while developing this batch, and no version was promoted.

1. Commit the intended version in both Cargo.toml and Cargo.lock through a reviewed
   PR. Keep the pinned compiler and dependency policies passing.
2. Land the approved source on the default branch and wait for its **latest** push
   CI attempt to finish successfully. A different source SHA, old successful
   attempt, missing job or skipped gate is rejected.
3. Configure the repository's `release` environment with required human reviewers.
   The workflow fails closed if this protection is absent. A project owner may
   approve deliberately; a second person is not imposed by Hermes source policy.
4. Manually dispatch Release from the default branch with the full source SHA,
   already-committed version and explicit preview/rc/stable channel. Confirmation
   must bind that exact channel, tag and SHA. Reruns are rejected; use a fresh
   dispatch and approval if anything changes.
5. Builds use the immutable source; trusted publication controls come from the
   dispatched default-branch workflow commit. Builds cannot push to the repository.
6. Review evidence, then approve the release environment. The publication job
   rechecks source reachability, CI attempt and recorded approval. It seals source,
   checksums, dependency/license inventory and platform build identities, attests
   assets, and creates a tag only at the approved commit. Existing tags are never
   moved. No version-file rewrite or push to main occurs.

`scripts/release.py`, `scripts/release_artifacts.py` and offline
`scripts/release_test.py` implement and test those decisions. The workflow has not
been exercised by creating a public test release. Configure repository protection
and review credentials/environment behavior before the first authorized dispatch.

Automated publication checks do not establish physical RC qualification. Stable
still requires downloaded-artifact installation, no critical/high defects and at
least seven days of daily use on each Linux reference machine. Publish known
limitations and rollback instructions. Mac remains beta until its own gates pass.
Native pacman package publication is still separate pending work; the current
release assets are dynamically linked Linux bundles and source.
