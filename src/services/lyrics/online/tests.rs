use std::{
    cell::Cell,
    os::unix::fs::{PermissionsExt, symlink},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use super::*;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hermes-online-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .expect("private fixture");
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

fn query() -> Query {
    Query::new("Synthetic Song (Remix)", "Synthetic Artist", 180.25).expect("query")
}

fn record() -> Record {
    Record {
        track_name: query().title,
        artist_name: query().artist,
        duration: 180.25,
        instrumental: false,
        plain_lyrics: Some("Synthetic first line\n\nSynthetic last line".to_owned()),
        synced_lyrics: Some("[00:01]Synthetic first line\n[00:05]Synthetic last line".to_owned()),
    }
}

fn response(record: &Record) -> Response {
    Response {
        status: 200,
        body: serde_json::to_vec(record).expect("response"),
    }
}

#[test]
fn transient_provider_failures_retry_with_backoff_and_then_return_lyrics() {
    let mut attempts = 0;
    let mut delays = Vec::new();
    let response = fetch_with_retry(
        &query(),
        &Cancellation::default(),
        |_, _, remaining| {
            assert!(!remaining.is_zero() && remaining <= LOOKUP_DEADLINE);
            attempts += 1;
            Ok(if attempts < 3 {
                Response {
                    status: 503,
                    body: Vec::new(),
                }
            } else {
                response(&record())
            })
        },
        |delay, _| {
            delays.push(delay);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(attempts, 3);
    assert_eq!(delays, RETRY_DELAYS);
    assert_eq!(response.status, 200);
}

#[test]
fn provider_retries_are_bounded_and_do_not_repeat_misses_rate_limits_or_bad_responses() {
    for status in [200, 404, 401, 429, 500, 502, 503, 504] {
        let mut attempts = 0;
        let retryable = matches!(status, 500 | 502 | 503 | 504);
        let result = fetch_with_retry(
            &query(),
            &Cancellation::default(),
            |_, _, _| {
                attempts += 1;
                Ok(Response {
                    status,
                    body: Vec::new(),
                })
            },
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(attempts, if retryable { 3 } else { 1 });
        assert_eq!(result.status, status);
    }
    let mut attempts = 0;
    assert!(
        fetch_with_retry(
            &query(),
            &Cancellation::default(),
            |_, _, _| {
                attempts += 1;
                Ok(Response {
                    status: 503,
                    body: vec![0; MAX_JSON_BYTES + 1],
                })
            },
            |_, _| panic!("oversized response must not retry"),
        )
        .is_err()
    );
    assert_eq!(attempts, 1);
    assert!(
        fetch_with_retry(
            &query(),
            &Cancellation::default(),
            |_, _, _| Err("transport error".to_owned()),
            |_, _| panic!("transport error must not retry"),
        )
        .is_err()
    );
}

#[test]
fn cancellation_during_backoff_prevents_another_request() {
    let cancellation = Cancellation::default();
    let mut attempts = 0;
    assert!(
        fetch_with_retry(
            &query(),
            &cancellation,
            |_, _, _| {
                attempts += 1;
                Ok(Response {
                    status: 503,
                    body: Vec::new(),
                })
            },
            |_, cancellation| {
                cancellation.cancel();
                Ok(())
            },
        )
        .is_err()
    );
    assert_eq!(attempts, 1);
    assert!(wait_for_retry(Duration::from_secs(60), &cancellation).is_err());
    assert!(wait_for_retry(Duration::ZERO, &cancellation).is_err());
}

#[test]
fn service_unavailable_is_not_reported_or_cached_as_missing_lyrics() {
    let fixture = Fixture::new();
    let error = lookup_with(&query(), &Cancellation::default(), &fixture.0, |_, _| {
        Ok(Response {
            status: 503,
            body: Vec::new(),
        })
    })
    .unwrap_err();
    assert!(error.contains("temporarily unavailable"));
    assert!(!error.contains("No matching"));
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
}

#[test]
fn query_rejects_empty_unknown_overlong_controls_and_nonfinite_duration() {
    for artist in [
        "",
        " ",
        "unknown",
        " Unknown artist ",
        "<unknown>",
        "[Unknown]",
        "(unknown)",
        "n/a",
    ] {
        assert!(Query::new("Song", artist, 180.0).is_err(), "{artist}");
    }
    for metadata in [
        "",
        " ",
        "bad\nvalue",
        "bad\0value",
        &"x".repeat(MAX_METADATA_BYTES + 1),
    ] {
        assert!(Query::new(metadata, "Artist", 180.0).is_err());
        assert!(Query::new("Song", metadata, 180.0).is_err());
    }
    for duration in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.0,
        0.0,
        0.99,
        3600.01,
    ] {
        assert!(Query::new("Song", "Artist", duration).is_err());
    }
    assert!(Query::new("Song", "Artist", 1.0).is_ok());
    assert!(Query::new("Song", "Artist", 3600.0).is_ok());
    assert_eq!(
        Query::new(" Song ", " Artist ", 1.0)
            .expect("trimmed")
            .title,
        "Song"
    );
}

#[test]
fn command_is_fixed_https_encoded_three_field_request_without_configs_or_credentials() {
    let query = Query::new("A & B?#{}é", "Artist / +", 180.25).expect("query");
    assert_eq!(
        query.url(),
        "https://lrclib.net/api/get?track_name=A%20%26%20B%3F%23%7B%7D%C3%A9&artist_name=Artist%20%2F%20%2B&duration=180.25"
    );
    let command = curl_command(&query);
    assert_eq!(command.get_program(), "/usr/bin/curl");
    let arguments: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_str().expect("UTF-8"))
        .collect();
    assert_eq!(arguments[0], "--disable");
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--proto", "=https"])
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--max-redirs", "0"])
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--connect-timeout", "4"])
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--max-time", "12"])
    );
    assert!(arguments.windows(2).any(|pair| pair == ["--proxy", ""]));
    for forbidden in [
        "--insecure",
        "-k",
        "--location",
        "-L",
        "--netrc",
        "--user",
        "--cookie",
        "--config",
    ] {
        assert!(!arguments.contains(&forbidden));
    }
    assert!(command.get_envs().next().is_none());
    assert!(!query.url().contains("album"));
    assert_eq!(query.url().matches('&').count(), 2);
}

#[test]
fn response_prefers_synced_falls_back_plain_and_requires_explicit_instrumental() {
    let mut record = record();
    let Lookup::Found(lyrics) = classify(&query(), &record).expect("synced") else {
        panic!("lyrics")
    };
    assert!(lyrics.synced);
    for synced in [
        None,
        Some("malformed".to_owned()),
        Some("[00:01]".to_owned()),
        Some("x".repeat(super::super::MAX_LRC_BYTES + 1)),
    ] {
        record.synced_lyrics = synced;
        let Lookup::Found(lyrics) = classify(&query(), &record).expect("plain") else {
            panic!("plain lyrics")
        };
        assert!(!lyrics.synced);
        assert_eq!(lyrics.active_line_at(Duration::from_secs(5)), None);
    }
    record.plain_lyrics = None;
    assert_eq!(
        classify(&query(), &record).expect("no lyrics"),
        Lookup::NotFound
    );
    record.instrumental = true;
    assert_eq!(
        classify(&query(), &record).expect("instrumental"),
        Lookup::Instrumental
    );
    let mut invalid = serde_json::to_value(record).expect("record");
    invalid
        .as_object_mut()
        .expect("object")
        .remove("instrumental");
    assert!(serde_json::from_value::<Record>(invalid).is_err());
}

#[test]
fn metadata_validation_keeps_remix_punctuation_and_duration_identity() {
    let mut record = record();
    record.track_name = "  SYNTHETIC SONG (REMIX) ".to_owned();
    record.artist_name = "synthetic artist".to_owned();
    record.duration += 2.0;
    assert!(matches!(classify(&query(), &record), Ok(Lookup::Found(_))));
    record.duration += 0.001;
    assert_eq!(
        classify(&query(), &record).expect("wrong duration"),
        Lookup::NotFound
    );
    record.duration = query().duration_seconds - 2.0;
    assert!(matches!(classify(&query(), &record), Ok(Lookup::Found(_))));
    for title in [
        "Synthetic Song",
        "Synthetic Song [Remix]",
        "Synthetic Song (Live)",
        "Synthetic Song (Remix) Extended",
    ] {
        record.track_name = title.to_owned();
        assert_eq!(
            classify(&query(), &record).expect("wrong version"),
            Lookup::NotFound
        );
    }
    record.track_name = query().title;
    record.artist_name = "Another Artist".to_owned();
    assert_eq!(
        classify(&query(), &record).expect("wrong artist"),
        Lookup::NotFound
    );
    record.artist_name = query().artist;
    for duration in [f64::NAN, f64::INFINITY, 0.0, 3601.0] {
        record.duration = duration;
        assert!(classify(&query(), &record).is_err());
    }
}

#[test]
fn statuses_malformed_json_and_oversized_responses_are_not_network_misses() {
    let fixture = Fixture::new();
    let run = |response| {
        lookup_with(&query(), &Cancellation::default(), &fixture.0, |_, _| {
            Ok(response)
        })
    };
    assert_eq!(
        run(Response {
            status: 404,
            body: Vec::new()
        })
        .expect("404"),
        Lookup::NotFound
    );
    for status in [0, 301, 401, 429, 500, 503] {
        assert!(
            run(Response {
                status,
                body: Vec::new()
            })
            .is_err()
        );
    }
    for body in [
        b"not json".to_vec(),
        b"[]".to_vec(),
        b"{}".to_vec(),
        vec![0xff],
        vec![b'x'; MAX_JSON_BYTES + 1],
    ] {
        assert!(run(Response { status: 200, body }).is_err());
    }
    let mut value = serde_json::to_value(record()).expect("record");
    value["duration"] = serde_json::Value::Null;
    assert!(
        run(Response {
            status: 200,
            body: serde_json::to_vec(&value).expect("null duration")
        })
        .is_err()
    );
    assert!(
        lookup_with(&query(), &Cancellation::default(), &fixture.0, |_, _| Err(
            "synthetic network failure".to_owned()
        ))
        .is_err()
    );
    assert_eq!(fs::read_dir(&fixture.0).expect("cache").count(), 0);
}

#[test]
fn cancellation_prevents_transport_cache_hits_and_cache_writes() {
    let fixture = Fixture::new();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        lookup_with(&query(), &cancellation, &fixture.0, |_, _| panic!(
            "cancelled transport"
        ))
        .is_err()
    );
    let cancellation = Cancellation::default();
    assert!(
        lookup_with(&query(), &cancellation, &fixture.0, |_, cancellation| {
            cancellation.cancel();
            Ok(response(&record()))
        })
        .is_err()
    );
    assert_eq!(fs::read_dir(&fixture.0).expect("cache").count(), 0);
    Cache::open(&fixture.0)
        .expect("cache")
        .write(&query(), &record())
        .expect("write");
    assert!(
        lookup_with(&query(), &cancellation, &fixture.0, |_, _| panic!(
            "cache should not invoke transport"
        ))
        .is_err()
    );
}

#[test]
fn cache_precedes_transport_and_has_private_positive_lrclib_identity_only() {
    let fixture = Fixture::new();
    let called = Cell::new(0);
    let run = || {
        lookup_with(&query(), &Cancellation::default(), &fixture.0, |_, _| {
            called.set(called.get() + 1);
            let mut value = serde_json::to_value(record()).expect("record");
            value["albumName"] = "Never retained album".into();
            value["source"] = "Untrusted provider label".into();
            Ok(Response {
                status: 200,
                body: serde_json::to_vec(&value).expect("synthetic response"),
            })
        })
        .expect("lookup")
    };
    assert!(matches!(run(), Lookup::Found(_)));
    assert_eq!(run(), run());
    assert_eq!(called.get(), 1);
    let path = fixture.0.join(query().filename());
    let metadata = fs::metadata(&path).expect("entry");
    assert_eq!(metadata.mode() & 0o777, 0o600);
    let bytes = fs::read(path).expect("bytes");
    let entry: CacheEntry = serde_json::from_slice(&bytes).expect("cache");
    assert_eq!(entry.source, "LRCLIB");
    assert_eq!(entry.query, query());
    assert!(!String::from_utf8(bytes).expect("UTF-8").contains("album"));
    assert_eq!(fs::read_dir(&fixture.0).expect("entries").count(), 1);
}

#[test]
fn cache_revalidates_request_response_source_and_version_before_accepting() {
    let fixture = Fixture::new();
    let cache = Cache::open(&fixture.0).expect("cache");
    let path = fixture.0.join(query().filename());
    for field in [
        "title",
        "artist",
        "duration_seconds",
        "source",
        "version",
        "response_title",
        "response_duration",
    ] {
        cache.write(&query(), &record()).expect("write");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("entry")).expect("JSON");
        match field {
            "title" => value["query"]["title"] = "Synthetic Song".into(),
            "artist" => value["query"]["artist"] = "Another Artist".into(),
            "duration_seconds" => value["query"]["duration_seconds"] = 181.0.into(),
            "source" => value["source"] = "Other provider".into(),
            "version" => value["version"] = 2.into(),
            "response_title" => value["record"]["trackName"] = "Synthetic Song".into(),
            "response_duration" => value["record"]["duration"] = 185.0.into(),
            _ => unreachable!(),
        }
        fs::write(&path, serde_json::to_vec(&value).expect("bytes")).expect("overwrite");
        assert!(cache.read(&query()).is_none(), "{field}");
    }
}

#[test]
fn cache_rejects_symlinks_public_permissions_invalid_and_oversized_files() {
    let fixture = Fixture::new();
    let cache = Cache::open(&fixture.0).expect("cache");
    let path = fixture.0.join(query().filename());
    fs::write(&path, "{}").expect("malformed entry");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("private");
    assert!(cache.read(&query()).is_none());
    File::create(&path)
        .expect("oversized")
        .set_len(MAX_JSON_BYTES as u64 + 1)
        .expect("size");
    assert!(cache.read(&query()).is_none());
    fs::remove_file(&path).expect("remove oversized");
    let target = fixture.0.join("unrelated");
    fs::write(&target, "untouched").expect("target");
    symlink(&target, &path).expect("symlink");
    assert!(cache.read(&query()).is_none());
    assert!(cache.write(&query(), &record()).is_err());
    assert_eq!(fs::read_to_string(&target).expect("target"), "untouched");
    fs::remove_file(&path).expect("remove symlink");
    cache.write(&query(), &record()).expect("write");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("public file");
    assert!(cache.read(&query()).is_none());
    let link = fixture.0.join("linked");
    symlink(&fixture.0, &link).expect("directory symlink");
    assert!(Cache::open(&link).is_err());
    assert!(Cache::open(&link.join("child")).is_err());
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o755)).expect("public directory");
    assert!(Cache::open(&fixture.0).is_err());
}

#[test]
fn cache_fixed_slots_bound_growth_and_collisions_never_return_other_tracks() {
    let fixture = Fixture::new();
    let cache = Cache::open(&fixture.0).expect("cache");
    let mut first = None;
    let mut collision = None;
    for index in 0..CACHE_SLOTS + 1 {
        let query =
            Query::new(&format!("Fixture {index}"), "Synthetic Artist", 180.25).expect("query");
        let mut record = record();
        record.track_name = query.title.clone();
        if let Some(previous) = &first {
            let previous: &Query = previous;
            if previous.filename() == query.filename() {
                collision = Some(previous.clone());
            }
        } else {
            first = Some(query.clone());
        }
        cache.write(&query, &record).expect("cache write");
    }
    assert!(fs::read_dir(&fixture.0).expect("entries").count() <= CACHE_SLOTS as usize);
    if let Some(collided) = collision {
        assert!(cache.read(&collided).is_none());
    }
    let query = first.expect("first query");
    let mut other = record();
    other.track_name = "Wrong version".to_owned();
    cache
        .write(&query, &other)
        .expect("synthetic invalid cache");
    assert!(cache.read(&query).is_none());
}

#[test]
fn cache_temporary_slot_is_exclusive_bounded_and_does_not_clobber_existing_files() {
    let fixture = Fixture::new();
    let cache = Cache::open(&fixture.0).expect("cache");
    let temporary = fixture.0.join(format!(".{}.tmp", query().filename()));
    fs::write(&temporary, "pre-existing temporary").expect("existing temporary");
    assert!(cache.write(&query(), &record()).is_err());
    assert_eq!(
        fs::read_to_string(&temporary).expect("temporary"),
        "pre-existing temporary"
    );
    assert_eq!(fs::read_dir(&fixture.0).expect("entries").count(), 1);
    fs::remove_file(&temporary).expect("remove temporary");
    cache.write(&query(), &record()).expect("atomic write");
    assert!(!temporary.exists());
    assert!(cache.read(&query()).is_some());
}

#[test]
fn bounded_reader_caps_even_unknown_length_streams() {
    struct Endless(usize);
    impl Read for Endless {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            buffer.fill(b'x');
            self.0 += buffer.len();
            Ok(buffer.len())
        }
    }
    for limit in [MAX_JSON_BYTES + 4, MAX_STDERR_BYTES] {
        let mut reader = Endless(0);
        assert!(read_bounded(&mut reader, limit).is_err());
        assert_eq!(reader.0, limit + 1);
        assert_eq!(read_bounded(&b"abcd"[..], 4).expect("at limit"), b"abcd");
    }
}

fn synthetic_child(program: &str, arguments: &[&str]) -> Child {
    Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("synthetic child, never curl")
}

#[test]
fn subprocess_collects_http_status_and_reaps_without_any_network() {
    let child = synthetic_child("/usr/bin/printf", &["%s", "{}\n404"]);
    let pid = child.id();
    let response = collect(child, &Cancellation::default(), Duration::from_secs(2))
        .expect("synthetic response");
    assert_eq!(response.status, 404);
    assert_eq!(response.body, b"{}");
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    for output in ["bad", "{}\nABC"] {
        assert!(
            collect(
                synthetic_child("/usr/bin/printf", &["%s", output]),
                &Cancellation::default(),
                Duration::from_secs(2)
            )
            .is_err()
        );
    }
}

#[test]
fn subprocess_cancel_timeout_failure_and_drop_kill_and_reap() {
    let cancellation = Cancellation::default();
    let shared = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let token = Cancellation::from_shared(shared.clone());
    let cancel = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        shared.store(true, Ordering::Release);
    });
    let child = synthetic_child("/usr/bin/sleep", &["30"]);
    let pid = child.id();
    assert!(
        collect(child, &token, Duration::from_secs(2))
            .expect_err("cancelled")
            .contains("cancelled")
    );
    cancel.join().expect("cancel thread");
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    let child = synthetic_child("/usr/bin/sleep", &["30"]);
    let pid = child.id();
    assert!(
        collect(child, &cancellation, Duration::from_millis(20))
            .expect_err("timeout")
            .contains("timed out")
    );
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    assert!(
        collect(
            synthetic_child("/usr/bin/false", &[]),
            &cancellation,
            Duration::from_secs(2)
        )
        .is_err()
    );
    let child = synthetic_child("/usr/bin/sleep", &["30"]);
    let pid = child.id();
    drop(Process {
        child,
        readers: Vec::new(),
    });
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
fn subprocess_overflow_kills_and_reaps_while_pipe_is_still_streaming() {
    let child = synthetic_child("/usr/bin/head", &["-c", "2097152", "/dev/zero"]);
    let pid = child.id();
    assert!(
        collect(child, &Cancellation::default(), Duration::from_secs(2))
            .expect_err("overflow")
            .contains("budget")
    );
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    let fixture = Fixture::new();
    let path = fixture.0.join("synthetic-stderr");
    fs::write(&path, vec![b'x'; MAX_STDERR_BYTES + 1]).expect("synthetic stderr");
    let child = Command::new("/usr/bin/tee")
        .arg("/dev/stderr")
        .stdin(File::open(path).expect("fixture input"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("synthetic tee");
    let pid = child.id();
    assert!(
        collect(child, &Cancellation::default(), Duration::from_secs(2))
            .expect_err("stderr overflow")
            .contains("budget")
    );
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
}
