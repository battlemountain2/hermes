# Migration staging and storage failures

## Linux temporary-file lifecycle

Migration publication first requests an anonymous `O_TMPFILE` on the destination
filesystem. The file has private permissions and an owned, close-on-exec file
descriptor. Hermes writes, syncs and verifies the complete bytes before linking
that descriptor into its final name with `linkat`. Publication remains exclusive,
and the destination directory is synced before advancing the recovery journal.
Linux reclaims an unpublished inode when the process closes its last descriptor,
including after abrupt termination. [Linux temporary-file semantics](https://man7.org/linux/man-pages/man2/open.2.html)

The safe Rustix filesystem API supplies descriptor-relative creation and linking.
The existing descriptor-anchored directory traversal, bounded source/journal
formats and verified backups remain the migration's recovery protocol.

Filesystems or older kernels that reject anonymous temporary files use the
private named reservation path. Ordinary errors such as storage exhaustion and
permissions propagate to the caller. Named cleanup checks the held content inode
and reservation identity. Replaced entries and unknown legacy stages retain their
bytes for review. The fallback provides best-effort cleanup inside its exclusive
private reservation; arbitrary concurrent same-user edits cannot receive an
atomic compare-and-unlink guarantee.

Anonymous staging prevents new named crash leftovers on supporting filesystems.
Previously captured stages and abrupt-crash leftovers on the named fallback still
require explicit ownership review. Physical filesystem/platform acceptance determines which storage
capabilities a release can advertise.

## Durability during retry

A failed directory sync can leave a visible publication whose directory entry
still needs synchronization. Retry now syncs a reused verified backup or preserved
destination before advancing the journal. A repeated sync failure leaves the
previous checkpoint available. Fault-injection regressions exercise failed backup
and destination directory syncs after successful linking and again during retry.

## Automated acceptance

- Stage lifetime checks cover private permissions, file/link lifetime, exclusive
  publication and preservation of unrelated entries.
- Forced named reservations exercise ordinary cleanup, existing destinations,
  moved/replaced directories and replaced or unowned content.
- An existing recovery fixture explicitly checks preservation of unknown stage
  contents while recovering verified data.
- The storage-exhaustion test mounts a private 128 KiB tmpfs separately at config,
  data and state roots. A bounded filler produces kernel `ENOSPC`. Import must
  preserve originals, complete earlier publications and unrelated/conflicting
  destination files. Freeing the filler then permits recovery to complete.
- The installed-binary UI fixture interrupts the same import three times with
  `SIGXFSZ`, checks for zero new named stages after each exit, then resumes and
  verifies originals, collections, backups and normal closure.

The tmpfs fixtures run under private Bubblewrap namespaces and require the same
namespace prerequisites as the existing quality gate. Fault injection models
directory-sync failure. Physical power-cut, quota/device removal, filesystem
durability and platform acceptance have additional release checks.
