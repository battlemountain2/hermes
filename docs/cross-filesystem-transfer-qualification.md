# Bounded cross-filesystem transfer qualification

These Linux tests exercise native staged transfer between two separate tmpfs
mounts. Each mount is capped at 4 MiB. The containing private tmpfs is capped at
64 KiB. The child verifies tmpfs magic, exact capacities, distinct device IDs and
an actual cross-device rename error before creating payloads. Bubblewrap exposes
the host filesystem read-only. Only tiny generated reservation and sentinel files
exist outside these mounts. Each child has a 20-second deadline.

The tests follow the re-executed Bubblewrap pattern in
`src/paths/legacy_import/tests.rs`. Bubblewrap and rootless user, PID, network and
mount namespaces are required. Missing helpers or namespace restrictions fail
with the child status and captured output. There are no ignored cases or fallback
to host-disk exhaustion. CI already prepares the namespace prerequisites.

## Cases

- Successful copy and move use the normal asynchronous `transfer_item` entry
  point, with absent, regular, symlink and dangling-symlink destinations. Sources
  include regular files, symlinks and dangling symlinks. Assertions cover complete
  bytes, destination device, exact link text, source retention/removal and
  unrelated source/destination contents.
- Genuine ENOSPC fills only the bounded destination tmpfs, then frees 128 KiB for
  a partial 1 MiB copy. The actual GIO copy runs inside
  `native_transfer_with_copy`. Before returning its real `NoSpace` error, the
  test verifies a nonempty incomplete stage on the destination device inside a
  private 0700 staging directory. The normal asynchronous entry point must also
  fail with `NoSpace` while storage remains constrained. Both originals survive,
  staging is removed, then freeing the filler permits a successful retry.
- Pre-cancelled and real GIO-callback-cancelled copies and moves retain the source
  and original destination, remove staging and support retry. Callback assertions
  inspect the destination before publication. A fast tmpfs copy can first report
  progress after all bytes have been staged, so this qualifies cancellation before
  publication without requiring a partial-copy callback.

Each failure case covers both copy and move across all four destination states.
Referent sentinels and dangling targets detect accidental symlink following.
Directory listings reject leftover staging or other unexpected entries.

## Reproduction and evidence

Build once in the integrating checkout, then run the three tests directly from
the printed test executable.

```sh
cargo test --locked --bin hermes --no-run -j 2
TEST_BINARY='/absolute/path/to/target/debug/deps/hermes-<hash>'
"$TEST_BINARY" adapters::local_operations::transfer::tests::cross_filesystem:: --test-threads=1 --nocapture
```

The ordinary test inventory includes these tests automatically. The existing
`scripts/check-test-suite.py` runner provides the broader private Home/display/bus
environment and executes them with the other nonignored tests.

During implementation, local Bubblewrap and Python GIO probes verified separate
tmpfs devices, real EXDEV, real ENOSPC with an incomplete destination file and
GIO cancellation. Rustfmt and diff whitespace checks are separate static gates.
Cargo compilation and execution of the Rust regressions belong to the parent
integration run. Record that run's source identity and results separately.

```sh
rustfmt --edition 2024 --check src/adapters/local_operations/transfer/tests.rs src/adapters/local_operations/transfer/cross_filesystem_tests.rs
git diff --check
```

## Remaining qualifications

These fixtures use separate mounts of the same filesystem type. Distinct physical
filesystems, remote GIO backends, device removal, physical storage pressure,
crash durability, cancellation summaries and shutdown acceptance remain pending.
Architecture-specific and physical Asahi/ARM qualification remain pending, as
does macOS qualification. No physical-platform result is inferred from tmpfs
behavior. Descriptor-relative transfer race hardening retains its existing gate.
