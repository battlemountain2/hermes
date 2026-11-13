# Audio preview reliability investigation

## Implemented follow-up — October 22, 2026

The generic helper binding now handles a deleted running executable without selecting the
newly rebuilt file at its former path. `src/sandbox/helper_binary.rs` opens `/proc/self/exe`
only when the selected path is the missing, deleted current executable, pins that inode for
the command lifetime, and passes it to bubblewrap's read-only data binding with private executable
permissions. Only the forked launch child clears its descriptor's close-on-exec flag and resets
the offset. No document paths, user files or arbitrary helper paths receive that special handling.
Normal launches retain their existing read-only path binding without copying the binary.

Persistent browser workers and one-shot preview sandboxes share this binding. This fixes the
observed missing-executable startup cause rather than increasing audio limits or changing tags.
The data-binding fallback copies the old executable for that worker launch, so a deleted debug
binary can have a temporary launch/memory cost; ordinary launches do not pay it. An already
running application without this code still needs a user-controlled reopen to acquire the fix.

The adjacent ignored regression copies its own test executable to a private directory, then
unlinks it and repeats with a different file at the original path. It verifies the pinned inode,
0700 permissions and repeated launches without using that replacement. Generated silent FLAC
metadata and cover-art requests exercise the framed worker channel too; no music is played.
The child verifies its private directory/path before unlinking anything. Historical investigation
details below describe the pre-fix observation, not a request to restart or modify a user process.

## Scope and safety

Initial read-only investigation on 2026-10-20, limited to audio metadata and album artwork.
No playback, window interaction, restart, dependency installation, tag changes, media
uploads, Cargo invocation, commits, or changes to decoder limits were performed.
Office and thumbnail UI implementation remain with the parent agent. The initial
investigation changed only this document; the separately authorized helper-binary
fix and its isolated tests are described below.

## Exact reported track

- Path: `/home/bry/Music/Underscores - U/09 Wish U Well.flac`
- Length: 51,540,203 bytes (the screenshot's decimal 51.5 MB).
- SHA-256: `ca547a772f4a432adf16d1c5281dba13260b3530d41062a2a1ca753ef59f5353`
- Returned title/artist/album: `Wish U Well` / `underscores` / `U`.
- Returned properties: FLAC, 261.263708 seconds, 48 kHz, stereo, 24-bit,
  1,578,181 bits/second.

The existing built worker successfully handled all four relevant operations on
this exact file in fresh, isolated sandboxes:

| Wire operation | Result | Framed response bytes |
| --- | --- | ---: |
| 18, music tags | Success | 114 |
| 19, selected audio metadata | Success | 1,727 |
| 26, selected artwork | Success | 1,658,085 |
| 4, thumbnail artwork | Success | 146,458 |

The same worker also returned the full 1,727-byte metadata response ten consecutive
times after processing all twelve album-art requests below. Each request passed
fresh input/output descriptors through SCM_RIGHTS, read the complete eight-byte
header and payload, and received the success completion byte. These direct wire
requests bypassed host caches. No audio samples were played or decoded for playback.

The launch reproduced `spawn_worker` isolation: bubblewrap with an empty environment,
unshared namespaces, private `/tmp`, read-only system libraries and executable,
`prlimit --as=1342177280 --cpu=10 --fsize=33554432`, and the
`--preview-helper browser-worker` entry point. Therefore the reported file size does
not, by itself, reproduce a decoder failure under the existing limits.

## Concrete worker-startup failure

At investigation time the running application was PID 398058. Reading its
executable link, without interacting with its window, returned:

```text
/home/bry/hermes/target/debug/strata (deleted)
```

Its old executable inode was 14409374; the rebuilt on-disk executable had a
different inode (14413957 at the later check). The application runs under an
existing Cargo watcher, and its executable had been replaced while it remained
open. PIDs/inodes are observations from this run, not permanent configuration.

`src/sandbox.rs::helper_executable` uses `std::env::current_exe()` directly in
production. On Linux, that resolves the executable link, including the deleted
suffix. `src/sandbox/browser.rs::spawn_worker` passes the resulting path directly
to bubblewrap's `--ro-bind`. Bubblewrap starts asynchronously; spawn success is
not proof that its bind or child executable succeeded. Its stderr is discarded.

Launching the otherwise identical worker with that observed deleted path produced:

```text
bwrap: Can't find source path /home/bry/hermes/target/debug/strata (deleted): No such file or directory
```

Bubblewrap exited 1. The request pipe returned zero bytes, so the host's
`Response::read` produces `Incomplete browser header`, wrapped as
`Unable to read wire response: Incomplete browser header`. Retrying a fresh worker
cannot repair the same nonexistent executable path. This fails before the audio
decoder starts and is not specific to FLAC or large input files.

The same launch with the current, existing on-disk executable returned valid tags,
metadata and artwork. Thus there is a concrete, independently reproduced startup
cause matching the screenshot error. The screenshot itself does not prove that
this was its only possible cause.

Mounting `/proc/398058/exe` directly through bubblewrap was also tried; this host
rejected that bind with `Permission denied`. Do not assume a different process's
proc executable path is a deployable fix. Using `/proc/self/exe` as the literal
bind source would refer to bubblewrap after exec, not the original application.

This root cause required separately authorized generic executable binding work,
not an audio decoder change. Decoder memory/CPU/file limits are unchanged; no
automatic user-application restart is needed to hide the condition.

## Helper binding fix

`src/sandbox/helper_binary.rs` exposes:

```rust
pub(super) fn bind(command: &mut Command, executable: &Path) -> io::Result<()>
```

Ordinary launches retain the existing `--ro-bind PATH /app/strata` arguments,
without opening or copying the binary. Only a nonexistent path ending in
` (deleted)` that exactly equals this process's `current_exe()` is eligible for
the fallback. Existing literal filenames with that suffix and unrelated helper
paths are never substituted with the running binary.

For the fallback, the application opens its own `/proc/self/exe` before launching
bubblewrap and retains that File inside the Command's `pre_exec` closure. This
pins the old running executable inode, not a newly rebuilt binary at its previous
pathname. Bubblewrap receives `--perms 0700 --ro-bind-data FD /app/strata`, so it
can materialize an executable, read-only private copy from the inherited descriptor.
No executable copy is made for normal launches.

The child-only closure uses checked, async-signal-safe `fcntl` to clear CLOEXEC
and `lseek` to reset the descriptor offset before exec. The parent descriptor
remains CLOEXEC. Every binding opens its own File; no descriptor or file offset
is shared between separate worker/one-shot Commands. Sequential reuse of a single
Command is covered by the regression. File ownership keeps the descriptor alive
until the Command is dropped. Pin/setup failures return `io::Error` instead of
silently falling back to the rebuilt pathname. Browser retry setup errors now
report `Unable to spawn sandbox worker` rather than hiding behind an earlier EOF.

The parent owns module registration and one-shot `sandbox_command` integration;
the browser binding is integrated here. New unsafe operations have narrow lint
expectations documenting their ownership and async-signal-safety conditions.

## Reported missing folder covers

All six folders have a readable `cover.jpg` and an embedded front-cover picture
in their first lexicographically sorted FLAC. A single fresh worker, reused across
the entire list, successfully returned 256-by-256 PNGs for both the adjacent JPEG
(operation 1) and that track's embedded artwork (operation 4):

| Folder | Adjacent JPEG bytes | JPEG frame bytes | Embedded frame bytes |
| --- | ---: | ---: | ---: |
| Turnstile - Nonstop Feeling (2015) | 730,005 | 115,157 | 90,816 |
| Underscores - U | 145,138 | 158,027 | 146,458 |
| Vince Staples - Big Fish Theory | 831,165 | 151,168 | 149,302 |
| Vince Staples - Ramona Park Broke My Heart | 257,626 | 25,171 | 22,118 |
| Vince Staples - Summertime '06 | 960,933 | 121,239 | 89,757 |
| Weezer - Pinkerton (1995) | 215,825 | 113,498 | 96,376 |

The filesystem uses a straight apostrophe in `Summertime '06`, unlike the curly
apostrophe in the report. None of these cover files approaches the generation
size ceiling. No missing/corrupt cover or image-dimension failure was reproduced.

The deleted-helper startup failure can also block newly requested folder covers
while older cached covers remain visible. That is a plausible shared explanation,
not proof of the UI/cache cause: no live folder rendering or cache state was
changed or inspected. Parent-owned folder-source selection/cache work remains
independent, and should not be replaced with a global decoder-limit workaround.

## Parent validation handoff

- Changed paths: `src/sandbox/helper_binary.rs`,
  `src/sandbox/helper_binary/tests.rs`, `src/sandbox/browser.rs`, and this document.
- Production audio decoder source and adjacent audio tests remain unchanged;
  the exact user file and all twelve named cover requests already pass isolated
  decoding at current limits.
- No Cargo commands or shared-target writes were initiated here. Standalone
  std-only compilation/tests ran in `/tmp/hermes-helper-binary-validation-YlIIoK`.
  Three regular tests and one explicitly invoked optional regression pass, along
  with formatting checks for the new files.
- The optional `deleted_running_binary_is_pinned_without_using_replacement` test
  copies only the test executable into a private temporary directory. Each child
  unlinks its own copied pathname; one also creates an incompatible replacement
  script there. The test compares pinned descriptor device/inode with the original
  running executable, confirms a distinct replacement inode, then uses bubblewrap
  to assert mode 0700 and execute the original test binary twice from the same
  Command. No user application is unlinked or restarted.
- Parent validation still needs full-crate compilation, both one-shot/browser
  integration, generated audio metadata/artwork responses after replacement, and
  PID-namespace teardown/cancellation coverage.
- Re-run the existing generated artwork-resolution regression and isolated
  metadata/policy cases after that fix. Do not include the known flaky ordinary
  audio pause/resume case as evidence about this failure.
- Physical UI confirmation of folder cover refresh remains user-controlled.
