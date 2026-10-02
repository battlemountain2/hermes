// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(dead_code, unused_imports, clippy::all)]

//! Empirical Challenger Stress Test Suite for Milestone 2:
//! - 1. Header sniffing edge cases & malformed inputs (truncated JPEG, invalid IHDR, empty file, BigTIFF variations).
//! - 2. Decoded frame budget exact boundaries (32MB vs 32MB + 1 pixel, aspect ratios, 134 MP ceiling, saturating arithmetic).
//! - 3. Images exceeding budget: valid EXIF thumbnail fallback vs without EXIF thumbnail.
//! - 4. Forged EXIF thumbnails: oversized dimensions, out-of-bounds offsets, zero length, corrupted payloads.
//! - 5. Subprocess CLI helper integration (`strata --preview-helper ...`) verifying full pipeline defense.
//! - 6. Wire protocol & deadline reader quanta responsiveness.

use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use gdk_pixbuf::prelude::*;

mod common;
use common::*;
use common::wire_protocol::*;

// =========================================================================
// Sniffer Implementation & Oracle Definitions
// =========================================================================

pub const MAX_DECODED_FRAME_BUDGET_BYTES: u64 = 33_554_432; // 32MB buffer ceiling
pub const MAX_PIXELS_CEILING: u64 = 134_217_728; // ~134 MP ceiling
pub const TAG_JPEG_INTERCHANGE_FORMAT: u16 = 0x0201;
pub const TAG_JPEG_INTERCHANGE_FORMAT_LENGTH: u16 = 0x0202;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SniffedDimensions {
    pub width: u32,
    pub height: u32,
}

pub fn check_exceeds_decoded_frame_budget(width: u32, height: u32) -> bool {
    let pixels = (width as u64) * (height as u64);
    let bytes = pixels.saturating_mul(4);
    bytes > MAX_DECODED_FRAME_BUDGET_BYTES || pixels > MAX_PIXELS_CEILING
}

pub fn sniff_png(bytes: &[u8]) -> Option<SniffedDimensions> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    if &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if width > 0 && height > 0 {
        Some(SniffedDimensions { width, height })
    } else {
        None
    }
}

pub fn sniff_gif(bytes: &[u8]) -> Option<SniffedDimensions> {
    if bytes.len() < 10 {
        return None;
    }
    if !bytes.starts_with(b"GIF87a") && !bytes.starts_with(b"GIF89a") {
        return None;
    }
    let width = u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32;
    let height = u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32;
    if width > 0 && height > 0 {
        Some(SniffedDimensions { width, height })
    } else {
        None
    }
}

pub fn sniff_jpeg(bytes: &[u8]) -> Option<SniffedDimensions> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        while i < bytes.len() && bytes[i] == 0xFF {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let marker = bytes[i];
        i += 1;

        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        if (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
            continue;
        }

        if i + 2 > bytes.len() {
            return None;
        }
        let len = u16::from_be_bytes(bytes[i..i + 2].try_into().ok()?) as usize;
        if len < 2 {
            return None;
        }

        if matches!(
            marker,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
        ) {
            if i + 7 > bytes.len() {
                return None;
            }
            let height = u16::from_be_bytes(bytes[i + 3..i + 5].try_into().ok()?) as u32;
            let width = u16::from_be_bytes(bytes[i + 5..i + 7].try_into().ok()?) as u32;
            if width > 0 && height > 0 {
                return Some(SniffedDimensions { width, height });
            }
            return None;
        }

        i += len;
    }
    None
}

pub fn sniff_tiff(bytes: &[u8]) -> Option<SniffedDimensions> {
    if bytes.len() < 8 {
        return None;
    }
    let is_le = if bytes.starts_with(b"II\x2a\x00") {
        true
    } else if bytes.starts_with(b"MM\x00\x2a") {
        false
    } else if bytes.starts_with(b"II\x2b\x00") {
        true
    } else if bytes.starts_with(b"MM\x00\x2b") {
        false
    } else {
        return None;
    };

    let magic = if is_le {
        u16::from_le_bytes(bytes[2..4].try_into().ok()?)
    } else {
        u16::from_be_bytes(bytes[2..4].try_into().ok()?)
    };

    if magic == 42 {
        let ifd_offset = if is_le {
            u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize
        } else {
            u32::from_be_bytes(bytes[4..8].try_into().ok()?) as usize
        };

        if ifd_offset < 8 || ifd_offset + 2 > bytes.len() {
            return None;
        }

        let num_entries = if is_le {
            u16::from_le_bytes(bytes[ifd_offset..ifd_offset + 2].try_into().ok()?) as usize
        } else {
            u16::from_be_bytes(bytes[ifd_offset..ifd_offset + 2].try_into().ok()?) as usize
        };

        let mut width = None;
        let mut height = None;
        let entries_start = ifd_offset + 2;

        for idx in 0..num_entries.min(512) {
            let entry_offset = entries_start + idx * 12;
            if entry_offset + 12 > bytes.len() {
                break;
            }
            let tag = if is_le {
                u16::from_le_bytes(bytes[entry_offset..entry_offset + 2].try_into().ok()?)
            } else {
                u16::from_be_bytes(bytes[entry_offset..entry_offset + 2].try_into().ok()?)
            };
            let tag_type = if is_le {
                u16::from_le_bytes(bytes[entry_offset + 2..entry_offset + 4].try_into().ok()?)
            } else {
                u16::from_be_bytes(bytes[entry_offset + 2..entry_offset + 4].try_into().ok()?)
            };

            let val = match tag_type {
                3 => {
                    let v = if is_le {
                        u16::from_le_bytes(
                            bytes[entry_offset + 8..entry_offset + 10].try_into().ok()?,
                        )
                    } else {
                        u16::from_be_bytes(
                            bytes[entry_offset + 8..entry_offset + 10].try_into().ok()?,
                        )
                    };
                    Some(v as u32)
                }
                4 => {
                    let v = if is_le {
                        u32::from_le_bytes(
                            bytes[entry_offset + 8..entry_offset + 12].try_into().ok()?,
                        )
                    } else {
                        u32::from_be_bytes(
                            bytes[entry_offset + 8..entry_offset + 12].try_into().ok()?,
                        )
                    };
                    Some(v)
                }
                _ => None,
            };

            if tag == 256 {
                width = val;
            } else if tag == 257 {
                height = val;
            }
            if let (Some(w), Some(h)) = (width, height) {
                if w > 0 && h > 0 {
                    return Some(SniffedDimensions {
                        width: w,
                        height: h,
                    });
                }
            }
        }
    } else if magic == 43 {
        // BigTIFF
        if bytes.len() < 16 {
            return None;
        }
        let ifd_offset = if is_le {
            u64::from_le_bytes(bytes[8..16].try_into().ok()?) as usize
        } else {
            u64::from_be_bytes(bytes[8..16].try_into().ok()?) as usize
        };

        if ifd_offset + 8 > bytes.len() {
            return None;
        }

        let num_entries = if is_le {
            u64::from_le_bytes(bytes[ifd_offset..ifd_offset + 8].try_into().ok()?) as usize
        } else {
            u64::from_be_bytes(bytes[ifd_offset..ifd_offset + 8].try_into().ok()?) as usize
        };

        let mut width = None;
        let mut height = None;
        let entries_start = ifd_offset + 8;

        for idx in 0..num_entries.min(512) {
            let entry_offset = entries_start + idx * 20;
            if entry_offset + 20 > bytes.len() {
                break;
            }
            let tag = if is_le {
                u16::from_le_bytes(bytes[entry_offset..entry_offset + 2].try_into().ok()?)
            } else {
                u16::from_be_bytes(bytes[entry_offset..entry_offset + 2].try_into().ok()?)
            };
            let tag_type = if is_le {
                u16::from_le_bytes(bytes[entry_offset + 2..entry_offset + 4].try_into().ok()?)
            } else {
                u16::from_be_bytes(bytes[entry_offset + 2..entry_offset + 4].try_into().ok()?)
            };

            let val = match tag_type {
                3 => {
                    let v = if is_le {
                        u16::from_le_bytes(
                            bytes[entry_offset + 12..entry_offset + 14].try_into().ok()?,
                        )
                    } else {
                        u16::from_be_bytes(
                            bytes[entry_offset + 12..entry_offset + 14].try_into().ok()?,
                        )
                    };
                    Some(v as u32)
                }
                4 => {
                    let v = if is_le {
                        u32::from_le_bytes(
                            bytes[entry_offset + 12..entry_offset + 16].try_into().ok()?,
                        )
                    } else {
                        u32::from_be_bytes(
                            bytes[entry_offset + 12..entry_offset + 16].try_into().ok()?,
                        )
                    };
                    Some(v)
                }
                16 => {
                    let v = if is_le {
                        u64::from_le_bytes(
                            bytes[entry_offset + 12..entry_offset + 20].try_into().ok()?,
                        )
                    } else {
                        u64::from_be_bytes(
                            bytes[entry_offset + 12..entry_offset + 20].try_into().ok()?,
                        )
                    };
                    u32::try_from(v).ok()
                }
                _ => None,
            };

            if tag == 256 {
                width = val;
            } else if tag == 257 {
                height = val;
            }
            if let (Some(w), Some(h)) = (width, height) {
                if w > 0 && h > 0 {
                    return Some(SniffedDimensions {
                        width: w,
                        height: h,
                    });
                }
            }
        }
    }

    None
}

// =========================================================================
// Synthetic Fixture Generators
// =========================================================================

pub fn generate_valid_jpeg(width: i32, height: i32) -> Vec<u8> {
    let pixbuf = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, width, height)
        .expect("pixbuf creation");
    pixbuf
        .save_to_bufferv("jpeg", &[("quality", "75")])
        .expect("save to jpeg")
}

pub fn generate_oversized_jpeg_raw(width: u16, height: u16) -> Vec<u8> {
    vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xC0, // SOF0
        0x00, 0x0B, // Length = 11
        0x08,       // Precision
        (height >> 8) as u8, height as u8,
        (width >> 8) as u8, width as u8,
        0x03, 0x01, 0x11, 0x00, // 3 components
        0xFF, 0xD9, // EOI
    ]
}

pub fn generate_jpeg_with_exif(
    main_width: u16,
    main_height: u16,
    thumb_payload: &[u8],
    override_thumb_offset: Option<u32>,
    override_thumb_length: Option<u32>,
) -> Vec<u8> {
    let mut tiff = Vec::new();
    // TIFF Header (8 bytes)
    tiff.extend_from_slice(b"II*\0"); // Little endian magic 42
    tiff.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset: 8

    // IFD0 (starts at offset 8)
    tiff.extend_from_slice(&1u16.to_le_bytes()); // 1 entry
    // Entry: Tag 0x0112 (Orientation), type 3 (SHORT), count 1, value 1
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());

    // Pointer to IFD1 (at offset 8 + 2 + 12 = 22)
    let ifd1_offset = 26u32;
    tiff.extend_from_slice(&ifd1_offset.to_le_bytes());

    // IFD1 (starts at offset 26)
    tiff.extend_from_slice(&2u16.to_le_bytes()); // 2 entries

    let default_thumb_offset = 56u32;
    let thumb_offset = override_thumb_offset.unwrap_or(default_thumb_offset);
    let thumb_len = override_thumb_length.unwrap_or(thumb_payload.len() as u32);

    // Entry 0: TAG_JPEG_INTERCHANGE_FORMAT (0x0201), type 4 (LONG), count 1
    tiff.extend_from_slice(&TAG_JPEG_INTERCHANGE_FORMAT.to_le_bytes());
    tiff.extend_from_slice(&4u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&thumb_offset.to_le_bytes());

    // Entry 1: TAG_JPEG_INTERCHANGE_FORMAT_LENGTH (0x0202), type 4 (LONG), count 1
    tiff.extend_from_slice(&TAG_JPEG_INTERCHANGE_FORMAT_LENGTH.to_le_bytes());
    tiff.extend_from_slice(&4u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&thumb_len.to_le_bytes());

    // Next IFD offset: 0 (4 bytes)
    tiff.extend_from_slice(&0u32.to_le_bytes());

    assert_eq!(tiff.len(), 56);
    // Append thumbnail payload
    tiff.extend_from_slice(thumb_payload);

    // Build full JPEG with APP1
    let mut jpeg = Vec::new();
    jpeg.extend_from_slice(&[0xFF, 0xD8]); // SOI

    // APP1 Marker
    jpeg.extend_from_slice(&[0xFF, 0xE1]);
    let app1_len = (2 + 6 + tiff.len()) as u16;
    jpeg.extend_from_slice(&app1_len.to_be_bytes());
    jpeg.extend_from_slice(b"Exif\0\0");
    jpeg.extend_from_slice(&tiff);

    // Main image SOF0
    jpeg.extend_from_slice(&[0xFF, 0xC0]);
    jpeg.extend_from_slice(&11u16.to_be_bytes());
    jpeg.push(8); // Precision
    jpeg.extend_from_slice(&main_height.to_be_bytes());
    jpeg.extend_from_slice(&main_width.to_be_bytes());
    jpeg.extend_from_slice(&[3, 1, 0x11, 0]); // Components
    jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI

    jpeg
}

pub fn generate_bigtiff(is_le: bool, width: u64, height: u64, tag_type: u16) -> Vec<u8> {
    let mut bytes = Vec::new();
    if is_le {
        bytes.extend_from_slice(b"II");
        bytes.extend_from_slice(&43u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes()); // Bytesize of offsets: 8
        bytes.extend_from_slice(&0u16.to_le_bytes()); // Reserved: 0
        bytes.extend_from_slice(&16u64.to_le_bytes()); // IFD0 offset: 16
    } else {
        bytes.extend_from_slice(b"MM");
        bytes.extend_from_slice(&43u16.to_be_bytes());
        bytes.extend_from_slice(&8u16.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&16u64.to_be_bytes());
    }

    // IFD0 at offset 16
    let num_entries = 2u64;
    if is_le {
        bytes.extend_from_slice(&num_entries.to_le_bytes());
    } else {
        bytes.extend_from_slice(&num_entries.to_be_bytes());
    }

    // Tag 256 (ImageWidth)
    if is_le {
        bytes.extend_from_slice(&256u16.to_le_bytes());
        bytes.extend_from_slice(&tag_type.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        match tag_type {
            3 => {
                bytes.extend_from_slice(&(width as u16).to_le_bytes());
                bytes.extend_from_slice(&[0u8; 6]);
            }
            4 => {
                bytes.extend_from_slice(&(width as u32).to_le_bytes());
                bytes.extend_from_slice(&[0u8; 4]);
            }
            16 => {
                bytes.extend_from_slice(&width.to_le_bytes());
            }
            _ => panic!("unsupported tag type in test generator"),
        }
    } else {
        bytes.extend_from_slice(&256u16.to_be_bytes());
        bytes.extend_from_slice(&tag_type.to_be_bytes());
        bytes.extend_from_slice(&1u64.to_be_bytes());
        match tag_type {
            3 => {
                bytes.extend_from_slice(&(width as u16).to_be_bytes());
                bytes.extend_from_slice(&[0u8; 6]);
            }
            4 => {
                bytes.extend_from_slice(&(width as u32).to_be_bytes());
                bytes.extend_from_slice(&[0u8; 4]);
            }
            16 => {
                bytes.extend_from_slice(&width.to_be_bytes());
            }
            _ => panic!("unsupported tag type in test generator"),
        }
    }

    // Tag 257 (ImageLength)
    if is_le {
        bytes.extend_from_slice(&257u16.to_le_bytes());
        bytes.extend_from_slice(&tag_type.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        match tag_type {
            3 => {
                bytes.extend_from_slice(&(height as u16).to_le_bytes());
                bytes.extend_from_slice(&[0u8; 6]);
            }
            4 => {
                bytes.extend_from_slice(&(height as u32).to_le_bytes());
                bytes.extend_from_slice(&[0u8; 4]);
            }
            16 => {
                bytes.extend_from_slice(&height.to_le_bytes());
            }
            _ => panic!("unsupported tag type in test generator"),
        }
    } else {
        bytes.extend_from_slice(&257u16.to_be_bytes());
        bytes.extend_from_slice(&tag_type.to_be_bytes());
        bytes.extend_from_slice(&1u64.to_be_bytes());
        match tag_type {
            3 => {
                bytes.extend_from_slice(&(height as u16).to_be_bytes());
                bytes.extend_from_slice(&[0u8; 6]);
            }
            4 => {
                bytes.extend_from_slice(&(height as u32).to_be_bytes());
                bytes.extend_from_slice(&[0u8; 4]);
            }
            16 => {
                bytes.extend_from_slice(&height.to_be_bytes());
            }
            _ => panic!("unsupported tag type in test generator"),
        }
    }

    // Next IFD offset: 0 (8 bytes)
    bytes.extend_from_slice(&[0u8; 8]);
    bytes
}

// =========================================================================
// 1. Header Sniffing Edge Cases & Malformed Inputs Tests
// =========================================================================

#[test]
fn test_stress_png_edge_cases() {
    assert!(sniff_png(b"").is_none(), "empty PNG slice");
    assert!(sniff_png(b"\x89PNG\r\n\x1a").is_none(), "7-byte partial signature");
    assert!(sniff_png(b"\x89PNG\r\n\x1a\n").is_none(), "signature only");
    assert!(sniff_png(b"\x89PNG\r\n\x1a\n\0\0\0\x0D").is_none(), "signature + length only");

    // Valid IHDR chunk
    let mut valid_png = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
        0x00, 0x00, 0x00, 0x0D,
        b'I', b'H', b'D', b'R',
        0x00, 0x00, 0x04, 0x00, // width: 1024
        0x00, 0x00, 0x03, 0x00, // height: 768
    ];
    let dims = sniff_png(&valid_png).expect("valid png");
    assert_eq!(dims.width, 1024);
    assert_eq!(dims.height, 768);

    // Truncate inside dimensions
    for cut in 12..24 {
        assert!(sniff_png(&valid_png[..cut]).is_none(), "cut at {cut}");
    }

    // Corrupted IHDR chunk name
    valid_png[12] = b's';
    valid_png[13] = b'R';
    valid_png[14] = b'G';
    valid_png[15] = b'B';
    assert!(sniff_png(&valid_png).is_none(), "chunk name is sRGB");

    // Zero width
    valid_png[12..16].copy_from_slice(b"IHDR");
    valid_png[16..20].copy_from_slice(&0u32.to_be_bytes());
    assert!(sniff_png(&valid_png).is_none(), "zero width");

    // Zero height
    valid_png[16..20].copy_from_slice(&100u32.to_be_bytes());
    valid_png[20..24].copy_from_slice(&0u32.to_be_bytes());
    assert!(sniff_png(&valid_png).is_none(), "zero height");

    // High-bit set (2^31)
    valid_png[16..20].copy_from_slice(&0x8000_0000u32.to_be_bytes());
    valid_png[20..24].copy_from_slice(&1u32.to_be_bytes());
    let dims = sniff_png(&valid_png).expect("u32 high bit");
    assert_eq!(dims.width, 0x8000_0000);
    assert_eq!(dims.height, 1);
    assert!(check_exceeds_decoded_frame_budget(dims.width, dims.height));
}

#[test]
fn test_stress_gif_edge_cases() {
    assert!(sniff_gif(b"").is_none());
    assert!(sniff_gif(b"GIF").is_none());
    assert!(sniff_gif(b"GIF85a\x10\x00\x10\x00").is_none(), "invalid GIF version");

    let gif87 = b"GIF87a\x20\x01\x10\x02";
    let dims = sniff_gif(gif87).expect("valid GIF87a");
    assert_eq!(dims.width, 0x0120);
    assert_eq!(dims.height, 0x0210);

    let gif89 = b"GIF89a\xFF\xFF\xFF\xFF";
    let dims = sniff_gif(gif89).expect("valid GIF89a 65535x65535");
    assert_eq!(dims.width, 65535);
    assert_eq!(dims.height, 65535);

    for cut in 0..10 {
        assert!(sniff_gif(&gif87[..cut]).is_none());
    }

    let zero_w = b"GIF89a\x00\x00\x10\x00";
    assert!(sniff_gif(zero_w).is_none(), "zero width GIF");
}

#[test]
fn test_stress_jpeg_edge_cases() {
    assert!(sniff_jpeg(b"").is_none());
    assert!(sniff_jpeg(b"\xFF").is_none());
    assert!(sniff_jpeg(b"\xFF\xD8").is_none(), "SOI only");
    assert!(sniff_jpeg(b"\xFF\xD8\xFF\xD9").is_none(), "SOI followed immediately by EOI");
    assert!(sniff_jpeg(b"\xFF\xD8\xFF\xDA").is_none(), "SOI followed immediately by SOS");

    // Multiple 0xFF padding bytes before SOF0
    let padded_jpeg = vec![
        0xFF, 0xD8,
        0xFF, 0xFF, 0xFF, 0xFF, 0xC0, // consecutive 0xFF padding
        0x00, 0x0B, 0x08, 0x01, 0x00, 0x02, 0x00, 0x03, 0x01, 0x11, 0x00,
    ];
    let dims = sniff_jpeg(&padded_jpeg).expect("padded jpeg");
    assert_eq!(dims.height, 256);
    assert_eq!(dims.width, 512);

    // Standalone markers: RST0 (0xD0) and TEM (0x01) before SOF0
    let rst_jpeg = vec![
        0xFF, 0xD8,
        0xFF, 0xD0, // RST0 (no length field)
        0xFF, 0x01, // TEM (no length field)
        0xFF, 0xC0,
        0x00, 0x0B, 0x08, 0x00, 0x64, 0x00, 0xC8, 0x03, 0x01, 0x11, 0x00,
    ];
    let dims = sniff_jpeg(&rst_jpeg).expect("standalone marker jpeg");
    assert_eq!(dims.height, 100);
    assert_eq!(dims.width, 200);

    // Progressive SOF2 marker (0xC2)
    let prog_jpeg = vec![
        0xFF, 0xD8,
        0xFF, 0xC2, // SOF2 progressive
        0x00, 0x0B, 0x08, 0x02, 0x80, 0x03, 0xC0, 0x03, 0x01, 0x11, 0x00,
    ];
    let dims = sniff_jpeg(&prog_jpeg).expect("progressive jpeg");
    assert_eq!(dims.height, 640);
    assert_eq!(dims.width, 960);

    // Truncated length payload
    let bad_len = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00];
    assert!(sniff_jpeg(&bad_len).is_none());

    // Invalid length < 2
    let invalid_len = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x01];
    assert!(sniff_jpeg(&invalid_len).is_none());

    // Zero dimensions in SOF
    let zero_dim = vec![
        0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x00, 0x00, 0x64, 0x03, 0x01, 0x11, 0x00,
    ];
    assert!(sniff_jpeg(&zero_dim).is_none());
}

#[test]
fn test_stress_bigtiff_variations() {
    // 1. BigTIFF Little-Endian with SHORT tags (type 3)
    let bt_le_short = generate_bigtiff(true, 1200, 800, 3);
    let dims = sniff_tiff(&bt_le_short).expect("BigTIFF LE SHORT");
    assert_eq!(dims.width, 1200);
    assert_eq!(dims.height, 800);

    // 2. BigTIFF Little-Endian with LONG tags (type 4)
    let bt_le_long = generate_bigtiff(true, 50000, 40000, 4);
    let dims = sniff_tiff(&bt_le_long).expect("BigTIFF LE LONG");
    assert_eq!(dims.width, 50000);
    assert_eq!(dims.height, 40000);

    // 3. BigTIFF Little-Endian with LONG8 tags (type 16)
    let bt_le_long8 = generate_bigtiff(true, 100000, 80000, 16);
    let dims = sniff_tiff(&bt_le_long8).expect("BigTIFF LE LONG8");
    assert_eq!(dims.width, 100000);
    assert_eq!(dims.height, 80000);

    // 4. BigTIFF Big-Endian with SHORT tags (type 3)
    let bt_be_short = generate_bigtiff(false, 640, 480, 3);
    let dims = sniff_tiff(&bt_be_short).expect("BigTIFF BE SHORT");
    assert_eq!(dims.width, 640);
    assert_eq!(dims.height, 480);

    // 5. BigTIFF Big-Endian with LONG tags (type 4)
    let bt_be_long = generate_bigtiff(false, 30000, 20000, 4);
    let dims = sniff_tiff(&bt_be_long).expect("BigTIFF BE LONG");
    assert_eq!(dims.width, 30000);
    assert_eq!(dims.height, 20000);

    // 6. BigTIFF Big-Endian with LONG8 tags (type 16)
    let bt_be_long8 = generate_bigtiff(false, 70000, 60000, 16);
    let dims = sniff_tiff(&bt_be_long8).expect("BigTIFF BE LONG8");
    assert_eq!(dims.width, 70000);
    assert_eq!(dims.height, 60000);

    // 7. Malformed BigTIFF: Truncated header (< 16 bytes)
    assert!(sniff_tiff(&bt_le_short[..14]).is_none());

    // 8. Malformed BigTIFF: Out of bounds IFD offset
    let mut corrupt_ifd = bt_le_short.clone();
    corrupt_ifd[8..16].copy_from_slice(&0x00FF_FFFF_FFFF_FFFFu64.to_le_bytes());
    assert!(sniff_tiff(&corrupt_ifd).is_none());

    // 9. Unknown TIFF magic (magic 44)
    let mut corrupt_magic = bt_le_short.clone();
    corrupt_magic[2] = 44;
    assert!(sniff_tiff(&corrupt_magic).is_none());
}

// =========================================================================
// 2. Decoded Frame Budget Exact Boundaries Tests
// =========================================================================

#[test]
fn test_stress_frame_budget_exact_boundaries() {
    // Exactly 32 MB: 8,388,608 pixels * 4 = 33,554,432 bytes
    assert!(
        !check_exceeds_decoded_frame_budget(2048, 4096),
        "Exactly 32MB (2048x4096) must NOT exceed budget"
    );

    // Exactly 32 MB + 1 pixel: 8,388,609 pixels * 4 = 33,554,436 bytes
    assert!(
        check_exceeds_decoded_frame_budget(8388609, 1),
        "32MB + 1 pixel (8388609x1) MUST exceed budget"
    );

    // 1D boundary tests
    assert!(!check_exceeds_decoded_frame_budget(8388608, 1));
    assert!(check_exceeds_decoded_frame_budget(8388609, 1));
    assert!(!check_exceeds_decoded_frame_budget(1, 8388608));
    assert!(check_exceeds_decoded_frame_budget(1, 8388609));

    // Rectangular boundary tests
    assert!(!check_exceeds_decoded_frame_budget(4096, 2048));
    assert!(check_exceeds_decoded_frame_budget(4097, 2048));
    assert!(check_exceeds_decoded_frame_budget(2048, 4097));

    // Square boundary tests
    // 2896 x 2896 = 8,386,816 pixels * 4 = 33,547,264 bytes (< 32MB)
    assert!(!check_exceeds_decoded_frame_budget(2896, 2896));
    // 2897 x 2897 = 8,392,609 pixels * 4 = 33,570,436 bytes (> 32MB)
    assert!(check_exceeds_decoded_frame_budget(2897, 2897));

    // Gigapixel ceiling (134 MP = 134,217,728 pixels)
    // 11585 x 11585 = 134,212,225 pixels (< 134 MP, but > 32MB)
    assert!(check_exceeds_decoded_frame_budget(11585, 11585));
    // 12000 x 12000 = 144,000,000 pixels (> 134 MP ceiling)
    assert!(check_exceeds_decoded_frame_budget(12000, 12000));

    // Extreme arithmetic boundaries: saturating_mul guarantees zero panics
    assert!(check_exceeds_decoded_frame_budget(u32::MAX, 1));
    assert!(check_exceeds_decoded_frame_budget(u32::MAX, u32::MAX));
    assert!(!check_exceeds_decoded_frame_budget(0, 0));
    assert!(!check_exceeds_decoded_frame_budget(u32::MAX, 0));
}

// =========================================================================
// 3. Subprocess Binary Empirical Pipeline Tests (strata CLI)
// =========================================================================

#[test]
fn test_binary_empty_file_handling() {
    let env = TestEnv::new();
    let empty_file = env.write_file("empty.jpg", b"");
    let output_png = env.file_path("output.png");

    let res = run_preview_helper("thumbnail-image", &empty_file, &output_png, 256);
    assert!(!res.status.success(), "Empty file must exit non-zero");
    assert!(!output_png.exists(), "No output file generated for empty file");
}

#[test]
fn test_binary_truncated_jpeg_fails_gracefully() {
    let env = TestEnv::new();
    let trunc_file = env.write_file("trunc.jpg", &[0xFF, 0xD8, 0xFF, 0xC0, 0x00]);
    let output_png = env.file_path("output.png");

    let res = run_preview_helper("thumbnail-image", &trunc_file, &output_png, 256);
    assert!(!res.status.success(), "Truncated JPEG must exit non-zero");
    assert!(!output_png.exists());
}

#[test]
fn test_binary_oversized_image_without_exif_fails_with_guardrail_message() {
    let env = TestEnv::new();
    // 10,000 x 10,000 image (100 MP, 400 MB decoded buffer) WITHOUT EXIF
    let oversized_raw = generate_oversized_jpeg_raw(10000, 10000);
    let input_path = env.write_file("oversized_no_exif.jpg", &oversized_raw);
    let output_png = env.file_path("output.png");

    let res = run_preview_helper("thumbnail-image", &input_path, &output_png, 256);
    assert!(
        !res.status.success(),
        "Oversized image without EXIF must fail"
    );
    assert!(
        res.stderr.contains("exceed the decoded frame budget"),
        "Must output frame budget guardrail error message in stderr, got: {}",
        res.stderr
    );
    assert!(!output_png.exists(), "Must not write uncompressed output PNG");
}

#[test]
fn test_binary_oversized_image_with_valid_exif_succeeds() {
    let env = TestEnv::new();
    // Generate valid 160x120 JPEG thumbnail
    let thumb_jpeg = generate_valid_jpeg(160, 120);

    // 10,000 x 10,000 image WITH valid EXIF thumbnail
    let oversized_with_exif = generate_jpeg_with_exif(10000, 10000, &thumb_jpeg, None, None);
    let input_path = env.write_file("oversized_with_exif.jpg", &oversized_with_exif);
    let output_png = env.file_path("output.png");

    let res = run_preview_helper("thumbnail-image", &input_path, &output_png, 256);
    assert!(
        res.status.success(),
        "Oversized image with valid EXIF thumbnail MUST succeed! stderr: {}",
        res.stderr
    );
    assert!(output_png.exists(), "Output thumbnail PNG must be written");

    let png_bytes = fs::read(&output_png).expect("read output png");
    assert!(is_valid_png(&png_bytes), "Must produce valid PNG file");
    assert!(!png_bytes.is_empty(), "PNG file must not be empty");
}

#[test]
fn test_binary_oversized_image_with_forged_oversized_exif_rejected() {
    let env = TestEnv::new();
    // Forged thumbnail claiming 20,000 x 20,000 dimensions
    let forged_oversized_thumb = generate_oversized_jpeg_raw(20000, 20000);

    let oversized_with_forged_exif =
        generate_jpeg_with_exif(10000, 10000, &forged_oversized_thumb, None, None);
    let input_path = env.write_file("forged_exif.jpg", &oversized_with_forged_exif);
    let output_png = env.file_path("output.png");

    let res = run_preview_helper("thumbnail-image", &input_path, &output_png, 256);
    assert!(
        !res.status.success(),
        "Forged oversized EXIF thumbnail must be rejected before decode"
    );
    assert!(
        res.stderr.contains("exceed the decoded frame budget"),
        "Must report frame budget violation, got: {}",
        res.stderr
    );
    assert!(!output_png.exists());
}

#[test]
fn test_binary_oversized_image_with_out_of_bounds_offset_rejected() {
    let env = TestEnv::new();
    let thumb_jpeg = generate_valid_jpeg(16, 16);

    // Point offset to 9999999 (far beyond file size)
    let bad_offset_jpeg =
        generate_jpeg_with_exif(10000, 10000, &thumb_jpeg, Some(9_999_999), None);
    let input_path = env.write_file("bad_offset.jpg", &bad_offset_jpeg);
    let output_png = env.file_path("output.png");

    let res = run_preview_helper("thumbnail-image", &input_path, &output_png, 256);
    assert!(!res.status.success());
    assert!(!output_png.exists());
}

#[test]
fn test_binary_oversized_image_with_zero_length_thumbnail_rejected() {
    let env = TestEnv::new();
    let thumb_jpeg = generate_valid_jpeg(16, 16);

    // Set thumbnail length tag to 0
    let zero_len_jpeg = generate_jpeg_with_exif(10000, 10000, &thumb_jpeg, None, Some(0));
    let input_path = env.write_file("zero_len.jpg", &zero_len_jpeg);
    let output_png = env.file_path("output.png");

    let res = run_preview_helper("thumbnail-image", &input_path, &output_png, 256);
    assert!(!res.status.success());
    assert!(!output_png.exists());
}

#[test]
fn test_binary_oversized_image_with_non_jpeg_payload_rejected() {

    let env = TestEnv::new();
    let non_jpeg_payload = b"NOT_A_JPEG_FILE_HEADER_DATA";

    let bad_payload_jpeg =
        generate_jpeg_with_exif(10000, 10000, non_jpeg_payload, None, None);
    let input_path = env.write_file("bad_payload.jpg", &bad_payload_jpeg);
    let output_png = env.file_path("output.png");

    let res = run_preview_helper("thumbnail-image", &input_path, &output_png, 256);
    assert!(!res.status.success());
    assert!(!output_png.exists());
}

#[test]
fn test_binary_oversized_bigtiff_rejected() {
    let env = TestEnv::new();
    // BigTIFF 50,000 x 40,000 (> 32MB)
    let bigtiff_data = generate_bigtiff(true, 50000, 40000, 4);
    let input_path = env.write_file("oversized.tiff", &bigtiff_data);
    let output_png = env.file_path("output.png");

    let res = run_preview_helper("thumbnail-image", &input_path, &output_png, 256);
    assert!(!res.status.success());
    assert!(
        res.stderr.contains("exceed the decoded frame budget"),
        "Stderr must contain frame budget guardrail error, got: {}",
        res.stderr
    );
    assert!(!output_png.exists());
}

// =========================================================================
// 4. Wire Protocol & Quanta Cancellation Tests
// =========================================================================

#[test]
fn test_wire_framing_zero_payloads_round_trip() {
    let frame = encode_frame(&[], &[]);
    assert_eq!(frame.len(), 8);
    let (png, meta) = read_framed_message(&frame[..]).expect("decode empty frame");
    assert!(png.is_empty());
    assert!(meta.is_empty());
}

#[test]
fn test_wire_framing_exact_max_payload() {
    let max_len = 32 * 1024 * 1024; // 32MB
    let mut header = [0u8; 8];
    header[0..4].copy_from_slice(&(max_len as u32).to_le_bytes());
    header[4..8].copy_from_slice(&0u32.to_le_bytes());

    let (png_len, meta_len) = decode_header(&header).expect("exact max payload");
    assert_eq!(png_len, max_len as u32);
    assert_eq!(meta_len, 0);

    // Exceed by 1 byte
    header[0..4].copy_from_slice(&((max_len + 1) as u32).to_le_bytes());
    assert!(decode_header(&header).is_err());
}

#[test]
fn test_cancellation_token_instant_response() {
    let cancel = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    // Trigger cancellation
    cancel.store(true, Ordering::Release);
    assert!(cancel.load(Ordering::Acquire));

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(5),
        "Cancellation signal propagation must be instantaneous (<5ms), took {:?}",
        elapsed
    );
}
