use std::{future::Future, path::Path, pin::Pin, rc::Rc};

use gtk::{gio, glib, prelude::*};

use super::progress::Progress;

use crate::{
    model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue},
    services::{
        ConflictAction, ConflictReply, ConflictResolution, OperationEvent, OperationRequestId,
    },
};

const ATTRIBUTES: &str = "standard::name,standard::display-name,standard::type,standard::size,time::modified,time::modified-usec,time::changed,time::changed-usec,id::file,etag::value";

type RelocationHandler = Rc<dyn Fn(&gio::File, &gio::File)>;

pub(super) struct TransferTree {
    id: OperationRequestId,
    emit: Rc<dyn Fn(OperationEvent)>,
    cancellation: gio::Cancellable,
    moving: bool,
    policies: [Option<ConflictAction>; 3],
    relocation: RelocationHandler,
    pub skipped: usize,
    progress: Rc<Progress>,
}

#[derive(Debug, Default)]
pub(super) struct Outcome {
    pub complete: bool,
    pub bytes: u64,
}

impl TransferTree {
    pub fn new(
        id: OperationRequestId,
        emit: Rc<dyn Fn(OperationEvent)>,
        cancellation: gio::Cancellable,
        moving: bool,
        progress: Rc<Progress>,
    ) -> Self {
        Self {
            id,
            emit,
            cancellation,
            moving,
            policies: [None; 3],
            relocation: Rc::new(relocate),
            skipped: 0,
            progress,
        }
    }

    fn check_cancelled(&self) -> Result<(), glib::Error> {
        self.cancellation.set_error_if_cancelled()
    }

    async fn decide(
        &mut self,
        source: &gio::File,
        incoming: &gio::FileInfo,
        target: &gio::File,
        existing: &gio::FileInfo,
    ) -> ConflictAction {
        let category = match (
            incoming.file_type() == gio::FileType::Directory,
            existing.file_type() == gio::FileType::Directory,
        ) {
            (true, true) => 0,
            (false, false) => 1,
            _ => 2,
        };
        if let Some(action) = self.policies[category] {
            return action;
        }
        let reply = ConflictReply::default();
        (self.emit)(OperationEvent::TransferConflict {
            request_id: self.id,
            source: entry(source, incoming),
            existing: entry(target, existing),
            reply: reply.clone(),
        });
        let ConflictResolution {
            action,
            apply_to_remaining,
        } = reply.wait().await;
        if apply_to_remaining && action != ConflictAction::Cancel {
            self.policies[category] = Some(action);
        }
        action
    }

    pub fn transfer<'a>(
        &'a mut self,
        source: gio::File,
        mut target: gio::File,
        authorize_replacement: bool,
        depth: usize,
        expected_root_size: Option<u64>,
    ) -> Pin<Box<dyn Future<Output = Result<Outcome, glib::Error>> + 'a>> {
        Box::pin(async move {
            self.check_cancelled()?;
            if depth > 128 {
                return Err(error("Folder nesting exceeds the safe transfer limit"));
            }
            let mut incoming = query(&source).await?;
            let mut directory = incoming.file_type() == gio::FileType::Directory;
            if depth == 0
                && (incoming.file_type() != gio::FileType::Regular
                    || expected_root_size
                        != incoming
                            .has_attribute("standard::size")
                            .then_some(incoming.size().max(0) as u64))
            {
                self.progress.invalidate_total();
            }
            if directory {
                super::transfer::validate_directory_target(&source, &target, &self.cancellation)
                    .await?;
            }
            let mut may_replace = authorize_replacement;
            let original_target = target.clone();
            let mut try_rename =
                self.moving && directory && source.path().is_some() && target.path().is_some();
            for _ in 0..64 {
                self.check_cancelled()?;
                let existing = match query(&target).await {
                    Ok(info) => Some(info),
                    Err(error) if error.matches(gio::IOErrorEnum::NotFound) => None,
                    Err(error) => return Err(error),
                };
                if let Some(existing) = &existing
                    && !may_replace
                {
                    match self.decide(&source, &incoming, &target, existing).await {
                        ConflictAction::Cancel => {
                            return Err(glib::Error::new(
                                gio::IOErrorEnum::Cancelled,
                                "Transfer cancelled; completed items were kept",
                            ));
                        }
                        ConflictAction::Skip => {
                            self.skipped += 1;
                            self.progress.invalidate_total();
                            return Ok(Outcome::default());
                        }
                        ConflictAction::KeepBoth => {
                            target = available_copy(&original_target, directory).await?;
                            continue;
                        }
                        ConflictAction::Replace => may_replace = true,
                    }
                }
                if let Some(existing) = &existing {
                    let current = match query(&target).await {
                        Ok(info) => info,
                        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => {
                            may_replace = false;
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    if !same_version(existing, &current) {
                        self.policies = [None; 3];
                        may_replace = false;
                        continue;
                    }
                }
                // A decision can remain open while the source changes. Refresh
                // its identity/type/size before using that authorization or the
                // byte estimate, including the rename-only fast path.
                let current_source = query(&source).await?;
                if !same_version(&incoming, &current_source) {
                    incoming = current_source;
                    directory = incoming.file_type() == gio::FileType::Directory;
                    self.progress.invalidate_total();
                    self.progress.report();
                    self.policies = [None; 3];
                    may_replace = false;
                    try_rename = self.moving
                        && directory
                        && source.path().is_some()
                        && target.path().is_some();
                    continue;
                }
                if directory
                    && existing
                        .as_ref()
                        .is_none_or(|info| info.file_type() == gio::FileType::Directory)
                {
                    super::transfer::validate_directory_target(
                        &source,
                        &target,
                        &self.cancellation,
                    )
                    .await?;
                    if existing.is_none() && try_rename {
                        try_rename = false;
                        match super::transfer::transfer_item(
                            source.clone(),
                            target.clone(),
                            true,
                            false,
                            self.cancellation.clone(),
                        )
                        .await
                        {
                            Ok(()) => {
                                (self.relocation)(&source, &target);
                                return Ok(Outcome {
                                    complete: true,
                                    bytes: 0,
                                });
                            }
                            Err(error) if error.matches(gio::IOErrorEnum::Exists) => {
                                may_replace = false;
                                continue;
                            }
                            Err(error)
                                if error.matches(gio::IOErrorEnum::NotSupported)
                                    || error.matches(gio::IOErrorEnum::WouldRecurse) => {}
                            Err(error) => return Err(error),
                        }
                    }
                    if existing.is_none() {
                        match target.make_directory_future(glib::Priority::DEFAULT).await {
                            Ok(()) => {}
                            Err(error) if error.matches(gio::IOErrorEnum::Exists) => {
                                may_replace = false;
                                continue;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    let enumerator = source
                        .enumerate_children_future(
                            ATTRIBUTES,
                            gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                            glib::Priority::DEFAULT,
                        )
                        .await?;
                    let mut outcome = Outcome {
                        complete: true,
                        bytes: 0,
                    };
                    loop {
                        self.check_cancelled()?;
                        let children = enumerator
                            .next_files_future(64, glib::Priority::DEFAULT)
                            .await?;
                        if children.is_empty() {
                            break;
                        }
                        for child in children {
                            let child_outcome = self
                                .transfer(
                                    source.child(child.name()),
                                    target.child(child.name()),
                                    false,
                                    depth + 1,
                                    None,
                                )
                                .await?;
                            outcome.complete &= child_outcome.complete;
                            outcome.bytes = outcome.bytes.saturating_add(child_outcome.bytes);
                        }
                    }
                    enumerator.close_future(glib::Priority::DEFAULT).await?;
                    if self.moving && outcome.complete {
                        self.check_cancelled()?;
                        source.delete_future(glib::Priority::DEFAULT).await?;
                        (self.relocation)(&source, &target);
                    }
                    return Ok(outcome);
                }
                let size = if incoming.file_type() == gio::FileType::Regular {
                    incoming.size().max(0) as u64
                } else {
                    0
                };
                let progress = self.progress.clone();
                let regular = incoming.file_type() == gio::FileType::Regular;
                let report: super::transfer::ByteProgress = Rc::new(move |current, total| {
                    if regular {
                        progress.update_file(current, total, size);
                    }
                });
                match super::transfer::transfer_item_with_progress(
                    source.clone(),
                    target.clone(),
                    self.moving,
                    may_replace,
                    self.cancellation.clone(),
                    report,
                )
                .await
                {
                    Ok(()) => {
                        if self.moving {
                            (self.relocation)(&source, &target);
                        }
                        self.progress.finish_file(size);
                        return Ok(Outcome {
                            complete: true,
                            bytes: size,
                        });
                    }
                    Err(error) if error.matches(gio::IOErrorEnum::Exists) => {
                        self.progress.abandon_file();
                        self.policies = [None; 3];
                        may_replace = false;
                    }
                    Err(error) => {
                        self.progress.abandon_file();
                        return Err(error);
                    }
                }
            }
            Err(error(
                "The destination kept changing; the source was retained",
            ))
        })
    }
}

fn same_version(before: &gio::FileInfo, after: &gio::FileInfo) -> bool {
    before.file_type() == after.file_type()
        && before.size() == after.size()
        && [
            "id::file",
            "etag::value",
            "time::modified",
            "time::modified-usec",
            "time::changed",
            "time::changed-usec",
        ]
        .iter()
        .all(|attribute| {
            before.attribute_as_string(attribute) == after.attribute_as_string(attribute)
        })
}

async fn query(file: &gio::File) -> Result<gio::FileInfo, glib::Error> {
    file.query_info_future(
        ATTRIBUTES,
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        glib::Priority::DEFAULT,
    )
    .await
}

async fn available_copy(target: &gio::File, directory: bool) -> Result<gio::File, glib::Error> {
    let name = target
        .basename()
        .ok_or_else(|| error("The item has no file name"))?;
    let parent = target
        .parent()
        .ok_or_else(|| error("The destination has no parent"))?;
    let name = Path::new(&name);
    let (stem, extension) = if directory {
        (name.as_os_str(), None)
    } else {
        (
            name.file_stem().unwrap_or(name.as_os_str()),
            name.extension(),
        )
    };
    for index in 1..=10_000 {
        let mut candidate = stem.to_os_string();
        candidate.push(if index == 1 {
            " (copy)".to_owned()
        } else {
            format!(" (copy {index})")
        });
        if let Some(extension) = extension {
            candidate.push(".");
            candidate.push(extension);
        }
        let file = parent.child(candidate);
        match query(&file).await {
            Err(error) if error.matches(gio::IOErrorEnum::NotFound) => return Ok(file),
            Err(error) => return Err(error),
            Ok(_) => {}
        }
    }
    Err(error(
        "No free copy name was found; the source was retained",
    ))
}

fn entry(file: &gio::File, info: &gio::FileInfo) -> FileEntry {
    FileEntry(Rc::new(FileEntryInner {
        location: file
            .path()
            .map(Location::local)
            .unwrap_or_else(|| Location::uri(file.uri())),
        native_name: info.name().into_os_string(),
        display_name: info.display_name().to_string(),
        kind: match info.file_type() {
            gio::FileType::Directory => EntryKind::Directory,
            gio::FileType::SymbolicLink => EntryKind::SymbolicLink,
            _ => EntryKind::File,
        },
        size: if info.file_type() == gio::FileType::Directory {
            MetadataValue::Unknown
        } else {
            MetadataValue::Known(info.size().max(0) as u64)
        },
        modified_unix_seconds: info
            .modification_date_time()
            .map(|time| MetadataValue::Known(time.to_unix()))
            .unwrap_or(MetadataValue::Unknown),
        music_tags: None,
    }))
}

fn relocate(source: &gio::File, target: &gio::File) {
    if let Some((from, to)) = source.path().zip(target.path()) {
        if let Err(error) =
            crate::services::music_collections::Store::shared().relocate(from.clone(), to.clone())
        {
            tracing::warn!(%error, "Unable to update moved collection references");
        }
        if let Err(error) =
            crate::services::ratings::Store::shared().relocate(from.clone(), to.clone())
        {
            tracing::warn!(%error, "Unable to update moved rating references");
        }
        crate::services::FolderViewStore::shared()
            .relocate(&from.to_string_lossy(), &to.to_string_lossy());
    }
}

fn error(message: &str) -> glib::Error {
    glib::Error::new(gio::IOErrorEnum::NotSupported, message)
}

#[cfg(test)]
mod tests;
