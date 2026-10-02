// SPDX-License-Identifier: GPL-3.0-or-later

//! Only the single-threaded supervisor and its setup child use this
//! module. The GTK application and decoder runtimes must never fork through it.

use std::{fs, io};

#[expect(
    unsafe_code,
    reason = "POSIX process management syscalls (fork, waitpid, _exit, unshare)"
)]
unsafe extern "C" {
    fn fork() -> i32;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn _exit(status: i32) -> !;
}

pub(super) fn require_single_thread() -> io::Result<()> {
    if let Ok(entries) = fs::read_dir("/proc/self/task") {
        let count = entries.take(2).count();
        if count > 1 {
            return Err(io::Error::other(
                "Browser supervisor must be single-threaded",
            ));
        }
    }
    Ok(())
}

#[expect(
    unsafe_code,
    reason = "libc fork is required to reuse the codec-free supervisor image without an exec per file"
)]
pub(super) fn fork_child() -> io::Result<Option<i32>> {
    require_single_thread()?;
    // SAFETY: Only the single-threaded supervisor process calls this.
    // No shared mutexes or external background threads exist.
    let pid = unsafe { fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        Ok(None)
    } else {
        Ok(Some(pid))
    }
}

#[expect(
    unsafe_code,
    reason = "Calls POSIX waitpid to reap child processes"
)]
pub(super) fn wait(pid: i32) -> io::Result<bool> {
    let mut status = 0;
    loop {
        // SAFETY: waitpid called with valid pid pointer
        let ret = unsafe { waitpid(pid, &mut status, 0) };
        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        let exited = (status & 0x7f) == 0;
        let exit_code = (status >> 8) & 0xff;
        return Ok(exited && exit_code == 0);
    }
}

#[expect(
    unsafe_code,
    reason = "Calls POSIX _exit to terminate without running atexit handlers"
)]
pub(super) fn exit(result: Result<(), String>) -> ! {
    let status = match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("Browser decoder failed: {error}");
            1
        }
    };
    // SAFETY: _exit safely terminates the child process immediately
    unsafe { _exit(status) }
}
