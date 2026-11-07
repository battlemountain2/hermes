use super::*;
use std::os::unix::net::UnixStream;

#[test]
fn test_wire_framing_round_trip() {
    let png = b"\x89PNG\r\n\x1a\nvalid_pixels";
    let meta = b"{\"key\":\"val\"}";
    let frame = encode_frame(png, meta);
    assert_eq!(frame.len(), 8 + png.len() + meta.len());

    let (p, m) = read_framed_message(&frame[..]).expect("decode frame");
    assert_eq!(p, png);
    assert_eq!(m, meta);
}

#[test]
fn test_wire_oversized_payload_rejected() {
    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&(MAX_PAYLOAD_SIZE + 1).to_le_bytes());
    let err = decode_header(&header).expect_err("must reject");
    assert!(matches!(err, WireError::PayloadTooLarge { .. }));
}

#[test]
fn test_wire_m4_operations_parsing() {
    assert_eq!(
        Operation::parse(14).expect("generated fixture must provide the expected value"),
        Operation::PreviewModel
    );
    assert_eq!(
        Operation::parse(15).expect("generated fixture must provide the expected value"),
        Operation::PreviewArchiveCover
    );
    assert_eq!(
        Operation::parse(16).expect("generated fixture must provide the expected value"),
        Operation::PreviewSpreadsheet
    );
    assert_eq!(
        Operation::parse(17).expect("generated fixture must provide the expected value"),
        Operation::PreviewAudioWaveform
    );
}

#[test]
fn test_scm_rights_descriptor_passing() {
    let (sock1, sock2) = UnixStream::pair().expect("unix stream pair");
    let (read_pipe, write_pipe) = rust_pipe().expect("pipe creation");

    send(&sock1, &read_pipe, &write_pipe, Operation::Image).expect("send descriptors");

    let (op, in_fd, out_fd) = receive(&sock2)
        .expect("recv descriptors")
        .expect("received");
    assert_eq!(op, Operation::Image);
    assert!(in_fd.as_raw_fd() >= 0);
    assert!(out_fd.as_raw_fd() >= 0);
}

#[expect(
    unsafe_code,
    reason = "Calls pipe2 and wraps returned descriptors into OwnedFd in test"
)]
fn rust_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    unsafe extern "C" {
        fn pipe2(pipefd: *mut i32, flags: i32) -> i32;
    }
    // SAFETY: pipe2 called with valid pointer and O_CLOEXEC = 0x80000
    let ret = unsafe { pipe2(fds.as_mut_ptr(), 0x80000) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Valid file descriptors returned by pipe2
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: Valid file descriptors returned by pipe2
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((read, write))
}
