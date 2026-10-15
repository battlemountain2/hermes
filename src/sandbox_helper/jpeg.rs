use std::{fs::File, io::Read, path::Path};

use gdk_pixbuf::{Pixbuf, PixbufLoader, PixbufRotation, prelude::*};

use super::sniff;

pub(super) fn external_preview_safe(path: &Path) -> bool {
    let Ok(input) = File::open(path) else {
        return false;
    };
    let mut prefix = Vec::new();
    input.take(128 * 1024).read_to_end(&mut prefix).is_ok()
        && prefix.starts_with(&[0xff, 0xd8])
        && native_frame_supported(&prefix)
        && sniff::sniff_jpeg_dimensions(&prefix).is_some_and(|dimensions| {
            dimensions.width > 0
                && dimensions.height > 0
                && u64::from(dimensions.width) * u64::from(dimensions.height)
                    <= sniff::MAX_PIXELS_CEILING
        })
}

pub(super) fn scaled_preview(path: &Path, size: i32) -> Option<Vec<u8>> {
    if !sniff::is_jpeg(path) {
        return None;
    }
    let mut input = File::open(path).ok()?;
    if input.metadata().ok()?.len() > 128 * 1024 * 1024 {
        return None;
    }
    let mut prefix = Vec::new();
    (&mut input)
        .take(128 * 1024)
        .read_to_end(&mut prefix)
        .ok()?;
    if !native_frame_supported(&prefix) {
        return None;
    }
    let dimensions = sniff::sniff_jpeg_dimensions(&prefix)?;
    if sniff::exceeds_decoded_frame_budget(dimensions.width, dimensions.height) {
        return None;
    }
    let scale =
        (f64::from(size.max(1)) / f64::from(dimensions.width.max(dimensions.height))).min(1.0);
    let target = (
        (f64::from(dimensions.width) * scale).round().max(1.0) as i32,
        (f64::from(dimensions.height) * scale).round().max(1.0) as i32,
    );
    let primary_orientation = match exif(&prefix) {
        Some(tiff) => {
            let primary = directory(tiff, number(tiff, 4, 4)?)?;
            orientation(tiff, primary.0, primary.1)?
        }
        None => 1,
    };
    let loader = PixbufLoader::with_type("jpeg").ok()?;
    loader.set_size(target.0, target.1);
    loader.write(&prefix).ok()?;
    let mut chunk = [0; 64 * 1024];
    loop {
        let count = input.read(&mut chunk).ok()?;
        if count == 0 {
            break;
        }
        loader.write(&chunk[..count]).ok()?;
    }
    loader.close().ok()?;
    let pixels = loader.pixbuf()?;
    let primary_orientation = match pixels.option("orientation") {
        Some(value) => {
            let orientation = value.parse::<u16>().ok()?;
            if !(1..=8).contains(&orientation) {
                return None;
            }
            orientation
        }
        None => primary_orientation,
    };
    let pixels = pixels.apply_embedded_orientation().unwrap_or(pixels);
    let profile = pixels.option("icc-profile");
    let target = if (5..=8).contains(&primary_orientation) {
        (target.1, target.0)
    } else {
        target
    };
    let pixels = if (pixels.width(), pixels.height()) != target {
        pixels.scale_simple(target.0, target.1, gdk_pixbuf::InterpType::Hyper)?
    } else {
        pixels
    };
    let mut options = vec![("compression", "1")];
    if let Some(profile) = profile.as_ref() {
        options.push(("icc-profile", profile.as_str()));
    }
    pixels.save_to_bufferv("png", &options).ok()
}

fn native_frame_supported(data: &[u8]) -> bool {
    let mut offset = 2;
    while offset + 4 <= data.len() {
        if data[offset] != 0xff {
            return false;
        }
        while data.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let Some(&marker) = data.get(offset) else {
            return false;
        };
        offset += 1;
        if marker == 0xda || marker == 0xd9 {
            return false;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }
        let Some(length) = data.get(offset..offset + 2) else {
            return false;
        };
        let length = usize::from(u16::from_be_bytes([length[0], length[1]]));
        if length < 2 {
            return false;
        }
        let Some(segment) = data.get(offset + 2..offset + length) else {
            return false;
        };
        if matches!(marker, 0xc0 | 0xc2) {
            return segment.first() == Some(&8)
                && segment.get(5) == Some(&3)
                && segment.len() == 6 + usize::from(segment[5]) * 3;
        }
        offset += length;
    }
    false
}

pub(super) fn thumbnail(path: &Path, size: i32) -> Option<Vec<u8>> {
    if !sniff::is_jpeg(path) {
        return None;
    }
    let mut data = Vec::new();
    File::open(path)
        .ok()?
        .take(128 * 1024)
        .read_to_end(&mut data)
        .ok()?;
    let dimensions = sniff::sniff_jpeg_dimensions(&data)?;
    let tiff = exif(&data)?;
    let primary = directory(tiff, number(tiff, 4, 4)?)?;
    let thumbnail_directory = directory(tiff, primary.2)?;
    let primary_orientation = orientation(tiff, primary.0, primary.1)?;
    let thumbnail_orientation = orientation(tiff, thumbnail_directory.0, thumbnail_directory.1)?;
    let embedded = sniff::extract_exif_thumbnail_bytes(&data)?;
    let embedded_dimensions = sniff::sniff_jpeg_dimensions(embedded)?;
    if sniff::exceeds_decoded_frame_budget(embedded_dimensions.width, embedded_dimensions.height) {
        return None;
    }
    let (width, height) =
        oriented_dimensions(dimensions.width, dimensions.height, primary_orientation);
    let target = sniff::image_target_dimensions(width, height, size);
    let embedded_size = oriented_dimensions(
        embedded_dimensions.width,
        embedded_dimensions.height,
        thumbnail_orientation,
    );
    if !adequate(embedded_size, target) {
        return None;
    }
    let loader = PixbufLoader::with_type("jpeg").ok()?;
    loader.write(embedded).ok()?;
    loader.close().ok()?;
    let pixels = orient(&loader.pixbuf()?, thumbnail_orientation)?;
    let scaled = pixels.scale_simple(
        target.0 as i32,
        target.1 as i32,
        gdk_pixbuf::InterpType::Bilinear,
    )?;
    let profile = pixels.option("icc-profile");
    let mut options = vec![("compression", "1")];
    if let Some(profile) = profile.as_ref() {
        options.push(("icc-profile", profile.as_str()));
    }
    scaled.save_to_bufferv("png", &options).ok()
}

fn exif(data: &[u8]) -> Option<&[u8]> {
    if !data.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut offset = 2;
    while offset + 4 <= data.len() {
        if data[offset] != 0xff {
            return None;
        }
        while data.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *data.get(offset)?;
        offset += 1;
        if marker == 0xda || marker == 0xd9 {
            return None;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }
        let length = u16::from_be_bytes(data.get(offset..offset + 2)?.try_into().ok()?) as usize;
        if length < 2 {
            return None;
        }
        let segment = data.get(offset + 2..offset.checked_add(length)?)?;
        if marker == 0xe1 && segment.starts_with(b"Exif\0\0") {
            return segment.get(6..);
        }
        offset += length;
    }
    None
}

fn number(tiff: &[u8], offset: usize, width: usize) -> Option<usize> {
    let bytes = tiff.get(offset..offset.checked_add(width)?)?;
    match (tiff.get(..4)?, width) {
        (b"II*\0", 2) => Some(u16::from_le_bytes(bytes.try_into().ok()?) as usize),
        (b"II*\0", 4) => Some(u32::from_le_bytes(bytes.try_into().ok()?) as usize),
        (b"MM\0*", 2) => Some(u16::from_be_bytes(bytes.try_into().ok()?) as usize),
        (b"MM\0*", 4) => Some(u32::from_be_bytes(bytes.try_into().ok()?) as usize),
        _ => None,
    }
}

fn directory(tiff: &[u8], offset: usize) -> Option<(usize, usize, usize)> {
    if offset < 8 {
        return None;
    }
    let count = number(tiff, offset, 2)?;
    if count > 512 {
        return None;
    }
    let start = offset.checked_add(2)?;
    let end = start.checked_add(count.checked_mul(12)?)?;
    tiff.get(start..end)?;
    Some((start, end, number(tiff, end, 4)?))
}

fn orientation(tiff: &[u8], start: usize, end: usize) -> Option<u16> {
    for offset in (start..end).step_by(12) {
        if number(tiff, offset, 2)? == 0x0112 {
            if number(tiff, offset + 2, 2)? != 3 || number(tiff, offset + 4, 4)? != 1 {
                return None;
            }
            let value = number(tiff, offset + 8, 2)? as u16;
            return (1..=8).contains(&value).then_some(value);
        }
    }
    Some(1)
}

fn oriented_dimensions(width: u32, height: u32, orientation: u16) -> (u32, u32) {
    if orientation >= 5 {
        (height, width)
    } else {
        (width, height)
    }
}

fn adequate(dimensions: (u32, u32), target: (u32, u32)) -> bool {
    let horizontal = u64::from(dimensions.0) * u64::from(target.1);
    let vertical = u64::from(dimensions.1) * u64::from(target.0);
    dimensions.0 >= target.0
        && dimensions.1 >= target.1
        && horizontal > 0
        && vertical > 0
        && horizontal.abs_diff(vertical) <= horizontal.max(vertical) / 64
}

fn orient(pixels: &Pixbuf, orientation: u16) -> Option<Pixbuf> {
    match orientation {
        1 => Some(pixels.clone()),
        2 => pixels.flip(true),
        3 => pixels.rotate_simple(PixbufRotation::Upsidedown),
        4 => pixels.flip(false),
        5 => pixels.rotate_simple(PixbufRotation::Clockwise)?.flip(true),
        6 => pixels.rotate_simple(PixbufRotation::Clockwise),
        7 => pixels.rotate_simple(PixbufRotation::Clockwise)?.flip(false),
        8 => pixels.rotate_simple(PixbufRotation::Counterclockwise),
        _ => None,
    }
}

#[cfg(test)]
pub(super) mod tests;
