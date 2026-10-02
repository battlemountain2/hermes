// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    io::{self, Read, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
};

pub const HEADER_SIZE: usize = 8;
pub const MAX_PAYLOAD_SIZE: u32 = 32 * 1024 * 1024; // 32MB limit
pub const MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_METADATA_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Operation {
    Image = 1,
    Raw = 2,
    Pdf = 3,
    Video = 4,
    ImageMetadata = 5,
    MediaMetadata = 6,
    PreviewImage = 7,
    DocumentMermaid = 8,
    DocumentMath = 9,
    DocumentMathInline = 10,
    ThreeMfThumbnail = 11,
    FreeCadThumbnail = 12,
    PreviewGeoTiff = 13,
    PreviewModel = 14,
    PreviewArchiveCover = 15,
    PreviewSpreadsheet = 16,
    PreviewAudioWaveform = 17,
}

impl Operation {
    pub(crate) fn parse(value: u8) -> io::Result<Self> {
        match value {
            1 => Ok(Self::Image),
            2 => Ok(Self::Raw),
            3 => Ok(Self::Pdf),
            4 => Ok(Self::Video),
            5 => Ok(Self::ImageMetadata),
            6 => Ok(Self::MediaMetadata),
            7 => Ok(Self::PreviewImage),
            8 => Ok(Self::DocumentMermaid),
            9 => Ok(Self::DocumentMath),
            10 => Ok(Self::DocumentMathInline),
            11 => Ok(Self::ThreeMfThumbnail),
            12 => Ok(Self::FreeCadThumbnail),
            13 => Ok(Self::PreviewGeoTiff),
            14 => Ok(Self::PreviewModel),
            15 => Ok(Self::PreviewArchiveCover),
            16 => Ok(Self::PreviewSpreadsheet),
            17 => Ok(Self::PreviewAudioWaveform),
            _ => Err(io::Error::other("Unknown browser operation")),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum WireError {
    IncompleteHeader,
    PayloadTooLarge { length: u32, max: u32 },
    UnexpectedEof,
    IoError(String),
}

/// Encodes an 8-byte framed wire protocol response:
/// [png_len: u32 little endian, metadata_len: u32 little endian] + png + metadata
pub fn encode_frame(png: &[u8], metadata: &[u8]) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(HEADER_SIZE + png.len() + metadata.len());
    let png_len = png.len() as u32;
    let meta_len = metadata.len() as u32;

    buffer.extend_from_slice(&png_len.to_le_bytes());
    buffer.extend_from_slice(&meta_len.to_le_bytes());
    buffer.extend_from_slice(png);
    buffer.extend_from_slice(metadata);
    buffer
}

/// Decodes the 8-byte framing header from a byte slice.
pub fn decode_header(header: &[u8]) -> Result<(u32, u32), WireError> {
    if header.len() < HEADER_SIZE {
        return Err(WireError::IncompleteHeader);
    }
    let png_len = u32::from_le_bytes(
        header[0..4]
            .try_into()
            .map_err(|_| WireError::IncompleteHeader)?,
    );
    let meta_len = u32::from_le_bytes(
        header[4..8]
            .try_into()
            .map_err(|_| WireError::IncompleteHeader)?,
    );

    if png_len > MAX_PAYLOAD_SIZE {
        return Err(WireError::PayloadTooLarge {
            length: png_len,
            max: MAX_PAYLOAD_SIZE,
        });
    }
    if meta_len > MAX_PAYLOAD_SIZE {
        return Err(WireError::PayloadTooLarge {
            length: meta_len,
            max: MAX_PAYLOAD_SIZE,
        });
    }

    Ok((png_len, meta_len))
}

/// Reads a framed message from any reader implementing `Read`.
pub fn read_framed_message<R: Read>(mut reader: R) -> Result<(Vec<u8>, Vec<u8>), WireError> {
    let mut header_buf = [0u8; HEADER_SIZE];
    reader
        .read_exact(&mut header_buf)
        .map_err(|e| match e.kind() {
            io::ErrorKind::UnexpectedEof => WireError::IncompleteHeader,
            _ => WireError::IoError(e.to_string()),
        })?;

    let (png_len, meta_len) = decode_header(&header_buf)?;

    let mut png = vec![0u8; png_len as usize];
    reader
        .read_exact(&mut png)
        .map_err(|_| WireError::UnexpectedEof)?;

    let mut metadata = vec![0u8; meta_len as usize];
    reader
        .read_exact(&mut metadata)
        .map_err(|_| WireError::UnexpectedEof)?;

    Ok((png, metadata))
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub(crate) struct Response {
    pub(crate) png: Vec<u8>,
    pub(crate) metadata: Vec<u8>,
}

impl Response {
    pub(crate) fn read(reader: &mut impl Read) -> io::Result<Self> {
        let (png, metadata) = read_framed_message(reader).map_err(|err| match err {
            WireError::IncompleteHeader => {
                io::Error::new(io::ErrorKind::UnexpectedEof, "Incomplete browser header")
            }
            WireError::PayloadTooLarge { .. } => io::Error::other("Oversized browser response"),
            WireError::UnexpectedEof => {
                io::Error::new(io::ErrorKind::UnexpectedEof, "Unexpected EOF in payload")
            }
            WireError::IoError(msg) => io::Error::other(msg),
        })?;
        Ok(Self { png, metadata })
    }

    pub(crate) fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        if self.png.len() as u64 > MAX_OUTPUT_BYTES
            || self.metadata.len() as u64 > MAX_METADATA_BYTES
        {
            return Err(io::Error::other("Oversized browser response"));
        }
        let frame = encode_frame(&self.png, &self.metadata);
        writer.write_all(&frame)?;
        writer.flush()
    }
}

// POSIX socket ancillary data structures for SCM_RIGHTS descriptor passing

#[repr(C)]
struct Msghdr {
    msg_name: *mut std::ffi::c_void,
    msg_namelen: u32,
    msg_iov: *mut Iovec,
    msg_iovlen: usize,
    msg_control: *mut std::ffi::c_void,
    msg_controllen: usize,
    msg_flags: i32,
}

#[repr(C)]
struct Iovec {
    iov_base: *mut std::ffi::c_void,
    iov_len: usize,
}

#[repr(C)]
struct Cmsghdr {
    cmsg_len: usize,
    cmsg_level: i32,
    cmsg_type: i32,
}

#[repr(C)]
struct ScmRights2Buffer {
    hdr: Cmsghdr,
    fds: [RawFd; 2],
}

const SOL_SOCKET: i32 = 1;
const SCM_RIGHTS: i32 = 1;
const MSG_NOSIGNAL: i32 = 0x4000;
const MSG_CMSG_CLOEXEC: i32 = 0x40000000;

#[expect(
    unsafe_code,
    reason = "POSIX socket API bindings for SCM_RIGHTS descriptor passing over Unix domain sockets"
)]
unsafe extern "C" {
    fn sendmsg(sockfd: i32, msg: *const Msghdr, flags: i32) -> isize;
    fn recvmsg(sockfd: i32, msg: *mut Msghdr, flags: i32) -> isize;
}

/// Passes two open file descriptors [input_fd, output_write_pipe_fd] and 1 operation byte over a Unix socket.
#[expect(
    unsafe_code,
    reason = "Constructs msghdr struct and invokes sendmsg for SCM_RIGHTS descriptor transfer"
)]
pub(crate) fn send(
    socket: &impl AsRawFd,
    input: &impl AsRawFd,
    output: &impl AsRawFd,
    operation: Operation,
) -> io::Result<()> {
    let mut cmsg = ScmRights2Buffer {
        hdr: Cmsghdr {
            cmsg_len: std::mem::size_of::<ScmRights2Buffer>(),
            cmsg_level: SOL_SOCKET,
            cmsg_type: SCM_RIGHTS,
        },
        fds: [input.as_raw_fd(), output.as_raw_fd()],
    };
    let mut op_byte = [operation as u8];
    let mut iov = Iovec {
        iov_base: op_byte.as_mut_ptr() as *mut std::ffi::c_void,
        iov_len: 1,
    };
    let msg = Msghdr {
        msg_name: std::ptr::null_mut(),
        msg_namelen: 0,
        msg_iov: &mut iov,
        msg_iovlen: 1,
        msg_control: &mut cmsg as *mut _ as *mut std::ffi::c_void,
        msg_controllen: std::mem::size_of::<ScmRights2Buffer>(),
        msg_flags: 0,
    };

    // SAFETY: sendmsg called with valid socket descriptor and properly aligned msghdr
    let count = unsafe { sendmsg(socket.as_raw_fd(), &msg, MSG_NOSIGNAL) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    if count != 1 {
        return Err(io::Error::other("Incomplete browser request"));
    }
    Ok(())
}

/// Receives 1 operation byte and two open file descriptors [input_fd, output_write_pipe_fd] from a Unix socket.
#[expect(
    unsafe_code,
    reason = "Constructs msghdr buffer, invokes recvmsg, and takes ownership of received file descriptors"
)]
pub(crate) fn receive(socket: &impl AsRawFd) -> io::Result<Option<(Operation, OwnedFd, OwnedFd)>> {
    let mut cmsg = ScmRights2Buffer {
        hdr: Cmsghdr {
            cmsg_len: 0,
            cmsg_level: 0,
            cmsg_type: 0,
        },
        fds: [-1, -1],
    };
    let mut op_byte = [0u8; 1];
    let mut iov = Iovec {
        iov_base: op_byte.as_mut_ptr() as *mut std::ffi::c_void,
        iov_len: 1,
    };
    let mut msg = Msghdr {
        msg_name: std::ptr::null_mut(),
        msg_namelen: 0,
        msg_iov: &mut iov,
        msg_iovlen: 1,
        msg_control: &mut cmsg as *mut _ as *mut std::ffi::c_void,
        msg_controllen: std::mem::size_of::<ScmRights2Buffer>(),
        msg_flags: 0,
    };

    // SAFETY: recvmsg called with valid socket descriptor and properly sized buffer
    let count = unsafe { recvmsg(socket.as_raw_fd(), &mut msg, MSG_CMSG_CLOEXEC) };
    if count < 0 {
        return Err(io::Error::last_os_error());
    }
    if count == 0 {
        return Ok(None);
    }
    if cmsg.hdr.cmsg_level != SOL_SOCKET
        || cmsg.hdr.cmsg_type != SCM_RIGHTS
        || cmsg.fds[0] < 0
        || cmsg.fds[1] < 0
    {
        return Err(io::Error::other("Invalid browser request descriptors"));
    }

    let operation = Operation::parse(op_byte[0])?;
    // SAFETY: File descriptors received from the kernel via SCM_RIGHTS with MSG_CMSG_CLOEXEC
    let input_fd = unsafe { OwnedFd::from_raw_fd(cmsg.fds[0]) };
    // SAFETY: File descriptors received from the kernel via SCM_RIGHTS with MSG_CMSG_CLOEXEC
    let output_fd = unsafe { OwnedFd::from_raw_fd(cmsg.fds[1]) };

    Ok(Some((operation, input_fd, output_fd)))
}

#[cfg(test)]
mod tests {
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
        assert_eq!(Operation::parse(14).unwrap(), Operation::PreviewModel);
        assert_eq!(Operation::parse(15).unwrap(), Operation::PreviewArchiveCover);
        assert_eq!(Operation::parse(16).unwrap(), Operation::PreviewSpreadsheet);
        assert_eq!(Operation::parse(17).unwrap(), Operation::PreviewAudioWaveform);
    }

    #[test]
    fn test_scm_rights_descriptor_passing() {
        let (sock1, sock2) = UnixStream::pair().expect("unix stream pair");
        let (read_pipe, write_pipe) = rust_pipe().expect("pipe creation");

        send(&sock1, &read_pipe, &write_pipe, Operation::Image).expect("send descriptors");

        let (op, in_fd, out_fd) = receive(&sock2).expect("recv descriptors").expect("received");
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
}
