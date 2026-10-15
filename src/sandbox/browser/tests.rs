// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::io::Write;
use wire::MAX_OUTPUT_BYTES;

#[test]
fn test_wire_operation_parse() {
    assert_eq!(
        Operation::parse(19).unwrap(),
        Operation::PreviewAudioMetadata
    );
    assert_eq!(Operation::parse(1).unwrap(), Operation::Image);
    assert_eq!(Operation::parse(20).unwrap(), Operation::Heif);
    assert_eq!(Operation::parse(21).unwrap(), Operation::PreviewHeif);
    assert_eq!(Operation::parse(2).unwrap(), Operation::Raw);
    assert_eq!(Operation::parse(3).unwrap(), Operation::Pdf);
    assert_eq!(Operation::parse(4).unwrap(), Operation::Video);
    assert_eq!(Operation::parse(5).unwrap(), Operation::ImageMetadata);
    assert_eq!(Operation::parse(6).unwrap(), Operation::MediaMetadata);
    assert_eq!(Operation::parse(7).unwrap(), Operation::PreviewImage);
    assert_eq!(Operation::parse(8).unwrap(), Operation::DocumentMermaid);
    assert_eq!(Operation::parse(9).unwrap(), Operation::DocumentMath);
    assert_eq!(Operation::parse(10).unwrap(), Operation::DocumentMathInline);
    assert_eq!(Operation::parse(11).unwrap(), Operation::ThreeMfThumbnail);
    assert_eq!(Operation::parse(12).unwrap(), Operation::FreeCadThumbnail);
    assert!(Operation::parse(0).is_err());
    assert!(Operation::parse(99).is_err());
}

#[test]
fn heif_thumbnails_and_full_previews_use_distinct_cached_operations() {
    assert_eq!(map_thumbnail_op(&ParseOperation::ThumbnailHeif), Some(Operation::Heif));
    assert_eq!(map_preview_op(&ParseOperation::PreviewHeif), Some(Operation::PreviewHeif));
    assert_ne!(map_thumbnail_op(&ParseOperation::ThumbnailHeif), map_preview_op(&ParseOperation::PreviewHeif));
}

#[test]
fn thumbnails_and_selected_previews_have_independent_worker_queues() {
    for operation in [
        ParseOperation::ThumbnailImage,
        ParseOperation::ThumbnailHeif,
        ParseOperation::ThumbnailRaw,
        ParseOperation::ThumbnailPdf,
        ParseOperation::ThumbnailVideo,
    ] {
        assert!(map_preview_op(&operation).is_none());
        assert!(map_thumbnail_op(&operation).is_some());
    }
    assert_eq!(map_thumbnail_op(&ParseOperation::ThumbnailVideo), Some(Operation::Video));
    assert!(map_thumbnail_op(&ParseOperation::PreviewImage).is_none());
    assert!(map_thumbnail_op(&ParseOperation::PreviewAudioMetadata).is_none());
    assert!(map_preview_op(&ParseOperation::PreviewImage).is_some());
}

#[test]
#[ignore = "Requires FFmpeg, bubblewrap and a built worker; generated media only"]
fn generated_media_thumbnails_reuse_workers_cache_and_leave_photo_previews_unblocked() {
    use gtk::prelude::*;
    let directory = std::env::temp_dir().join(format!("hermes-media-pool-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("clip.mov");
    let replacement = directory.join("replacement.mov");
    for (output, input) in [
        (&path, "testsrc2=size=1920x1080:rate=24"),
        (&replacement, "color=c=blue:size=1920x1080:rate=24"),
    ] {
        assert!(std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i", input,
                "-t", "2", "-an", "-c:v", "libx264", "-threads", "2", "-preset", "veryfast"])
            .arg(output).status().unwrap().success());
    }
    let executable = crate::sandbox::helper_executable().unwrap();
    let mut one_shot = Vec::new();
    let mut baseline = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        baseline = crate::sandbox::parse_one_shot_with_executable(
            &path, ParseOperation::ThumbnailVideo, 256, &Cancellation::default(), &executable,
        ).unwrap().data;
        one_shot.push(start.elapsed());
    }
    let start = Instant::now();
    let first = thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())
        .expect("Pooled video operation").unwrap();
    let cold = start.elapsed();
    let mut warm = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        let result = thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())
            .unwrap().unwrap();
        warm.push(start.elapsed());
        assert_eq!(first, result);
    }
    let decode = |bytes: &[u8]| {
        let loader = gtk::gdk_pixbuf::PixbufLoader::new();
        loader.write(bytes).unwrap();
        loader.close().unwrap();
        loader.pixbuf().unwrap()
    };
    let expected = decode(&baseline);
    let actual = decode(&first);
    assert_eq!((actual.width(), actual.height()), (256, 144));
    assert_eq!(actual.read_pixel_bytes(), expected.read_pixel_bytes());
    std::fs::rename(&replacement, &path).unwrap();
    let changed = thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())
        .unwrap().unwrap();
    assert_ne!(changed, first);
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &cancelled).unwrap().is_err());
    std::fs::write(&path, b"not valid media").unwrap();
    assert!(thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())
        .unwrap().is_err());

    let photo = directory.join("photo.jpg");
    let pixels = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 1600, 1200).unwrap();
    pixels.fill(0x2040d0ff);
    pixels.savev(&photo, "jpeg", &[]).unwrap();
    let occupied: Vec<_> = (0..pool().limit.load(Ordering::Relaxed))
        .map(|_| pool().acquire(&Cancellation::default()).unwrap()).collect();
    let queued = Cancellation::default();
    let cancel = queued.clone();
    let cancellation_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        cancel.cancel();
    });
    assert!(thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &queued)
        .expect("Cancelled queued thumbnails must not fall back to a new one-shot process").is_err());
    cancellation_thread.join().unwrap();
    let cancellation = Cancellation::default();
    let timeout = cancellation.clone();
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = completed.clone();
    let guard = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !done.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(10));
        }
        if !done.load(Ordering::Acquire) { timeout.cancel(); }
    });
    let start = Instant::now();
    let selected = preview(&photo, &ParseOperation::PreviewImage, &cancellation).unwrap();
    completed.store(true, Ordering::Release);
    guard.join().unwrap();
    drop(occupied);
    let selected = selected.expect("Photo preview must not wait for thumbnail workers");
    assert_eq!(decode(&selected.0).width(), 1400);
    eprintln!("MOV thumbnails: one-shot={one_shot:?}, pooled cold={cold:?}, cached={warm:?}; photo with full thumbnail queue={:?}", start.elapsed());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn metadata_only_audio_uses_distinct_wire_operation_and_json_payload() {
    assert_eq!(
        map_preview_op(&ParseOperation::PreviewAudioMetadata),
        Some(Operation::PreviewAudioMetadata)
    );
    let response = Response {
        png: br#"{"title":"Example","properties":{},"embedded_tags":{}}"#.to_vec(),
        metadata: Vec::new(),
    };
    let mut buffer = Vec::new();
    response.write(&mut buffer).unwrap();
    let decoded = Response::read(&mut &buffer[..]).unwrap();
    assert!(decoded.metadata.is_empty());
    assert!(!decoded.png.starts_with(b"\x89PNG"));
    assert!(
        serde_json::from_slice::<crate::services::preview::AudioMetadata>(&decoded.png).is_ok()
    );
}

#[test]
fn decoder_failures_have_bounded_framed_responses() {
    let response = Response::failure("Image dimensions exceed the decoded frame budget");
    let mut buffer = Vec::new();
    response.write(&mut buffer).expect("write failure frame");
    let decoded = Response::read(&mut &buffer[..]).expect("read failure frame");
    assert_eq!(
        decoded.error_message(),
        Some("Image dimensions exceed the decoded frame budget".into())
    );
    let response = Response::failure(&"🦀".repeat(10000));
    assert!(response.metadata.len() < 17000);
    assert_eq!(
        response.error_message().expect("error").chars().count(),
        4096
    );
    assert_eq!(
        Response {
            png: vec![1],
            metadata: response.metadata
        }
        .error_message(),
        None
    );
}

#[test]
fn test_response_read_write() {
    let response = Response {
        png: b"\x89PNG\r\n\x1a\nfake".to_vec(),
        metadata: b"{\"test\":\"data\"}".to_vec(),
    };
    let mut buffer = Vec::new();
    response.write(&mut buffer).expect("write response");

    let read_back = Response::read(&mut &buffer[..]).expect("read response");
    assert_eq!(read_back.png, response.png);
    assert_eq!(read_back.metadata, response.metadata);
}

#[test]
fn test_response_oversized_rejected() {
    let mut buffer = Vec::new();
    let huge_png_len = (MAX_OUTPUT_BYTES + 1) as u32;
    buffer.extend_from_slice(&huge_png_len.to_le_bytes());
    buffer.extend_from_slice(&0u32.to_le_bytes());

    let err = Response::read(&mut &buffer[..]).expect_err("must reject oversized");
    assert!(err.to_string().contains("Oversized") || err.to_string().contains("PayloadTooLarge"));
}

#[test]
fn test_deadline_reader_immediate_cancellation() {
    let (read_pipe, _write_pipe) = create_pipe().expect("pipe");
    let mut file = File::from(read_pipe);
    let cancellation = Cancellation::default();
    cancellation.cancel();

    let mut deadline_reader = DeadlineReader {
        reader: &mut file,
        deadline: Instant::now() + Duration::from_secs(12),
        cancellation: &cancellation,
    };

    let mut buf = [0u8; 10];
    let err = deadline_reader.read(&mut buf).expect_err("cancelled");
    assert!(err.to_string().contains("cancelled"));
}

#[test]
fn test_deadline_reader_quanta_responsiveness() {
    let (read_pipe, _write_pipe) = create_pipe().expect("pipe");
    let mut file = File::from(read_pipe);
    let cancellation = Cancellation::default();
    let cancellation_clone = cancellation.clone();

    // Spawn thread to cancel after 10ms
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(10));
        cancellation_clone.cancel();
    });

    let start = Instant::now();
    let mut deadline_reader = DeadlineReader {
        reader: &mut file,
        deadline: Instant::now() + Duration::from_secs(12),
        cancellation: &cancellation,
    };

    let mut buf = [0u8; 10];
    let err = deadline_reader.read(&mut buf).expect_err("cancelled");
    let elapsed = start.elapsed();

    assert!(err.to_string().contains("cancelled"));
    // Must respond rapidly within quanta (<= 100ms) instead of waiting 12s
    assert!(elapsed < Duration::from_millis(100));
}

#[test]
fn test_deadline_reader_successful_read() {
    let (read_pipe, write_pipe) = create_pipe().expect("pipe");
    let mut read_file = File::from(read_pipe);
    let mut write_file = File::from(write_pipe);

    write_file.write_all(b"hello world").expect("write");
    drop(write_file);

    let cancellation = Cancellation::default();
    let mut deadline_reader = DeadlineReader {
        reader: &mut read_file,
        deadline: Instant::now() + Duration::from_secs(12),
        cancellation: &cancellation,
    };

    let mut buf = [0u8; 5];
    let count = deadline_reader.read(&mut buf).expect("read");
    assert_eq!(count, 5);
    assert_eq!(&buf, b"hello");
}

#[test]
fn test_pool_limit_configuration() {
    let initial = worker_limit();
    assert!(initial >= 1 && initial <= MAX_WORKERS);

    set_worker_limit(8);
    assert_eq!(worker_limit(), 8);

    // Clamping to MAX_WORKERS
    set_worker_limit(100);
    assert_eq!(worker_limit(), MAX_WORKERS);

    // Restore initial
    set_worker_limit(initial);
}

#[test]
fn test_pool_cache_lifecycle() {
    let pool = Pool::new();
    let path = Path::new("/tmp/test_image.png");
    let identity =
        Identity::from_metadata(&std::fs::metadata(std::env::current_exe().unwrap()).unwrap());
    let op = Operation::PreviewImage;
    let data = vec![1, 2, 3, 4];

    assert!(pool.check_cache(path, identity, op).is_none());

    pool.put_cache(path, identity, op, data.clone());
    let cached = pool.check_cache(path, identity, op).expect("cache hit");
    assert_eq!(cached, data);

    // Miss on different op
    assert!(pool.check_cache(path, identity, Operation::Image).is_none());
}
