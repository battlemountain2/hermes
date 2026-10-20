use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use crate::sandbox::Cancellation;

use super::{Lyrics, parse_lrc, parse_plain};

#[cfg(test)]
mod tests;

pub const SOURCE: &str = "LRCLIB";
const ENDPOINT: &str = "https://lrclib.net/api/get";
const MAX_METADATA_BYTES: usize = 512;
const MAX_JSON_BYTES: usize = 1024 * 1024;
const MAX_STDERR_BYTES: usize = 4096;
const DEADLINE: Duration = Duration::from_secs(13);
const LOOKUP_DEADLINE: Duration = Duration::from_secs(20);
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(500), Duration::from_millis(1500)];
const NO_FOLLOW: i32 = 0o400000;
const NONBLOCK: i32 = 0o4000;
const DIRECTORY: i32 = 0o200000;
const CACHE_SLOTS: u64 = 32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Query {
    title: String,
    artist: String,
    duration_seconds: f64,
}

impl Query {
    pub fn new(title: &str, artist: &str, duration_seconds: f64) -> Result<Self, String> {
        let title = validate_metadata(title)?;
        let artist = validate_metadata(artist)?;
        if matches!(
            artist.to_lowercase().as_str(),
            "unknown"
                | "unknown artist"
                | "unknown_artist"
                | "unknown-artist"
                | "unknownartist"
                | "<unknown>"
                | "[unknown]"
                | "(unknown)"
                | "n/a"
        ) {
            return Err("Lyrics lookup requires a known artist".to_owned());
        }
        if !duration_seconds.is_finite() || !(1.0..=3600.0).contains(&duration_seconds) {
            return Err("Lyrics duration must be between 1 and 3600 seconds".to_owned());
        }
        Ok(Self {
            title: title.to_owned(),
            artist: artist.to_owned(),
            duration_seconds,
        })
    }

    fn url(&self) -> String {
        format!(
            "{ENDPOINT}?track_name={}&artist_name={}&duration={}",
            encode(&self.title),
            encode(&self.artist),
            self.duration_seconds
        )
    }

    fn filename(&self) -> String {
        let mut hash = 0xcbf29ce484222325u64;
        for bytes in [
            self.title.as_bytes(),
            self.artist.as_bytes(),
            &self.duration_seconds.to_bits().to_le_bytes(),
        ] {
            for byte in bytes {
                hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
            }
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("lyrics-v1-{:02}.json", hash % CACHE_SLOTS)
    }
}

fn validate_metadata(value: &str) -> Result<&str, String> {
    if value.len() > MAX_METADATA_BYTES || value.chars().any(char::is_control) {
        return Err("Invalid lyrics metadata".to_owned());
    }
    let value = value.trim();
    if value.is_empty() {
        return Err("Lyrics lookup requires title and artist".to_owned());
    }
    Ok(value)
}

fn encode(value: &str) -> String {
    let mut encoded = String::new();
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    encoded
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Lookup {
    Found(Lyrics),
    Instrumental,
    NotFound,
}

pub fn lookup(query: &Query, cancellation: &Cancellation) -> Result<Lookup, String> {
    let directory = gtk::glib::user_cache_dir().join("strata/lyrics-lrclib");
    lookup_with(query, cancellation, &directory, fetch)
}

fn check_cancelled(cancellation: &Cancellation) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err("Lyrics lookup cancelled".to_owned())
    } else {
        Ok(())
    }
}

#[derive(Debug)]
struct Response {
    status: u16,
    body: Vec<u8>,
}

fn lookup_with(
    query: &Query,
    cancellation: &Cancellation,
    directory: &Path,
    transport: impl FnOnce(&Query, &Cancellation) -> Result<Response, String>,
) -> Result<Lookup, String> {
    check_cancelled(cancellation)?;
    if Query::new(&query.title, &query.artist, query.duration_seconds).as_ref() != Ok(query) {
        return Err("Invalid lyrics query".to_owned());
    }
    let cache = Cache::open(directory).ok();
    if let Some(response) = cache.as_ref().and_then(|cache| cache.read(query)) {
        check_cancelled(cancellation)?;
        return classify(query, &response);
    }
    check_cancelled(cancellation)?;
    let response = transport(query, cancellation)?;
    check_cancelled(cancellation)?;
    if response.body.len() > MAX_JSON_BYTES {
        return Err("LRCLIB response exceeds its byte budget".to_owned());
    }
    if response.status == 404 {
        return Ok(Lookup::NotFound);
    }
    if response.status != 200 {
        return Err(match response.status {
            429 => "LRCLIB is receiving too many requests. Please try again later.".to_owned(),
            500 | 502 | 503 | 504 => {
                "LRCLIB is temporarily unavailable. Please try again shortly; local lyrics still work."
                    .to_owned()
            }
            status => format!("LRCLIB returned HTTP {status}"),
        });
    }
    let record: Record =
        serde_json::from_slice(&response.body).map_err(|_| "Invalid LRCLIB response".to_owned())?;
    let result = classify(query, &record)?;
    check_cancelled(cancellation)?;
    if matches!(result, Lookup::Found(_))
        && let Some(cache) = cache
    {
        let _cached = cache.write(query, &record);
    }
    check_cancelled(cancellation)?;
    Ok(result)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    track_name: String,
    artist_name: String,
    duration: f64,
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

fn classify(query: &Query, record: &Record) -> Result<Lookup, String> {
    let title = validate_metadata(&record.track_name)?;
    let artist = validate_metadata(&record.artist_name)?;
    if !record.duration.is_finite() || !(1.0..=3600.0).contains(&record.duration) {
        return Err("Invalid LRCLIB duration".to_owned());
    }
    if title.to_lowercase() != query.title.to_lowercase()
        || artist.to_lowercase() != query.artist.to_lowercase()
        || (record.duration - query.duration_seconds).abs() > 2.0
    {
        return Ok(Lookup::NotFound);
    }
    if record.instrumental {
        return Ok(Lookup::Instrumental);
    }
    let synced = record
        .synced_lyrics
        .as_deref()
        .and_then(parse_lrc)
        .filter(|lyrics| lyrics.lines.iter().any(|line| !line.text.is_empty()));
    let lyrics = synced.or_else(|| record.plain_lyrics.as_deref().and_then(parse_plain));
    Ok(lyrics.map_or(Lookup::NotFound, Lookup::Found))
}

fn curl_command(query: &Query) -> Command {
    let mut command = Command::new("/usr/bin/curl");
    command
        .env_clear()
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--globoff",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-redirs",
            "0",
            "--proxy",
            "",
            "--noproxy",
            "*",
            "--connect-timeout",
            "4",
            "--max-time",
            "12",
            "--max-filesize",
            "1048576",
            "--header",
            "Accept: application/json",
            "--user-agent",
            "Hermes-Lyrics",
            "--write-out",
            "\n%{http_code}",
            "--url",
        ])
        .arg(query.url())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn read_bounded(mut reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Cannot read lyrics transport output".to_owned())?;
    if bytes.len() > limit {
        return Err("Lyrics transport output exceeds its byte budget".to_owned());
    }
    Ok(bytes)
}

struct Process {
    child: Child,
    readers: Vec<JoinHandle<()>>,
}

impl Drop for Process {
    fn drop(&mut self) {
        let _killed = self.child.kill();
        let _waited = self.child.wait();
        for reader in self.readers.drain(..) {
            let _joined = reader.join();
        }
    }
}

fn fetch(query: &Query, cancellation: &Cancellation) -> Result<Response, String> {
    fetch_with_retry(
        query,
        cancellation,
        |query, cancellation, remaining| {
            let child = curl_command(query).spawn().map_err(|_| {
                "Online lyrics require the optional /usr/bin/curl program".to_owned()
            })?;
            collect(child, cancellation, DEADLINE.min(remaining))
        },
        wait_for_retry,
    )
}

fn wait_for_retry(delay: Duration, cancellation: &Cancellation) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < delay {
        check_cancelled(cancellation)?;
        thread::sleep(Duration::from_millis(10).min(delay.saturating_sub(started.elapsed())));
    }
    check_cancelled(cancellation)?;
    Ok(())
}

fn fetch_with_retry(
    query: &Query,
    cancellation: &Cancellation,
    mut transport: impl FnMut(&Query, &Cancellation, Duration) -> Result<Response, String>,
    mut wait: impl FnMut(Duration, &Cancellation) -> Result<(), String>,
) -> Result<Response, String> {
    let started = Instant::now();
    for attempt in 0..=RETRY_DELAYS.len() {
        check_cancelled(cancellation)?;
        let remaining = LOOKUP_DEADLINE.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("Lyrics lookup timed out".to_owned());
        }
        let response = transport(query, cancellation, remaining)?;
        check_cancelled(cancellation)?;
        if response.body.len() > MAX_JSON_BYTES {
            return Err("LRCLIB response exceeds its byte budget".to_owned());
        }
        let Some(delay) = RETRY_DELAYS.get(attempt) else {
            return Ok(response);
        };
        if !matches!(response.status, 500 | 502 | 503 | 504)
            || *delay >= LOOKUP_DEADLINE.saturating_sub(started.elapsed())
        {
            return Ok(response);
        }
        wait(*delay, cancellation)?;
    }
    unreachable!()
}

fn collect(
    child: Child,
    cancellation: &Cancellation,
    deadline: Duration,
) -> Result<Response, String> {
    let started = Instant::now();
    let mut process = Process {
        child,
        readers: Vec::new(),
    };
    let stdout = process
        .child
        .stdout
        .take()
        .ok_or("Missing lyrics transport stdout")?;
    let stderr = process
        .child
        .stderr
        .take()
        .ok_or("Missing lyrics transport stderr")?;
    let (sender, receiver) = mpsc::sync_channel(2);
    for (reader, limit, stdout_stream) in [
        (
            Box::new(stdout) as Box<dyn Read + Send>,
            MAX_JSON_BYTES + 4,
            true,
        ),
        (
            Box::new(stderr) as Box<dyn Read + Send>,
            MAX_STDERR_BYTES,
            false,
        ),
    ] {
        let sender = sender.clone();
        let handle = thread::Builder::new()
            .name("lyrics-response".to_owned())
            .spawn(move || {
                let result = read_bounded(reader, limit);
                let _sent = sender.send((stdout_stream, result));
            })
            .map_err(|_| "Cannot start lyrics transport reader".to_owned())?;
        process.readers.push(handle);
    }
    drop(sender);
    let mut stdout = None;
    let mut stderr = None;
    let mut status: Option<ExitStatus> = None;
    loop {
        check_cancelled(cancellation)?;
        if started.elapsed() >= deadline {
            return Err("Lyrics lookup timed out".to_owned());
        }
        while let Ok((stdout_stream, result)) = receiver.try_recv() {
            if stdout_stream {
                stdout = Some(result?);
            } else {
                stderr = Some(result?);
            }
        }
        if status.is_none() {
            status = process
                .child
                .try_wait()
                .map_err(|_| "Cannot wait for lyrics transport".to_owned())?;
        }
        if let Some(status) = status
            && stdout.is_some()
            && stderr.is_some()
        {
            if !status.success() {
                return Err("LRCLIB network request failed".to_owned());
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    check_cancelled(cancellation)?;
    let mut body = stdout.ok_or("Missing lyrics response")?;
    let suffix = body
        .len()
        .checked_sub(4)
        .ok_or("Missing LRCLIB HTTP status")?;
    let status = &body[suffix..];
    if status[0] != b'\n' || !status[1..].iter().all(u8::is_ascii_digit) {
        return Err("Invalid LRCLIB HTTP status".to_owned());
    }
    let status = u16::from(status[1] - b'0') * 100
        + u16::from(status[2] - b'0') * 10
        + u16::from(status[3] - b'0');
    body.truncate(suffix);
    if body.len() > MAX_JSON_BYTES {
        return Err("LRCLIB response exceeds its byte budget".to_owned());
    }
    Ok(Response { status, body })
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    version: u32,
    source: String,
    query: Query,
    record: Record,
}

struct Cache {
    directory: File,
}

impl Cache {
    fn open(directory: &Path) -> Result<Self, String> {
        if !directory.is_absolute() {
            return Err("Lyrics cache path must be absolute".to_owned());
        }
        let mut parent = PathBuf::new();
        for component in directory.components() {
            if !matches!(component, Component::RootDir | Component::Normal(_)) {
                return Err("Invalid lyrics cache path".to_owned());
            }
            parent.push(component);
            match fs::symlink_metadata(&parent) {
                Ok(metadata) if metadata.is_dir() => (),
                Ok(_) => return Err("Lyrics cache must not traverse symlinks".to_owned()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::DirBuilder::new()
                        .mode(0o700)
                        .create(&parent)
                        .map_err(|_| "Cannot create lyrics cache".to_owned())?;
                }
                Err(_) => return Err("Cannot inspect lyrics cache".to_owned()),
            }
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | DIRECTORY)
            .open(directory)
            .map_err(|_| "Cannot open lyrics cache".to_owned())?;
        let metadata = directory
            .metadata()
            .map_err(|_| "Cannot inspect lyrics cache".to_owned())?;
        let owner = fs::metadata("/proc/self")
            .map_err(|_| "Cannot identify cache owner".to_owned())?
            .uid();
        if metadata.uid() != owner || metadata.mode() & 0o077 != 0 {
            return Err("Lyrics cache must be private and owned".to_owned());
        }
        Ok(Self { directory })
    }

    fn path(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd())).join(name)
    }

    fn valid_file(&self, metadata: &fs::Metadata) -> bool {
        metadata.is_file()
            && metadata.len() <= MAX_JSON_BYTES as u64
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1
            && self
                .directory
                .metadata()
                .is_ok_and(|directory| directory.uid() == metadata.uid())
    }

    fn read(&self, query: &Query) -> Option<Record> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NO_FOLLOW | NONBLOCK)
            .open(self.path(&query.filename()))
            .ok()?;
        if !self.valid_file(&file.metadata().ok()?) {
            return None;
        }
        let bytes = read_bounded(file, MAX_JSON_BYTES).ok()?;
        let entry: CacheEntry = serde_json::from_slice(&bytes).ok()?;
        if entry.version != 1
            || entry.source != SOURCE
            || &entry.query != query
            || Query::new(
                &entry.query.title,
                &entry.query.artist,
                entry.query.duration_seconds,
            )
            .as_ref()
                != Ok(query)
            || !matches!(classify(query, &entry.record), Ok(Lookup::Found(_)))
        {
            return None;
        }
        Some(entry.record)
    }

    fn write(&self, query: &Query, record: &Record) -> Result<(), String> {
        let bytes = serde_json::to_vec(&CacheEntry {
            version: 1,
            source: SOURCE.to_owned(),
            query: query.clone(),
            record: Record {
                track_name: record.track_name.clone(),
                artist_name: record.artist_name.clone(),
                duration: record.duration,
                instrumental: record.instrumental,
                plain_lyrics: record.plain_lyrics.clone(),
                synced_lyrics: record.synced_lyrics.clone(),
            },
        })
        .map_err(|_| "Cannot encode lyrics cache".to_owned())?;
        if bytes.len() > MAX_JSON_BYTES {
            return Err("Lyrics cache entry exceeds its byte budget".to_owned());
        }
        let destination = self.path(&query.filename());
        match fs::symlink_metadata(&destination) {
            Ok(metadata) if self.valid_file(&metadata) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err("Unsafe lyrics cache destination".to_owned()),
        }
        let temporary = self.path(&format!(".{}.tmp", query.filename()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(NO_FOLLOW | NONBLOCK)
            .open(&temporary)
            .map_err(|_| "Cannot create lyrics cache entry".to_owned())?;
        let _temporary = Temporary(temporary.clone());
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| "Cannot save lyrics cache entry".to_owned())?;
        drop(file);
        fs::rename(temporary, destination)
            .map_err(|_| "Cannot publish lyrics cache entry".to_owned())
    }
}

struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _removed = fs::remove_file(&self.0);
    }
}
