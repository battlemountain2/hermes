// SPDX-License-Identifier: GPL-3.0-or-later
#![allow(dead_code)]

/// Fast dimensions sniffer and frame budget verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDimensions {
    pub width: u32,
    pub height: u32,
}

pub const MAX_DECODED_FRAME_BUDGET_BYTES: u64 = 33_554_432; // 32MB buffer ceiling
pub const MAX_PIXELS_CEILING: u64 = 134_217_728; // ~134 MP ceiling

/// Returns true if the decoded RGBA frame (width * height * 4) exceeds the decoded frame budget.
pub fn exceeds_decoded_frame_budget(width: u32, height: u32) -> bool {
    let pixels = (width as u64) * (height as u64);
    let bytes = pixels.saturating_mul(4);
    bytes > MAX_DECODED_FRAME_BUDGET_BYTES || pixels > MAX_PIXELS_CEILING
}

/// Sniffs PNG dimensions from the IHDR chunk.
pub fn sniff_png_dimensions(bytes: &[u8]) -> Option<ImageDimensions> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() < 24 {
        return None;
    }
    // Bytes 12..16 must be "IHDR"
    if &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some(ImageDimensions { width, height })
}

/// Sniffs GIF dimensions from the Logical Screen Descriptor.
pub fn sniff_gif_dimensions(bytes: &[u8]) -> Option<ImageDimensions> {
    if bytes.len() < 10 {
        return None;
    }
    if !bytes.starts_with(b"GIF87a") && !bytes.starts_with(b"GIF89a") {
        return None;
    }
    let width = u16::from_le_bytes(bytes[6..8].try_into().ok()?) as u32;
    let height = u16::from_le_bytes(bytes[8..10].try_into().ok()?) as u32;
    Some(ImageDimensions { width, height })
}

/// Sniffs JPEG dimensions by scanning for SOF0 (0xFFC0) or SOF2 (0xFFC2).
pub fn sniff_jpeg_dimensions(bytes: &[u8]) -> Option<ImageDimensions> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 8 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if marker == 0xC0 || marker == 0xC1 || marker == 0xC2 {
            let height = u16::from_be_bytes(bytes[i + 5..i + 7].try_into().ok()?) as u32;
            let width = u16::from_be_bytes(bytes[i + 7..i + 9].try_into().ok()?) as u32;
            return Some(ImageDimensions { width, height });
        }
        // Advance by segment length
        if i + 3 < bytes.len() {
            let len = u16::from_be_bytes(bytes[i + 2..i + 4].try_into().ok()?) as usize;
            i += 2 + len;
        } else {
            break;
        }
    }
    None
}

/// Sniffs TIFF dimensions from IFD0 tags.
pub fn sniff_tiff_dimensions(bytes: &[u8]) -> Option<ImageDimensions> {
    if bytes.len() < 8 {
        return None;
    }
    let is_le = if bytes.starts_with(b"II\x2a\x00") {
        true
    } else if bytes.starts_with(b"MM\x00\x2a") {
        false
    } else {
        return None;
    };

    let ifd_offset = if is_le {
        u32::from_le_bytes(bytes[4..8].try_into().ok()?) as usize
    } else {
        u32::from_be_bytes(bytes[4..8].try_into().ok()?) as usize
    };

    if ifd_offset + 2 > bytes.len() {
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

    for idx in 0..num_entries {
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

        let val = if tag_type == 3 {
            // SHORT
            if is_le {
                u16::from_le_bytes(bytes[entry_offset + 8..entry_offset + 10].try_into().ok()?)
                    as u32
            } else {
                u16::from_be_bytes(bytes[entry_offset + 8..entry_offset + 10].try_into().ok()?)
                    as u32
            }
        } else if tag_type == 4 {
            // LONG
            if is_le {
                u32::from_le_bytes(bytes[entry_offset + 8..entry_offset + 12].try_into().ok()?)
            } else {
                u32::from_be_bytes(bytes[entry_offset + 8..entry_offset + 12].try_into().ok()?)
            }
        } else {
            0
        };

        if tag == 256 {
            width = Some(val);
        } else if tag == 257 {
            height = Some(val);
        }
    }

    match (width, height) {
        (Some(w), Some(h)) => Some(ImageDimensions {
            width: w,
            height: h,
        }),
        _ => None,
    }
}
