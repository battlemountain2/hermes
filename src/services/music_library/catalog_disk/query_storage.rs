//! Private staged files shared by the normalized query catalog.

use super::*;

pub(crate) struct Staged {
    // Close SQLite before removing the private stage or releasing its lock.
    connection: Option<Connection>,
    stage: Option<Stage>,
    directory: DirectoryHandle,
    name: OsString,
    sources: Sources,
    reused: bool,
}

impl Staged {
    pub(crate) fn open(
        path: &Path,
        root: &Path,
        cancellation: &Cancellation,
    ) -> Result<Self, String> {
        cancelled(cancellation)?;
        let sources = Sources::new(root)?;
        let (directory, name) = location(path, true, cancellation)?;
        let mut stage = Stage::build(&directory, cancellation)?;
        let mut reused = match fs::symlink_metadata(directory.pinned().join(&name)) {
            Ok(_) => {
                stage.copy(existing(&directory, &name)?, cancellation)?;
                true
            }
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => false,
            Err(failure) => return Err(error(failure)),
        };
        let connection = match connect(&stage, true) {
            Ok(connection) => connection,
            Err(failure) if reused => {
                if versioned_database(&stage)? {
                    // An incompatible schema may fail before SQL can read its
                    // version. Preserve every versioned file in this case.
                    return Err(failure);
                }
                // Only the copied, private database is discarded. Publication
                // leaves the previous cache intact until a new build succeeds.
                drop(stage);
                stage = Stage::build(&directory, cancellation)?;
                reused = false;
                connect(&stage, true)?
            }
            Err(failure) => return Err(failure),
        };
        cancelled(cancellation)?;
        sources.root_unchanged()?;
        Ok(Self {
            connection: Some(connection),
            stage: Some(stage),
            directory,
            name,
            sources,
            reused,
        })
    }

    pub(crate) fn reused(&self) -> bool {
        self.reused
    }

    pub(crate) fn connection(&self) -> &Connection {
        self.connection
            .as_ref()
            .expect("live query staging connection")
    }

    pub(crate) fn connection_mut(&mut self) -> &mut Connection {
        self.connection
            .as_mut()
            .expect("live query staging connection")
    }

    pub(crate) fn root_key(&self) -> Vec<u8> {
        [self.sources.identity.device, self.sources.identity.inode]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect()
    }

    pub(crate) fn reset(&mut self, cancellation: &Cancellation) -> Result<(), String> {
        if let Some(connection) = self.connection.take() {
            connection.close().map_err(|(_, failure)| error(failure))?;
        }
        self.stage.take();
        let stage = Stage::build(&self.directory, cancellation)?;
        self.connection = Some(connect(&stage, true)?);
        self.stage = Some(stage);
        self.reused = false;
        Ok(())
    }

    pub(crate) fn publish(mut self, cancellation: &Cancellation) -> Result<Database, String> {
        cancelled(cancellation)?;
        self.sources.root_unchanged()?;
        self.connection
            .take()
            .expect("live query staging connection")
            .close()
            .map_err(|(_, failure)| error(failure))?;
        let mut stage = self.stage.take().expect("live query staging directory");
        stage.seal(cancellation)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | NON_BLOCK)
            .open(stage.file())
            .map_err(error)?;
        private_file(&file)?;
        file.sync_all().map_err(error)?;

        // The active reader keeps its original private inode. A separate copy
        // can replace the public cache without changing this revision's pages.
        let published = Stage::copied(&self.directory, file, cancellation)?;
        let output = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | NON_BLOCK)
            .open(published.file())
            .map_err(error)?;
        private_file(&output)?;
        output.sync_all().map_err(error)?;
        let connection = connect(&stage, false)?;
        self.sources.root_unchanged()?;
        cancelled(cancellation)?;
        cache_policy::publish(&self.directory, &self.name, &published, cancellation)?;
        self.directory.0.sync_all().map_err(error)?;
        // Stage parent descriptors share this lock description. Release it
        // explicitly before returning a long-lived reader.
        self.directory.0.unlock().map_err(error)?;
        Ok(Database {
            connection,
            _stage: stage,
            sources: self.sources,
        })
    }
}

pub(crate) struct Database {
    connection: Connection,
    _stage: Stage,
    sources: Sources,
}

impl Database {
    pub(crate) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(crate) fn check_root(&self) -> Result<(), String> {
        self.sources.root_unchanged()
    }
}

pub(crate) fn encode_identity(identity: &FileIdentity) -> Vec<u8> {
    identity_bytes(identity)
}

pub(crate) fn decode_identity(bytes: &[u8]) -> Result<FileIdentity, String> {
    super::decode_identity(bytes)
}

fn connect(stage: &Stage, writable: bool) -> Result<Connection, String> {
    let connection = stage.open_connection(writable)?;
    configure_with_limits(&connection, writable, 64 * 1024, 32)?;
    if writable {
        sqlite_capacity::enforce(&connection, stage.allowance)?;
    }
    connection
        .execute_batch("PRAGMA temp_store=FILE;")
        .map_err(error)?;
    Ok(connection)
}

fn versioned_database(stage: &Stage) -> Result<bool, String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(stage.file())
        .map_err(error)?;
    let mut header = [0u8; 64];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(header.starts_with(b"SQLite format 3\0")
            && u32::from_be_bytes(
                header[60..64]
                    .try_into()
                    .expect("fixed SQLite header field"),
            ) > 0),
        Err(failure) if failure.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(failure) => Err(error(failure)),
    }
}

#[cfg(test)]
mod tests;
