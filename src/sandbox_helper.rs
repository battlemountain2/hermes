// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
};

use gdk_pixbuf::prelude::*;
use gtk::gio;

pub(crate) fn run(arguments: &[String]) -> Result<(), String> {
    let [operation, input, output, value] = arguments else {
        return Err("Invalid preview helper arguments".to_owned());
    };
    let input = Path::new(input);
    let output = Path::new(output);
    let value = value
        .parse::<i32>()
        .map_err(|_| "Invalid preview helper size or page".to_owned())?;

    if operation == "extract-pdf-text" {
        let text = extract_pdf_text(input, value.max(1) as usize)?;
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

    let (png, metadata) = match operation.as_str() {
        "thumbnail-image" => (render_image(input, value.clamp(16, 256))?, None),
        "thumbnail-heif" => (render_imagemagick(input, value.clamp(16, 256))?, None),
        "thumbnail-raw" => (render_raw(input, value.clamp(16, 256))?, None),
        "thumbnail-pdf" => (render_pdf_thumbnail(input, value.clamp(16, 256))?, None),
        "thumbnail-video" => (render_media(input, value.clamp(16, 256))?, None),
        "preview-image" => (render_image(input, 1400)?, None),
        "preview-heif" => (render_imagemagick(input, 1400)?, None),
        "preview-pdf" => {
            let (png, page, pages) = render_pdf_page(input, value)?;
            (png, Some(format!("{page} {pages}")))
        }
        "preview-media" => {
            render_media_preview(input, output)?;
            return Ok(());
        }
        "preview-audio" => {
            render_audio_preview(input, output)?;
            return Ok(());
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

fn render_image(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    // Prefer the bounded external decoder while glycin can inherit Strata's file-size limit and
    // receive SIGXFSZ when allocating shared memory for large decoded JPEG pixel buffers.
    render_imagemagick(path, size).or_else(|_| render_pixbuf(path, size))
}

fn render_raw(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    render_image(path, size).or_else(|_| render_dcraw(path, size))
}

fn render_imagemagick(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    for executable in ["magick", "convert"] {
        let output = Command::new(executable)
            .arg(path)
            .args(["-auto-orient", "-thumbnail"])
            .arg(format!("{size}x{size}"))
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

fn render_pdf_thumbnail(path: &Path, size: i32) -> Result<Vec<u8>, String> {
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

fn render_pdf_page(path: &Path, requested_page: i32) -> Result<(Vec<u8>, i32, i32), String> {
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
    let png = render_pdf_surface(&page, 1400.0, 1800.0, 2_500_000.0)?;
    Ok((png, page_index, pages))
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
    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    let member = if extension.eq_ignore_ascii_case("docx") {
        "word/document.xml"
    } else if extension.eq_ignore_ascii_case("odt") {
        "content.xml"
    } else {
        return Err("This office document format is not supported".to_owned());
    };
    let xml = bounded_command_output(
        Command::new("bsdtar").args(["-xOf"]).arg(path).arg(member),
        byte_limit.saturating_mul(4).max(byte_limit),
    )?;
    let xml = String::from_utf8_lossy(&xml);
    Ok(office_xml_text(&xml, byte_limit).into_bytes())
}

fn office_xml_text(xml: &str, byte_limit: usize) -> String {
    let mut text = String::with_capacity(xml.len().min(byte_limit));
    let mut in_tag = false;
    for character in xml.chars() {
        match character {
            '<' => {
                in_tag = true;
                if !text.ends_with(char::is_whitespace) {
                    text.push(' ');
                }
            }
            '>' => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
        if text.len() >= byte_limit {
            break;
        }
    }
    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    let mut text = text.trim().to_owned();
    if text.len() > byte_limit {
        let mut end = byte_limit;
        while !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        text.truncate(end);
    }
    text
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
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-threads", "2", "-i"])
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-sn",
            "-dn",
            "-t",
            "30",
            "-vf",
            "scale=w='min(1280,iw)':h=-2:force_original_aspect_ratio=decrease",
            "-c:v",
            "libvpx",
            "-threads",
            "2",
            "-deadline",
            "realtime",
            "-cpu-used",
            "8",
            "-b:v",
            "2M",
            "-maxrate",
            "3M",
            "-bufsize",
            "4M",
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
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Unable to normalize media preview".to_owned())
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

fn render_media(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let output = Command::new("ffmpegthumbnailer")
        .arg("-i")
        .arg(path)
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
