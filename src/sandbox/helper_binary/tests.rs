use super::*;
use std::{
    fs,
    io::Read,
    os::fd::OwnedFd,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    os::unix::net::UnixStream,
    path::PathBuf,
    process::Stdio,
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

struct FixtureDirectory(PathBuf);

impl FixtureDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hermes-helper-binary-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("expected test value from SystemTime::now() .duration_since(UNIX_EPOCH)")
                .as_nanos()
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("expected test value from fs::DirBuilder::new().mode(0o700).create(&path)");
        Self(path)
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn arguments(command: &Command) -> Vec<String> {
    command
        .get_args()
        .map(|value| value.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn normal_helper_binding_keeps_the_existing_path_without_copying_data() {
    let executable =
        std::env::current_exe().expect("expected test value from std::env::current_exe()");
    let mut command = Command::new("bwrap");
    bind(&mut command, &executable)
        .expect("expected test value from bind(&mut command, &executable)");
    assert_eq!(
        arguments(&command),
        [
            "--ro-bind",
            executable
                .to_str()
                .expect("expected test value from executable.to_str()"),
            "/app/strata"
        ]
    );
}

#[test]
fn an_existing_literal_deleted_suffix_is_not_replaced_with_the_running_binary() {
    let directory = FixtureDirectory::new();
    let executable = directory.0.join("helper (deleted)");
    fs::write(&executable, b"not the running binary").expect("write generated fixture");
    let mut command = Command::new("bwrap");
    bind(&mut command, &executable)
        .expect("expected test value from bind(&mut command, &executable)");
    assert_eq!(
        arguments(&command),
        [
            "--ro-bind",
            executable
                .to_str()
                .expect("expected test value from executable.to_str()"),
            "/app/strata"
        ]
    );
}

#[test]
fn unrelated_missing_deleted_helper_does_not_pin_the_running_binary() {
    let directory = FixtureDirectory::new();
    let executable = directory.0.join("missing (deleted)");
    let mut command = Command::new("bwrap");
    bind(&mut command, &executable)
        .expect("expected test value from bind(&mut command, &executable)");
    assert_eq!(
        arguments(&command),
        [
            "--ro-bind",
            executable
                .to_str()
                .expect("expected test value from executable.to_str()"),
            "/app/strata"
        ]
    );
}

#[test]
#[ignore = "Requires bubblewrap/FFmpeg; replaces an isolated test executable and reads generated audio metadata/artwork only"]
fn deleted_running_binary_is_pinned_without_using_replacement() {
    const CHILD_MODE: &str = "HERMES_HELPER_BINARY_TEST_CHILD";
    let module_name = module_path!()
        .split_once("::")
        .expect("expected test value from module_path!().split_once(\"::\")")
        .1;
    let test_name =
        format!("{module_name}::deleted_running_binary_is_pinned_without_using_replacement");
    if let Ok(mode) = std::env::var(CHILD_MODE) {
        let original_path =
            std::env::current_exe().expect("expected test value from std::env::current_exe()");
        let directory = PathBuf::from(std::env::var_os("HERMES_HELPER_BINARY_TEST_ROOT").expect(
            "expected test value from std::env::var_os(\"HERMES_HELPER_BINARY_TEST_ROOT\")",
        ));
        assert_eq!(original_path, directory.join(&mode));
        assert_eq!(
            fs::metadata(&directory)
                .expect("expected test value from fs::metadata(&directory)")
                .mode()
                & 0o777,
            0o700
        );
        let original_identity = fs::metadata(&original_path)
            .expect("expected test value from fs::metadata(&original_path)");
        fs::remove_file(&original_path).expect("remove generated fixture");
        if mode == "replaced" {
            fs::write(&original_path, b"#!/bin/sh\nexit 97\n").expect("write generated fixture");
            fs::set_permissions(&original_path, fs::Permissions::from_mode(0o700)).expect("expected test value from fs::set_permissions(&original_path, fs::Permissions::from_mode(0o700))");
            let replacement_identity = fs::metadata(&original_path)
                .expect("expected test value from fs::metadata(&original_path)");
            assert_ne!(replacement_identity.ino(), original_identity.ino());
        }
        let deleted_path =
            std::env::current_exe().expect("expected test value from std::env::current_exe()");
        assert!(deleted_path.as_os_str().as_bytes().ends_with(b" (deleted)"));
        assert!(!deleted_path.exists());
        let mut command = Command::new("bwrap");
        command.args([
            "--unshare-all",
            "--die-with-parent",
            "--new-session",
            "--clearenv",
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
            "/etc/ld.so.cache",
            "/etc/ld.so.cache",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/app",
        ]);
        bind(&mut command, &deleted_path)
            .expect("expected test value from bind(&mut command, &deleted_path)");
        let bind_arguments = arguments(&command);
        let descriptor = &bind_arguments[bind_arguments
            .iter()
            .position(|argument| argument == "--ro-bind-data")
            .expect("expected test value from bind_arguments .iter() .position(|argument| argument == \"--ro-bind-data\")")
            + 1];
        let pinned_identity = fs::metadata(format!("/proc/self/fd/{descriptor}")).expect(
            "expected test value from fs::metadata(format!(\"/proc/self/fd/{descriptor}\"))",
        );
        assert_eq!(pinned_identity.dev(), original_identity.dev());
        assert_eq!(pinned_identity.ino(), original_identity.ino());
        assert!(
            arguments(&command)
                .windows(3)
                .any(|args| args == ["--perms", "0700", "--ro-bind-data"])
        );
        command.args([
            "--",
            "/usr/bin/sh",
            "-c",
            "test \"$(/usr/bin/stat -c %a /app/strata)\" = 700 && exec /app/strata --list",
        ]);
        for _ in 0..2 {
            let output = command
                .output()
                .expect("expected test value from command.output()");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains(&test_name));
        }
        verify_deleted_helper_wire(&directory, &deleted_path, &bind_arguments);
        return;
    }
    let directory = FixtureDirectory::new();
    for mode in ["unlinked", "replaced"] {
        let copied_executable = directory.0.join(mode);
        fs::copy(std::env::current_exe().expect("expected test value from std::env::current_exe()"), &copied_executable).expect("expected test value from fs::copy(std::env::current_exe().unwrap(), &copied_executable)");
        let output = Command::new(&copied_executable)
            .args(["--exact", &test_name, "--ignored", "--nocapture"])
            .env(CHILD_MODE, mode)
            .env("HERMES_HELPER_BINARY_TEST_ROOT", &directory.0)
            .output()
            .expect("expected test value from Command::new(&copied_executable) .args([\"--exact\", &test_name, \"--ignored\", \"--nocapture\"]) .env(CHILD_MODE, mode) .env(\"HERMES...");
        assert!(
            output.status.success(),
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
}

fn verify_deleted_helper_wire(directory: &Path, executable: &Path, base_arguments: &[String]) {
    use crate::sandbox::browser::wire::{self, Operation, Response};
    let (mut host, worker) =
        UnixStream::pair().expect("expected test value from UnixStream::pair()");
    host.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("expected test value from host.set_read_timeout(Some(Duration::from_secs(10)))");
    let mut command = Command::new("bwrap");
    let prefix_length = base_arguments
        .iter()
        .position(|arg| arg == "--perms")
        .expect(
            "expected test value from base_arguments .iter() .position(|arg| arg == \"--perms\")",
        );
    command.args(&base_arguments[..prefix_length]);
    bind(&mut command, executable)
        .expect("expected test value from bind(&mut command, executable)");
    let probe = format!(
        "{}::deleted_helper_wire_worker_fixture",
        module_path!()
            .split_once("::")
            .expect("expected test value from module_path!().split_once(\"::\")")
            .1
    );
    command.args([
        "--setenv",
        "HERMES_HELPER_BINARY_WIRE_PROBE",
        "1",
        "--",
        "/app/strata",
        "--exact",
        &probe,
        "--ignored",
        "--nocapture",
    ]);
    command.stdin(Stdio::from(OwnedFd::from(worker)));
    command.stdout(Stdio::null());
    let mut child = command
        .spawn()
        .expect("expected test value from command.spawn()");
    let cover = directory.join("cover.png");
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 64, 32).expect("expected test value from gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 64, 32)");
    pixels.fill(0x285080ff);
    pixels
        .savev(&cover, "png", &[])
        .expect("expected test value from pixels.savev(&cover, \"png\", &[])");
    let track = directory.join("sample.flac");
    assert!(
        Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=stereo",
                "-i"
            ])
            .arg(&cover)
            .args([
                "-t",
                "0.05",
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:a",
                "flac",
                "-c:v",
                "png",
                "-threads",
                "1",
                "-disposition:v",
                "attached_pic"
            ])
            .arg(&track)
            .status()
            .expect("expected test value from Command::new(\"ffmpeg\") .args([ \"-v\", \"error\", \"-nostdin\", \"-y\", \"-f\", \"lavfi\", \"-i\", \"anullsrc=r=48000:cl=stereo\", \"-i\" ]) .arg...")
            .success()
    );
    let input = File::open(track).expect("expected test value from File::open(track)");
    for operation in [
        Operation::PreviewAudioMetadata,
        Operation::PreviewAudioArtwork,
    ] {
        let (mut reader, output) =
            UnixStream::pair().expect("expected test value from UnixStream::pair()");
        reader
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect(
                "expected test value from reader .set_read_timeout(Some(Duration::from_secs(10)))",
            );
        wire::send(&host, &input, &output, operation)
            .expect("expected test value from wire::send(&host, &input, &output, operation)");
        drop(output);
        let response = Response::read(&mut reader)
            .expect("expected test value from Response::read(&mut reader)");
        let mut completion = [0];
        host.read_exact(&mut completion)
            .expect("expected test value from host.read_exact(&mut completion)");
        assert_eq!(completion, [1], "{:?}", response.error_message());
        if operation == Operation::PreviewAudioMetadata {
            assert!(serde_json::from_slice::<serde_json::Value>(&response.png).is_ok());
        } else {
            assert!(response.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        }
    }
    drop(host);
    assert!(
        child
            .wait()
            .expect("expected test value from child.wait()")
            .success()
    );
}

#[test]
#[ignore = "Internal socket fixture; launched only by the deleted-helper regression"]
fn deleted_helper_wire_worker_fixture() {
    assert_eq!(
        std::env::var("HERMES_HELPER_BINARY_WIRE_PROBE").as_deref(),
        Ok("1")
    );
    crate::sandbox::browser::run()
        .expect("expected test value from crate::sandbox::browser::run()");
}
