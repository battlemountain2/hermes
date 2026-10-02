// SPDX-License-Identifier: GPL-3.0-or-later
#![allow(dead_code)]

use std::io::{self, Read};

pub const HEADER_SIZE: usize = 8;
pub const MAX_PAYLOAD_SIZE: u32 = 32 * 1024 * 1024; // 32MB limit

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
    let png_len = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let meta_len = u32::from_le_bytes(header[4..8].try_into().unwrap());

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
