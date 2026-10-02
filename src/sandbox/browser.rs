// SPDX-License-Identifier: GPL-3.0-or-later

//! Persistent pooled sandbox worker for rapid preview and thumbnail rendering.

use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Read},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::fs::MetadataExt,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        Condvar, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use super::{Cancellation, ParseOperation};
use wire::{Operation, Response};

mod process;
pub(crate) mod wire;
mod worker;
pub(crate) use worker::run;

const CACHE_ENTRIES: usize = 64;
const CACHE_TTL: Duration = Duration::from_secs(30);
const WAIT_QUANTUM: Duration = Duration::from_millis(20);
pub(crate) const MAX_WORKERS: usize = 16;
const DEFAULT_WORKER_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}
const POLLIN: i16 = 0x0001;

#[expect(
    unsafe_code,
    reason = "Standard POSIX poll and pipe2 syscalls for 20ms polling quanta"
)]
unsafe extern "C" {
    fn poll(fds: *mut PollFd, nfds: usize, timeout: i32) -> i32;
    fn pipe2(pipefd: *mut i32, flags: i32) -> i32;
}

#[expect(
    unsafe_code,
    reason = "Calls pipe2 syscall to create a unidirectional cloexec pipe"
)]
fn create_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: pipe2 called with valid pointer and O_CLOEXEC (0x80000)
    let ret = unsafe { pipe2(fds.as_mut_ptr(), 0x80000) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Valid descriptors returned by kernel
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: Valid descriptors returned by kernel
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((read, write))
}

struct DeadlineReader<'a, R: AsRawFd + Read> {
    reader: &'a mut R,
    deadline: Instant,
    cancellation: &'a Cancellation,
}

impl<R: AsRawFd + Read> Read for DeadlineReader<'_, R> {
    #[expect(
        unsafe_code,
        reason = "Calls POSIX poll syscall to poll reader file descriptor"
    )]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
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
            let wait_ms = remaining.min(WAIT_QUANTUM).as_millis() as i32;
            let mut pfd = PollFd {
                fd: self.reader.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            };
            // SAFETY: poll called with valid descriptor and nfds=1
            let ret = unsafe { poll(&mut pfd, 1, wait_ms) };
            if ret < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            if ret == 0 {
                // Quantum expired: loop to check cancellation
                continue;
            }
            return self.reader.read(buf);
        }
    }
}

pub(crate) fn default_worker_limit() -> usize {
    std::thread::available_parallelism()
        .map_or(2, |n| n.get().min(4))
        .min(MAX_WORKERS)
}

#[expect(dead_code, reason = "Pool worker limit query helper")]
pub(crate) fn worker_limit() -> usize {
    pool().limit.load(Ordering::Relaxed)
}

#[expect(dead_code, reason = "Pool worker limit setter helper")]
pub(crate) fn set_worker_limit(limit: usize) {
    let limit = limit.clamp(1, MAX_WORKERS);
    pool().limit.store(limit, Ordering::Relaxed);
    preview_pool().limit.store(limit, Ordering::Relaxed);
}

struct Worker {
    child: Child,
    socket: UnixStream,
    _created_at: Instant,
}

impl Drop for Worker {
    fn drop(&mut self) {
        terminate(&mut self.child);
    }
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

struct CacheEntry {
    path: PathBuf,
    mtime: u64,
    size: u64,
    operation: Operation,
    data: Vec<u8>,
    metadata: Option<Vec<u8>>,
    timestamp: Instant,
}

struct PoolState {
    count: usize,
    idle: VecDeque<(Worker, Instant)>,
}

struct Pool {
    limit: AtomicUsize,
    state: Mutex<PoolState>,
    changed: Condvar,
    cache: Mutex<Vec<CacheEntry>>,
}

impl Pool {
    fn new() -> Self {
        Self {
            limit: AtomicUsize::new(default_worker_limit()),
            state: Mutex::new(PoolState {
                count: 0,
                idle: VecDeque::new(),
            }),
            changed: Condvar::new(),
            cache: Mutex::new(Vec::with_capacity(CACHE_ENTRIES)),
        }
    }

    fn check_cache(
        &self,
        path: &Path,
        mtime: u64,
        size: u64,
        operation: Operation,
    ) -> Option<Vec<u8>> {
        self.check_cache_entry(path, mtime, size, operation)
            .map(|(data, _)| data)
    }

    fn check_cache_entry(
        &self,
        path: &Path,
        mtime: u64,
        size: u64,
        operation: Operation,
    ) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
        let mut cache = self.cache.lock().ok()?;
        cache.retain(|entry| entry.timestamp.elapsed() < CACHE_TTL);
        cache
            .iter()
            .find(|entry| {
                entry.path == path
                    && entry.mtime == mtime
                    && entry.size == size
                    && entry.operation == operation
            })
            .map(|entry| (entry.data.clone(), entry.metadata.clone()))
    }

    fn put_cache(
        &self,
        path: &Path,
        mtime: u64,
        size: u64,
        operation: Operation,
        data: Vec<u8>,
    ) {
        self.put_cache_entry(path, mtime, size, operation, data, None);
    }

    fn put_cache_entry(
        &self,
        path: &Path,
        mtime: u64,
        size: u64,
        operation: Operation,
        data: Vec<u8>,
        metadata: Option<Vec<u8>>,
    ) {
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() >= CACHE_ENTRIES {
                cache.remove(0);
            }
            cache.push(CacheEntry {
                path: path.to_owned(),
                mtime,
                size,
                operation,
                data,
                metadata,
                timestamp: Instant::now(),
            });
        }
    }

    fn acquire(&'static self, cancellation: &Cancellation) -> Result<Lease, String> {
        let mut state = self.state.lock().map_err(|e| e.to_string())?;

        loop {
            if cancellation.is_cancelled() {
                return Err("Preview cancelled".to_owned());
            }

            // Retire expired idle workers
            let now = Instant::now();
            while let Some((_, last_used)) = state.idle.front() {
                if now.duration_since(*last_used) >= DEFAULT_WORKER_IDLE_TIMEOUT {
                    let (worker, _) = state.idle.pop_front().unwrap();
                    drop(worker);
                    state.count = state.count.saturating_sub(1);
                } else {
                    break;
                }
            }

            // Reuse idle worker if available
            if let Some((worker, _)) = state.idle.pop_back() {
                return Ok(Lease {
                    worker: Some(worker),
                    pool: self,
                });
            }

            let limit = self.limit.load(Ordering::Relaxed);
            if state.count < limit {
                state.count += 1;
                drop(state);

                match spawn_worker() {
                    Ok(worker) => {
                        return Ok(Lease {
                            worker: Some(worker),
                            pool: self,
                        });
                    }
                    Err(error) => {
                        let mut state = self.state.lock().map_err(|e| e.to_string())?;
                        state.count = state.count.saturating_sub(1);
                        self.changed.notify_one();
                        return Err(format!("Unable to spawn sandbox worker: {error}"));
                    }
                }
            }

            // Wait for available worker
            let result = self
                .changed
                .wait_timeout(state, WAIT_QUANTUM)
                .map_err(|e| e.to_string())?;
            state = result.0;
        }
    }
}

struct Lease {
    worker: Option<Worker>,
    pool: &'static Pool,
}

impl Lease {
    fn discard(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            terminate(&mut worker.child);
        }
    }

    fn execute(
        &mut self,
        path: &Path,
        operation: Operation,
        cancellation: &Cancellation,
    ) -> Result<Response, String> {
        if cancellation.is_cancelled() {
            return Err("Preview cancelled".to_owned());
        }

        let mut attempts = 0;
        loop {
            attempts += 1;
            let worker = self
                .worker
                .as_mut()
                .ok_or_else(|| "No worker available".to_owned())?;

            let result = execute_single(worker, path, operation, cancellation);
            match result {
                Ok(response) => return Ok(response),
                Err(err) if attempts < 2 && should_retry(&err) => {
                    // Worker crashed: discard and retry once with fresh worker
                    self.discard();
                    match spawn_worker() {
                        Ok(fresh) => self.worker = Some(fresh),
                        Err(_) => return Err(err),
                    }
                }
                Err(err) => {
                    self.discard();
                    return Err(err);
                }
            }
        }
    }
}

fn should_retry(err: &str) -> bool {
    err.contains("Broken pipe")
        || err.contains("Unexpected EOF")
        || err.contains("Connection reset")
        || err.contains("Incomplete browser header")
}

fn execute_single(
    worker: &mut Worker,
    path: &Path,
    operation: Operation,
    cancellation: &Cancellation,
) -> Result<Response, String> {
    let file = File::open(path).map_err(|e| format!("Unable to open file: {e}"))?;
    let (pipe_read, pipe_write) =
        create_pipe().map_err(|e| format!("Unable to create pipe: {e}"))?;

    wire::send(&worker.socket, &file, &pipe_write, operation)
        .map_err(|e| format!("Unable to send wire request: {e}"))?;
    drop(pipe_write);

    let mut pipe_file = File::from(pipe_read);
    let mut deadline_reader = DeadlineReader {
        reader: &mut pipe_file,
        deadline: Instant::now() + Duration::from_secs(12),
        cancellation,
    };
    let response = Response::read(&mut deadline_reader)
        .map_err(|e| format!("Unable to read wire response: {e}"))?;

    let mut status_byte = [0u8; 1];
    let mut deadline_socket = DeadlineReader {
        reader: &mut worker.socket,
        deadline: Instant::now() + Duration::from_secs(12),
        cancellation,
    };
    deadline_socket
        .read_exact(&mut status_byte)
        .map_err(|e| format!("Unable to read completion byte: {e}"))?;

    if status_byte[0] != 1 {
        return Err("The sandboxed preview renderer failed".to_owned());
    }

    Ok(response)
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.pool.state.lock() {
            if let Some(worker) = self.worker.take() {
                state.idle.push_back((worker, Instant::now()));
            } else {
                state.count = state.count.saturating_sub(1);
            }
            self.pool.changed.notify_one();
        }
    }
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(Pool::new)
}

fn preview_pool() -> &'static Pool {
    static PREVIEW_POOL: OnceLock<Pool> = OnceLock::new();
    PREVIEW_POOL.get_or_init(Pool::new)
}

fn workers_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        std::process::Command::new("bwrap")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

fn spawn_worker() -> io::Result<Worker> {
    let (host_socket, worker_socket) = UnixStream::pair()?;
    let executable = std::env::current_exe()?;

    let mut command = std::process::Command::new("bwrap");
    command.args([
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin",
        "--setenv",
        "HOME",
        "/nonexistent",
        "--setenv",
        "XDG_CACHE_HOME",
        "/tmp/cache",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/app",
        "--dir",
        "/etc",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind-try",
        "/lib",
        "/lib",
        "--ro-bind-try",
        "/lib64",
        "/lib64",
        "--ro-bind-try",
        "/etc/fonts",
        "/etc/fonts",
        "--ro-bind-try",
        "/etc/ld.so.cache",
        "/etc/ld.so.cache",
        "--ro-bind-try",
        "/var/cache/fontconfig",
        "/var/cache/fontconfig",
        "--ro-bind-try",
        "/etc/ImageMagick-7",
        "/etc/ImageMagick-7",
        "--ro-bind-try",
        "/etc/ImageMagick-6",
        "/etc/ImageMagick-6",
        "--ro-bind",
    ]);
    command.arg(&executable).arg("/app/strata");
    command.args([
        "--",
        "/usr/bin/prlimit",
        "--as=1342177280",
        "--cpu=10",
        "--fsize=33554432",
        "--",
        "/app/strata",
        "--preview-helper",
        "browser-worker",
    ]);
    command.stdin(Stdio::from(OwnedFd::from(worker_socket)));
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());

    let child = command.spawn()?;
    Ok(Worker {
        child,
        socket: host_socket,
        _created_at: Instant::now(),
    })
}

fn map_preview_op(op: &ParseOperation) -> Option<Operation> {
    match op {
        ParseOperation::PreviewImage => Some(Operation::PreviewImage),
        ParseOperation::PreviewGeoTiff => Some(Operation::PreviewGeoTiff),
        ParseOperation::ThumbnailImage => Some(Operation::Image),
        ParseOperation::ThumbnailRaw => Some(Operation::Raw),
        ParseOperation::ThumbnailPdf => Some(Operation::Pdf),
        ParseOperation::PreviewModel => Some(Operation::PreviewModel),
        ParseOperation::PreviewArchiveCover => Some(Operation::PreviewArchiveCover),
        ParseOperation::PreviewSpreadsheet => Some(Operation::PreviewSpreadsheet),
        ParseOperation::PreviewAudioWaveform => Some(Operation::PreviewAudioWaveform),
        _ => None,
    }
}

pub(crate) fn preview(
    input: &Path,
    operation: &ParseOperation,
    cancellation: &Cancellation,
) -> Option<Result<(Vec<u8>, Option<Vec<u8>>), String>> {
    if !workers_supported() {
        return None;
    }
    let wire_op = map_preview_op(operation)?;
    let metadata = input.metadata().ok()?;
    let mtime = metadata.mtime() as u64;
    let size = metadata.size();

    if let Some(cached) = preview_pool().check_cache_entry(input, mtime, size, wire_op) {
        return Some(Ok(cached));
    }

    let mut lease = preview_pool().acquire(cancellation).ok()?;
    let result = lease.execute(input, wire_op, cancellation);
    match result {
        Ok(response) => {
            let meta = if response.metadata.is_empty() {
                None
            } else {
                Some(response.metadata)
            };
            preview_pool().put_cache_entry(
                input,
                mtime,
                size,
                wire_op,
                response.png.clone(),
                meta.clone(),
            );
            Some(Ok((response.png, meta)))
        }
        Err(err) => Some(Err(err)),
    }
}

pub(crate) fn thumbnail_parse(
    input: &Path,
    operation: ParseOperation,
    cancellation: &Cancellation,
) -> Option<Result<Vec<u8>, String>> {
    if !workers_supported() {
        return None;
    }
    let wire_op = map_preview_op(&operation)?;
    let metadata = input.metadata().ok()?;
    let mtime = metadata.mtime() as u64;
    let size = metadata.size();

    if let Some(cached_data) = pool().check_cache(input, mtime, size, wire_op) {
        return Some(Ok(cached_data));
    }

    let mut lease = pool().acquire(cancellation).ok()?;
    let result = lease.execute(input, wire_op, cancellation);
    match result {
        Ok(response) => {
            pool().put_cache(input, mtime, size, wire_op, response.png.clone());
            Some(Ok(response.png))
        }
        Err(err) => Some(Err(err)),
    }
}

#[cfg(test)]
mod tests;

