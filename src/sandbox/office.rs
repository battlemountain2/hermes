use std::{
    collections::VecDeque,
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, TryLockError,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use super::{Cancellation, ParseOperation, ParseOutput, PrivateOutput};

mod disk;

const CACHE_BYTES: u64 = 64 * 1024 * 1024;
const CACHE_ENTRIES: usize = 4;
const CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, Eq, PartialEq)]
struct Identity {
    path: PathBuf,
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    renderer_key: Option<disk::Key>,
}

impl Identity {
    fn read(path: &Path) -> Result<Self, String> {
        let mut identity = Self::read_source(path)?;
        identity.renderer_key = disk::key(&identity.path).ok();
        Ok(identity)
    }

    fn read_source(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > 50 * 1024 * 1024 {
            return Err("Office input exceeds the document preview limit".into());
        }
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
            bytes: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            renderer_key: None,
        })
    }
}

struct Document {
    bytes: Vec<u8>,
}

impl Document {
    fn new(bytes: Vec<u8>) -> Result<Self, String> {
        if !bytes.starts_with(b"%PDF-") || bytes.len() as u64 > super::MAX_OUTPUT_BYTES {
            return Err("Invalid converted Office PDF".into());
        }
        Ok(Self { bytes })
    }
}

#[derive(Default)]
struct Cache(VecDeque<(Identity, Arc<Document>, Instant)>);

impl Cache {
    fn document(
        &mut self,
        identity: &Identity,
        cancellation: &Cancellation,
        convert: impl FnOnce() -> Result<Vec<u8>, String>,
    ) -> Result<Arc<Document>, String> {
        self.0
            .retain(|(_, _, created)| created.elapsed() < CACHE_TTL);
        if let Some(index) = self.0.iter().position(|(source, _, _)| source == identity) {
            let entry = self
                .0
                .remove(index)
                .ok_or("Office cache entry disappeared")?;
            let document = entry.1.clone();
            self.0.push_back(entry);
            return Ok(document);
        }
        if cancellation.is_cancelled() {
            return Err("Preview cancelled".into());
        }
        let bytes = convert()?;
        if cancellation.is_cancelled() || Identity::read(&identity.path).as_ref() != Ok(identity) {
            return Err("Office source changed or preview was cancelled".into());
        }
        let document = Arc::new(Document::new(bytes)?);
        self.0.retain(|(source, _, _)| source.path != identity.path);
        while self.0.len() >= CACHE_ENTRIES
            || self
                .0
                .iter()
                .map(|(_, document, _)| document.bytes.len() as u64)
                .sum::<u64>()
                + document.bytes.len() as u64
                > CACHE_BYTES
        {
            self.0.pop_front();
        }
        self.0
            .push_back((identity.clone(), document.clone(), Instant::now()));
        Ok(document)
    }
}

pub(crate) fn available() -> bool {
    crate::sandbox_helper::office::render::renderer().is_some()
}

pub(crate) fn poster(input: &Path, cancellation: &Cancellation) -> Result<Vec<u8>, String> {
    let identity = Identity::read_source(input)?;
    let output = super::parse(
        &identity.path,
        ParseOperation::OfficePoster,
        800,
        cancellation,
    )?;
    if cancellation.is_cancelled() || Identity::read_source(input).as_ref() != Ok(&identity) {
        return Err("Office source changed or preview was cancelled".into());
    }
    Ok(output.data)
}

#[derive(Default)]
struct Admission {
    converter: Mutex<()>,
    foreground: AtomicUsize,
}

struct Foreground<'a>(&'a AtomicUsize);

impl Drop for Foreground<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Admission {
    fn run<Output>(
        &self,
        background: bool,
        cancellation: &Cancellation,
        operation: impl FnOnce() -> Result<Output, String>,
    ) -> Result<Output, String> {
        let _foreground = if background {
            None
        } else {
            self.foreground.fetch_add(1, Ordering::AcqRel);
            Some(Foreground(&self.foreground))
        };
        let started = Instant::now();
        loop {
            if cancellation.is_cancelled() {
                return Err("Preview cancelled".into());
            }
            if started.elapsed() >= super::OFFICE_WALL_TIME_LIMIT {
                return Err("Office conversion queue timed out".into());
            }
            if background && self.foreground.load(Ordering::Acquire) > 0 {
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            match self.converter.try_lock() {
                Ok(_admitted) => {
                    if background && self.foreground.load(Ordering::Acquire) > 0 {
                        continue;
                    }
                    return operation();
                }
                Err(TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(20)),
                Err(TryLockError::Poisoned(_)) => {
                    return Err("Office conversion queue is unavailable".into());
                }
            }
        }
    }
}

pub(super) fn admit<Output>(
    cancellation: &Cancellation,
    background: bool,
    operation: impl FnOnce() -> Result<Output, String>,
) -> Result<Output, String> {
    static ADMISSION: OnceLock<Admission> = OnceLock::new();
    ADMISSION
        .get_or_init(Admission::default)
        .run(background, cancellation, operation)
}

pub(crate) fn render(
    input: &Path,
    page: i32,
    cancellation: &Cancellation,
) -> Result<ParseOutput, String> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".into());
    }
    if !available() {
        return Err("Install LibreOffice for page and slide previews".into());
    }
    render_cached(
        input,
        page,
        cancellation,
        CACHE.get_or_init(|| Mutex::new(Cache::default())),
        |source| {
            let key = disk::key(source).ok();
            if let Some(bytes) = key.as_ref().and_then(|key| disk::load(key, cancellation)) {
                tracing::debug!("Office converted-document disk cache hit");
                return Ok(bytes);
            }
            let bytes =
                super::parse(source, ParseOperation::ConvertOfficePdf, 0, cancellation)?.data;
            if let Some(key) = key.as_ref()
                && let Err(error) = disk::store(key, &bytes, cancellation)
            {
                tracing::debug!(%error, "Office converted-document cache was not stored");
            }
            Ok(bytes)
        },
    )
}

fn render_cached(
    input: &Path,
    page: i32,
    cancellation: &Cancellation,
    cache: &Mutex<Cache>,
    convert: impl FnOnce(&Path) -> Result<Vec<u8>, String>,
) -> Result<ParseOutput, String> {
    let identity = Identity::read(input)?;
    let started = Instant::now();
    let document = loop {
        if cancellation.is_cancelled() {
            return Err("Preview cancelled".into());
        }
        if started.elapsed() >= super::OFFICE_WALL_TIME_LIMIT {
            return Err("Office conversion queue timed out".into());
        }
        match cache.try_lock() {
            Ok(mut cache) => {
                break cache.document(&identity, cancellation, || convert(&identity.path))?;
            }
            Err(TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(20)),
            Err(TryLockError::Poisoned(_)) => {
                return Err("Office preview cache is unavailable".into());
            }
        }
    };
    let directory = PrivateOutput::create().map_err(|error| error.to_string())?;
    let path = directory.path().join("document.pdf");
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut file| file.write_all(&document.bytes))
        .map_err(|error| error.to_string())?;
    let output = super::parse(&path, ParseOperation::PreviewPdf, page.max(0), cancellation)?;
    if cancellation.is_cancelled() || Identity::read(input).as_ref() != Ok(&identity) {
        return Err("Office source changed or preview was cancelled".into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod layout_tests;
