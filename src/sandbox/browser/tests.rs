// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::io::Write;
use wire::MAX_OUTPUT_BYTES;

#[test]
fn test_wire_operation_parse() {
    assert_eq!(
        Operation::parse(19).expect("expected test value from Operation::parse(19)"),
        Operation::PreviewAudioMetadata
    );
    assert_eq!(
        Operation::parse(1).expect("expected test value from Operation::parse(1)"),
        Operation::Image
    );
    assert_eq!(
        Operation::parse(20).expect("expected test value from Operation::parse(20)"),
        Operation::Heif
    );
    assert_eq!(
        Operation::parse(21).expect("expected test value from Operation::parse(21)"),
        Operation::PreviewHeif
    );
    assert_eq!(
        Operation::parse(2).expect("expected test value from Operation::parse(2)"),
        Operation::Raw
    );
    assert_eq!(
        Operation::parse(3).expect("expected test value from Operation::parse(3)"),
        Operation::Pdf
    );
    assert_eq!(
        Operation::parse(4).expect("expected test value from Operation::parse(4)"),
        Operation::Video
    );
    assert_eq!(
        Operation::parse(5).expect("expected test value from Operation::parse(5)"),
        Operation::ImageMetadata
    );
    assert_eq!(
        Operation::parse(6).expect("expected test value from Operation::parse(6)"),
        Operation::MediaMetadata
    );
    assert_eq!(
        Operation::parse(7).expect("expected test value from Operation::parse(7)"),
        Operation::PreviewImage
    );
    assert_eq!(
        Operation::parse(8).expect("expected test value from Operation::parse(8)"),
        Operation::DocumentMermaid
    );
    assert_eq!(
        Operation::parse(9).expect("expected test value from Operation::parse(9)"),
        Operation::DocumentMath
    );
    assert_eq!(
        Operation::parse(10).expect("expected test value from Operation::parse(10)"),
        Operation::DocumentMathInline
    );
    assert_eq!(
        Operation::parse(11).expect("expected test value from Operation::parse(11)"),
        Operation::ThreeMfThumbnail
    );
    assert_eq!(
        Operation::parse(12).expect("expected test value from Operation::parse(12)"),
        Operation::FreeCadThumbnail
    );
    assert!(Operation::parse(0).is_err());
    assert!(Operation::parse(99).is_err());
}

#[test]
fn selected_audio_artwork_has_its_own_high_resolution_operation() {
    assert_eq!(
        map_preview_op(&ParseOperation::PreviewAudioArtwork),
        Some(Operation::PreviewAudioArtwork)
    );
    assert_eq!(
        Operation::parse(26).expect("expected test value from Operation::parse(26)"),
        Operation::PreviewAudioArtwork
    );
    assert!(map_thumbnail_op(&ParseOperation::PreviewAudioArtwork).is_none());
    assert_ne!(
        map_preview_op(&ParseOperation::PreviewAudioArtwork),
        map_thumbnail_op(&ParseOperation::ThumbnailVideo)
    );
}

#[test]
#[ignore = "Requires built sandbox helper and FFmpeg; generated cover only, no playback"]
fn generated_embedded_artwork_preview_preserves_more_detail_than_thumbnail() {
    let root =
        std::env::temp_dir().join(format!("hermes-artwork-resolution-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create generated fixture directory");
    let cover = root.join("cover.png");
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 1400, 1400).expect("expected test value from gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 1400, 1400)");
    pixels.fill(0xee8020ff);
    pixels
        .savev(&cover, "png", &[])
        .expect("expected test value from pixels.savev(&cover, \"png\", &[])");
    let track = root.join("generated.mp3");
    assert!(std::process::Command::new("ffmpeg").args([
        "-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo",
        "-i",
    ]).arg(&cover).args([
        "-t", "0.2", "-map", "0:a", "-map", "1:v", "-c:a", "libmp3lame", "-c:v", "png",
        "-threads", "1", "-id3v2_version", "3", "-metadata:s:v", "title=Album cover",
        "-metadata:s:v", "comment=Cover (front)",
    ]).arg(&track).status().expect("expected test value from std::process::Command::new(\"ffmpeg\").args([ \"-v\", \"error\", \"-nostdin\", \"-y\", \"-f\", \"lavfi\", \"-i\", \"anullsrc=r=48000:cl=stereo\",...").success());
    let cancellation = Cancellation::default();
    let small = crate::sandbox::parse(&track, ParseOperation::ThumbnailVideo, 1024, &cancellation).expect("expected test value from crate::sandbox::parse(&track, ParseOperation::ThumbnailVideo, 1024, &cancellation)");
    let large = crate::sandbox::parse(&track, ParseOperation::PreviewAudioArtwork, 1024, &cancellation).expect("expected test value from crate::sandbox::parse(&track, ParseOperation::PreviewAudioArtwork, 1024, &cancellation)");
    let width = |bytes: &[u8]| {
        u32::from_be_bytes(
            bytes[16..20]
                .try_into()
                .expect("expected test value from bytes[16..20].try_into()"),
        )
    };
    assert_eq!(width(&small.data), 256);
    assert_eq!(width(&large.data), 1024);
    std::fs::remove_dir_all(root).expect("remove generated fixture");
}

#[test]
fn heif_thumbnails_and_full_previews_use_distinct_cached_operations() {
    assert_eq!(
        map_thumbnail_op(&ParseOperation::ThumbnailHeif),
        Some(Operation::Heif)
    );
    assert_eq!(
        map_preview_op(&ParseOperation::PreviewHeif),
        Some(Operation::PreviewHeif)
    );
    assert_ne!(
        map_thumbnail_op(&ParseOperation::ThumbnailHeif),
        map_preview_op(&ParseOperation::PreviewHeif)
    );
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
    assert_eq!(
        map_thumbnail_op(&ParseOperation::ThumbnailVideo),
        Some(Operation::Video)
    );
    assert!(map_thumbnail_op(&ParseOperation::PreviewImage).is_none());
    assert!(map_thumbnail_op(&ParseOperation::PreviewAudioMetadata).is_none());
    assert!(map_preview_op(&ParseOperation::PreviewImage).is_some());
}

#[test]
#[ignore = "Requires FFmpeg, bubblewrap and a built worker; generated media only"]
fn generated_media_thumbnails_reuse_workers_cache_and_leave_photo_previews_unblocked() {
    use gtk::prelude::*;
    let directory = std::env::temp_dir().join(format!("hermes-media-pool-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create generated fixture directory");
    let path = directory.join("clip.mov");
    let replacement = directory.join("replacement.mov");
    for (output, input) in [
        (&path, "testsrc2=size=1920x1080:rate=24"),
        (&replacement, "color=c=blue:size=1920x1080:rate=24"),
    ] {
        assert!(std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i", input,
                "-t", "2", "-an", "-c:v", "libx264", "-threads", "2", "-preset", "veryfast"])
            .arg(output).status().expect("expected test value from std::process::Command::new(\"ffmpeg\") .args([\"-v\", \"error\", \"-nostdin\", \"-y\", \"-f\", \"lavfi\", \"-i\", input, \"-t\", \"2\", \"-an\", \"-c:...").success());
    }
    let executable = crate::sandbox::helper_executable()
        .expect("expected test value from crate::sandbox::helper_executable()");
    let mut one_shot = Vec::new();
    let mut baseline = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        baseline = crate::sandbox::parse_one_shot_with_executable(
            &path, ParseOperation::ThumbnailVideo, 256, &Cancellation::default(), &executable,
        ).expect("expected test value from crate::sandbox::parse_one_shot_with_executable( &path, ParseOperation::ThumbnailVideo, 256, &Cancellation::default(), &executab...").data;
        one_shot.push(start.elapsed());
    }
    let start = Instant::now();
    let first = thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())
        .expect("Pooled video operation").expect("expected test value from thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default()) ");
    let cold = start.elapsed();
    let mut warm = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        let result = thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())
            .expect("expected test value from thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())").expect("expected test value from thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default()) .unwrap()");
        warm.push(start.elapsed());
        assert_eq!(first, result);
    }
    let decode = |bytes: &[u8]| {
        let loader = gtk::gdk_pixbuf::PixbufLoader::new();
        loader
            .write(bytes)
            .expect("expected test value from loader.write(bytes)");
        loader
            .close()
            .expect("expected test value from loader.close()");
        loader
            .pixbuf()
            .expect("expected test value from loader.pixbuf()")
    };
    let expected = decode(&baseline);
    let actual = decode(&first);
    assert_eq!((actual.width(), actual.height()), (256, 144));
    assert_eq!(actual.read_pixel_bytes(), expected.read_pixel_bytes());
    std::fs::rename(&replacement, &path)
        .expect("expected test value from std::fs::rename(&replacement, &path)");
    let changed = thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())
        .expect("expected test value from thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())").expect("expected test value from thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default()) .unwrap()");
    assert_ne!(changed, first);
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &cancelled).expect("expected test value from thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &cancelled)").is_err());
    std::fs::write(&path, b"not valid media").expect("write generated fixture");
    assert!(thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())
        .expect("expected test value from thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &Cancellation::default())").is_err());

    let photo = directory.join("photo.jpg");
    let pixels = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 1600, 1200).expect("expected test value from gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 1600, 1200)");
    pixels.fill(0x2040d0ff);
    pixels
        .savev(&photo, "jpeg", &[])
        .expect("expected test value from pixels.savev(&photo, \"jpeg\", &[])");
    let occupied: Vec<_> = (0..pool().limit.load(Ordering::Relaxed))
        .map(|_| {
            pool()
                .acquire(&Cancellation::default())
                .expect("expected test value from pool().acquire(&Cancellation::default())")
        })
        .collect();
    let queued = Cancellation::default();
    let cancel = queued.clone();
    let cancellation_thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        cancel.cancel();
    });
    assert!(
        thumbnail_parse(&path, ParseOperation::ThumbnailVideo, &queued)
            .expect("Cancelled queued thumbnails must not fall back to a new one-shot process")
            .is_err()
    );
    cancellation_thread
        .join()
        .expect("expected test value from cancellation_thread.join()");
    let cancellation = Cancellation::default();
    let timeout = cancellation.clone();
    let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let done = completed.clone();
    let guard = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !done.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(10));
        }
        if !done.load(Ordering::Acquire) {
            timeout.cancel();
        }
    });
    let start = Instant::now();
    let selected = preview(&photo, &ParseOperation::PreviewImage, &cancellation).expect(
        "expected test value from preview(&photo, &ParseOperation::PreviewImage, &cancellation)",
    );
    completed.store(true, Ordering::Release);
    guard.join().expect("expected test value from guard.join()");
    drop(occupied);
    let selected = selected.expect("Photo preview must not wait for thumbnail workers");
    assert_eq!(decode(&selected.0).width(), 1400);
    eprintln!(
        "MOV thumbnails: one-shot={one_shot:?}, pooled cold={cold:?}, cached={warm:?}; photo with full thumbnail queue={:?}",
        start.elapsed()
    );
    std::fs::remove_dir_all(directory).expect("remove generated fixture");
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
    response
        .write(&mut buffer)
        .expect("expected test value from response.write(&mut buffer)");
    let decoded = Response::read(&mut &buffer[..])
        .expect("expected test value from Response::read(&mut &buffer[..])");
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
    assert!((1..=MAX_WORKERS).contains(&initial));

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
    let identity = Identity::from_metadata(
        &std::fs::metadata(
            std::env::current_exe().expect("expected test value from std::env::current_exe()"),
        )
        .expect("expected test value from std::fs::metadata(std::env::current_exe().unwrap())"),
    );
    let op = Operation::PreviewImage;
    let data = vec![1, 2, 3, 4];

    assert!(pool.check_cache(path, identity, op).is_none());

    pool.put_cache(path, identity, op, data.clone());
    let cached = pool.check_cache(path, identity, op).expect("cache hit");
    assert_eq!(cached, data);

    // Miss on different op
    assert!(pool.check_cache(path, identity, Operation::Image).is_none());
}
