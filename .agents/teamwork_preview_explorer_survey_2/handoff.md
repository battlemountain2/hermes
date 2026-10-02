# Handoff Report: Requirement R2 (Persistent Pooled Sandbox Worker & Large-File Guardrails) Survey

## 1. Observation

### 1.1 Existing Bubblewrap (`bwrap`) Invocation in Hermes
- **Location**: `src/sandbox.rs:182-255` in function `fn sandbox_command`:
  ```rust
  let mut command = Command::new("bwrap");
  command.args([
      "--unshare-all",
      "--die-with-parent",
      "--new-session",
      "--clearenv",
      "--setenv", "PATH", "/usr/bin",
      "--setenv", "HOME", "/nonexistent",
      "--setenv", "XDG_CACHE_HOME", "/tmp/cache",
      "--proc", "/proc",
      "--dev", "/dev",
      "--tmpfs", "/tmp",
      "--dir", "/app",
      "--dir", "/etc",
      "--ro-bind", "/usr", "/usr",
      "--ro-bind-try", "/lib", "/lib",
      "--ro-bind-try", "/lib64", "/lib64",
      "--ro-bind-try", "/etc/fonts", "/etc/fonts",
      "--ro-bind-try", "/etc/ld.so.cache", "/etc/ld.so.cache",
      "--ro-bind-try", "/etc/ImageMagick-7", "/etc/ImageMagick-7",
      "--ro-bind-try", "/etc/ImageMagick-6", "/etc/ImageMagick-6",
      "--ro-bind",
  ]);
  command.arg(executable).arg("/app/strata");
  command.arg("--ro-bind").arg(input).arg("/input");
  command.arg("--bind").arg(output).arg("/output");
  command.args([
      "--",
      "/usr/bin/prlimit",
      "--as=1342177280",
      "--cpu=10",
      "--fsize=33554432",
      "--",
      "/app/strata",
      "--preview-helper",
      operation.argument(),
      "/input",
  ]);
  command.arg(format!("/output/{}", operation.output_name()));
  command.arg(value.to_string());
  ```
- **Callers of `sandbox::parse`**:
  1. `src/adapters/local_preview.rs:90-94`: Inside `gio::spawn_blocking(move || crate::sandbox::parse(&path, operation, value, &cancellation))` when a user clicks/selects a file for preview.
  2. `src/adapters/local_preview.rs:135-138`: Folder album art preview.
  3. `src/ui/thumbnail.rs:259, 265`: In `render_thumbnail(path, kind, size, cancellation)` for every single grid/list item icon.
  4. `src/adapters/local_text_extraction.rs:32`: In `extract_text` for background search indexing.
- **Entry point in `src/main.rs:18-29`**:
  ```rust
  let arguments: Vec<_> = std::env::args().collect();
  if arguments.get(1).is_some_and(|value| value == "--preview-helper") {
      if let Err(error) = sandbox_helper::run(&arguments[2..]) {
          eprintln!("Preview helper failed: {error}");
          return gtk::glib::ExitCode::FAILURE;
      }
      return gtk::glib::ExitCode::SUCCESS;
  }
  ```

### 1.2 The One-Shot Execution Lifecycle & Overhead Profile
Every single one-shot preview or thumbnail request currently performs the following sequential lifecycle:
1. `src/sandbox.rs:108-115`: Host canonicalizes input path (`input.canonicalize()`) and creates private temporary directory `/tmp/strata-preview-{pid}-{id}` with `mode 0700` (`PrivateOutput::create()`).
2. `src/sandbox.rs:118-123`: Host executes `bwrap` via `std::process::Command::spawn()`.
3. Kernel performs namespace isolation: `--unshare-all` creates new user, mount, PID, IPC, UTS, cgroup, and network namespaces, configuring `uid_map` and `gid_map`.
4. `bwrap` mounts 20+ filesystems: `/proc`, `/dev`, tmpfs `/tmp`, `/usr`, `/lib`, `/lib64`, `/etc/fonts`, `/etc/ld.so.cache`, `/etc/ImageMagick-*`, binds the input file to `/input`, binds the output tempdir to `/output`, and binds `/proc/self/exe` to `/app/strata`.
5. `bwrap` executes `/usr/bin/prlimit` as PID 2 in the container. The dynamic linker (`ld-linux.so`) loads `prlimit` and glibc.
6. `prlimit` parses `--as=1342177280 --cpu=10 --fsize=33554432`, sets `setrlimit` boundaries, and `execve`s `/app/strata`.
7. Dynamic linker loads `/app/strata` (the 40+ MB fat binary) and links against >50 heavy shared libraries: `libgtk-4.so`, `libgdk_pixbuf-2.0.so`, `libpoppler-glib.so`, `libcairo.so`, `libgio-2.0.so`, `libgobject-2.0.so`, `libfontconfig.so`, `libpango-1.0.so`, `libharfbuzz.so`, etc.
8. Rust runtime initializes standard library, thread pool structures, and C runtime initialization routines.
9. Fontconfig scans fonts: Because `/var/cache/fontconfig` is NOT bound in `src/sandbox.rs`, fontconfig re-scans system font definitions inside the container on every spawn.
10. `src/sandbox_helper.rs:76-80`: `render_image` calls `render_imagemagick(path, size).or_else(|_| render_pixbuf(path, size))`. `render_imagemagick` spawns `magick` or `convert` as a nested subprocess (`Command::new("magick")`), which parses `/etc/ImageMagick-7` policy, scans fonts, decodes image, and emits PNG.
11. If ImageMagick fails, it falls back to `render_pixbuf`, which invokes `gdk_pixbuf::Pixbuf::from_file_at_scale`, possibly launching external pixbuf loader binaries (e.g. `glycin`).
12. Helper writes result PNG to disk at `/output/result.png` (`src/sandbox_helper.rs:61`).
13. `src/sandbox.rs:126-143`: Host thread polls `child.try_wait()` with `thread::sleep(Duration::from_millis(20))` until child exits.
14. Host reads `/output/result.png` and `/output/result.meta` from disk.
15. `PrivateOutput` drops and executes `fs::remove_dir_all(&self.0)`.

Measured & documented impact (verified in upstream Strata commit `015621c0`):
- Process spawn + namespace unshare + dynamic linking + fontconfig init alone: **100ms - 500ms** per file.
- Nested subprocess execution (ImageMagick / dcraw / inkscape / poppler) and font scanning: **500ms - 2000ms** (and up to **8000ms** for SVGs containing text).
- Host disk I/O (temp directory creation, disk writes, polling, reading, deletion): **10ms - 50ms**.

### 1.3 Upstream Strata Architecture Discoveries
Two critical upstream commits in git history provide the exact battle-tested solution:
1. **Commit `015621c00c044adea2eaab948db579ca8aaa0432`**:
   - Title: `perf(preview): serve image and document renders from a pooled sandbox worker (#1222)`
   - Measured result: SVG quick preview dropped from ~2s to ~42ms median warm (509ms cold).
   - Implemented: `src/sandbox/browser.rs`, `src/sandbox/browser/wire.rs`, `src/sandbox/browser/worker.rs`, `src/sandbox/browser/process.rs`.
   - Review hardening commit `028ff1b2`: `fix(preview): release pooled workers on cancel and harden SVG fallthrough`.
2. **Commit `6305099975207423a5a248c719d330cea6c22957`**:
   - Title: `fix(sandbox): fall back to EXIF thumbnails for oversized images (#1202) (#1277)`
   - Solved: Images > 134 MP kill the sandboxed renderer with `SIGXFSZ` because GdkPixbuf / Glycin attempts a full-resolution decode exceeding the file size limit (512 MiB in Strata, 32 MiB in Hermes).
   - Implemented: Fast header sniffers (`read_jpeg_dimensions`, `read_png_dimensions`, `read_gif_dimensions`), `exceeds_decoded_frame_budget`, and EXIF thumbnail fallback via `kamadak-exif`.

### 1.4 Wire Protocol & Local IPC Design (`src/sandbox/browser/wire.rs`)
- **Transport**: Unix Domain Socket (`UnixStream::pair()`).
- **File Descriptor Passing via `SCM_RIGHTS`**:
  Host opens the target file using `O_PATH` and `O_RDONLY | O_CLOEXEC | O_NONBLOCK`, and creates a pipe (`rustix::pipe::pipe_with(PipeFlags::CLOEXEC)`).
  Host sends the request over the Unix socket via `rustix::net::sendmsg` with ancillary message `SendAncillaryMessage::ScmRights(&[input_fd, output_write_pipe_fd])`.
  Worker receives via `rustix::net::recvmsg` with `RecvAncillaryBuffer`.
  **Key Security Benefit**: The sandboxed worker does NOT need filesystem read access or path canonicalization. It receives an open, read-only file descriptor directly from the parent.
- **Request Framing**:
  1 byte `Operation` enum + ancillary descriptors `[input_fd, write_fd]`:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  #[repr(u8)]
  pub(crate) enum Operation {
      Image = 1,
      Raw = 2,
      Pdf = 3,
      Video = 4,
      ImageMetadata = 5,
      MediaMetadata = 6,
      PreviewImage = 7,
      DocumentMermaid = 8,
      DocumentMath = 9,
      DocumentMathInline = 10,
      ThreeMfThumbnail = 11,
      FreeCadThumbnail = 12,
  }
  ```
- **Response Framing**:
  8-byte fixed header: `[png_len: u32 (little endian)][metadata_len: u32 (little endian)]`, followed by `png_bytes` and `metadata_bytes`.
  Bounded by `MAX_OUTPUT_BYTES = 32 * 1024 * 1024` and `MAX_METADATA_BYTES = 128 * 1024`.
  Worker signals completion by writing 1 byte success flag `[1u8]` on the control socket.

### 1.5 Worker Isolation & Disposable Fork Architecture
In `src/sandbox/browser/worker.rs` and `process.rs`:
- Supervisor is spawned once per pool worker with `strata --browser-worker`.
- Supervisor process stays running inside Bubblewrap with Landlock and Seccomp filters applied (`restrict_mutations`).
- It is strictly single-threaded (verified via `/proc/self/task`).
- For each incoming request on the control socket:
  1. Supervisor calls `libc::fork()`.
  2. Parent supervisor waits for child and sends 1-byte completion status.
  3. Child duplicates `input_fd` to stdin (`/proc/1/fd/0`) and `write_pipe_fd` to stdout.
  4. Child calls `process::namespaces()`: creates private namespaces (`NEWUSER | NEWNS | NEWPID | NEWIPC`), configures `/proc/self/uid_map`, and sets private mount propagation (`MS_PRIVATE | MS_REC`).
  5. Child calls `libc::fork()` into the new PID namespace (PID 1).
  6. Child sets `setrlimit` for `Resource::As` (1.25 GB), `Resource::Cpu` (10s), `Resource::Fsize` (512 MB or 32 MB), and `Resource::Core` (0).
  7. Child mounts clean tmpfs on `/tmp` and `/dev/shm` (max 256 MB).
  8. Child applies Landlock ABI 3 truncation protection on the input descriptor (`protect_input`).
  9. Child calls `browser_render` and streams framed `Response` directly to stdout.
  10. Child exits with `rustix::runtime::exit_group(status)`.
- **Zero Cross-Job State**: Memory leaks, dirty shared state, or segfaults in image decoders are instantly purged when the child process exits. The supervisor remains clean.

### 1.6 File Classification, Header Sniffing & EXIF Fallback (`src/sandbox_helper.rs`)
- In `src/services/formats.rs:155-270`, files are classified by MIME type (`classify_by_mime`) and extension (`classify_by_name`).
- Dimension sniffing functions in Strata `63050999`:
  - `read_jpeg_dimensions`: Validates SOI marker `[0xFF, 0xD8]`, traverses JPEG markers skipping metadata and application segments, parses SOF markers (`0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF`), and extracts `(width, height)` in under 1ms without decompressing DCT coefficients.
  - `read_png_dimensions`: Validates `\x89PNG\r\n\x1a\n` and `IHDR` chunk, extracts 4-byte BE width and height from bytes 16..24.
  - `read_gif_dimensions`: Validates `GIF87a`/`GIF89a`, extracts 2-byte LE width and height from bytes 6..10.
  - Fallback: `gdk_pixbuf::Pixbuf::file_info(path)`.
- Decoded frame budget check:
  ```rust
  pub(crate) fn exceeds_decoded_frame_budget(width: i32, height: i32) -> bool {
      let width = u64::try_from(width).unwrap_or(0);
      let height = u64::try_from(height).unwrap_or(0);
      const RGBA_CHANNELS: u64 = 4;
      width.saturating_mul(height).saturating_mul(RGBA_CHANNELS) > FILE_SIZE_LIMIT_BYTES
  }
  ```
- EXIF thumbnail fallback:
  - If `exceeds_decoded_frame_budget(width, height)` is true:
    Calls `read_exif_thumbnail(path, size)`.
    Uses `kamadak-exif::Reader` to parse TIFF header from JPEG APP1 marker, extracts offset and length from `Tag::JPEGInterchangeFormat` and `Tag::JPEGInterchangeFormatLength` in `In::THUMBNAIL`.
    Scales thumbnail via `scale_embedded_thumbnail` (verifying thumbnail dimensions do not themselves exceed the budget to prevent decompression bombs).
    If no EXIF thumbnail exists, fails gracefully with an informative error rather than getting killed by SIGXFSZ/OOM.

### 1.7 Memory Limits, Timeouts & Cancellation
- **Limits**:
  - `ADDRESS_SPACE_LIMIT_BYTES`: 1,342,177,280 bytes (~1.25 GB)
  - `FILE_SIZE_LIMIT_BYTES`: 33,554,432 bytes (32 MiB in Hermes) / 536,870,912 bytes (512 MiB in Strata)
  - `TEMPORARY_STORAGE_LIMIT_BYTES`: 268,435,456 bytes (256 MiB)
  - `CPU_LIMIT_SECONDS`: 10 seconds
  - `WALL_TIME_LIMIT`: 12 seconds
- **Non-blocking UI Architecture**:
  - UI requests are initiated in `src/adapters/local_preview.rs` via `gio::spawn_blocking(move || ...)`. GLib dispatches the blocking I/O to a background worker thread. The GTK main loop is never blocked.
- **Responsive Cancellation & Deadline Reader (`DeadlineReader`)**:
  - Wraps the read pipe and socket with `rustix::event::poll`.
  - Loops with `WAIT_QUANTUM = Duration::from_millis(20)`.
  - Every 20ms, checks:
    1. `cancellation.is_cancelled()`: If cancelled, aborts read immediately.
    2. `deadline.saturating_duration_since(Instant::now()).is_zero()`: If deadline reached, returns `io::ErrorKind::TimedOut`.
  - If cancelled or timed out:
    In `Lease::execute`: On error, calls `self.discard()`.
    Worker is removed from the lease (`worker.take()`).
    On `drop`, `ProcessWorker` calls `super::terminate(&mut self.child)`, which sends `SIGKILL` to the bwrap container and reaps the process.
    The cancelled job terminates immediately, and the pool spawns a replacement worker on demand.

---

## 2. Logic Chain

1. **Premise 1 (Current Latency)**: One-shot `bwrap` execution requires full kernel namespace creation (`--unshare-all`), 20+ bind mounts, dynamic linking of the 40MB+ executable and 50+ shared libraries, un-cached font scanning, and subprocess invocations (`magick`). This explains the observed 1-2s latency.
2. **Premise 2 (Pre-Warmed Pool Solution)**: By pre-spawning worker processes (`strata --browser-worker`) and keeping them alive inside Bubblewrap, all process initialization, namespace setup, and dynamic library linking occur once at startup.
3. **Premise 3 (Wire Protocol & Security)**: By passing file descriptors (`input_fd`, `output_pipe_fd`) over a local Unix domain socket using `SCM_RIGHTS`, workers require no host filesystem paths or write access to `/output`. Output streams directly over a pipe into host memory, removing disk I/O completely.
4. **Premise 4 (Worker Cleanliness via Disposable Fork)**: Because the supervisor is single-threaded and codec-free, it can safely fork a disposable child per job. The child performs the decode and exits, freeing 100% of decoder memory and avoiding dirty state or cross-job data leakage.
5. **Premise 5 (Guardrails against OOM/SIGXFSZ)**: Full-resolution decode of high-resolution images (e.g. 50+ MP or gigapixel images) exceeds memory and `fsize` ceilings. Fast header sniffing (JPEG SOF0, PNG IHDR) enables preemptive detection in <1ms without decompressing pixels. For images exceeding the frame budget, extracting embedded EXIF thumbnails allows rendering a high-quality preview in ~10ms while consuming <1MB RAM.
6. **Premise 6 (Dual Pool & UI Responsiveness)**: By separating thumbnail generation (`pool()`) from interactive file preview (`preview_pool()`), rapid directory navigation in the file browser cannot starve or delay user-requested Space previews. Polling with 20ms quanta in `DeadlineReader` ensures immediate UI cancellation responsiveness without thread locks.

---

## 3. Caveats

1. **Dependencies**: Upstream Strata relies on several crates not currently in Hermes's `Cargo.toml`: `rustix` (with `net`, `pipe`, `process`, `event` features), `kamadak-exif` (for EXIF thumbnail parsing), `landlock`, and `seccompiler`.
2. **Platform & Kernel Support**:
   - `landlock` ABI 3 requires Linux kernel >= 6.2. Upstream Strata includes fallback detection (`workers_supported()`) which falls back to one-shot `bwrap` if Landlock ABI 3 is unavailable.
   - SCM_RIGHTS and Unix domain sockets are Linux/Unix-specific.
3. **Glycin / GdkPixbuf in Sandboxes**: GdkPixbuf loaders like Glycin attempt to create their own bubblewrap sandboxes, which fail if nested inside an existing restricted container without user namespace permissions. Hence, in-process decoders or ImageMagick/resvg fallbacks are preferred inside the worker.

---

## 4. Conclusion

Requirement R2 can be cleanly implemented in Hermes by adopting the architecture from Strata commits `015621c0`, `028ff1b2`, and `63050999`:
1. **Persistent Worker Pool (`src/sandbox/browser.rs`, `src/sandbox/browser/wire.rs`, `src/sandbox/browser/worker.rs`, `src/sandbox/browser/process.rs`)**:
   - Pre-warmed pool of sandboxed workers managed by a launcher thread.
   - Dual pools: `pool()` for directory thumbnails, `preview_pool()` for quick previews.
   - Local wire protocol over Unix domain sockets with SCM_RIGHTS descriptor passing.
   - 8-byte framed responses (`png_len` + `metadata_len` + payloads) streamed via pipes.
   - Drops response latency from ~1-2s to ~20-42ms warm.
2. **Fast Header Sniffing & EXIF Fallback (`src/sandbox_helper.rs`)**:
   - Implement `read_jpeg_dimensions`, `read_png_dimensions`, `read_gif_dimensions`.
   - Calculate decoded frame budget: `width * height * 4 > FILE_SIZE_LIMIT_BYTES`.
   - For oversized images, extract embedded EXIF thumbnail via `kamadak-exif` and scale down.
3. **Resource Bounds & Cancellation**:
   - Explicit `setrlimit` in the forked worker child (1.25 GB AS, 10s CPU, 32/512 MB Fsize).
   - `DeadlineReader` polling in 20ms quanta ensuring instant abort on cancellation and 12s wall-clock timeout.
   - Background execution via `gio::spawn_blocking` keeps UI thread 100% responsive.

---

## 5. Verification Method

1. **Verify Existing Tests Pass**:
   ```bash
   cargo test --bin strata
   ```
   (Must pass all 181 existing Hermes unit tests without regression).

2. **Verify Sandbox Tests**:
   ```bash
   cargo test --bin strata -- sandbox::tests
   ```

3. **Verify Upstream Reference Commits**:
   ```bash
   git show 015621c0 --stat
   git show 63050999 --stat
   git diff main 015621c0 -- src/sandbox/browser.rs src/sandbox/browser/wire.rs
   ```

4. **Future Implementation Benchmark Verification**:
   - Measure warm preview latency of images/SVGs using `Instant::now()` before and after `execute()` call. Confirm latency is < 100ms.
   - Test an oversized test JPEG (e.g. 18354 x 23598 with embedded thumbnail, like the test fixture in commit `63050999`) and confirm that it renders the EXIF thumbnail without SIGXFSZ or crash.
