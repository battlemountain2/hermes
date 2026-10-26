use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{Document, MAX_FILE_BYTES};

const NO_FOLLOW: i32 = 0o400000;
const NON_BLOCKING: i32 = 0o4000;

pub(super) struct Storage {
    path: PathBuf,
    directory: File,
    ownership: File,
    lock_path: PathBuf,
    expected: Option<Vec<u8>>,
}

impl Storage {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() || path.file_name().is_none() {
            return Err("Saved search storage path must be an absolute file path".into());
        }
        let parent = path.parent().ok_or("Invalid saved search storage path")?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(|error| error.to_string())?;
        let metadata = fs::symlink_metadata(parent).map_err(|error| error.to_string())?;
        if !metadata.is_dir() {
            return Err("Saved search directory must not be a symlink".into());
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | NON_BLOCKING)
            .open(parent)
            .map_err(|error| error.to_string())?;
        same_file(&directory, parent)?;
        if directory
            .metadata()
            .map_err(|error| error.to_string())?
            .permissions()
            .mode()
            & 0o777
            != 0o700
        {
            return Err("Saved search directory must have private 0700 permissions".into());
        }
        let path = parent
            .canonicalize()
            .map_err(|error| error.to_string())?
            .join(path.file_name().ok_or("Invalid saved search filename")?);
        let lock_path = path.with_file_name(format!(
            "{}.lock",
            path.file_name()
                .ok_or("Invalid saved search filename")?
                .to_string_lossy()
        ));
        reject_special(&lock_path)?;
        let ownership = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(NO_FOLLOW | NON_BLOCKING)
            .open(&lock_path)
            .map_err(|error| error.to_string())?;
        same_file(&ownership, &lock_path)?;
        ownership.try_lock().map_err(|error| {
            format!("Saved searches are in use by another owner or cannot be locked: {error}. Reload to retry.")
        })?;
        let storage = Self {
            path,
            directory,
            ownership,
            lock_path,
            expected: None,
        };
        storage.check_ownership()?;
        Ok(storage)
    }

    pub(super) fn load(&mut self) -> Result<Document, String> {
        self.check_ownership()?;
        let expected = read_bytes(&self.path)?;
        let mut data: Document = expected.as_ref().map_or_else(
            || Ok(Document::default()),
            |bytes| serde_json::from_slice(bytes).map_err(|error| error.to_string()),
        )?;
        data.validate()?;
        let encoded = serde_json::to_vec(&data).map_err(|error| error.to_string())?;
        if encoded.len() as u64 > MAX_FILE_BYTES {
            return Err("Canonical saved searches exceed 1 MiB".into());
        }
        self.expected = expected;
        Ok(data)
    }

    pub(super) fn save(&mut self, data: &Document, changed: bool) -> Result<(), String> {
        self.check_expected()?;
        if !changed {
            return Ok(());
        }
        let bytes = serde_json::to_vec(data).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err("Saved searches exceed 1 MiB; the change was not saved".into());
        }
        let (temporary, mut file) = self.temporary()?;
        let result: Result<(), String> = (|| {
            file.write_all(&bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            same_file(&file, &temporary.0)?;
            self.check_expected()?;
            if self.expected.is_none() {
                fs::hard_link(&temporary.0, &self.path).map_err(|error| error.to_string())?;
                fs::remove_file(&temporary.0).map_err(|error| error.to_string())?;
            } else {
                fs::rename(&temporary.0, &self.path).map_err(|error| error.to_string())?;
            }
            self.directory
                .sync_all()
                .map_err(|error| error.to_string())?;
            Ok(())
        })();
        drop(temporary);
        result?;
        self.expected = Some(bytes);
        Ok(())
    }

    fn check_ownership(&self) -> Result<(), String> {
        same_file(
            &self.directory,
            self.path.parent().ok_or("Invalid saved search path")?,
        )?;
        same_file(&self.ownership, &self.lock_path)?;
        if self
            .directory
            .metadata()
            .map_err(|error| error.to_string())?
            .permissions()
            .mode()
            & 0o777
            != 0o700
            || self
                .ownership
                .metadata()
                .map_err(|error| error.to_string())?
                .permissions()
                .mode()
                & 0o777
                != 0o600
        {
            return Err(
                "Saved search storage permissions changed; reload after restoring privacy".into(),
            );
        }
        Ok(())
    }

    fn check_expected(&self) -> Result<(), String> {
        self.check_ownership()?;
        if read_bytes(&self.path)? != self.expected {
            return Err("Saved searches changed outside this store; the external file is preserved. Reload before editing.".into());
        }
        Ok(())
    }

    fn temporary(&self) -> Result<(Temporary, File), String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = self.path.parent().ok_or("Invalid saved search path")?;
        for _attempt in 0..32 {
            let path = parent.join(format!(
                ".queries-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(file) => return Ok((Temporary(path), file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("Unable to create a private saved search temporary file".into())
    }
}

fn reject_special(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.nlink() == 1 => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err("Saved search storage must use regular, unlinked files, not symlinks".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn same_file(file: &File, path: &Path) -> Result<(), String> {
    let opened = file.metadata().map_err(|error| error.to_string())?;
    let current = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if opened.dev() != current.dev()
        || opened.ino() != current.ino()
        || current.file_type().is_symlink()
        || (!current.is_dir() && (!current.is_file() || current.nlink() != 1))
    {
        return Err("Saved search storage ownership changed; existing files are preserved".into());
    }
    Ok(())
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    reject_special(path)?;
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW | NON_BLOCKING)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    same_file(&file, path)?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err("Saved search file exceeds 1 MiB".into());
    }
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err("Saved search file must have private 0600 permissions".into());
    }
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("Saved search file exceeds 1 MiB".into());
    }
    same_file(&file, path)?;
    Ok(Some(bytes))
}

struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _removed = fs::remove_file(&self.0);
    }
}
