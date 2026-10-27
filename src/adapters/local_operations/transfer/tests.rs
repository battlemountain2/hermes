use std::{
    cell::RefCell,
    error::Error,
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};

use super::{
    failure, native_transfer_with_copy, replacement_supported, run_native, stage_and_commit,
    staged_native_transfer, transfer_item,
};
use crate::{
    adapters::local_operations::LocalOperationProvider,
    model::Location,
    services::{OperationEvent, OperationProvider, OperationRequestId, PasteRequest},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> std::io::Result<Self> {
        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hermes-safe-transfer-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root)?;
        let fixture = Self(root);
        fs::create_dir(fixture.0.join("source"))?;
        fs::create_dir(fixture.0.join("destination"))?;
        fs::write(fixture.source(), b"new contents")?;
        fs::write(fixture.target(), b"old contents")?;
        fs::write(fixture.0.join("source/untouched"), b"source original")?;
        fs::write(fixture.0.join("destination/untouched"), b"target original")?;
        Ok(fixture)
    }

    fn source(&self) -> PathBuf {
        self.0.join("source/item")
    }

    fn target(&self) -> PathBuf {
        self.0.join("destination/item")
    }

    fn assert_clean(&self) -> std::io::Result<()> {
        assert_eq!(
            fs::read(self.0.join("source/untouched"))?,
            b"source original"
        );
        assert_eq!(
            fs::read(self.0.join("destination/untouched"))?,
            b"target original"
        );
        for entry in fs::read_dir(self.0.join("destination"))? {
            assert!(
                !entry?
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".hermes-transfer-")
            );
        }
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove generated transfer fixture");
    }
}

fn transfer(
    source: &Path,
    target: &Path,
    move_source: bool,
    overwrite: bool,
) -> Result<(), glib::Error> {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            context.block_on(transfer_item(
                gio::File::for_path(source),
                gio::File::for_path(target),
                move_source,
                overwrite,
                gio::Cancellable::new(),
            ))
        })
        .expect("own fixture main context")
}

#[test]
fn missing_source_never_removes_the_existing_destination() -> Result<(), Box<dyn Error>> {
    for move_source in [false, true] {
        let fixture = Fixture::new()?;
        fs::remove_file(fixture.source())?;
        assert!(transfer(&fixture.source(), &fixture.target(), move_source, true).is_err());
        assert_eq!(fs::read(fixture.target())?, b"old contents");
        fixture.assert_clean()?;
    }
    Ok(())
}

#[test]
fn failed_partial_copy_keeps_both_originals_and_cleans_staging() -> Result<(), Box<dyn Error>> {
    for move_source in [false, true] {
        let fixture = Fixture::new()?;
        let result = stage_and_commit(
            &fixture.source(),
            &fixture.target(),
            move_source,
            true,
            &gio::Cancellable::new(),
            |stage| {
                fs::write(stage, b"partial data").expect("write staged fixture");
                assert_eq!(
                    fs::metadata(stage.parent().expect("stage parent"))
                        .expect("stage metadata")
                        .permissions()
                        .mode()
                        & 0o777,
                    0o700
                );
                Err(failure("generated copy failure"))
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(fixture.source())?, b"new contents");
        assert_eq!(fs::read(fixture.target())?, b"old contents");
        fixture.assert_clean()?;
    }
    Ok(())
}

#[test]
fn failed_atomic_publish_does_not_remove_the_source_or_new_conflict() -> Result<(), Box<dyn Error>>
{
    let fixture = Fixture::new()?;
    let result = stage_and_commit(
        &fixture.source(),
        &fixture.target(),
        true,
        true,
        &gio::Cancellable::new(),
        |stage| {
            fs::copy(fixture.source(), stage).expect("copy staged fixture");
            fs::remove_file(fixture.target()).expect("replace fixture conflict");
            fs::create_dir(fixture.target()).expect("create new folder conflict");
            fs::write(fixture.target().join("original"), b"keep").expect("write folder original");
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(fixture.source())?, b"new contents");
    assert_eq!(fs::read(fixture.target().join("original"))?, b"keep");
    fixture.assert_clean()?;
    Ok(())
}

#[test]
fn cancellation_after_staging_keeps_both_originals() -> Result<(), Box<dyn Error>> {
    for move_source in [false, true] {
        let fixture = Fixture::new()?;
        let cancellable = gio::Cancellable::new();
        let result = stage_and_commit(
            &fixture.source(),
            &fixture.target(),
            move_source,
            true,
            &cancellable,
            |stage| {
                fs::copy(fixture.source(), stage).expect("copy staged fixture");
                cancellable.cancel();
                Ok(())
            },
        );
        assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
        assert_eq!(fs::read(fixture.source())?, b"new contents");
        assert_eq!(fs::read(fixture.target())?, b"old contents");
        fixture.assert_clean()?;
    }
    Ok(())
}

#[test]
fn precancelled_transfer_never_creates_staging() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();
    assert!(
        staged_native_transfer(
            &fixture.source(),
            &fixture.target(),
            true,
            true,
            &cancellable
        )
        .is_err()
    );
    assert_eq!(fs::read(fixture.source())?, b"new contents");
    assert_eq!(fs::read(fixture.target())?, b"old contents");
    fixture.assert_clean()?;
    Ok(())
}

#[test]
fn cancellation_after_commit_does_not_roll_back_a_completed_move() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let cancellable = gio::Cancellable::new();
    staged_native_transfer(
        &fixture.source(),
        &fixture.target(),
        true,
        true,
        &cancellable,
    )?;
    cancellable.cancel();
    assert!(!fixture.source().exists());
    assert_eq!(fs::read(fixture.target())?, b"new contents");
    fixture.assert_clean()?;
    Ok(())
}

#[test]
fn cancelled_gio_copy_keeps_originals_and_removes_staging() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let contents = vec![b'x'; 1024 * 1024];
    fs::write(fixture.source(), &contents)?;
    let cancellable = gio::Cancellable::new();
    let result = stage_and_commit(
        &fixture.source(),
        &fixture.target(),
        true,
        true,
        &cancellable,
        |stage| {
            let mut progress = |_copied, _total| cancellable.cancel();
            gio::File::for_path(fixture.source()).copy(
                &gio::File::for_path(stage),
                gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NOFOLLOW_SYMLINKS,
                Some(&cancellable),
                Some(&mut progress),
            )
        },
    );
    assert!(cancellable.is_cancelled());
    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert_eq!(fs::read(fixture.source())?, contents);
    assert_eq!(fs::read(fixture.target())?, b"old contents");
    fixture.assert_clean()?;
    Ok(())
}

#[test]
fn aborting_and_dropping_the_future_cancels_worker_and_cleans_after_writer_stops()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let context = glib::MainContext::new();
    let source = fixture.source();
    let target = fixture.target();
    let (started_send, started_recv) = mpsc::channel();
    let (finished_send, finished_recv) = mpsc::channel();
    let (release_send, release_recv) = mpsc::channel();
    let cancellable = gio::Cancellable::new();
    let monitor = cancellable.clone();
    context.with_thread_default(|| {
        let task = context.spawn_local(run_native(cancellable, move |cancellable| {
            let result = stage_and_commit(&source, &target, true, true, &cancellable, |stage| {
                fs::write(stage, b"partial data").expect("write staged fixture");
                started_send
                    .send(stage.to_path_buf())
                    .expect("report writer startup");
                release_recv
                    .recv_timeout(Duration::from_secs(5))
                    .expect("release writer");
                assert!(cancellable.is_cancelled());
                fs::write(stage, b"last writer bytes").expect("stage survives until writer stops");
                Err(failure("cancelled fixture writer"))
            });
            finished_send.send(()).expect("report cleanup completion");
            result
        }));
        let deadline = Instant::now() + Duration::from_secs(5);
        let stage = loop {
            context.iteration(false);
            if let Ok(stage) = started_recv.try_recv() {
                break stage;
            }
            assert!(Instant::now() < deadline, "fixture worker did not start");
            std::thread::sleep(Duration::from_millis(1));
        };
        task.abort();
        drop(task);
        while !monitor.is_cancelled() {
            context.iteration(false);
            assert!(
                Instant::now() < deadline,
                "aborted future did not cancel writer"
            );
        }
        assert!(stage.exists());
        release_send.send(()).expect("release cancelled writer");
        finished_recv
            .recv_timeout(Duration::from_secs(5))
            .expect("wait for worker cleanup");
        assert!(!stage.exists());
        while context.pending() {
            context.iteration(false);
        }
    })?;
    assert_eq!(fs::read(fixture.source())?, b"new contents");
    assert_eq!(fs::read(fixture.target())?, b"old contents");
    fixture.assert_clean()?;
    Ok(())
}

#[test]
fn successful_copy_and_move_publish_complete_files() -> Result<(), Box<dyn Error>> {
    for move_source in [false, true] {
        for existing in [false, true] {
            let fixture = Fixture::new()?;
            if !existing {
                fs::remove_file(fixture.target())?;
            }
            transfer(&fixture.source(), &fixture.target(), move_source, existing)?;
            assert_eq!(fs::read(fixture.target())?, b"new contents");
            assert_eq!(fixture.source().exists(), !move_source);
            fixture.assert_clean()?;
        }
    }
    Ok(())
}

#[test]
fn same_filesystem_overwrite_move_skips_copy_and_preserves_inode() -> Result<(), Box<dyn Error>> {
    for source_link in [false, true] {
        for (existing, target_link) in [(false, false), (true, false), (true, true)] {
            let fixture = Fixture::new()?;
            if source_link {
                fs::remove_file(fixture.source())?;
                symlink(fixture.0.join("source/untouched"), fixture.source())?;
            }
            if !existing {
                fs::remove_file(fixture.target())?;
            } else if target_link {
                fs::remove_file(fixture.target())?;
                symlink(fixture.0.join("destination/untouched"), fixture.target())?;
            }
            let original = fs::symlink_metadata(fixture.source())?;
            native_transfer_with_copy(
                &fixture.source(),
                &fixture.target(),
                true,
                true,
                &gio::Cancellable::new(),
                |_stage| panic!("same-filesystem overwrite move must not copy"),
            )?;
            let moved = fs::symlink_metadata(fixture.target())?;
            assert_eq!((moved.dev(), moved.ino()), (original.dev(), original.ino()));
            assert!(fs::symlink_metadata(fixture.source()).is_err());
            if source_link {
                assert_eq!(
                    fs::read_link(fixture.target())?,
                    fixture.0.join("source/untouched")
                );
            } else {
                assert_eq!(fs::read(fixture.target())?, b"new contents");
            }
            fixture.assert_clean()?;
        }
    }
    Ok(())
}

#[test]
fn fast_move_cancellation_and_validation_failure_keep_originals() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let cancellable = gio::Cancellable::new();
    cancellable.cancel();
    let result = native_transfer_with_copy(
        &fixture.source(),
        &fixture.target(),
        true,
        true,
        &cancellable,
        |_stage| panic!("cancelled fast move must not copy"),
    );
    assert!(result.is_err_and(|error| error.matches(gio::IOErrorEnum::Cancelled)));
    assert_eq!(fs::read(fixture.source())?, b"new contents");
    assert_eq!(fs::read(fixture.target())?, b"old contents");
    let invalid_target = fixture.0.join("missing-parent/item");
    assert!(
        native_transfer_with_copy(
            &fixture.source(),
            &invalid_target,
            true,
            true,
            &gio::Cancellable::new(),
            |_stage| panic!("invalid fast move must not copy"),
        )
        .is_err()
    );
    assert_eq!(fs::read(fixture.source())?, b"new contents");
    assert_eq!(fs::read(fixture.target())?, b"old contents");
    fixture.assert_clean()?;
    Ok(())
}

#[test]
fn existing_no_overwrite_conflicts_preserve_both_originals() -> Result<(), Box<dyn Error>> {
    for move_source in [false, true] {
        let fixture = Fixture::new()?;
        assert!(
            transfer(&fixture.source(), &fixture.target(), move_source, false)
                .is_err_and(|error| error.matches(gio::IOErrorEnum::Exists))
        );
        assert_eq!(fs::read(fixture.source())?, b"new contents");
        assert_eq!(fs::read(fixture.target())?, b"old contents");
        fixture.assert_clean()?;
    }
    Ok(())
}

#[test]
fn links_are_copied_and_moved_without_touching_referents() -> Result<(), Box<dyn Error>> {
    for move_source in [false, true] {
        for dangling in [false, true] {
            let fixture = Fixture::new()?;
            let referent = fixture.0.join("referent");
            fs::write(&referent, b"link original")?;
            let link_path = if dangling {
                fixture.0.join("missing")
            } else {
                referent.clone()
            };
            fs::remove_file(fixture.source())?;
            symlink(&link_path, fixture.source())?;
            fs::remove_file(fixture.target())?;
            symlink(&referent, fixture.target())?;
            transfer(&fixture.source(), &fixture.target(), move_source, true)?;
            assert_eq!(fs::read_link(fixture.target())?, link_path);
            assert_eq!(fs::read(&referent)?, b"link original");
            assert_eq!(fs::symlink_metadata(fixture.source()).is_ok(), !move_source);
            fixture.assert_clean()?;
        }
    }
    Ok(())
}

#[test]
fn regular_file_replaces_a_dangling_destination_link_not_its_referent() -> Result<(), Box<dyn Error>>
{
    let fixture = Fixture::new()?;
    let referent = fixture.0.join("missing");
    fs::remove_file(fixture.target())?;
    symlink(&referent, fixture.target())?;
    transfer(&fixture.source(), &fixture.target(), false, true)?;
    assert!(!referent.exists());
    assert!(fs::symlink_metadata(fixture.target())?.is_file());
    assert_eq!(fs::read(fixture.target())?, b"new contents");
    fixture.assert_clean()?;
    Ok(())
}

#[test]
fn new_folder_copy_and_move_preserve_nested_files_and_links() -> Result<(), Box<dyn Error>> {
    for move_source in [false, true] {
        let fixture = Fixture::new()?;
        fs::remove_file(fixture.source())?;
        fs::remove_file(fixture.target())?;
        fs::create_dir_all(fixture.source().join("nested"))?;
        fs::write(fixture.source().join("nested/child"), b"child")?;
        symlink(
            fixture.0.join("source/untouched"),
            fixture.source().join("link"),
        )?;
        transfer(&fixture.source(), &fixture.target(), move_source, true)?;
        assert_eq!(fs::read(fixture.target().join("nested/child"))?, b"child");
        assert_eq!(
            fs::read_link(fixture.target().join("link"))?,
            fixture.0.join("source/untouched")
        );
        assert_eq!(fixture.source().exists(), !move_source);
        fixture.assert_clean()?;
    }
    Ok(())
}

#[test]
fn folder_merging_combines_contents_and_preserves_unrelated_files() -> Result<(), Box<dyn Error>> {
    for move_source in [false, true] {
        let fixture = Fixture::new()?;
        fs::remove_file(fixture.source())?;
        fs::create_dir(fixture.source())?;
        fs::write(fixture.source().join("new"), b"new contents")?;

        fs::remove_file(fixture.target())?;
        fs::create_dir(fixture.target())?;
        fs::write(fixture.target().join("old"), b"old contents")?;

        let result = transfer(&fixture.source(), &fixture.target(), move_source, true);
        assert!(result.is_ok(), "folder merge failed: {:?}", result.err());

        assert_eq!(fs::read(fixture.target().join("old"))?, b"old contents");
        assert_eq!(fs::read(fixture.target().join("new"))?, b"new contents");

        if move_source {
            assert!(!fixture.source().exists());
        } else {
            assert_eq!(fs::read(fixture.source().join("new"))?, b"new contents");
        }
        fixture.assert_clean()?;
    }
    Ok(())
}

#[test]
fn folder_merging_recursive_tree_with_conflicts_preserves_unrelated_and_updates_collisions()
-> Result<(), Box<dyn Error>> {
    for move_source in [false, true] {
        let fixture = Fixture::new()?;
        fs::remove_file(fixture.source())?;
        fs::remove_file(fixture.target())?;

        // Setup complex source tree
        fs::create_dir_all(fixture.source().join("sub/deep"))?;
        fs::write(fixture.source().join("root_keep.txt"), b"source root keep")?;
        fs::write(
            fixture.source().join("conflict.txt"),
            b"source conflict version",
        )?;
        fs::write(
            fixture.source().join("sub/nested_keep.txt"),
            b"source nested keep",
        )?;
        fs::write(
            fixture.source().join("sub/nested_conflict.txt"),
            b"source nested conflict",
        )?;
        fs::write(
            fixture.source().join("sub/deep/leaf.txt"),
            b"source deep leaf",
        )?;

        // Setup complex target tree
        fs::create_dir_all(fixture.target().join("sub"))?;
        fs::create_dir_all(fixture.target().join("other"))?;
        fs::write(
            fixture.target().join("root_untouched.txt"),
            b"target root untouched",
        )?;
        fs::write(
            fixture.target().join("conflict.txt"),
            b"target old conflict",
        )?;
        fs::write(
            fixture.target().join("sub/nested_untouched.txt"),
            b"target nested untouched",
        )?;
        fs::write(
            fixture.target().join("sub/nested_conflict.txt"),
            b"target old nested conflict",
        )?;
        fs::write(
            fixture.target().join("other/extra.txt"),
            b"target other extra",
        )?;

        let result = transfer(&fixture.source(), &fixture.target(), move_source, true);
        assert!(
            result.is_ok(),
            "recursive folder merge failed: {:?}",
            result.err()
        );

        // Target must preserve untouched files
        assert_eq!(
            fs::read(fixture.target().join("root_untouched.txt"))?,
            b"target root untouched"
        );
        assert_eq!(
            fs::read(fixture.target().join("sub/nested_untouched.txt"))?,
            b"target nested untouched"
        );
        assert_eq!(
            fs::read(fixture.target().join("other/extra.txt"))?,
            b"target other extra"
        );

        // Target must receive new files
        assert_eq!(
            fs::read(fixture.target().join("root_keep.txt"))?,
            b"source root keep"
        );
        assert_eq!(
            fs::read(fixture.target().join("sub/nested_keep.txt"))?,
            b"source nested keep"
        );
        assert_eq!(
            fs::read(fixture.target().join("sub/deep/leaf.txt"))?,
            b"source deep leaf"
        );

        // Conflicting files must be updated to source version
        assert_eq!(
            fs::read(fixture.target().join("conflict.txt"))?,
            b"source conflict version"
        );
        assert_eq!(
            fs::read(fixture.target().join("sub/nested_conflict.txt"))?,
            b"source nested conflict"
        );

        if move_source {
            assert!(
                !fixture.source().exists(),
                "source tree must be removed after move-merge"
            );
        } else {
            assert!(
                fixture.source().join("sub/deep/leaf.txt").exists(),
                "source must remain on copy-merge"
            );
        }
        fixture.assert_clean()?;
    }
    Ok(())
}

#[test]
fn type_conflicts_fail_closed_without_destroying_either_side() -> Result<(), Box<dyn Error>> {
    for move_source in [false, true] {
        for (source_folder, target_folder) in [(true, false), (false, true)] {
            let fixture = Fixture::new()?;
            if source_folder {
                fs::remove_file(fixture.source())?;
                fs::create_dir(fixture.source())?;
                fs::write(fixture.source().join("new"), b"new contents")?;
            }
            if target_folder {
                fs::remove_file(fixture.target())?;
                fs::create_dir(fixture.target())?;
                fs::write(fixture.target().join("old"), b"old contents")?;
            }
            let result = transfer(&fixture.source(), &fixture.target(), move_source, true);
            assert!(result.is_err_and(|error| {
                error
                    .to_string()
                    .contains("Cannot replace a file with a folder or a folder with a file")
            }));
            assert_eq!(
                fs::read(if source_folder {
                    fixture.source().join("new")
                } else {
                    fixture.source()
                })?,
                b"new contents"
            );
            assert_eq!(
                fs::read(if target_folder {
                    fixture.target().join("old")
                } else {
                    fixture.target()
                })?,
                b"old contents"
            );
            assert!(!fixture.target().join("new").exists());
            fixture.assert_clean()?;
        }
    }
    Ok(())
}

#[test]
fn racing_no_overwrite_conflict_is_not_replaced() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    fs::remove_file(fixture.target())?;
    let result = stage_and_commit(
        &fixture.source(),
        &fixture.target(),
        true,
        false,
        &gio::Cancellable::new(),
        |stage| {
            fs::copy(fixture.source(), stage).expect("copy staged fixture");
            fs::write(fixture.target(), b"racing original").expect("write new conflict");
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(fixture.source())?, b"new contents");
    assert_eq!(fs::read(fixture.target())?, b"racing original");
    fixture.assert_clean()?;
    Ok(())
}

#[test]
fn changed_source_and_aliases_are_not_deleted() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let result = stage_and_commit(
        &fixture.source(),
        &fixture.target(),
        true,
        true,
        &gio::Cancellable::new(),
        |stage| {
            fs::copy(fixture.source(), stage).expect("copy staged fixture");
            fs::write(fixture.source(), b"changed source contents")
                .expect("change generated source");
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(fixture.target())?, b"old contents");
    assert_eq!(fs::read(fixture.source())?, b"changed source contents");
    let alias = fixture.0.join("alias");
    symlink(fixture.0.join("source"), &alias)?;
    assert!(transfer(&fixture.source(), &alias.join("item"), true, true).is_err());
    assert_eq!(fs::read(fixture.source())?, b"changed source contents");
    fixture.assert_clean()?;
    Ok(())
}

#[test]
fn remote_replacement_requires_an_atomic_capability_not_gio_overwrite() {
    for source_type in [
        gio::FileType::Regular,
        gio::FileType::SymbolicLink,
        gio::FileType::Directory,
    ] {
        let error = replacement_supported(false, source_type, gio::FileType::Regular)
            .expect_err("remote atomic replacement is not guaranteed");
        assert!(error.to_string().contains("kept unchanged"));
    }
}

#[test]
fn unsupported_remote_source_does_not_remove_native_destination() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let context = glib::MainContext::new();
    let result = context.with_thread_default(|| {
        context.block_on(transfer_item(
            gio::File::for_uri("hermes-fixture-unsupported://host/item"),
            gio::File::for_path(fixture.target()),
            true,
            true,
            gio::Cancellable::new(),
        ))
    })?;
    assert!(result.is_err());
    assert_eq!(fs::read(fixture.target())?, b"old contents");
    fixture.assert_clean()?;
    Ok(())
}

#[test]
fn paste_continues_after_failures_and_reports_only_successful_roots() -> Result<(), Box<dyn Error>>
{
    let fixture = Fixture::new()?;
    let missing = fixture.0.join("source/missing");
    fs::write(fixture.0.join("destination/missing"), b"missing original")?;
    let folder = fixture.0.join("source/folder");
    fs::create_dir(&folder)?;
    fs::write(folder.join("child"), b"folder contents")?;
    fs::write(fixture.0.join("destination/folder"), b"destination file")?;
    let last = fixture.0.join("source/last");
    fs::write(&last, b"last contents")?;
    let success_locations = [Location::local(fixture.source()), Location::local(&last)];
    let events = Rc::new(RefCell::new(Vec::new()));
    let output = events.clone();
    let context = glib::MainContext::default();
    let _owner = context.acquire()?;
    let handle = LocalOperationProvider.paste(
        PasteRequest {
            id: OperationRequestId(701),
            destination: Location::local(fixture.0.join("destination")),
            sources: vec![
                Location::local(&missing),
                success_locations[0].clone(),
                Location::local(&folder),
                success_locations[1].clone(),
            ],
            target_names: Vec::new(),
            move_sources: true,
            overwrite_existing: true,
        },
        Rc::new(move |event| output.borrow_mut().push(event)),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !events
        .borrow()
        .iter()
        .any(|event| matches!(event, OperationEvent::TransferCompletedWithErrors { .. }))
    {
        context.iteration(false);
        assert!(Instant::now() < deadline, "paste did not complete");
        std::thread::sleep(Duration::from_millis(1));
    }
    drop(handle);
    let events = events.borrow();
    let successful = events
        .iter()
        .filter_map(|event| match event {
            OperationEvent::TransferItemCompleted {
                source,
                move_source,
                ..
            } => {
                assert!(*move_source);
                Some(source.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(successful, success_locations);
    assert!(events.iter().any(|event| matches!(
        event,
        OperationEvent::TransferProgress {
            completed: 4,
            total: 4,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(event, OperationEvent::TransferCompletedWithErrors { message, .. } if message.starts_with("2 items could not be transferred"))));
    assert!(!events.iter().any(|event| matches!(
        event,
        OperationEvent::Pasted { .. } | OperationEvent::Failed { .. }
    )));
    assert_eq!(fs::read(fixture.target())?, b"new contents");
    assert_eq!(
        fs::read(fixture.0.join("destination/last"))?,
        b"last contents"
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/missing"))?,
        b"missing original"
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/folder"))?,
        b"destination file"
    );
    assert!(folder.join("child").exists());
    assert!(!fixture.source().exists());
    assert!(!last.exists());
    fixture.assert_clean()?;
    Ok(())
}
