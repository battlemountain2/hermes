// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::Read,
    path::Path,
};

use gdk_pixbuf::prelude::*;

pub const MAX_DECODED_FRAME_BUDGET_BYTES: u64 = 33_554_432; // 32MB buffer ceiling
pub const MAX_PIXELS_CEILING: u64 = 134_217_728; // ~134 MP ceiling

pub const TAG_JPEG_INTERCHANGE_FORMAT: u16 = 0x0201; // 513
pub const TAG_JPEG_INTERCHANGE_FORMAT_LENGTH: u16 = 0x0202; // 514

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDimensions {
    pub width: u32,
    pub height: u32,
}

/// Returns true if the decoded RGBA frame (width * height * 4) exceeds the decoded frame budget.
pub fn exceeds_decoded_frame_budget(width: u32, height: u32) -> bool {
    let pixels = (width as u64) * (height as u64);
    let bytes = pixels.saturating_mul(4);
    bytes > MAX_DECODED_FRAME_BUDGET_BYTES || pixels > MAX_PIXELS_CEILING
}

/// Sniffs PNG dimensions from the IHDR chunk.
pub fn sniff_png_dimensions(bytes: &[u8]) -> Option<ImageDimensions> {
    if bytes.len() < 24 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    // Bytes 12..16 must be "IHDR"
    if &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if width > 0 && height > 0 {
        Some(ImageDimensions { width, height })
    } else {
        None
    }
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
    if width > 0 && height > 0 {
        Some(ImageDimensions { width, height })
    } else {
        None
    }
}

/// Sniffs JPEG dimensions by scanning for SOF0 (0xFFC0) or SOF2 (0xFFC2) and other SOFn markers.
pub fn sniff_jpeg_dimensions(bytes: &[u8]) -> Option<ImageDimensions> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        // Skip consecutive 0xFF padding bytes
        while i < bytes.len() && bytes[i] == 0xFF {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let marker = bytes[i];
        i += 1;

        // SOS (Start of Scan) or EOI (End of Image) terminates header scanning
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        // Standalone markers with no length payload
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

        // Frame header markers: SOF0..SOF3, SOF5..SOF7, SOF9..SOF11, SOF13..SOF15
        if matches!(
            marker,
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
        ) {
            if i + 7 > bytes.len() {
                return None;
            }
            // Byte i+2: precision
            // Bytes i+3..i+5: height
            // Bytes i+5..i+7: width
            let height = u16::from_be_bytes(bytes[i + 3..i + 5].try_into().ok()?) as u32;
            let width = u16::from_be_bytes(bytes[i + 5..i + 7].try_into().ok()?) as u32;
            if width > 0 && height > 0 {
                return Some(ImageDimensions { width, height });
            }
            return None;
        }

        i += len;
    }
    None
}

/// Sniffs TIFF dimensions from IFD0 tags (Standard TIFF & BigTIFF, little-endian and big-endian).
pub fn sniff_tiff_dimensions(bytes: &[u8]) -> Option<ImageDimensions> {
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
                    // SHORT
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
                    // LONG
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
                    return Some(ImageDimensions {
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
                    return Some(ImageDimensions {
                        width: w,
                        height: h,
                    });
                }
            }
        }
    }

    None
}

/// Sniffs image dimensions from a file by inspecting headers without decoding pixel rasters.
pub fn image_dimensions(path: &Path) -> Option<(u32, u32)> {
    let mut file = fs::File::open(path).ok()?;
    let mut buffer = vec![0u8; 65536];
    let count = file.read(&mut buffer).ok()?;
    buffer.truncate(count);

    if let Some(dim) = sniff_png_dimensions(&buffer) {
        return Some((dim.width, dim.height));
    }
    if let Some(dim) = sniff_gif_dimensions(&buffer) {
        return Some((dim.width, dim.height));
    }
    if let Some(dim) = sniff_jpeg_dimensions(&buffer) {
        return Some((dim.width, dim.height));
    }
    if let Some(dim) = sniff_tiff_dimensions(&buffer) {
        return Some((dim.width, dim.height));
    }

    // For large TIFFs where IFD0 is at the end of the file, read the whole file if under 2MB
    if count == 65536 {
        if let Ok(metadata) = file.metadata() {
            if metadata.len() <= 2 * 1024 * 1024 {
                if let Ok(full_data) = fs::read(path) {
                    if let Some(dim) = sniff_tiff_dimensions(&full_data) {
                        return Some((dim.width, dim.height));
                    }
                }
            }
        }
    }

    None
}

/// Extracts an embedded EXIF thumbnail from APP1 / IFD1 and scales it to `size`.
pub fn read_exif_thumbnail(path: &Path, size: i32) -> Option<Vec<u8>> {
    let mut file = fs::File::open(path).ok()?;
    // Read up to 4MB for EXIF header and thumbnail payload
    let mut data = vec![0u8; 4 * 1024 * 1024];
    let count = file.read(&mut data).ok()?;
    data.truncate(count);

    let thumb_bytes = extract_exif_thumbnail_bytes(&data)?;
    scale_embedded_thumbnail(thumb_bytes, size).ok()
}

/// Locates and returns the embedded JPEG thumbnail slice from an EXIF container.
fn extract_exif_thumbnail_bytes(data: &[u8]) -> Option<&[u8]> {
    // 1. In JPEG files, find APP1 marker (0xFFE1) with "Exif\0\0"
    if data.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 4 <= data.len() {
            if data[i] != 0xFF {
                i += 1;
                continue;
            }
            while i < data.len() && data[i] == 0xFF {
                i += 1;
            }
            if i >= data.len() {
                break;
            }
            let marker = data[i];
            i += 1;
            if marker == 0xDA || marker == 0xD9 {
                break;
            }
            if (0xD0..=0xD8).contains(&marker) || marker == 0x01 {
                continue;
            }
            if i + 2 > data.len() {
                break;
            }
            let len = u16::from_be_bytes(data[i..i + 2].try_into().ok()?) as usize;
            if len < 2 {
                break;
            }
            let seg_start = i + 2;
            let seg_end = (i + len).min(data.len());
            if marker == 0xE1 && seg_start + 6 <= seg_end && &data[seg_start..seg_start + 6] == b"Exif\0\0" {
                let tiff_bytes = &data[seg_start + 6..seg_end];
                return extract_thumbnail_from_tiff_block(tiff_bytes);
            }
            i += len;
        }
    }

    // 2. In TIFF / DNG files, parse the TIFF block directly
    if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        return extract_thumbnail_from_tiff_block(data);
    }

    None
}

/// Parses TIFF IFD0 and IFD1 to find tags 0x0201 (JPEGInterchangeFormat) and 0x0202 (JPEGInterchangeFormatLength).
fn extract_thumbnail_from_tiff_block(tiff: &[u8]) -> Option<&[u8]> {
    if tiff.len() < 8 {
        return None;
    }
    let is_le = if tiff.starts_with(b"II*\0") {
        true
    } else if tiff.starts_with(b"MM\0*") {
        false
    } else {
        return None;
    };

    let ifd0_offset = if is_le {
        u32::from_le_bytes(tiff[4..8].try_into().ok()?) as usize
    } else {
        u32::from_be_bytes(tiff[4..8].try_into().ok()?) as usize
    };

    if ifd0_offset + 2 > tiff.len() {
        return None;
    }

    let num_entries = if is_le {
        u16::from_le_bytes(tiff[ifd0_offset..ifd0_offset + 2].try_into().ok()?) as usize
    } else {
        u16::from_be_bytes(tiff[ifd0_offset..ifd0_offset + 2].try_into().ok()?) as usize
    };

    let ifd1_ptr_offset = ifd0_offset + 2 + num_entries * 12;
    if ifd1_ptr_offset + 4 > tiff.len() {
        return None;
    }

    let ifd1_offset = if is_le {
        u32::from_le_bytes(tiff[ifd1_ptr_offset..ifd1_ptr_offset + 4].try_into().ok()?) as usize
    } else {
        u32::from_be_bytes(tiff[ifd1_ptr_offset..ifd1_ptr_offset + 4].try_into().ok()?) as usize
    };

    if ifd1_offset == 0 || ifd1_offset + 2 > tiff.len() {
        return None;
    }

    let ifd1_num_entries = if is_le {
        u16::from_le_bytes(tiff[ifd1_offset..ifd1_offset + 2].try_into().ok()?) as usize
    } else {
        u16::from_be_bytes(tiff[ifd1_offset..ifd1_offset + 2].try_into().ok()?) as usize
    };

    let mut thumb_offset = None;
    let mut thumb_length = None;
    let entries_start = ifd1_offset + 2;

    for idx in 0..ifd1_num_entries.min(512) {
        let entry = entries_start + idx * 12;
        if entry + 12 > tiff.len() {
            break;
        }
        let tag = if is_le {
            u16::from_le_bytes(tiff[entry..entry + 2].try_into().ok()?)
        } else {
            u16::from_be_bytes(tiff[entry..entry + 2].try_into().ok()?)
        };
        let val = if is_le {
            u32::from_le_bytes(tiff[entry + 8..entry + 12].try_into().ok()?) as usize
        } else {
            u32::from_be_bytes(tiff[entry + 8..entry + 12].try_into().ok()?) as usize
        };

        if tag == TAG_JPEG_INTERCHANGE_FORMAT {
            thumb_offset = Some(val);
        } else if tag == TAG_JPEG_INTERCHANGE_FORMAT_LENGTH {
            thumb_length = Some(val);
        }
    }

    let (offset, length) = (thumb_offset?, thumb_length?);
    if length == 0 || offset >= tiff.len() || offset.checked_add(length)? > tiff.len() {
        return None;
    }

    let slice = &tiff[offset..offset + length];
    if slice.len() >= 4 && slice[0] == 0xFF && slice[1] == 0xD8 {
        Some(slice)
    } else {
        None
    }
}

/// Decodes an embedded thumbnail and scales it preserving aspect ratio to fit within `size` x `size`.
pub fn scale_embedded_thumbnail(data: &[u8], size: i32) -> Result<Vec<u8>, String> {
    if let Some(dim) = sniff_jpeg_dimensions(data)
        .or_else(|| sniff_png_dimensions(data))
        .or_else(|| sniff_gif_dimensions(data))
        .or_else(|| sniff_tiff_dimensions(data))
    {
        if exceeds_decoded_frame_budget(dim.width, dim.height) {
            return Err("Embedded thumbnail exceeds the decoded frame budget".to_owned());
        }
    }

    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(data).map_err(|error| error.to_string())?;
    loader.close().map_err(|error| error.to_string())?;
    let pixbuf = loader
        .pixbuf()
        .ok_or_else(|| "Unable to decode embedded thumbnail".to_owned())?;

    let width = pixbuf.width().max(1);
    let height = pixbuf.height().max(1);
    let scale = (f64::from(size) / f64::from(width))
        .min(f64::from(size) / f64::from(height))
        .min(1.0);
    let target_width = (f64::from(width) * scale).round().max(1.0) as i32;
    let target_height = (f64::from(height) * scale).round().max(1.0) as i32;

    let scaled = if target_width == width && target_height == height {
        pixbuf
    } else {
        pixbuf
            .scale_simple(target_width, target_height, gdk_pixbuf::InterpType::Bilinear)
            .ok_or_else(|| "Unable to scale embedded thumbnail".to_owned())?
    };

    scaled
        .save_to_bufferv("png", &[("compression", "1")])
        .map_err(|error| error.to_string())
}
