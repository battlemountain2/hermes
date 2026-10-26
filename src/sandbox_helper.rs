// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
};

use gdk_pixbuf::prelude::*;
use gtk::gio;

pub(crate) mod archive_cover;
pub(crate) mod audio;
pub(crate) mod geotiff;
mod heif;
mod jpeg;
pub(crate) mod model;
pub(crate) mod office;
mod scaled;
pub(crate) mod sniff;
pub(crate) mod metadata;
pub(crate) mod table;
pub(crate) mod waveform;

pub(crate) fn run(arguments: &[String]) -> Result<(), String> {
    if arguments
        .first()
        .is_some_and(|op| op == "browser-worker" || op == "--browser-worker")
    {
        return crate::sandbox::browser::run();
    }
    let [operation, input, output, value] = arguments else {
        return Err("Invalid preview helper arguments".to_owned());
    };
    let input = Path::new(input);
    let output = Path::new(output);
    let value = value
        .parse::<i32>()
        .map_err(|_| "Invalid preview helper size or page".to_owned())?;

    if operation == "image-metadata" || operation == "media-metadata" {
        let data = if operation == "image-metadata" { metadata::image(input)? } else { metadata::media(input)? };
        fs::write(output, data).map_err(|error| error.to_string())?;
        return Ok(());
    }

    if operation == "extract-pdf-text" {
        let text = extract_pdf_text(input, value.max(1) as usize)?;
        fs::write(output, text).map_err(|error| error.to_string())?;
        return Ok(());
    }
    if operation == "preview-audio-metadata" {
        let metadata = audio::read_audio_metadata(input)?;
        fs::write(output, metadata).map_err(|error| error.to_string())?;
        return Ok(());
    }
    if operation == "music-tags" {
        let tags = audio::read_music_tags(input)?;
        fs::write(output, tags).map_err(|error| error.to_string())?;
        return Ok(());
    }
    if operation == "music-waveform" {
        let peaks = waveform::analyze(input)?;
        let json = serde_json::to_vec(&peaks).map_err(|error| error.to_string())?;
        fs::write(output, json).map_err(|error| error.to_string())?;
        return Ok(());
    }
    if operation == "extract-spreadsheet-text" {
        let text = table::extract_spreadsheet_text(input, value.max(1) as usize)?;
        fs::write(output, text).map_err(|error| error.to_string())?;
        return Ok(());
    }
    if operation == "preview-archive" {
        let listing = archive_listing(input, value.max(1) as usize)?;
        fs::write(output, listing).map_err(|error| error.to_string())?;
        return Ok(());
    }
    if matches!(operation.as_str(), "preview-office" | "extract-office-text") {
        let text = extract_office_text(input, value.max(1) as usize)?;
        fs::write(output, text).map_err(|error| error.to_string())?;
        return Ok(());
    }
    if operation == "convert-office-pdf" {
        fs::write(output, office::render::convert(input)?).map_err(|error| error.to_string())?;
        return Ok(());
    }

    let (png, metadata) = match operation.as_str() {
        "thumbnail-image" => (render_image(input, value.clamp(16, 256))?, None),
        "thumbnail-heif" => (render_heif_thumbnail(input, value.clamp(16, 256))?, None),
        "thumbnail-raw" => (render_raw(input, value.clamp(16, 256))?, None),
        "thumbnail-pdf" => (render_pdf_thumbnail(input, value.clamp(16, 256))?, None),
        "thumbnail-office" => (office::render::thumbnail(input, value.clamp(16, 256))?, None),
        "office-poster" => (office::render::poster(input, value.clamp(16, 1400))?, None),
        "thumbnail-video" => (render_media(input, value.clamp(16, 256))?, None),
        "preview-audio-artwork" => (render_media(input, 1024)?, None),
        "preview-image" => (render_image(input, 1400)?, None),
        "preview-heif" => (render_heif_preview(input)?, None),
        "preview-pdf" => {
            let (png, page, pages, text_layer) = render_pdf_page(input, value)?;
            if let Some(text_bytes) = text_layer {
                let _ = fs::write(output.with_file_name("result.text"), text_bytes);
            }
            (png, Some(format!("{page} {pages}")))
        }
        "preview-media" => {
            render_media_preview(input, output)?;
            return Ok(());
        }
        "preview-animation" => {
            render_animation_preview(input, output)?;
            return Ok(());
        }
        "preview-audio" => {
            render_audio_preview(input, output)?;
            return Ok(());
        }
        "preview-geotiff" => {
            let (png, meta_json) = geotiff::render_geotiff(input, value.max(1))?;
            (png, Some(meta_json))
        }
        "preview-model" => (
            model::render_model(input, value.clamp(16, 800) as u32)?,
            None,
        ),
        "preview-archive-cover" => (
            archive_cover::render_cover(input, value.clamp(16, 1400))?,
            None,
        ),
        "preview-spreadsheet" => {
            let (png, json) = table::render_spreadsheet(input)?;
            (png, Some(json))
        }
        "preview-audio-waveform" => {
            let (png, meta_json) = audio::render_waveform_and_meta(input)?;
            (png, Some(meta_json))
        }
        _ => return Err("Unknown preview helper operation".to_owned()),
    };
    fs::write(output, png).map_err(|error| error.to_string())?;
    if let Some(metadata) = metadata {
        fs::write(output.with_file_name("result.meta"), metadata)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn render_pixbuf(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true)
        .map_err(|error| error.to_string())?
        .save_to_bufferv("png", &[])
        .map_err(|error| error.to_string())
}

pub(crate) fn render_image(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    if size <= 256
        && let Some(png) = jpeg::thumbnail(path, size)
    {
        return Ok(png);
    }
    if let Some((width, height)) = sniff::image_dimensions(path) {
        if sniff::exceeds_decoded_frame_budget(width, height) {
            if u64::from(width) * u64::from(height) <= sniff::MAX_PIXELS_CEILING
                && sniff::is_jpeg(path)
            {
                if size > 256
                    && jpeg::external_preview_safe(path)
                    && let Ok(png) = scaled::render(path, size)
                {
                    return Ok(png);
                }
                return render_imagemagick(path, size);
            }
            if !sniff::is_jpeg(path)
                && let Some(png) = sniff::read_exif_thumbnail(path, size)
            {
                return Ok(png);
            }
            return Err("Image dimensions exceed the decoded frame budget".to_owned());
        }
    }
    if size > 256
        && let Some(png) = jpeg::scaled_preview(path, size)
    {
        return Ok(png);
    }
    // Prefer the bounded external decoder while glycin can inherit Strata's file-size limit and
    // receive SIGXFSZ when allocating shared memory for large decoded JPEG pixel buffers.
    render_imagemagick(path, size).or_else(|_| render_pixbuf(path, size))
}

pub(crate) fn render_raw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    if let Some((width, height)) = sniff::image_dimensions(path) {
        if sniff::exceeds_decoded_frame_budget(width, height) {
            if let Some(png) = sniff::read_exif_thumbnail(path, size) {
                return Ok(png);
            }
            return Err("Image dimensions exceed the decoded frame budget".to_owned());
        }
    }
    render_image(path, size).or_else(|_| render_dcraw(path, size))
}

fn render_imagemagick(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let (width, height) = sniff::image_dimensions(path)
        .map(|(width, height)| sniff::image_target_dimensions(width, height, size))
        .unwrap_or((size.max(1) as u32, size.max(1) as u32));
    for executable in ["magick", "convert"] {
        let input = std::fs::File::open(path).map_err(|error| error.to_string())?;
        let mut command = Command::new(executable);
        if sniff::is_jpeg(path) {
            command
                .args([
                    "-limit", "memory", "96MiB", "-limit", "map", "0", "-limit", "disk", "0",
                ])
                .arg("-define")
                .arg(format!("jpeg:size={width}x{height}"));
            command.args(["-define", "png:compression-strategy=2"]);
        }
        let output = command
            .arg("-")
            .stdin(std::process::Stdio::from(input))
            .args(["-auto-orient", "-thumbnail"])
            .arg(format!("{size}x{size}"))
            .args(["-define", "png:compression-level=1"])
            .arg("png:-")
            .output();
        if let Ok(output) = output
            && output.status.success()
            && !output.stdout.is_empty()
        {
            return Ok(output.stdout);
        }
    }
    Err("ImageMagick could not render this image".to_owned())
}

pub(crate) fn render_heif_thumbnail(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    heif::thumbnail(path, size).or_else(|_| render_imagemagick(path, size))
}

pub(crate) fn render_heif_preview(path: &Path) -> Result<Vec<u8>, String> {
    heif::preview(path, 1400).or_else(|_| {
        if heif::external_preview_safe(path, 1400)
            && let Ok(png) = scaled::render(path, 1400)
        {
            return Ok(png);
        }
        render_imagemagick(path, 1400)
    })
}

fn render_dcraw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    for executable in ["dcraw_emu", "dcraw"] {
        let output = Command::new(executable)
            .args(["-e", "-c"])
            .arg(path)
            .output();
        let Ok(output) = output else {
            continue;
        };
        if !output.status.success() || output.stdout.is_empty() {
            continue;
        }
        let loader = gdk_pixbuf::PixbufLoader::new();
        if loader.write(&output.stdout).is_err() || loader.close().is_err() {
            continue;
        }
        let Some(pixbuf) = loader.pixbuf() else {
            continue;
        };
        let width = pixbuf.width().max(1);
        let height = pixbuf.height().max(1);
        let scale = (f64::from(size) / f64::from(width))
            .min(f64::from(size) / f64::from(height))
            .min(1.0);
        let Some(scaled) = pixbuf.scale_simple(
            (f64::from(width) * scale).round().max(1.0) as i32,
            (f64::from(height) * scale).round().max(1.0) as i32,
            gdk_pixbuf::InterpType::Bilinear,
        ) else {
            continue;
        };
        if let Ok(png) = scaled.save_to_bufferv("png", &[]) {
            return Ok(png);
        }
    }
    Err("No embedded RAW thumbnail could be decoded".to_owned())
}

pub(crate) fn render_pdf_thumbnail(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let uri = gio::File::for_path(path).uri();
    let document = poppler::Document::from_file(&uri, None).map_err(|error| error.to_string())?;
    let page = document
        .page(0)
        .ok_or_else(|| "This PDF has no pages".to_owned())?;
    render_pdf_surface(
        &page,
        f64::from(size),
        f64::from(size),
        f64::from(size * size),
    )
}

pub(crate) fn render_pdf_page(
    path: &Path,
    requested_page: i32,
) -> Result<(Vec<u8>, i32, i32, Option<Vec<u8>>), String> {
    let uri = gio::File::for_path(path).uri();
    let document = poppler::Document::from_file(&uri, None).map_err(|error| error.to_string())?;
    let pages = document.n_pages();
    if pages <= 0 {
        return Err("This PDF has no pages".to_owned());
    }
    let page_index = requested_page.clamp(0, pages - 1);
    let page = document
        .page(page_index)
        .ok_or_else(|| "Unable to load that PDF page".to_owned())?;
    let (page_width, page_height) = page.size();
    if page_width <= 0.0 || page_height <= 0.0 {
        return Err("The PDF page has invalid dimensions".to_owned());
    }
    let scale = (1400.0 / page_width)
        .min(1800.0 / page_height)
        .min((2_500_000.0 / (page_width * page_height)).sqrt());
    let width = (page_width * scale).ceil().max(1.0) as i32;
    let height = (page_height * scale).ceil().max(1.0) as i32;

    let png = render_pdf_surface(&page, 1400.0, 1800.0, 2_500_000.0)?;
    let text_layer = pdf_text_layer(&page, width, height, scale);
    Ok((png, page_index, pages, text_layer))
}

#[expect(
    unsafe_code,
    reason = "poppler-rs exposes no safe binding for poppler_page_get_text_layout"
)]
fn pdf_text_layer(page: &poppler::Page, width: i32, height: i32, scale: f64) -> Option<Vec<u8>> {
    use gtk::glib::translate::ToGlibPtr;

    let text = page.text()?;
    if text.is_empty() {
        return None;
    }
    let mut rects = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: page is a valid PopplerPage; rects/count are valid out-pointers.
    let ok = unsafe {
        poppler::ffi::poppler_page_get_text_layout(page.to_glib_none().0, &mut rects, &mut count)
    };
    if ok == gtk::glib::ffi::GFALSE || rects.is_null() {
        return None;
    }
    // SAFETY: on success poppler returned a g_malloc'd array of count rectangles.
    let layout = unsafe { std::slice::from_raw_parts(rects, count as usize) };
    let glyphs: Vec<[f32; 4]> = layout
        .iter()
        .map(|rect| {
            [
                (rect.x1 * scale) as f32,
                (rect.y1 * scale) as f32,
                (rect.x2 * scale) as f32,
                (rect.y2 * scale) as f32,
            ]
        })
        .collect();
    // SAFETY: rects came from g_malloc and is freed exactly once here.
    unsafe { gtk::glib::ffi::g_free(rects.cast()) };
    if glyphs.len() != text.chars().count() {
        return None;
    }
    let layer = crate::services::preview::PdfTextLayer {
        width: width as f32,
        height: height as f32,
        text: text.to_string(),
        glyphs,
    };
    serde_json::to_vec(&layer)
        .ok()
        .filter(|bytes| bytes.len() as u64 <= crate::sandbox::MAX_TEXT_LAYER_BYTES)
}

fn extract_pdf_text(path: &Path, byte_limit: usize) -> Result<Vec<u8>, String> {
    let uri = gio::File::for_path(path).uri();
    let document = poppler::Document::from_file(&uri, None).map_err(|error| error.to_string())?;
    let mut output = String::new();
    for page_index in 0..document.n_pages() {
        let Some(page) = document.page(page_index) else {
            continue;
        };
        let Some(text) = page.text() else {
            continue;
        };
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&text);
        if output.len() >= byte_limit {
            break;
        }
    }
    if output.len() > byte_limit {
        let mut end = byte_limit;
        while !output.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        output.truncate(end);
    }
    Ok(output.into_bytes())
}

fn archive_listing(path: &Path, byte_limit: usize) -> Result<Vec<u8>, String> {
    let output = bounded_command_output(Command::new("bsdtar").arg("-tf").arg(path), byte_limit)?;
    let listing = String::from_utf8_lossy(&output);
    let mut lines = listing.lines().take(500).collect::<Vec<_>>().join("\n");
    if listing.lines().count() > 500 {
        lines.push_str("\n…additional entries omitted");
    }
    Ok(lines.into_bytes())
}

fn extract_office_text(path: &Path, byte_limit: usize) -> Result<Vec<u8>, String> {
    office::extract_text(path, byte_limit)
}

fn bounded_command_output(command: &mut Command, byte_limit: usize) -> Result<Vec<u8>, String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut output = Vec::with_capacity(byte_limit.min(1024 * 1024));
    let Some(stdout) = child.stdout.take() else {
        return Err("The document tool produced no output".to_owned());
    };
    stdout
        .take(byte_limit.saturating_add(1) as u64)
        .read_to_end(&mut output)
        .map_err(|error| error.to_string())?;
    if output.len() > byte_limit {
        output.truncate(byte_limit);
        let _killed = child.kill();
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() && output.is_empty() {
        return Err("The document tool could not read this file".to_owned());
    }
    Ok(output)
}

fn render_pdf_surface(
    page: &poppler::Page,
    max_width: f64,
    max_height: f64,
    max_pixels: f64,
) -> Result<Vec<u8>, String> {
    let (page_width, page_height) = page.size();
    if page_width <= 0.0 || page_height <= 0.0 {
        return Err("The PDF page has invalid dimensions".to_owned());
    }
    let scale = (max_width / page_width)
        .min(max_height / page_height)
        .min((max_pixels / (page_width * page_height)).sqrt());
    let width = (page_width * scale).ceil().max(1.0) as i32;
    let height = (page_height * scale).ceil().max(1.0) as i32;
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
        .map_err(|error| error.to_string())?;
    let context = cairo::Context::new(&surface).map_err(|error| error.to_string())?;
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.paint().map_err(|error| error.to_string())?;
    context.scale(scale, scale);
    page.render(&context);
    surface.flush();
    let mut png = Vec::new();
    surface
        .write_to_png(&mut png)
        .map_err(|error| error.to_string())?;
    Ok(png)
}

fn render_media_preview(path: &Path, output: &Path) -> Result<(), String> {
    let mut status = render_media_preview_attempt(path, output, false)?;
    if media_preview_should_retry(status) {
        status = render_media_preview_attempt(path, output, true)?;
    }
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("Unable to normalize media preview: ffmpeg {status}"))
}

fn media_preview_should_retry(status: std::process::ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    matches!(status.signal(), Some(9 | 24))
}

fn render_media_preview_attempt(
    path: &Path,
    output: &Path,
    compact: bool,
) -> Result<std::process::ExitStatus, String> {
    let duration = if compact {
        compact_media_duration(path)
    } else {
        "30"
    };
    let filter = if compact {
        "fps=24,scale=w='min(640,iw)':h='min(480,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2"
    } else {
        "fps=30,scale=w='min(960,iw)':h='min(720,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2"
    };
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-threads",
            "2",
            "-filter_threads",
            "2",
            "-i",
        ])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-sn",
            "-dn",
            "-t",
            duration,
            "-vf",
            filter,
            "-c:v",
            "libvpx",
            "-threads",
            "2",
            "-deadline",
            "realtime",
            "-cpu-used",
            "8",
            "-b:v",
            if compact { "1M" } else { "2M" },
            "-maxrate",
            if compact { "2M" } else { "3M" },
            "-bufsize",
            if compact { "2M" } else { "4M" },
            "-c:a",
            "libopus",
            "-b:a",
            "96k",
            "-f",
            "webm",
            "-y",
        ])
        .arg(output)
        .status()
        .map_err(|error| error.to_string())?;
    Ok(status)
}

fn render_audio_preview(path: &Path, output: &Path) -> Result<(), String> {
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-threads",
            "2",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=640x360:r=1",
            "-i",
        ])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-sn",
            "-dn",
            "-t",
            "30",
            "-c:v",
            "libtheora",
            "-pix_fmt",
            "yuv420p",
            "-r",
            "1",
            "-c:a",
            "libvorbis",
            "-q:a",
            "4",
            "-f",
            "ogg",
            "-y",
        ])
        .arg(output)
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Unable to normalize audio preview".to_owned())
}

fn compact_media_duration(path: &Path) -> &'static str {
    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-threads",
            "2",
            "-probesize",
            "65536",
            "-analyzeduration",
            "0",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,width,height,r_frame_rate",
            "-of",
            "json",
        ])
        .arg(path)
        .output();
    probe
        .ok()
        .filter(|output| output.status.success() && output.stdout.len() <= 65536)
        .map(|output| compact_duration_from_probe(&output.stdout))
        .unwrap_or("30")
}

fn compact_duration_from_probe(data: &[u8]) -> &'static str {
    let Ok(probe) = serde_json::from_slice::<serde_json::Value>(data) else {
        return "30";
    };
    let Some(stream) = probe.get("streams").and_then(|streams| streams.get(0)) else {
        return "30";
    };
    let pixels = stream
        .get("width")
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
        .saturating_mul(
            stream
                .get("height")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
        );
    let frame_rate = stream
        .get("r_frame_rate")
        .and_then(|value| value.as_str())
        .and_then(|rate| rate.split_once('/'))
        .and_then(|(numerator, denominator)| {
            let numerator = numerator.parse::<f64>().ok()?;
            let denominator = denominator.parse::<f64>().ok()?;
            (denominator > 0.0).then_some(numerator / denominator)
        })
        .unwrap_or(0.0);
    if stream.get("codec_name").and_then(|value| value.as_str()) == Some("hevc")
        && frame_rate.is_finite()
        && pixels as f64 * frame_rate > 60_000_000.0
    {
        "10"
    } else {
        "30"
    }
}

fn render_animation_preview(path: &Path, output: &Path) -> Result<(), String> {
    let status = Command::new("ffmpeg").args(["-nostdin", "-v", "error", "-threads", "2",
        "-filter_threads", "2", "-ignore_loop", "1", "-i"])
        .arg(path).args(["-map", "0:v:0", "-an", "-sn", "-dn", "-t", "30",
            "-vf", "fps=30,scale=w='min(960,iw)':h='min(720,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2,format=yuva420p",
            "-c:v", "libvpx", "-threads", "2", "-deadline", "realtime", "-cpu-used", "8",
            "-auto-alt-ref", "0", "-b:v", "2M", "-f", "webm", "-y"])
        .arg(output).status().map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("Unable to normalize animation preview: ffmpeg {status}"))
}

pub(crate) fn render_media(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let input = fs::File::open(path).map_err(|error| error.to_string())?;
    let output = Command::new("ffmpegthumbnailer")
        .arg("-i")
        .arg("/proc/self/fd/0")
        .stdin(Stdio::from(input))
        .args(["-o", "/dev/stdout", "-s"])
        .arg(size.to_string())
        .args(["-q", "8"])
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() && !output.stdout.is_empty() {
        Ok(output.stdout)
    } else {
        Err("Unable to render media thumbnail".to_owned())
    }
}

#[cfg(test)]
mod tests;
