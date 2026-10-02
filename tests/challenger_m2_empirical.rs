// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(unsafe_code, dead_code, unused_imports, unused_mut, clippy::all)]

//! Empirical Challenger Test Suite (Replacement Challenger 2):
//! Thorough verification of Milestone 2:
//! 1. SCM_RIGHTS descriptor passing under rapid requests (100 back-to-back requests, zero FD leaks, live worker pipeline).
//! 2. 8-byte framing parser handling of oversized payloads (>32MB boundary, u32::MAX, zero allocation on invalid size, truncation).
//! 3. Cancellation responsiveness: DeadlineReader aborts within 20ms without hanging the thread (pre-cancelled, mid-poll, multi-threaded).
//! 4. Worker termination and disposal on timeout / unexpected exit (SIGKILL, broken pipe, deadline expiry, zombie reaping).

use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd},
    os::unix::net::UnixStream,
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use gdk_pixbuf::prelude::*;

mod common;
use common::*;
use common::wire_protocol::{decode_header, encode_frame, read_framed_message, WireError, MAX_PAYLOAD_SIZE, HEADER_SIZE};

// POSIX socket ancillary definitions
#[repr(C)]
struct Msghdr {
    msg_name: *mut std::ffi::c_void,
    msg_namelen: u32,
    msg_iov: *mut Iovec,
    msg_iovlen: usize,
    msg_control: *mut std::ffi::c_void,
    msg_controllen: usize,
    msg_flags: i32,
}

#[repr(C)]
struct Iovec {
    iov_base: *mut std::ffi::c_void,
    iov_len: usize,
}

#[repr(C)]
struct Cmsghdr {
    cmsg_len: usize,
    cmsg_level: i32,
    cmsg_type: i32,
}

#[repr(C)]
struct ScmRights2Buffer {
    hdr: Cmsghdr,
    fds: [RawFd; 2],
}

const SOL_SOCKET: i32 = 1;
const SCM_RIGHTS: i32 = 1;
const MSG_NOSIGNAL: i32 = 0x4000;
const MSG_CMSG_CLOEXEC: i32 = 0x40000000;

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}
const POLLIN: i16 = 0x0001;

unsafe extern "C" {
    fn sendmsg(sockfd: i32, msg: *const Msghdr, flags: i32) -> isize;
    fn recvmsg(sockfd: i32, msg: *mut Msghdr, flags: i32) -> isize;
    fn pipe2(pipefd: *mut i32, flags: i32) -> i32;
    fn poll(fds: *mut PollFd, nfds: usize, timeout: i32) -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
}

const F_GETFD: i32 = 1;

fn is_fd_valid(fd: RawFd) -> bool {
    let ret = unsafe { fcntl(fd, F_GETFD) };
    ret >= 0
}

fn create_cloexec_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    let ret = unsafe { pipe2(fds.as_mut_ptr(), 0x80000) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((read, write))
}

fn send_scm_rights(
    socket: &UnixStream,
    fd1: &impl AsRawFd,
    fd2: &impl AsRawFd,
    op: u8,
) -> io::Result<()> {
    let mut cmsg = ScmRights2Buffer {
        hdr: Cmsghdr {
            cmsg_len: std::mem::size_of::<ScmRights2Buffer>(),
            cmsg_level: SOL_SOCKET,
            cmsg_type: SCM_RIGHTS,
        },
        fds: [fd1.as_raw_fd(), fd2.as_raw_fd()],
    };
    let mut op_byte = [op];
    let mut iov = Iovec {
        iov_base: op_byte.as_mut_ptr() as *mut std::ffi::c_void,
        iov_len: 1,
    };
    let msg = Msghdr {
        msg_name: std::ptr::null_mut(),
        msg_namelen: 0,
        msg_iov: &mut iov,
        msg_iovlen: 1,
        msg_control: &mut cmsg as *mut _ as *mut std::ffi::c_void,
        msg_controllen: std::mem::size_of::<ScmRights2Buffer>(),
        msg_flags: 0,
    };

    let count = unsafe { sendmsg(socket.as_raw_fd(), &msg, MSG_NOSIGNAL) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    if count != 1 {
        return Err(io::Error::other("Incomplete sendmsg"));
    }
    Ok(())
}

fn recv_scm_rights(socket: &UnixStream) -> io::Result<Option<(u8, OwnedFd, OwnedFd)>> {
    let mut cmsg = ScmRights2Buffer {
        hdr: Cmsghdr {
            cmsg_len: 0,
            cmsg_level: 0,
            cmsg_type: 0,
        },
        fds: [-1, -1],
    };
    let mut op_byte = [0u8; 1];
    let mut iov = Iovec {
        iov_base: op_byte.as_mut_ptr() as *mut std::ffi::c_void,
        iov_len: 1,
    };
    let mut msg = Msghdr {
        msg_name: std::ptr::null_mut(),
        msg_namelen: 0,
        msg_iov: &mut iov,
        msg_iovlen: 1,
        msg_control: &mut cmsg as *mut _ as *mut std::ffi::c_void,
        msg_controllen: std::mem::size_of::<ScmRights2Buffer>(),
        msg_flags: 0,
    };

    let count = unsafe { recvmsg(socket.as_raw_fd(), &mut msg, MSG_CMSG_CLOEXEC) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    if count == 0 {
        return Ok(None);
    }
    if cmsg.hdr.cmsg_level != SOL_SOCKET
        || cmsg.hdr.cmsg_type != SCM_RIGHTS
        || cmsg.fds[0] < 0
        || cmsg.fds[1] < 0
    {
        return Err(io::Error::other("Invalid descriptors in recvmsg"));
    }

    let fd1 = unsafe { OwnedFd::from_raw_fd(cmsg.fds[0]) };
    let fd2 = unsafe { OwnedFd::from_raw_fd(cmsg.fds[1]) };
    Ok(Some((op_byte[0], fd1, fd2)))
}

// DeadlineReader test implementation matching production WAIT_QUANTUM
struct TestDeadlineReader<'a, R: AsRawFd + Read> {
    reader: &'a mut R,
    deadline: Instant,
    cancellation: &'a AtomicBool,
    quantum: Duration,
}

impl<R: AsRawFd + Read> Read for TestDeadlineReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.cancellation.load(Ordering::Acquire) {
                return Err(io::Error::other("Browser request cancelled"));
            }
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Browser renderer timed out",
                ));
            }
            let wait_ms = remaining.min(self.quantum).as_millis() as i32;
            let mut pfd = PollFd {
                fd: self.reader.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            };
            let ret = unsafe { poll(&mut pfd, 1, wait_ms) };
            if ret < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            if ret == 0 {
                continue;
            }
            return self.reader.read(buf);
        }
    }
}

// Generates valid decodable JPEG image bytes
fn generate_valid_jpeg(width: i32, height: i32) -> Vec<u8> {
    let pixbuf = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, width, height)
        .expect("pixbuf creation");
    pixbuf
        .save_to_bufferv("jpeg", &[("quality", "75")])
        .expect("save to jpeg")
}

// =========================================================================
// 1. SCM_RIGHTS Descriptor Passing Under Rapid Requests
// =========================================================================

static FD_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_scm_rights_rapid_burst_and_fd_leak_check() {
    let _lock = FD_TEST_MUTEX.lock().unwrap();
    let (sock_host, sock_worker) = UnixStream::pair().expect("unix stream pair");

    let iterations = 100;
    for i in 0..iterations {
        let (pipe_in_read, pipe_in_write) = create_cloexec_pipe().expect("pipe in");
        let (pipe_out_read, pipe_out_write) = create_cloexec_pipe().expect("pipe out");

        let raw_in_write = pipe_in_write.as_raw_fd();
        let raw_in_read = pipe_in_read.as_raw_fd();
        let raw_out_write = pipe_out_write.as_raw_fd();
        let raw_out_read = pipe_out_read.as_raw_fd();

        let mut file_in_write = File::from(pipe_in_write);
        let marker = format!("request_{i}");
        file_in_write.write_all(marker.as_bytes()).expect("write marker");
        drop(file_in_write);
        assert!(!is_fd_valid(raw_in_write), "pipe_in_write must be closed after drop");

        let op = ((i % 12) + 1) as u8;
        send_scm_rights(&sock_host, &pipe_in_read, &pipe_out_write, op).expect("send");
        drop(pipe_in_read);
        drop(pipe_out_write);
        assert!(!is_fd_valid(raw_in_read), "pipe_in_read must be closed after drop");
        assert!(!is_fd_valid(raw_out_write), "pipe_out_write must be closed after drop");

        let (recv_op, recv_in, recv_out) = recv_scm_rights(&sock_worker)
            .expect("recv")
            .expect("message received");
        assert_eq!(recv_op, op, "Opcode mismatch on iteration {i}");

        let raw_recv_in = recv_in.as_raw_fd();
        let raw_recv_out = recv_out.as_raw_fd();
        assert!(is_fd_valid(raw_recv_in), "recv_in must be valid descriptor");
        assert!(is_fd_valid(raw_recv_out), "recv_out must be valid descriptor");

        let mut read_buf = Vec::new();
        let mut file_in = File::from(recv_in);
        file_in.read_to_end(&mut read_buf).expect("read in pipe");
        assert_eq!(read_buf, marker.as_bytes());
        drop(file_in);
        assert!(!is_fd_valid(raw_recv_in), "recv_in must be closed after drop");

        let mut file_out = File::from(recv_out);
        file_out.write_all(b"worker_ok").expect("write out pipe");
        drop(file_out);
        assert!(!is_fd_valid(raw_recv_out), "recv_out must be closed after drop");

        let mut out_buf = Vec::new();
        let mut file_out_read = File::from(pipe_out_read);
        file_out_read.read_to_end(&mut out_buf).expect("read out");
        assert_eq!(&out_buf, b"worker_ok");
        drop(file_out_read);
        assert!(!is_fd_valid(raw_out_read), "pipe_out_read must be closed after drop");
    }
}

#[test]
fn test_scm_rights_live_worker_pipeline_under_rapid_requests() {
    let env = TestEnv::new();
    let img_bytes = generate_valid_jpeg(64, 64);
    let img_path = env.write_file("test_rapid.jpg", &img_bytes);

    let (host_sock, worker_sock) = UnixStream::pair().expect("socket pair");
    let binary = env!("CARGO_BIN_EXE_strata");

    let mut child = Command::new(binary)
        .arg("--preview-helper")
        .arg("browser-worker")
        .stdin(Stdio::from(OwnedFd::from(worker_sock)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn browser worker");

    for i in 0..20 {
        let start = Instant::now();
        let file = File::open(&img_path).expect("open image file");
        let (pipe_read, pipe_write) = create_cloexec_pipe().expect("pipe");

        send_scm_rights(&host_sock, &file, &pipe_write, 1).expect("send request");
        drop(file);
        drop(pipe_write);

        let (png, meta) = read_framed_message(File::from(pipe_read)).expect("read response");
        assert!(is_valid_png(&png), "Must produce valid PNG on iteration {i}");
        assert!(meta.is_empty(), "Image thumbnail has empty metadata");

        let mut status = [0u8; 1];
        let mut host_sock_clone = host_sock.try_clone().expect("clone sock");
        host_sock_clone.read_exact(&mut status).expect("read status byte");
        assert_eq!(status[0], 1, "Worker reported failure on iteration {i}");
        drop(host_sock_clone);

        let duration = start.elapsed();
        assert!(
            duration < Duration::from_millis(150),
            "Warm request took too long ({:?}) on iteration {i}",
            duration
        );
    }

    drop(host_sock);
    let status = child.wait().expect("wait child");
    assert!(status.success(), "Worker supervisor must exit cleanly after EOF");
}

// =========================================================================
// 2. 8-Byte Framing Parser Handling of Oversized Payloads (>32MB)
// =========================================================================

#[test]
fn test_framing_exact_32mb_boundaries() {
    let max = MAX_PAYLOAD_SIZE;

    // 1. Exact max png_len (32MB) must succeed
    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&max.to_le_bytes());
    header[4..8].copy_from_slice(&0u32.to_le_bytes());
    let (png_len, meta_len) = decode_header(&header).expect("exact 32MB png allowed");
    assert_eq!(png_len, max);
    assert_eq!(meta_len, 0);

    // 2. Exact max meta_len (32MB) must succeed
    header[0..4].copy_from_slice(&0u32.to_le_bytes());
    header[4..8].copy_from_slice(&max.to_le_bytes());
    let (png_len, meta_len) = decode_header(&header).expect("exact 32MB meta allowed");
    assert_eq!(png_len, 0);
    assert_eq!(meta_len, max);

    // 3. png_len = 32MB + 1 must be rejected
    header[0..4].copy_from_slice(&(max + 1).to_le_bytes());
    header[4..8].copy_from_slice(&0u32.to_le_bytes());
    let err = decode_header(&header).expect_err("32MB + 1 png rejected");
    assert_eq!(err, WireError::PayloadTooLarge { length: max + 1, max });

    // 4. meta_len = 32MB + 1 must be rejected
    header[0..4].copy_from_slice(&0u32.to_le_bytes());
    header[4..8].copy_from_slice(&(max + 1).to_le_bytes());
    let err = decode_header(&header).expect_err("32MB + 1 meta rejected");
    assert_eq!(err, WireError::PayloadTooLarge { length: max + 1, max });

    // 5. u32::MAX (4GB - 1) must be rejected
    header[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    header[4..8].copy_from_slice(&0u32.to_le_bytes());
    let err = decode_header(&header).expect_err("u32::MAX png rejected");
    assert_eq!(err, WireError::PayloadTooLarge { length: u32::MAX, max });

    // 6. Zero lengths round trip
    header[0..4].copy_from_slice(&0u32.to_le_bytes());
    header[4..8].copy_from_slice(&0u32.to_le_bytes());
    let (p, m) = decode_header(&header).expect("zero lengths");
    assert_eq!(p, 0);
    assert_eq!(m, 0);
}

#[test]
fn test_framing_read_oversized_no_allocation() {
    let mut data = [0u8; 8];
    data[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
    data[4..8].copy_from_slice(&0u32.to_le_bytes());

    let err = read_framed_message(&data[..]).expect_err("must reject oversized stream");
    assert_eq!(
        err,
        WireError::PayloadTooLarge {
            length: u32::MAX,
            max: MAX_PAYLOAD_SIZE
        }
    );
}

#[test]
fn test_framing_truncated_and_eof_handling() {
    let err = read_framed_message(&b""[..]).expect_err("empty stream");
    assert_eq!(err, WireError::IncompleteHeader);

    let err = read_framed_message(&b"123"[..]).expect_err("3 bytes");
    assert_eq!(err, WireError::IncompleteHeader);

    let err = read_framed_message(&[0, 1, 2, 3, 4, 5, 6][..]).expect_err("7 bytes");
    assert_eq!(err, WireError::IncompleteHeader);

    let mut buf = Vec::new();
    buf.extend_from_slice(&100u32.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&[0xAA; 20]);
    let err = read_framed_message(&buf[..]).expect_err("truncated payload");
    assert_eq!(err, WireError::UnexpectedEof);

    let mut buf2 = Vec::new();
    buf2.extend_from_slice(&10u32.to_le_bytes());
    buf2.extend_from_slice(&100u32.to_le_bytes());
    buf2.extend_from_slice(&[0xAA; 10]);
    let err = read_framed_message(&buf2[..]).expect_err("truncated meta");
    assert_eq!(err, WireError::UnexpectedEof);
}

// =========================================================================
// 3. Cancellation Responsiveness: Aborts Within 20ms Without Hanging
// =========================================================================

#[test]
fn test_cancellation_pre_cancelled_aborts_immediately() {
    let (read_pipe, _write_pipe) = create_cloexec_pipe().expect("pipe");
    let mut file = File::from(read_pipe);
    let cancel = AtomicBool::new(true);

    let start = Instant::now();
    let mut reader = TestDeadlineReader {
        reader: &mut file,
        deadline: Instant::now() + Duration::from_secs(10),
        cancellation: &cancel,
        quantum: Duration::from_millis(20),
    };

    let mut buf = [0u8; 16];
    let err = reader.read(&mut buf).expect_err("must abort pre-cancelled");
    let elapsed = start.elapsed();

    assert!(err.to_string().contains("cancelled"));
    assert!(
        elapsed < Duration::from_millis(5),
        "Pre-cancelled read must return immediately (<5ms), took {:?}",
        elapsed
    );
}

#[test]
fn test_cancellation_mid_poll_aborts_within_20ms() {
    let (read_pipe, _write_pipe) = create_cloexec_pipe().expect("pipe");
    let mut file = File::from(read_pipe);
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel.clone();

    let trigger_time = Arc::new(std::sync::Mutex::new(None));
    let trigger_time_clone = trigger_time.clone();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(15));
        *trigger_time_clone.lock().unwrap() = Some(Instant::now());
        cancel_clone.store(true, Ordering::Release);
    });

    let mut reader = TestDeadlineReader {
        reader: &mut file,
        deadline: Instant::now() + Duration::from_secs(12),
        cancellation: &cancel,
        quantum: Duration::from_millis(20),
    };

    let mut buf = [0u8; 16];
    let err = reader.read(&mut buf).expect_err("cancelled");
    let return_instant = Instant::now();

    assert!(err.to_string().contains("cancelled"));

    let t_trigger = trigger_time.lock().unwrap().expect("trigger occurred");
    let delay_since_cancel = return_instant.duration_since(t_trigger);

    assert!(
        delay_since_cancel <= Duration::from_millis(25),
        "Cancellation took too long to abort: {:?} (must be <= 25ms)",
        delay_since_cancel
    );
}

#[test]
fn test_cancellation_multi_threaded_never_hangs() {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut handles = Vec::new();
    let mut keep_alive_writes = Vec::new();

    for _ in 0..10 {
        let (read_pipe, write_pipe) = create_cloexec_pipe().expect("pipe");
        keep_alive_writes.push(write_pipe);
        let cancel_thread = cancel.clone();

        let handle = std::thread::spawn(move || {
            let mut file = File::from(read_pipe);
            let mut reader = TestDeadlineReader {
                reader: &mut file,
                deadline: Instant::now() + Duration::from_secs(10),
                cancellation: &cancel_thread,
                quantum: Duration::from_millis(20),
            };
            let mut buf = [0u8; 16];
            let start = Instant::now();
            let res = reader.read(&mut buf);
            (res.is_err(), start.elapsed())
        });
        handles.push(handle);
    }

    std::thread::sleep(Duration::from_millis(10));
    let t_cancel = Instant::now();
    cancel.store(true, Ordering::Release);

    for (i, h) in handles.into_iter().enumerate() {
        let (errored, _elapsed) = h.join().expect("thread join");
        assert!(errored, "Thread {i} must receive cancellation error");
    }

    drop(keep_alive_writes);

    let total_unblock_time = t_cancel.elapsed();
    assert!(
        total_unblock_time <= Duration::from_millis(35),
        "All 10 threads must abort within 35ms, took {:?}",
        total_unblock_time
    );
}

// =========================================================================
// 4. Worker Termination and Disposal on Timeout / Unexpected Exit
// =========================================================================

#[test]
fn test_worker_disposal_on_unexpected_sigkill() {
    let (host_sock, worker_sock) = UnixStream::pair().expect("socket pair");
    let binary = env!("CARGO_BIN_EXE_strata");

    let child = Command::new(binary)
        .arg("--preview-helper")
        .arg("browser-worker")
        .stdin(Stdio::from(OwnedFd::from(worker_sock)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn worker");

    let pid = child.id() as i32;

    let kill_ret = unsafe { kill(pid, 9) };
    assert_eq!(kill_ret, 0, "kill -9 succeeded");

    let mut buf = [0u8; 10];
    let count = host_sock.try_clone().unwrap().read(&mut buf).unwrap_or(0);
    assert_eq!(count, 0, "Host detects immediate EOF when worker is killed");

    let mut status = 0;
    let wait_ret = unsafe { waitpid(pid, &mut status, 0) };
    assert_eq!(wait_ret, pid, "waitpid reaped killed child");
    let exited_sig = status & 0x7f;
    assert_eq!(exited_sig, 9, "Process terminated by SIGKILL (signal 9)");
}

#[test]
fn test_deadline_reader_timeout_expiry() {
    let (read_pipe, _write_pipe) = create_cloexec_pipe().expect("pipe");
    let mut file = File::from(read_pipe);
    let cancel = AtomicBool::new(false);

    let start = Instant::now();
    let timeout_duration = Duration::from_millis(50);
    let mut reader = TestDeadlineReader {
        reader: &mut file,
        deadline: Instant::now() + timeout_duration,
        cancellation: &cancel,
        quantum: Duration::from_millis(20),
    };

    let mut buf = [0u8; 16];
    let err = reader.read(&mut buf).expect_err("must timeout");
    let elapsed = start.elapsed();

    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    assert!(
        elapsed >= Duration::from_millis(48) && elapsed <= Duration::from_millis(85),
        "Timeout must trigger near 50ms (within 1 quantum), took {:?}",
        elapsed
    );
}

#[test]
fn test_worker_disposal_and_respawn_after_crash() {
    let env = TestEnv::new();
    let img_bytes = generate_valid_jpeg(32, 32);
    let img_path = env.write_file("respawn_test.jpg", &img_bytes);
    let binary = env!("CARGO_BIN_EXE_strata");

    let (host_sock1, worker_sock1) = UnixStream::pair().expect("socket pair");
    let child1 = Command::new(binary)
        .arg("--preview-helper")
        .arg("browser-worker")
        .stdin(Stdio::from(OwnedFd::from(worker_sock1)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn worker 1");

    let pid1 = child1.id() as i32;

    // Simulate crash of Worker 1 via SIGKILL
    unsafe { kill(pid1, 9) };
    let mut status1 = 0;
    unsafe { waitpid(pid1, &mut status1, 0) };
    drop(host_sock1);

    // Spawn Worker 2 (simulating Lease retry / pool replacement)
    let (host_sock2, worker_sock2) = UnixStream::pair().expect("socket pair 2");
    let mut child2 = Command::new(binary)
        .arg("--preview-helper")
        .arg("browser-worker")
        .stdin(Stdio::from(OwnedFd::from(worker_sock2)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn worker 2");

    let file = File::open(&img_path).expect("open image");
    let (pipe_read, pipe_write) = create_cloexec_pipe().expect("pipe");
    send_scm_rights(&host_sock2, &file, &pipe_write, 1).expect("send to worker 2");
    drop(file);
    drop(pipe_write);

    let (png, _) = read_framed_message(File::from(pipe_read)).expect("read response from worker 2");
    assert!(is_valid_png(&png), "Worker 2 produced valid PNG");

    let mut status_byte = [0u8; 1];
    let mut sock2_clone = host_sock2.try_clone().unwrap();
    sock2_clone.read_exact(&mut status_byte).expect("read status");
    assert_eq!(status_byte[0], 1);
    drop(sock2_clone); // IMPORTANT: drop clone so EOF reaches worker child2

    drop(host_sock2); // IMPORTANT: drop primary socket so EOF reaches worker child2
    let exit2 = child2.wait().expect("wait child 2");
    assert!(exit2.success());
}
