use super::*;
use std::os::unix::fs::OpenOptionsExt;

pub(super) struct Storage {
    path: PathBuf,
    expected: Option<Vec<u8>>,
    _ownership: fs::File,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self, String> {
        fs::create_dir_all(path.parent().ok_or("Invalid collections path")?)
            .map_err(|error| error.to_string())?;
        let lock_path = path.with_extension("json.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| !metadata.is_file()) {
            return Err("Collection ownership file is not a regular file".into());
        }
        let ownership = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(lock_path)
            .map_err(|error| error.to_string())?;
        let mut attempts = 0;
        loop {
            match ownership.try_lock() {
                Ok(()) => break,
                Err(error) => {
                    attempts += 1;
                    if attempts >= 10 {
                        return Err(format!(
                            "Collections are in use by another Hermes instance or cannot be locked: {error}. Close the other instance and restart to edit."
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
        Ok(Self {
            path: path.to_owned(),
            expected: read_bytes(path)?,
            _ownership: ownership,
        })
    }

    pub fn load(&self) -> Result<Collections, String> {
        let Some(bytes) = self.expected.as_ref() else {
            return Ok(Collections::default());
        };
        let data = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        validate(&data)?;
        Ok(data)
    }

    pub fn save(&mut self, data: &Collections) -> Result<(), String> {
        let expected = serde_json::to_vec(data).map_err(|error| error.to_string())?;
        write_guarded(&self.path, data, || {
            if read_bytes(&self.path)? == self.expected {
                Ok(())
            } else {
                Err("Collections changed outside this Hermes instance; the external file is preserved. Restart before editing further.".into())
            }
        })?;
        self.expected = Some(expected);
        Ok(())
    }
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_BYTES => {}
        Ok(_) => return Err("Collection file is not a regular file or exceeds 16 MiB".into()),
        Err(error) => return Err(error.to_string()),
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|error| error.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Collections exceed 16 MiB".into());
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests;
