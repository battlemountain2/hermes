// SPDX-License-Identifier: GPL-3.0-or-later
#![allow(dead_code)]

pub mod sniffer;
pub mod status_bar;
pub mod wire_protocol;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Isolated test environment managing a unique temporary directory.
pub struct TestEnv {
    dir: PathBuf,
}

impl TestEnv {
    pub fn new() -> Self {
        let count = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("hermes-e2e-{pid}-{timestamp}-{count}"));
        fs::create_dir_all(&dir).expect("failed to create temp dir for test");
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    pub fn file_path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub fn write_file(&self, name: &str, content: &[u8]) -> PathBuf {
        let path = self.file_path(name);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&path, content).expect("failed to write test fixture file");
        path
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[allow(dead_code)]
pub struct HelperOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// Invokes the compiled binary with `--preview-helper <operation> <input> <output> <value>`
pub fn run_preview_helper(
    operation: &str,
    input: &Path,
    output: &Path,
    value: i32,
) -> HelperOutput {
    let binary = env!("CARGO_BIN_EXE_strata");
    let cmd_output = Command::new(binary)
        .arg("--preview-helper")
        .arg(operation)
        .arg(input)
        .arg(output)
        .arg(value.to_string())
        .output()
        .expect("failed to execute strata binary");

    HelperOutput {
        status: cmd_output.status,
        stdout: String::from_utf8_lossy(&cmd_output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&cmd_output.stderr).to_string(),
    }
}

/// Checks whether an operation is supported by the binary's `--preview-helper` entry point.
pub fn is_op_supported(operation: &str) -> bool {
    let env = TestEnv::new();
    let dummy_in = env.write_file("dummy.in", b"");
    let dummy_out = env.file_path("dummy.out");
    let res = run_preview_helper(operation, &dummy_in, &dummy_out, 1);
    !res.stderr.contains("Unknown preview helper operation")
}

/// Verifies whether a byte slice contains a valid PNG signature (`\x89PNG\r\n\x1a\n`).
pub fn is_valid_png(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}
