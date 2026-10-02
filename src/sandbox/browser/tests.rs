// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::io::Write;
use wire::MAX_OUTPUT_BYTES;

#[test]
fn test_wire_operation_parse() {
    assert_eq!(Operation::parse(1).unwrap(), Operation::Image);
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
    let mtime = 123456789;
    let size = 1024;
    let op = Operation::PreviewImage;
    let data = vec![1, 2, 3, 4];

    assert!(pool.check_cache(path, mtime, size, op).is_none());

    pool.put_cache(path, mtime, size, op, data.clone());
    let cached = pool.check_cache(path, mtime, size, op).expect("cache hit");
    assert_eq!(cached, data);

    // Miss on different mtime
    assert!(pool.check_cache(path, mtime + 1, size, op).is_none());
    // Miss on different op
    assert!(pool.check_cache(path, mtime, size, Operation::Image).is_none());
}
