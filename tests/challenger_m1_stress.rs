// SPDX-License-Identifier: GPL-3.0-or-later

//! Empirical Challenger 2 Stress Test Suite for Milestone 1:
//! - Cross-mount filesystem queries across ext4, tmpfs, devtmpfs, vfat, procfs, sysfs
//! - Path resolution boundary conditions (symlinks, broken symlinks, unicode, spaces, non-existent)
//! - GLib async event loop stress (500 rapid consecutive queries, zero deadlock/hang/crash)
//! - Simulation of window.rs event observer logic and selection aggregates

use gio::glib;
use gio::prelude::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::time::Instant;

mod common;
use common::status_bar::*;
use common::TestEnv;

/// Helper to run an async GIO query on a fresh or default GLib MainContext
fn query_gio_free_space_async(path: &Path) -> Result<Option<u64>, String> {
    let ctx = glib::MainContext::new();
    let file = gio::File::for_path(path);
    
    ctx.block_on(async move {
        match file.query_filesystem_info_future("filesystem::free", glib::Priority::DEFAULT).await {
            Ok(info) => {
                if info.has_attribute("filesystem::free") {
                    Ok(Some(info.attribute_uint64("filesystem::free")))
                } else {
                    Ok(None)
                }
            }
            Err(_) => Ok(None), // Expected for virtual/non-existent filesystems
        }
    })
}

// -------------------------------------------------------------------------
// 1. Cross-Mount Filesystem Query Tests
// -------------------------------------------------------------------------

#[test]
fn test_cross_mount_ext4_root_query() {
    let res = query_gio_free_space_async(Path::new("/")).expect("query root");
    assert!(res.is_some(), "Root filesystem (ext4) must report free space");
    assert!(res.unwrap() > 0, "Root free space must be > 0");
}

#[test]
fn test_cross_mount_tmpfs_tmp_query() {
    let res = query_gio_free_space_async(Path::new("/tmp")).expect("query tmp");
    assert!(res.is_some(), "Tmpfs (/tmp) must report free space");
    assert!(res.unwrap() > 0, "Tmpfs free space must be > 0");
}

#[test]
fn test_cross_mount_tmpfs_shm_query() {
    let res = query_gio_free_space_async(Path::new("/dev/shm")).expect("query /dev/shm");
    assert!(res.is_some(), "Tmpfs (/dev/shm) must report free space");
}

#[test]
fn test_cross_mount_devtmpfs_query() {
    let res = query_gio_free_space_async(Path::new("/dev")).expect("query /dev");
    // /dev is a devtmpfs mount; must not hang or panic
    let _ = res;
}

#[test]
fn test_cross_mount_vfat_efi_query() {
    let efi = Path::new("/boot/efi");
    if efi.exists() {
        let res = query_gio_free_space_async(efi).expect("query /boot/efi");
        // /boot/efi is vfat; must report free space if mounted
        assert!(res.is_some(), "VFAT /boot/efi must report free space when mounted");
    }
}

#[test]
fn test_cross_mount_procfs_no_hang() {
    let proc = Path::new("/proc");
    let start = Instant::now();
    let res = query_gio_free_space_async(proc).expect("query /proc");
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 500, "Procfs query must not hang");
    let _ = res;
}

#[test]
fn test_cross_mount_sysfs_no_hang() {
    let sys = Path::new("/sys");
    let start = Instant::now();
    let res = query_gio_free_space_async(sys).expect("query /sys");
    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 500, "Sysfs query must not hang");
    let _ = res;
}

#[test]
fn test_cross_mount_user_runtime_tmpfs() {
    let run_user = Path::new("/run/user/1001");
    if run_user.exists() {
        let res = query_gio_free_space_async(run_user).expect("query /run/user/1001");
        assert!(res.is_some());
    }
}

// -------------------------------------------------------------------------
// 2. Path Resolution Edge Cases & Symlinks
// -------------------------------------------------------------------------

#[test]
fn test_path_resolution_nonexistent_path() {
    let path = Path::new("/tmp/nonexistent_subfolder_challenge_12345/abc");
    let res = query_gio_free_space_async(path).expect("query nonexistent path");
    assert_eq!(res, None, "Nonexistent path should return None without error");
}

#[test]
fn test_path_resolution_empty_path() {
    let path = Path::new("");
    let res = query_gio_free_space_async(path);
    // Should not panic
    assert!(res.is_ok());
}

#[test]
fn test_path_resolution_unicode_and_spaces() {
    let env = TestEnv::new();
    let unicode_dir = env.path().join("🦀 rust 📁 test 空间 with spaces & symbols");
    fs::create_dir_all(&unicode_dir).expect("create unicode dir");
    
    let res = query_gio_free_space_async(&unicode_dir).expect("query unicode path");
    assert!(res.is_some(), "Valid directory with unicode/spaces must return free space");
}

#[test]
fn test_path_resolution_cross_mount_symlink() {
    let env = TestEnv::new();
    let link = env.file_path("link_to_root");
    let _ = symlink("/", &link);
    
    if link.exists() {
        let res = query_gio_free_space_async(&link).expect("query symlink to root");
        assert!(res.is_some(), "Symlink resolving to root must report free space");
    }
}

#[test]
fn test_path_resolution_broken_symlink() {
    let env = TestEnv::new();
    let link = env.file_path("broken_link");
    let target = env.file_path("deleted_target");
    let _ = symlink(&target, &link);
    
    // Broken symlink: target does not exist
    let res = query_gio_free_space_async(&link).expect("query broken symlink");
    // Should gracefully return None without crashing
    let _ = res;
}

#[test]
fn test_path_resolution_file_path() {
    let env = TestEnv::new();
    let file = env.write_file("regular_file.txt", b"hello world");
    let res = query_gio_free_space_async(&file).expect("query regular file");
    assert!(res.is_some(), "Filesystem free space query on regular file must return free space of host fs");
}

#[test]
fn test_path_resolution_deeply_nested_directory() {
    let env = TestEnv::new();
    let mut deep = env.path().to_path_buf();
    for i in 0..25 {
        deep = deep.join(format!("level_{i}"));
    }
    fs::create_dir_all(&deep).expect("create deep dirs");
    let res = query_gio_free_space_async(&deep).expect("query deep path");
    assert!(res.is_some(), "Deeply nested directory must report free space");
}

// -------------------------------------------------------------------------
// 3. GLib MainContext Async Stress Test (500 Rapid Queries)
// -------------------------------------------------------------------------

#[test]
fn test_glib_maincontext_rapid_stress_queries() {
    let ctx = glib::MainContext::new();
    let num_queries = 500;
    
    let targets = [
        Path::new("/"),
        Path::new("/tmp"),
        Path::new("/dev/shm"),
        Path::new("/proc"),
        Path::new("/sys"),
        Path::new("/tmp/does_not_exist_challenge"),
    ];

    let start = Instant::now();
    ctx.block_on(async {
        for i in 0..num_queries {
            let target = targets[i % targets.len()];
            let file = gio::File::for_path(target);
            let _ = file.query_filesystem_info_future("filesystem::free", glib::Priority::DEFAULT).await;
        }
    });

    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 5000, "500 sequential async queries completed in {:?}, under 5s ceiling", elapsed);
}

// -------------------------------------------------------------------------
// 4. Status Bar Event Simulation & Formatting Invariants
// -------------------------------------------------------------------------

#[test]
fn test_status_bar_formatting_invariants() {
    // Singular & plural item counts
    assert_eq!(format_item_count(0), "0 items");
    assert_eq!(format_item_count(1), "1 item");
    assert_eq!(format_item_count(2), "2 items");
    assert_eq!(format_item_count(999_999), "999999 items");

    // Selection formatting
    assert_eq!(format_selection_info(0, 0), "");
    assert_eq!(format_selection_info(0, 1024), "");
    assert_eq!(format_selection_info(1, 0), "1 selected, 0 B");
    assert_eq!(format_selection_info(1, 1_500), "1 selected, 1.5 KB");
    assert_eq!(format_selection_info(5, 5_000_000), "5 selected, 5.0 MB");
    assert_eq!(format_selection_info(10, 10_000_000_000), "10 selected, 10.0 GB");

    // Free space formatting
    assert_eq!(format_free_space(0), "0 B free");
    assert_eq!(format_free_space(1_000), "1.0 KB free");
    assert_eq!(format_free_space(1_000_000), "1.0 MB free");
    assert_eq!(format_free_space(1_000_000_000), "1.0 GB free");
    assert_eq!(format_free_space(1_000_000_000_000), "1.0 TB free");
}

// -------------------------------------------------------------------------
// 5. Adversarial Challenge & Race Condition Invariant Tests
// -------------------------------------------------------------------------

#[test]
fn test_async_generation_token_mitigation_model() {
    use std::cell::Cell;
    use std::rc::Rc;

    // Simulation of status bar generation token pattern
    struct GenerationGuardedStatusBar {
        generation: Rc<Cell<u64>>,
        current_text: Rc<Cell<Option<u64>>>,
    }

    impl GenerationGuardedStatusBar {
        fn new() -> Self {
            Self {
                generation: Rc::new(Cell::new(0)),
                current_text: Rc::new(Cell::new(None)),
            }
        }

        fn update_free_space(&self, delay_ms: u64, free_bytes: u64, ctx: &glib::MainContext) {
            let generation_id = self.generation.get() + 1;
            self.generation.set(generation_id);
            
            let current_gen = self.generation.clone();
            let label = self.current_text.clone();
            
            ctx.spawn_local(async move {
                // Simulate varying async response latency
                if delay_ms > 0 {
                    glib::timeout_future(std::time::Duration::from_millis(delay_ms)).await;
                }
                // Only commit if generation hasn't advanced
                if current_gen.get() == generation_id {
                    label.set(Some(free_bytes));
                }
            });
        }

        fn clear_free_space(&self) {
            self.generation.set(self.generation.get() + 1);
            self.current_text.set(None);
        }
    }

    let ctx = glib::MainContext::new();
    let bar = GenerationGuardedStatusBar::new();

    ctx.block_on(async {
        // Query 1: Slow query (100ms) with 500GB free space
        bar.update_free_space(100, 500_000_000_000, &ctx);
        // Query 2: Rapid subsequent query (10ms) with 50GB free space
        bar.update_free_space(10, 50_000_000_000, &ctx);

        // Wait for both to settle
        glib::timeout_future(std::time::Duration::from_millis(150)).await;

        // With generation guard, Query 1 must NOT overwrite Query 2's result
        assert_eq!(bar.current_text.get(), Some(50_000_000_000));

        // Now test clear_free_space during in-flight query
        bar.update_free_space(100, 100_000_000_000, &ctx);
        bar.clear_free_space();

        glib::timeout_future(std::time::Duration::from_millis(150)).await;
        // In-flight query must be discarded because generation changed
        assert_eq!(bar.current_text.get(), None);
    });
}

#[test]
fn test_selection_bytes_saturating_add() {
    let sizes = [1_000_000_000u64, 5_000_000_000u64, 500_000u64];
    let total: u64 = sizes.iter().copied().fold(0u64, |acc, x| acc.saturating_add(x));
    assert_eq!(total, 6_000_500_000);

    // Extreme test with saturating add near u64::MAX
    let extreme_sizes = [u64::MAX - 10, 50];
    let extreme_total = extreme_sizes.iter().copied().fold(0u64, |acc, x| acc.saturating_add(x));
    assert_eq!(extreme_total, u64::MAX);
}
