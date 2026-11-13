use std::{
    fs::File,
    io,
    os::{fd::AsRawFd, unix::ffi::OsStrExt, unix::process::CommandExt},
    path::Path,
    process::Command,
};

#[cfg(test)]
#[path = "helper_binary/tests.rs"]
mod tests;

const F_GETFD: i32 = 1;
const F_SETFD: i32 = 2;
const FD_CLOEXEC: i32 = 1;

#[expect(
    unsafe_code,
    reason = "Declares Linux fcntl and lseek; only descriptor flags and file offsets are changed in the forked launcher child"
)]
unsafe extern "C" {
    fn fcntl(fd: i32, operation: i32, ...) -> i32;
    fn lseek(fd: i32, offset: i64, whence: i32) -> i64;
}

#[expect(
    unsafe_code,
    reason = "Registers a pre_exec closure retaining an owned File; its child-only operations are async-signal-safe fcntl and lseek with checked return values"
)]
#[expect(
    clippy::undocumented_unsafe_blocks,
    reason = "The owned File remains captured for the Command lifetime; pre_exec only clears CLOEXEC and resets its offset using async-signal-safe syscalls, without allocation, locks, or shared Rust state"
)]
pub(super) fn bind(command: &mut Command, executable: &Path) -> io::Result<()> {
    if !executable.as_os_str().as_bytes().ends_with(b" (deleted)")
        || executable.try_exists()?
        || std::env::current_exe()? != executable
    {
        command.arg("--ro-bind").arg(executable).arg("/app/strata");
        return Ok(());
    }

    let file = File::open("/proc/self/exe").map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("Unable to pin deleted running helper binary: {error}"),
        )
    })?;
    if file.as_raw_fd() <= 2 {
        return Err(io::Error::other(
            "Unable to pin helper binary without an independent descriptor",
        ));
    }
    command
        .args(["--perms", "0700", "--ro-bind-data"])
        .arg(file.as_raw_fd().to_string())
        .arg("/app/strata");
    let prepare = move || {
        let descriptor = file.as_raw_fd();
        let flags = unsafe { fcntl(descriptor, F_GETFD) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { fcntl(descriptor, F_SETFD, flags & !FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { lseek(descriptor, 0, 0) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    };
    unsafe { command.pre_exec(prepare) };
    Ok(())
}
