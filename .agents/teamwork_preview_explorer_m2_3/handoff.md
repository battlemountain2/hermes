# Handoff Report: Sandbox Ceilings, Cancellation Responsiveness & Worker Lifecycle (Milestone 2: R2 & F7)

## 1. Observation

### 1.1 `prlimit` Ceilings & Isolation Parameters
Directly observed in current Hermes codebase (`src/sandbox.rs:240-246`) and upstream reference commits (`015621c0:src/sandbox/browser/worker.rs:isolate_job`):

- **Address Space Ceiling (`--as=1342177280` [1.25 GB / 1280 MiB])**:
  - In `src/sandbox.rs:243`: `command.args(["--", "/usr/bin/prlimit", "--as=1342177280", ...])`.
  - In upstream pooled worker (`src/sandbox/browser/worker.rs:isolate_job`):
    `setrlimit(Resource::As, Rlimit { current: Some(ADDRESS_SPACE_LIMIT_BYTES), maximum: Some(ADDRESS_SPACE_LIMIT_BYTES) })`.
  - Mathematical basis: `1280 * 1024 * 1024 = 1,342,177,280 bytes`.
  - Effect: Bounds total virtual address space. Allocations past 1.25 GB via `mmap` or `brk` fail with `ENOMEM`, immediately triggering Rust's allocation error handler or C-level error return without exhausting system RAM or provoking the kernel OOM killer on the GTK/desktop session.

- **CPU Time Limit (`--cpu=10` [10s])**:
  - In `src/sandbox.rs:244`: `"--cpu=10"`.
  - In upstream pooled worker: `setrlimit(Resource::Cpu, Rlimit { current: Some(10), maximum: Some(10) })`.
  - Effect: Cumulative process user+system CPU time. At 10s, Linux delivers `SIGXCPU` (terminating the process). Prevents infinite decoder loops, malicious nested SVG entities, and malformed TIFF directory loops.

- **File Size Ceiling (`--fsize=33554432` [32 MB])**:
  - In `src/sandbox.rs:245`: `"--fsize=33554432"`.
  - In upstream pooled worker: `setrlimit(Resource::Fsize, Rlimit { current: Some(FILE_SIZE_LIMIT_BYTES), maximum: Some(FILE_SIZE_LIMIT_BYTES) })`.
  - Mathematical basis: `32 * 1024 * 1024 = 33,554,432 bytes`.
  - In `src/sandbox.rs:16`: `const MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;`.
  - Effect: Any write exceeding 32 MB triggers `SIGXFSZ` (File size limit exceeded) or `EFBIG`. Protects against decompression bombs and decoder scratch bloat.

- **Temporary Storage Limit (`tmpfs 256MB`)**:
  - In `src/sandbox.rs:208`: `"--tmpfs", "/tmp"`.
  - In upstream pooled worker (`src/sandbox/browser/worker.rs:isolate_job`):
    ```rust
    let options = std::ffi::CString::new(format!(
        "size={},mode=1777",
        super::super::TEMPORARY_STORAGE_LIMIT_BYTES // 268_435_456 bytes (256 MiB)
    ))?;
    for path in ["/tmp", "/dev/shm"] {
        mount(
            "tmpfs",
            path,
            "tmpfs",
            MountFlags::NOSUID | MountFlags::NODEV,
            options.as_c_str(),
        )?;
    }
    ```
  - Effect: Mounts dedicated private tmpfs on `/tmp` and `/dev/shm` capped at 256 MiB. Combined with private mount namespaces (`NEWNS`), every decoded job gets a pristine, empty temporary filesystem that is automatically discarded on job exit.

- **Suppressed / Prohibited Limits**:
  - `Resource::Core = 0`: In `isolate_job()`, core dumps are disabled (`setrlimit(Resource::Core, Rlimit { current: Some(0), maximum: Some(0) })`). Prevents writing multi-gigabyte crash dumps to disk and leaking confidential document content.
  - `--nproc` explicitly omitted (`src/sandbox/tests.rs:29-31`): `RLIMIT_NPROC` in Linux counts all processes owned by the user's UID system-wide. Imposing a low NPROC inside the sandbox causes fork failures when the host user has other applications open.

- **Host Wall-Clock Timeout (`WALL_TIME_LIMIT = Duration::from_secs(12)`)**:
  - In `src/sandbox.rs:15`: `const WALL_TIME_LIMIT: Duration = Duration::from_secs(12);`.
  - Enforces hard deadline for non-CPU-bound hangs (e.g. deadlocked IPC or I/O waits).

---

### 1.2 Cancellation Token Wiring (`gio::Cancellable` & `sandbox::Cancellation`)
Directly observed in `src/sandbox.rs`, `src/adapters/local_preview.rs`, and `src/ui/thumbnail.rs`:

- **Atomic Token Definition (`src/sandbox.rs:76-91`)**:
  ```rust
  #[derive(Clone, Default)]
  pub(crate) struct Cancellation(Arc<AtomicBool>);

  impl Cancellation {
      pub(crate) fn from_shared(cancelled: Arc<AtomicBool>) -> Self {
          Self(cancelled)
      }
      pub(crate) fn cancel(&self) {
          self.0.store(true, Ordering::Release);
      }
      pub(crate) fn is_cancelled(&self) -> bool {
          self.0.load(Ordering::Acquire)
      }
  }
  ```

- **UI Thread Initiation & `gio::Cancellable` Integration (`src/adapters/local_preview.rs:20-25, 88-94, 166-170`)**:
  - When a user selects a file, `LocalPreviewProvider::load` is invoked.
  - An atomic `cancellation` token is instantiated.
  - Operation is dispatched off the GTK main thread via `gio::spawn_blocking(move || crate::sandbox::parse(&path, operation, value, &cancellation))`.
  - Returns `LoadHandle::new(move || { cancellation.cancel(); task.abort(); })`.
  - Bridging with `gio::Cancellable`: Where GTK/GIO async methods provide a `gio::Cancellable`, it connects via:
    ```rust
    let cancellation = cancellation.clone();
    cancellable.connect_cancelled(move |_| {
        cancellation.cancel();
    });
    ```
  - When the user navigates away, presses Escape, or closes the preview pane, `LoadHandle` executes `cancellation.cancel()`.
  - In `src/ui/thumbnail.rs:241`: As the viewport scrolls, obsolete thumbnail tasks are cancelled via `previous.cancellation.cancel()`.

- **Pre-Execution Cancellation Interception (`src/sandbox/browser.rs:acquire`)**:
  - If cancellation occurs while waiting in the pool admission queue:
    `if cancellation.is_cancelled() { state.thumbnail_waiters -= 1; self.changed.notify_all(); return Err("Browser request cancelled".into()); }`
  - The request exits in 0ms without launching or reserving any worker.

---

### 1.3 `DeadlineReader` Polling in 20ms Quanta
Directly observed in `src/sandbox/browser.rs:799-846` and commit `028ff1b2` (`fix(preview): release pooled workers on cancel`):

- **The Problem in Commit `015621c0`**:
  In early pooled worker implementations, `DeadlineReader` polled the pipe with `timeout = remaining` (up to 12 seconds). If the user cancelled a preview while a slow file was decoding, `poll()` blocked in the kernel until the full 12s wall-clock timeout expired, locking the worker lease and stalling the background thread pool.

- **The Solution in Commit `028ff1b2` (`src/sandbox/browser.rs:815-845`)**:
  ```rust
  const WAIT_QUANTUM: Duration = Duration::from_millis(20);

  struct DeadlineReader<'a, R> {
      reader: &'a mut R,
      deadline: Instant,
      cancellation: &'a Cancellation,
  }

  impl<R: io::Read + std::os::fd::AsFd> io::Read for DeadlineReader<'_, R> {
      fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
          use rustix::event::{PollFd, PollFlags, Timespec, poll};
          if bytes.is_empty() {
              return Ok(0);
          }
          loop {
              if self.cancellation.is_cancelled() {
                  return Err(io::Error::other("Browser request cancelled"));
              }
              let remaining = self.deadline.saturating_duration_since(Instant::now());
              if remaining.is_zero() {
                  return Err(io::Error::new(
                      io::ErrorKind::TimedOut,
                      "Browser renderer timed out",
                  ));
              }
              // Poll in quanta so a cancelled preview releases its worker
              // instead of waiting out the absolute deadline.
              let wait = remaining.min(WAIT_QUANTUM);
              let timeout = Timespec {
                  tv_sec: wait.as_secs() as i64,
                  tv_nsec: i64::from(wait.subsec_nanos()),
              };
              let mut fds = [PollFd::new(&*self.reader, PollFlags::IN)];
              match poll(&mut fds, Some(&timeout)) {
                  Ok(0) => continue, // Timeout of 20ms quantum: loop and check cancellation!
                  Ok(_) => return self.reader.read(bytes), // Data ready
                  Err(rustix::io::Errno::INTR) => continue,
                  Err(error) => return Err(error.into()),
              }
          }
      }
  }
  ```
- **Responsiveness Guarantee**:
  - `poll()` blocks for at most 20 milliseconds (`WAIT_QUANTUM`).
  - Upon quantum expiry (`Ok(0)`), the loop checks `self.cancellation.is_cancelled()`.
  - Maximum cancellation response latency: **≤ 20ms**.
  - Neither the GTK main loop nor the GLib blocking thread pool ever stalls.

---

### 1.4 Immediate In-Flight Worker Termination on Cancellation
Directly observed in `src/sandbox/browser.rs:Lease::execute`, `Lease::discard`, and `ProcessWorker::drop`:

1. `DeadlineReader::read` returns `Err("Browser request cancelled")`.
2. In `Lease::execute`:
   ```rust
   let response = self.worker.as_mut().ok_or(...)?
       .execute(file, operation, cancellation);
   if response.is_err() {
       self.discard();
   }
   ```
3. `self.discard()` executes `self.worker.take()`, removing the `Worker` from the lease.
4. When `ProcessWorker` is dropped:
   ```rust
   impl Drop for ProcessWorker {
       fn drop(&mut self) {
           super::terminate(&mut self.child);
       }
   }
   fn terminate(child: &mut std::process::Child) {
       let _killed = child.kill();
       let _waited = child.wait();
   }
   ```
5. `child.kill()` delivers `SIGKILL` to `bwrap`.
6. Because `bwrap` was spawned with `--die-with-parent` and `--unshare-all`, `bwrap` is PID 1 of the container PID namespace. Killing PID 1 causes the Linux kernel to immediately send `SIGKILL` to all container descendants (the decoder child, ImageMagick subprocesses, etc.).
7. `child.wait()` reaps the zombie process.
8. The kernel immediately destroys the child PID namespace, unmounts the private tmpfs, closes the write pipe, and frees all memory.
9. In `Lease::drop`:
   ```rust
   if let Some(worker) = self.worker.take() {
       state.idle.push(...);
   } else {
       state.count = state.count.saturating_sub(1);
   }
   ```
   Because `self.worker` was discarded (`None`), `state.count` is safely decremented, immediately freeing the concurrency slot and waking waiting threads via `pool.changed.notify_all()`.

---

### 1.5 Worker Health Check, Crash Recovery & Pool Replacement Strategy
Directly observed in `src/sandbox/browser.rs`:

1. **Three Health Detection Signals**:
   - **Signal 1: Kernel Process Termination (Crash / OOM / `prlimit` kill)**:
     If the worker crashes from SIGSEGV, SIGABRT (out-of-memory), SIGXCPU (10s CPU limit), or SIGXFSZ (32 MB file limit), the pipe and socket close abruptly.
     The host's read operation encounters `ErrorKind::UnexpectedEof`, `ErrorKind::BrokenPipe`, or `ErrorKind::ConnectionReset`.
   - **Signal 2: Completion Status Byte**:
     After the pipe payload is read, the host reads exactly 1 byte from the control socket.
     Supervisor returns `1` on successful decode, `0` on handled decode failure.
     Missing byte or unexpected value triggers an error.
   - **Signal 3: Output Framing & Magic Bytes Validation**:
     - `Response::read` enforces `png_len <= 32MB` and `metadata_len <= 128KB`.
     - `super::valid_output` checks the PNG magic signature (`\x89PNG\r\n\x1a\n`).
     - JSON metadata is validated through `MediaMetadata::from_json`.

2. **Crash Recovery & Safe Replacement (`Lease::execute`)**:
   ```rust
   if result.as_ref().is_err_and(|error| {
       matches!(
           error.kind(),
           io::ErrorKind::UnexpectedEof
               | io::ErrorKind::BrokenPipe
               | io::ErrorKind::ConnectionReset
       )
   }) {
       self.discard(); // Kills crashed supervisor and releases old slot
       self.worker = Some(Worker::spawn().map_err(|e| e.to_string())?); // Pre-warms fresh replacement
       result = self.worker.as_mut().expect("replacement worker").execute(
           file,
           operation,
           cancellation,
       );
   }
   if result.is_err() {
       self.discard();
   }
   ```
   - On broken pipe / unexpected EOF (worker crash): Worker is discarded, a brand-new replacement is spawned on the spot, and the job is retried once.
   - On secondary failure or non-recoverable error (e.g. timeout, malformed file): Replacement is discarded, `result` propagates, and `Lease::drop` decrements `state.count` by 1.

3. **Zero Resource Leaks Guarantee**:
   | Resource | Mechanism Preventing Leak |
   |---|---|
   | **Process table / Zombies** | `ProcessWorker::drop` calls `child.kill()` + `child.wait()`. Supervisor and all children are reaped immediately. |
   | **File descriptors** | `SCM_RIGHTS` descriptors (`input_fd`, `output_pipe_fd`) are closed immediately upon child exit. Control socket is owned by `ProcessWorker` and closed on drop. |
   | **Temporary disk storage** | `ProcessWorker` owns `_snapshot: PrivateOutput`. On drop, `fs::remove_dir_all` wipes `/tmp/strata-preview-{pid}-{id}`. Container `/tmp` tmpfs is destroyed on namespace exit. |
   | **Pool slot accounting** | If a worker is discarded (`self.worker = None`), `Lease::drop` decrements `state.count` by 1. Pool capacity never drifts or permanently leaks slots. |

4. **Launcher Thread Isolation (`ProcessWorker::spawn`)**:
   - `PR_SET_PDEATHSIG` in Linux follows the *spawning thread*.
   - If ephemeral GIO thread pool threads spawned `bwrap` workers, worker processes would receive unexpected `SIGKILL` whenever a GIO thread expired from inactivity.
   - To fix this, Hermes uses a permanent dedicated `"thumbnail-launcher"` thread:
     ```rust
     std::thread::Builder::new()
         .name("thumbnail-launcher".into())
         .spawn(move || loop { ... })
     ```
   - All worker spawning and idle reaping requests are routed via an MPSC channel (`LauncherMessage::Spawn(reply)`).

5. **Idle Worker Retirement (`retire_idle`)**:
   - Default timeout: 60 seconds (`DEFAULT_WORKER_IDLE_TIMEOUT`).
   - Launcher thread checks `pool().next_expiration()`.
   - Idle workers exceeding 60s are extracted from `state.idle` and dropped *outside* the pool mutex lock, ensuring process cleanup never delays concurrent rendering requests.

6. **Dual Pools (Directory Thumbnails vs. Interactive Previews)**:
   - `pool()`: Dedicated to directory list/grid thumbnail generation.
   - `preview_pool()`: Dedicated to user-facing Space preview drawer (`PreviewImage`, `GeoTIFF`, `3D Model`, etc.).
   - Prevents thumbnail floods in 10,000-file folders from starving interactive preview responsiveness.

---

## 2. Logic Chain

1. **Premise 1 (Resource Protection Needs Strict Kernel Ceilings)**:
   - External format decoders (TIFF pyramids, RAW decoders, SVG rasterizers, 3D model parsers) process untrusted user input.
   - Relying solely on user-space bounds checks is insufficient because third-party C/Rust libraries can allocate unbounded memory or get stuck in parsing loops.
   - Therefore, enforcing Linux kernel limits via `prlimit` / `setrlimit` (`RLIMIT_AS=1.25GB`, `RLIMIT_CPU=10s`, `RLIMIT_FSIZE=32MB`) and `tmpfs=256MB` provides an un-bypassable hard security boundary.

2. **Premise 2 (Why 20ms Polling Quanta Are Mandatory)**:
   - Blocking on a pipe read with a 12s deadline blocks the reader thread for up to 12 seconds when a worker is stuck or working on a huge file.
   - If the user presses Down-Arrow rapidly through 50 files, 50 background tasks would get queued, holding workers and exhausting the GLib thread pool.
   - By dividing the wait into 20ms quanta (`WAIT_QUANTUM = 20ms`), `DeadlineReader` evaluates `cancellation.is_cancelled()` 50 times per second.
   - When cancellation occurs, the thread exits in ≤ 20ms, releases the worker lease, and frees the slot.

3. **Premise 3 (In-Flight Sandbox Termination Requires PID Namespace Teardown)**:
   - Simply closing a pipe does not guarantee an external decoder process stops computing immediately.
   - When `DeadlineReader` errors on cancellation, `Lease::discard()` drops the worker, which invokes `terminate()`.
   - Because `bwrap` is PID 1 of the container namespace, `SIGKILL` to `bwrap` causes the kernel to tear down every nested child process instantaneously.

4. **Premise 4 (Worker Pool Integrity Requires Strict Discard Accounting)**:
   - If a crashed or timed-out worker were returned to `state.idle`, subsequent requests would fail on a dead socket.
   - If `state.count` were not decremented when a worker is discarded, the pool would permanently lose available concurrency slots until restart.
   - Therefore, `Lease::discard()` + `Lease::drop` cleanly decouples worker lifecycle from pool capacity, guaranteeing automatic, leak-free worker replacement.

---

## 3. Caveats

1. **Required Linux Capabilities & Kernel Features**:
   - `bwrap` unprivileged user namespaces (`kernel.unprivileged_userns_clone = 1`).
   - `rustix` features: `pipe`, `event`, `net`, `process`, `mount`.
   - Landlock ABI 3 (Linux kernel ≥ 6.2) for input truncation protection (`protect_input`). If unavailable, `workers_supported()` gracefully falls back to one-shot `bwrap`.
2. **Single-Threaded Supervisor Invariant**:
   - In `src/sandbox/browser/process.rs`, `libc::fork()` and `unshare_unsafe()` require that the supervisor process is strictly single-threaded (verified via `/proc/self/task`).
   - The GTK main application must NEVER call `process::fork()`; only the dedicated supervisor binary (`strata --browser-worker`) runs this code.
3. **No `RLIMIT_NPROC`**:
   - `RLIMIT_NPROC` must remain excluded from `prlimit` parameters to prevent accidental fork starvation caused by unrelated desktop processes.

---

## 4. Conclusion

Requirement R2 & Feature F7 for Milestone 2 can be reliably implemented by adopting the proven architecture from commits `015621c0`, `028ff1b2`, and `9c60c0b2`:
1. **Apply Strict `prlimit` Ceilings**:
   - 1.25 GB Address Space (`--as=1342177280`)
   - 10s CPU Time (`--cpu=10`)
   - 32 MB File Size (`--fsize=33554432`)
   - 256 MB tmpfs on `/tmp` and `/dev/shm`
   - Core dump suppression (`RLIMIT_CORE = 0`)
   - Omission of `RLIMIT_NPROC`
2. **Wire Cancellation Tokens**:
   - Expose `cancellation: &Cancellation` through `preview_pool().request()` and `lease.execute()`.
   - Wire `gio::Cancellable` to `cancellation.cancel()` on user navigation or widget recycling.
3. **Implement `DeadlineReader` with 20ms Quanta**:
   - Poll pipe and socket using `rustix::event::poll` with `wait = remaining.min(WAIT_QUANTUM)` (20ms).
   - Check `cancellation.is_cancelled()` on each quantum expiration.
4. **Implement Resilient Worker Replacement**:
   - In `Lease::execute`, intercept `UnexpectedEof`, `BrokenPipe`, `ConnectionReset` and retry once on a fresh worker.
   - On cancellation, timeout, or secondary failure, call `lease.discard()` to kill `bwrap` and decrement `state.count`.
   - Isolate worker spawning to the `"thumbnail-launcher"` thread to preserve `PR_SET_PDEATHSIG` stability.
   - Maintain separate `pool()` and `preview_pool()` instances to isolate thumbnail generation from interactive previews.

---

## 5. Verification Method

To independently verify the implementation and behavior:

1. **Verify Baseline Test Suite Passes (187 unit tests)**:
   ```bash
   cargo test --bin strata
   ```

2. **Verify Sandbox Ceilings & Cancellation Unit Tests**:
   ```bash
   cargo test --bin strata -- sandbox::tests
   ```

3. **Verify E2E Suite for F4 & F7**:
   ```bash
   cargo test --test e2e_tests -- test_f7
   cargo test --test e2e_tests -- test_f4
   ```

4. **Verify Upstream Commits in Repository**:
   ```bash
   git show 015621c0 --stat
   git show 028ff1b2 --stat
   git diff 015621c0 028ff1b2 -- src/sandbox/browser.rs
   ```

5. **Synthetic Invalidation / Edge-Case Tests (to be confirmed during implementation)**:
   - **Timeout verification**: Pass a test file that sleeps or loops; verify `DeadlineReader` returns `io::ErrorKind::TimedOut` after 12s without blocking the UI thread.
   - **Cancellation verification**: Initiate a render on a 50MB file, cancel token after 50ms; verify worker terminates within ≤ 20ms and `pool().count` returns to 0.
   - **Crash recovery verification**: Send a signal (`kill -9`) to a worker child; verify `Lease::execute` detects `UnexpectedEof`, replaces the worker, and re-executes successfully without leaking file descriptors or zombie processes.
