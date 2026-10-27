#[cfg(test)]
mod tests;

use std::{
    fs::{self, DirBuilder, Metadata},
    future::Future,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
};

use gtk::{gio, glib, prelude::*};

use super::copy_recursively;

fn failure(message: &str) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::NotSupported, message)
}

fn io_error(error: std::io::Error) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::Failed, &error.to_string())
}

fn check_cancelled(cancellable: &gio::Cancellable) -> Result<(), glib::Error> {
    if cancellable.is_cancelled() {
        Err(glib::Error::new(
            gio::IOErrorEnum::Cancelled,
            "Transfer cancelled before committing the replacement",
        ))
    } else {
        Ok(())
    }
}

fn replacement_supported(
    native: bool,
    source_type: gio::FileType,
    target_type: gio::FileType,
) -> Result<(), glib::Error> {
    if source_type == gio::FileType::Directory || target_type == gio::FileType::Directory {
        return Err(failure(
            "Cannot replace a file with a folder or a folder with a file; the existing destination was kept unchanged.",
        ));
    }
    if !native {
        return Err(failure(
            "This backend does not provide a guaranteed atomic replacement. The existing destination was kept unchanged; paste under a different name or into another folder.",
        ));
    }
    if !matches!(
        source_type,
        gio::FileType::Regular | gio::FileType::SymbolicLink
    ) || !matches!(
        target_type,
        gio::FileType::Regular | gio::FileType::SymbolicLink
    ) {
        return Err(failure(
            "Only regular files and symbolic links can be safely replaced",
        ));
    }
    Ok(())
}

fn merge_directory(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    overwrite: bool,
    cancellable: gio::Cancellable,
) -> Pin<Box<dyn Future<Output = Result<(), glib::Error>>>> {
    Box::pin(async move {
        check_cancelled(&cancellable)?;
        if !target.query_exists(Some(&cancellable)) {
            target
                .make_directory_future(glib::Priority::DEFAULT)
                .await?;
        }
        let enumerator = source
            .enumerate_children_future(
                "standard::name,standard::type",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
            )
            .await?;
        loop {
            check_cancelled(&cancellable)?;
            let children = enumerator
                .next_files_future(64, glib::Priority::DEFAULT)
                .await?;
            if children.is_empty() {
                break;
            }
            for child in children {
                check_cancelled(&cancellable)?;
                let child_source = source.child(child.name());
                let child_target = target.child(child.name());
                Box::pin(transfer_item(
                    child_source,
                    child_target,
                    move_source,
                    overwrite,
                    cancellable.clone(),
                ))
                .await?;
            }
        }
        if move_source {
            check_cancelled(&cancellable)?;
            source.delete_future(glib::Priority::DEFAULT).await?;
        }
        Ok(())
    })
}

pub(super) async fn transfer_item(
    source: gio::File,
    target: gio::File,
    move_source: bool,
    overwrite: bool,
    cancellable: gio::Cancellable,
) -> Result<(), glib::Error> {
    check_cancelled(&cancellable)?;
    let source_type = source
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await?
        .file_type();
    let target_type = match target
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        )
        .await
    {
        Ok(info) => Some(info.file_type()),
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => None,
        Err(error) => return Err(error),
    };
    let native_paths = source.path().zip(target.path());
    if let Some(target_type) = target_type {
        if !overwrite {
            return Err(glib::Error::new(
                gio::IOErrorEnum::Exists,
                "The destination already exists",
            ));
        }
        if source_type == gio::FileType::Directory && target_type == gio::FileType::Directory {
            return merge_directory(source, target, move_source, overwrite, cancellable).await;
        }
        replacement_supported(native_paths.is_some(), source_type, target_type)?;
    }
    if let Some((source_path, target_path)) = native_paths
        && matches!(
            source_type,
            gio::FileType::Regular | gio::FileType::SymbolicLink
        )
    {
        return run_native(cancellable, move |cancellable| {
            staged_native_transfer(
                &source_path,
                &target_path,
                move_source,
                overwrite,
                &cancellable,
            )
        })
        .await;
    }
    check_cancelled(&cancellable)?;
    if move_source {
        let (transfer, _progress) = source.move_future(
            &target,
            gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NOFOLLOW_SYMLINKS,
            glib::Priority::DEFAULT,
        );
        transfer.await
    } else {
        copy_recursively(source, target).await
    }
}

struct CancelWorkerOnDrop(Option<gio::Cancellable>);

impl Drop for CancelWorkerOnDrop {
    fn drop(&mut self) {
        if let Some(cancellable) = self.0.take() {
            cancellable.cancel();
        }
    }
}

async fn run_native(
    cancellable: gio::Cancellable,
    job: impl FnOnce(gio::Cancellable) -> Result<(), glib::Error> + Send + 'static,
) -> Result<(), glib::Error> {
    let mut guard = CancelWorkerOnDrop(Some(cancellable.clone()));
    let result = gio::spawn_blocking(move || job(cancellable)).await;
    guard.0 = None;
    result.map_err(|_| failure("The native transfer worker failed"))?
}

struct StagingDirectory(PathBuf);

impl StagingDirectory {
    fn create(parent: &Path) -> Result<Self, glib::Error> {
        static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);
        for _attempt in 0..128 {
            let path = parent.join(format!(
                ".hermes-transfer-{}-{}",
                std::process::id(),
                NEXT_STAGE.fetch_add(1, Ordering::Relaxed)
            ));
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error(error)),
            }
        }
        Err(failure(
            "Unable to reserve a private staging folder beside the destination",
        ))
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            tracing::warn!(path = %self.0.display(), %error, "Unable to clean up transfer staging folder");
        }
    }
}

fn unchanged(before: &Metadata, after: &Metadata) -> bool {
    before.dev() == after.dev()
        && before.ino() == after.ino()
        && before.len() == after.len()
        && before.mtime() == after.mtime()
        && before.mtime_nsec() == after.mtime_nsec()
        && before.ctime() == after.ctime()
        && before.ctime_nsec() == after.ctime_nsec()
}

fn staged_native_transfer(
    source: &Path,
    target: &Path,
    move_source: bool,
    overwrite: bool,
    cancellable: &gio::Cancellable,
) -> Result<(), glib::Error> {
    native_transfer_with_copy(
        source,
        target,
        move_source,
        overwrite,
        cancellable,
        |stage| {
            gio::File::for_path(source).copy(
                &gio::File::for_path(stage),
                gio::FileCopyFlags::ALL_METADATA | gio::FileCopyFlags::NOFOLLOW_SYMLINKS,
                Some(cancellable),
                None,
            )
        },
    )
}

fn native_transfer_with_copy(
    source: &Path,
    target: &Path,
    move_source: bool,
    overwrite: bool,
    cancellable: &gio::Cancellable,
    copy: impl FnOnce(&Path) -> Result<(), glib::Error>,
) -> Result<(), glib::Error> {
    check_cancelled(cancellable)?;
    let original = validate_native_transfer(source, target, overwrite)?;
    if move_source && overwrite {
        let parent = target
            .parent()
            .ok_or_else(|| failure("Invalid destination"))?;
        if original.dev() == fs::metadata(parent).map_err(io_error)?.dev() {
            check_cancelled(cancellable)?;
            return fs::rename(source, target).map_err(io_error);
        }
    }
    stage_and_commit(source, target, move_source, overwrite, cancellable, copy)
}

fn validate_native_transfer(
    source: &Path,
    target: &Path,
    overwrite: bool,
) -> Result<Metadata, glib::Error> {
    let original = fs::symlink_metadata(source).map_err(io_error)?;
    if !original.is_file() && !original.file_type().is_symlink() {
        return Err(failure(
            "Only regular files and symbolic links can be staged",
        ));
    }
    match fs::symlink_metadata(target) {
        Ok(metadata) => {
            if !overwrite {
                return Err(glib::Error::new(
                    gio::IOErrorEnum::Exists,
                    "The destination already exists",
                ));
            }
            if metadata.is_dir() || (!metadata.is_file() && !metadata.file_type().is_symlink()) {
                return Err(failure(
                    "The existing destination is not a regular file or symbolic link; it was kept unchanged",
                ));
            }
            if original.dev() == metadata.dev() && original.ino() == metadata.ino() {
                return Err(failure(
                    "Source and destination refer to the same file; both were kept unchanged",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(error)),
    }
    Ok(original)
}

fn stage_and_commit(
    source: &Path,
    target: &Path,
    move_source: bool,
    overwrite: bool,
    cancellable: &gio::Cancellable,
    copy: impl FnOnce(&Path) -> Result<(), glib::Error>,
) -> Result<(), glib::Error> {
    check_cancelled(cancellable)?;
    let original = validate_native_transfer(source, target, overwrite)?;
    let staging = StagingDirectory::create(
        target
            .parent()
            .ok_or_else(|| failure("Invalid destination"))?,
    )?;
    let stage = staging.0.join("item");
    copy(&stage)?;
    let staged = fs::symlink_metadata(&stage).map_err(io_error)?;
    if !staged.is_file() && !staged.file_type().is_symlink() {
        return Err(failure(
            "The staged item is not a regular file or symbolic link",
        ));
    }
    if !unchanged(&original, &fs::symlink_metadata(source).map_err(io_error)?) {
        return Err(failure(
            "The source changed during transfer; the destination was kept unchanged",
        ));
    }
    check_cancelled(cancellable)?;
    if overwrite {
        fs::rename(&stage, target).map_err(io_error)?;
    } else {
        fs::hard_link(&stage, target).map_err(io_error)?;
    }
    if move_source {
        if !fs::symlink_metadata(source).is_ok_and(|metadata| unchanged(&original, &metadata)) {
            return Err(failure(
                "The destination was committed, but the source changed and was not removed",
            ));
        }
        fs::remove_file(source).map_err(|error| failure(&format!(
            "The destination was committed, but the source could not be removed and was kept: {error}"
        )))?;
    }
    Ok(())
}
