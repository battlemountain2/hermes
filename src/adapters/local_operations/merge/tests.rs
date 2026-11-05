use super::*;
use std::{
    cell::RefCell,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hermes-merge-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("source")).unwrap();
        fs::create_dir(path.join("destination")).unwrap();
        Self(path)
    }

    fn collision(&self, name: &str) {
        fs::write(self.0.join("source").join(name), b"incoming").unwrap();
        fs::write(self.0.join("destination").join(name), b"existing").unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn run(
    fixture: &Fixture,
    moving: bool,
    decide: impl Fn(&FileEntry) -> ConflictResolution + 'static,
) -> (Result<Outcome, glib::Error>, usize, Vec<(PathBuf, PathBuf)>) {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let relocations = Rc::new(RefCell::new(Vec::new()));
            let output = relocations.clone();
            let emit = Rc::new(move |event| {
                if let OperationEvent::TransferConflict { source, reply, .. } = event {
                    reply.respond(decide(&source));
                }
            });
            let mut tree =
                TransferTree::new(OperationRequestId(1), emit, gio::Cancellable::new(), moving);
            tree.relocation = Rc::new(move |source, target| {
                output
                    .borrow_mut()
                    .push((source.path().unwrap(), target.path().unwrap()))
            });
            let result = context.block_on(tree.transfer(
                gio::File::for_path(fixture.0.join("source")),
                gio::File::for_path(fixture.0.join("destination")),
                false,
                0,
            ));
            let saved = relocations.borrow().clone();
            (result, tree.skipped, saved)
        })
        .unwrap()
}

fn choice(action: ConflictAction, apply_to_remaining: bool) -> ConflictResolution {
    ConflictResolution {
        action,
        apply_to_remaining,
    }
}

#[test]
fn approving_all_folder_merges_still_asks_for_each_file() {
    let fixture = Fixture::new();
    fixture.collision("a.txt");
    fixture.collision("b.txt");
    fs::write(fixture.0.join("destination/unrelated"), b"keep").unwrap();
    let files = Rc::new(RefCell::new(Vec::new()));
    let responses = files.clone();
    let (result, skipped, _) = run(&fixture, false, move |source| {
        if source.is_directory() {
            choice(ConflictAction::Replace, true)
        } else {
            responses.borrow_mut().push(source.display_name.clone());
            choice(ConflictAction::Skip, false)
        }
    });
    assert!(!result.unwrap().complete);
    assert_eq!(skipped, 2);
    assert_eq!(files.borrow().len(), 2);
    assert_eq!(
        fs::read(fixture.0.join("destination/a.txt")).unwrap(),
        b"existing"
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/b.txt")).unwrap(),
        b"existing"
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/unrelated")).unwrap(),
        b"keep"
    );
}

#[test]
fn mixed_move_choices_preserve_skipped_sources_and_relocate_completed_leaves() {
    let fixture = Fixture::new();
    for name in ["replace.txt", "both.txt", "skip.txt"] {
        fixture.collision(name);
    }
    fs::write(fixture.0.join("destination/both (copy).txt"), b"keep copy").unwrap();
    let (result, skipped, relocations) = run(&fixture, true, |source| {
        choice(
            match source.display_name.as_str() {
                "source" | "replace.txt" => ConflictAction::Replace,
                "both.txt" => ConflictAction::KeepBoth,
                "skip.txt" => ConflictAction::Skip,
                _ => panic!("unexpected conflict"),
            },
            false,
        )
    });
    assert!(!result.unwrap().complete);
    assert_eq!(skipped, 1);
    assert_eq!(relocations.len(), 2);
    assert_eq!(
        fs::read(fixture.0.join("source/skip.txt")).unwrap(),
        b"incoming"
    );
    assert!(!fixture.0.join("source/replace.txt").exists());
    assert!(!fixture.0.join("source/both.txt").exists());
    assert_eq!(
        fs::read(fixture.0.join("destination/replace.txt")).unwrap(),
        b"incoming"
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/skip.txt")).unwrap(),
        b"existing"
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/both.txt")).unwrap(),
        b"existing"
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/both (copy).txt")).unwrap(),
        b"keep copy"
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/both (copy 2).txt")).unwrap(),
        b"incoming"
    );
}

#[test]
fn apply_to_remaining_files_is_local_to_one_transfer() {
    for _ in 0..2 {
        let fixture = Fixture::new();
        fixture.collision("a");
        fixture.collision("b");
        let asked = Rc::new(RefCell::new(0));
        let observed = asked.clone();
        let (result, _, _) = run(&fixture, false, move |source| {
            if !source.is_directory() {
                *observed.borrow_mut() += 1;
            }
            choice(ConflictAction::Replace, true)
        });
        assert!(result.unwrap().complete);
        assert_eq!(*asked.borrow(), 1);
    }
}

#[test]
fn cancelling_a_merge_keeps_completed_moves_and_remaining_originals() {
    let fixture = Fixture::new();
    for name in ["a", "b", "c"] {
        fixture.collision(name);
    }
    let count = Rc::new(RefCell::new(0));
    let observed = count.clone();
    let (result, _, relocated) = run(&fixture, true, move |source| {
        if source.is_directory() {
            return choice(ConflictAction::Replace, false);
        }
        *observed.borrow_mut() += 1;
        choice(
            if *observed.borrow() == 1 {
                ConflictAction::Replace
            } else {
                ConflictAction::Cancel
            },
            false,
        )
    });
    assert!(result.unwrap_err().matches(gio::IOErrorEnum::Cancelled));
    assert_eq!(relocated.len(), 1);
    assert_eq!(fs::read_dir(fixture.0.join("source")).unwrap().count(), 2);
    assert_eq!(fs::read(&relocated[0].1).unwrap(), b"incoming");
}

#[test]
fn alias_back_into_source_is_rejected_before_creating_a_target() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("source/keep"), b"original").unwrap();
    std::os::unix::fs::symlink(fixture.0.join("source"), fixture.0.join("alias")).unwrap();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let mut tree = TransferTree::new(
                OperationRequestId(1),
                Rc::new(|_| panic!("must reject before any conflict prompt")),
                gio::Cancellable::new(),
                false,
            );
            let result = context.block_on(tree.transfer(
                gio::File::for_path(fixture.0.join("source")),
                gio::File::for_path(fixture.0.join("alias/source")),
                false,
                0,
            ));
            assert!(result.is_err());
            assert!(!fixture.0.join("source/source").exists());
            assert_eq!(
                fs::read(fixture.0.join("source/keep")).unwrap(),
                b"original"
            );
        })
        .unwrap();
}

#[test]
fn replacing_a_changed_destination_requires_another_decision() {
    let fixture = Fixture::new();
    fixture.collision("a");
    let target = fixture.0.join("destination/a");
    let calls = Rc::new(RefCell::new(0));
    let observed = calls.clone();
    let (result, skipped, _) = run(&fixture, false, move |source| {
        if source.is_directory() {
            return choice(ConflictAction::Replace, false);
        }
        *observed.borrow_mut() += 1;
        if *observed.borrow() == 1 {
            fs::write(&target, b"changed while deciding").unwrap();
            choice(ConflictAction::Replace, false)
        } else {
            choice(ConflictAction::Skip, false)
        }
    });
    assert!(!result.unwrap().complete);
    assert_eq!(skipped, 1);
    assert_eq!(*calls.borrow(), 2);
    assert_eq!(
        fs::read(fixture.0.join("destination/a")).unwrap(),
        b"changed while deciding"
    );
    assert_eq!(fs::read(fixture.0.join("source/a")).unwrap(), b"incoming");
}

#[test]
fn a_new_same_filesystem_folder_move_preserves_its_inode() {
    use std::os::unix::fs::MetadataExt;
    let fixture = Fixture::new();
    fixture.collision("a");
    fs::remove_dir_all(fixture.0.join("destination")).unwrap();
    let inode = fs::metadata(fixture.0.join("source")).unwrap().ino();
    let (result, _, relocated) = run(&fixture, true, |_| panic!("no collision"));
    assert!(result.unwrap().complete);
    assert!(!fixture.0.join("source").exists());
    assert_eq!(
        fs::metadata(fixture.0.join("destination")).unwrap().ino(),
        inode
    );
    assert_eq!(relocated.len(), 1);
    assert_eq!(
        fs::read(fixture.0.join("destination/a")).unwrap(),
        b"incoming"
    );
}
