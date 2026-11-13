use gdk_pixbuf::{InterpType, PixbufLoader, prelude::*};

pub(super) type Palette = ((f64, f64, f64), (f64, f64, f64));

pub(super) const BACKDROP_OPACITY: f32 = 0.12;

fn linear(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub(super) fn background(palette: Palette, theme: (f64, f64, f64)) -> (f64, f64, f64) {
    let luminance = |color: [f64; 3]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
    let source = [palette.0.0, palette.0.1, palette.0.2].map(linear);
    let source_light = luminance(source);
    let target_light = luminance([theme.0, theme.1, theme.2].map(linear));
    let channels = source.map(|channel| {
        let adjusted = if source_light > target_light {
            channel * target_light / source_light
        } else if source_light < 1.0 {
            channel + (1.0 - channel) * (target_light - source_light) / (1.0 - source_light)
        } else {
            channel
        };
        if adjusted <= 0.0031308 {
            adjusted * 12.92
        } else {
            1.055 * adjusted.powf(1.0 / 2.4) - 0.055
        }
    });
    (channels[0], channels[1], channels[2])
}

pub(super) fn legible_palette(
    palette: Palette,
    background: (f64, f64, f64),
    foreground: (f64, f64, f64),
) -> Palette {
    let blend = |color: (f64, f64, f64), target: (f64, f64, f64), amount: f64| {
        (
            color.0 + (target.0 - color.0) * amount,
            color.1 + (target.1 - color.1) * amount,
            color.2 + (target.2 - color.2) * amount,
        )
    };
    let darkest = blend(background, (0.0, 0.0, 0.0), f64::from(BACKDROP_OPACITY));
    let lightest = blend(background, (1.0, 1.0, 1.0), f64::from(BACKDROP_OPACITY));
    let visible = |color| {
        (0..=20)
            .map(|step| blend(color, foreground, f64::from(step) / 20.0))
            .find(|candidate| {
                contrast(*candidate, darkest) >= 3.0 && contrast(*candidate, lightest) >= 3.0
            })
            .unwrap_or(foreground)
    };
    (visible(palette.0), visible(palette.1))
}

fn contrast(first: (f64, f64, f64), second: (f64, f64, f64)) -> f64 {
    let luminance = |color: (f64, f64, f64)| {
        linear(color.0) * 0.2126 + linear(color.1) * 0.7152 + linear(color.2) * 0.0722
    };
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

pub(super) fn presentation(bytes: &[u8]) -> Option<(Vec<u8>, Palette)> {
    if bytes.len() < 24
        || bytes.len() > 16 * 1024 * 1024
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || &bytes[12..16] != b"IHDR"
    {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if width == 0 || height == 0 || width > 1400 || height > 1400 {
        return None;
    }
    let loader = PixbufLoader::with_type("png").ok()?;
    loader.write(bytes).ok()?;
    loader.close().ok()?;
    let pixels = loader.pixbuf()?;
    if pixels.width() > 1400 || pixels.height() > 1400 {
        return None;
    }
    let small = pixels.scale_simple(32, 32, InterpType::Bilinear)?;
    let data = small.read_pixel_bytes();
    let stride = small.rowstride() as usize;
    let channels = small.n_channels() as usize;
    let palette = palette(data.as_ref(), stride, channels)?;
    let blurred = small
        .scale_simple(8, 8, InterpType::Bilinear)?
        .scale_simple(64, 64, InterpType::Bilinear)?
        .save_to_bufferv("png", &[])
        .ok()?;
    Some((blurred, palette))
}

fn palette(bytes: &[u8], stride: usize, channels: usize) -> Option<Palette> {
    if !(3..=4).contains(&channels)
        || stride < 32 * channels
        || bytes.len() < 31 * stride + 32 * channels
    {
        return None;
    }
    let mut bins = [(0.0_f64, [0.0_f64; 3]); 512];
    for row in 0..32 {
        for column in 0..32 {
            let offset = row * stride + column * channels;
            let color = [bytes[offset], bytes[offset + 1], bytes[offset + 2]].map(f64::from);
            let high = color.iter().copied().fold(0.0, f64::max);
            let low = color.iter().copied().fold(255.0, f64::min);
            let alpha = if channels == 4 {
                f64::from(bytes[offset + 3]) / 255.0
            } else {
                1.0
            };
            let weight = (0.2 + (high - low) / 255.0) * alpha;
            let index = (bytes[offset] as usize / 32) * 64
                + (bytes[offset + 1] as usize / 32) * 8
                + bytes[offset + 2] as usize / 32;
            bins[index].0 += weight;
            for (total, value) in bins[index].1.iter_mut().zip(color) {
                *total += value * weight;
            }
        }
    }
    bins.sort_by(|first, second| second.0.total_cmp(&first.0));
    let color = |bin: (f64, [f64; 3])| {
        let values = bin.1.map(|value| (value / bin.0 / 255.0).clamp(0.0, 1.0));
        (values[0], values[1], values[2])
    };
    if bins[0].0 <= 0.0 {
        return None;
    }
    let first = color(bins[0]);
    let second = bins
        .iter()
        .copied()
        .skip(1)
        .find(|bin| bin.0 > bins[0].0 * 0.03)
        .map(color)
        .unwrap_or(first);
    Some((first, second))
}

#[cfg(test)]
mod tests;
