use std::{fs::File, path::Path};

use gdk_pixbuf::{Colorspace, InterpType, Pixbuf, glib};
use libheif_rs::{ColorSpace, HeifContext, ImageHandle, LibHeif, RgbChroma};

const MAX_THUMBNAIL_IDS: usize = 32;

pub(super) fn thumbnail(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let decoder = LibHeif::new_checked().map_err(|error| error.to_string())?;
    let file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    let reader = libheif_rs::StreamReader::new(file, length);
    let mut context =
        HeifContext::read_from_reader(Box::new(reader)).map_err(|error| error.to_string())?;
    context.set_max_decoding_threads(1);
    let primary = context
        .primary_image_handle()
        .map_err(|error| error.to_string())?;
    let target = super::sniff::image_target_dimensions(primary.width(), primary.height(), size);
    let mut identifiers = [0; MAX_THUMBNAIL_IDS];
    let count = primary
        .thumbnail_ids(&mut identifiers)
        .min(MAX_THUMBNAIL_IDS);
    let mut handles: Vec<_> = identifiers[..count]
        .iter()
        .filter_map(|identifier| primary.thumbnail(*identifier).ok())
        .filter(|handle| adequate_thumbnail(handle.width(), handle.height(), target))
        .collect();
    handles.sort_by_key(|handle| u64::from(handle.width()) * u64::from(handle.height()));
    for handle in &handles {
        if let Ok(png) = decode(&decoder, handle, size) {
            return Ok(png);
        }
    }
    decode(&decoder, &primary, size)
}

fn adequate_thumbnail(width: u32, height: u32, target: (u32, u32)) -> bool {
    width >= target.0
        && height >= target.1
        && bounded_dimensions(width, height)
        && (u64::from(width) * u64::from(target.1))
            .abs_diff(u64::from(height) * u64::from(target.0))
            <= 2 * u64::from(width.max(height))
}

fn bounded_dimensions(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && !super::sniff::exceeds_decoded_frame_budget(width, height)
}

fn decode(decoder: &LibHeif, handle: &ImageHandle, size: i32) -> Result<Vec<u8>, String> {
    if !bounded_dimensions(handle.width(), handle.height()) {
        return Err("HEIF dimensions exceed the decoded thumbnail budget".into());
    }
    let image = decoder
        .decode(handle, ColorSpace::Rgb(RgbChroma::Rgba), None)
        .map_err(|error| error.to_string())?;
    let planes = image.planes();
    let plane = planes
        .interleaved
        .ok_or("HEIF decoder returned no RGBA pixels")?;
    if !bounded_dimensions(plane.width, plane.height)
        || plane.storage_bits_per_pixel != 32
        || plane.stride < plane.width as usize * 4
        || plane.stride > i32::MAX as usize
        || plane.data.len() < plane.stride.saturating_mul(plane.height as usize)
    {
        return Err("HEIF decoder returned invalid RGBA pixels".into());
    }
    let pixels = Pixbuf::from_bytes(
        &glib::Bytes::from(plane.data),
        Colorspace::Rgb,
        true,
        8,
        plane.width as i32,
        plane.height as i32,
        plane.stride as i32,
    );
    let (width, height) = super::sniff::image_target_dimensions(plane.width, plane.height, size);
    let scaled = pixels
        .scale_simple(width as i32, height as i32, InterpType::Bilinear)
        .ok_or("Could not resize HEIF thumbnail")?;
    if let Some(profile) = image.color_profile_raw() {
        scaled.save_to_bufferv(
            "png",
            &[
                ("compression", "1"),
                ("icc-profile", &glib::base64_encode(&profile.data)),
            ],
        )
    } else {
        scaled.save_to_bufferv("png", &[("compression", "1")])
    }
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests;
