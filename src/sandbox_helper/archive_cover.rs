// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use gdk_pixbuf::prelude::*;
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

const MAX_ARCHIVE_ENTRIES: usize = 4096;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024; // 8 MiB
const MAX_XML_BYTES: u64 = 1024 * 1024; // 1 MiB
const MAX_CONTAINER_BYTES: u64 = 256 * 1024; // 256 KiB
const MAX_PIXELS_CEILING: u64 = 16_777_216; // 16 MP

pub(crate) fn is_image_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "gif"
    )
}

pub(crate) fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
    let mut a_chars = a.chars().peekable();
    let mut b_chars = b.chars().peekable();

    while let (Some(&ac), Some(&bc)) = (a_chars.peek(), b_chars.peek()) {
        if ac.is_ascii_digit() && bc.is_ascii_digit() {
            let mut a_num = 0u64;
            while let Some(&c) = a_chars.peek() {
                if c.is_ascii_digit() {
                    a_num = a_num.saturating_mul(10).saturating_add((c as u8 - b'0') as u64);
                    a_chars.next();
                } else {
                    break;
                }
            }
            let mut b_num = 0u64;
            while let Some(&c) = b_chars.peek() {
                if c.is_ascii_digit() {
                    b_num = b_num.saturating_mul(10).saturating_add((c as u8 - b'0') as u64);
                    b_chars.next();
                } else {
                    break;
                }
            }
            match a_num.cmp(&b_num) {
                std::cmp::Ordering::Equal => continue,
                ord => return ord,
            }
        } else {
            let ac_lower = ac.to_ascii_lowercase();
            let bc_lower = bc.to_ascii_lowercase();
            match ac_lower.cmp(&bc_lower) {
                std::cmp::Ordering::Equal => {
                    a_chars.next();
                    b_chars.next();
                }
                ord => return ord,
            }
        }
    }

    a.len().cmp(&b.len())
}

fn attribute(tag: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>, String> {
    for attr in tag.attributes() {
        let attr = attr.map_err(|_| "Invalid XML attribute".to_string())?;
        if attr.key.local_name().as_ref() == name {
            return String::from_utf8(attr.value.into_owned())
                .map(Some)
                .map_err(|_| "Invalid XML attribute encoding".to_string());
        }
    }
    Ok(None)
}

fn normalize_zip_path(base_dir: &str, relative: &str) -> Result<String, String> {
    let unescaped = quick_xml::escape::unescape(relative)
        .map_err(|_| "Invalid relative path escaping".to_string())?;
    let path = if base_dir.is_empty() {
        PathBuf::from(unescaped.as_ref())
    } else {
        PathBuf::from(base_dir).join(unescaped.as_ref())
    };

    let mut components = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::Normal(c) => components.push(c.to_string_lossy().to_string()),
            std::path::Component::ParentDir => {
                if components.pop().is_none() {
                    return Err("Path traversal outside package root".to_string());
                }
            }
            std::path::Component::CurDir => {}
            _ => return Err("Invalid path component".to_string()),
        }
    }
    Ok(components.join("/"))
}

fn extract_cbz_cover(input: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(input).map_err(|e| format!("Unable to open CBZ file: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| "Invalid CBZ archive".to_string())?;

    let mut candidate_names: Vec<String> = Vec::new();
    let entry_count = archive.len().min(MAX_ARCHIVE_ENTRIES);

    for i in 0..entry_count {
        if let Ok(entry) = archive.by_index(i) {
            let name = entry.name().to_string();
            if !entry.is_dir()
                && !name.contains("__MACOSX")
                && !name.starts_with('.')
                && is_image_extension(
                    Path::new(&name)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or_default(),
                )
            {
                candidate_names.push(name);
            }
        }
    }

    if candidate_names.is_empty() {
        return Err("Comic archive has no bounded image".to_string());
    }

    candidate_names.sort_by(|a, b| compare_names(a, b));
    let chosen_name = &candidate_names[0];

    let chosen_entry = archive
        .by_name(chosen_name)
        .map_err(|_| "Failed to read selected image entry".to_string())?;

    let mut image_bytes = Vec::new();
    chosen_entry
        .take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut image_bytes)
        .map_err(|e| format!("Failed to read cover image data: {e}"))?;

    if image_bytes.len() > MAX_IMAGE_BYTES {
        return Err("Cover image exceeds 8MB limit".to_string());
    }

    Ok(image_bytes)
}

fn extract_epub_cover(input: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(input).map_err(|e| format!("Unable to open EPUB file: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| "Invalid EPUB archive".to_string())?;

    // 1. Read META-INF/container.xml
    let container_file = archive
        .by_name("META-INF/container.xml")
        .map_err(|_| "EPUB missing META-INF/container.xml".to_string())?;
    let mut container_xml = Vec::new();
    container_file
        .take(MAX_CONTAINER_BYTES + 1)
        .read_to_end(&mut container_xml)
        .map_err(|e| format!("Failed to read container.xml: {e}"))?;

    let mut reader = Reader::from_reader(container_xml.as_slice());
    let mut opf_path = None;

    loop {
        match reader.read_event().map_err(|_| "Invalid container.xml".to_string())? {
            Event::Start(tag) | Event::Empty(tag) if tag.local_name().as_ref() == b"rootfile" => {
                if let Some(path) = attribute(&tag, b"full-path")? {
                    opf_path = Some(path);
                    break;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    let opf_path = opf_path.ok_or("No rootfile full-path found in container.xml")?;
    let opf_dir = Path::new(&opf_path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("")
        .to_string();

    // 2. Read OPF document
    let opf_file = archive
        .by_name(&opf_path)
        .map_err(|_| format!("EPUB package file not found: {opf_path}"))?;
    let mut opf_xml = Vec::new();
    opf_file
        .take(MAX_XML_BYTES + 1)
        .read_to_end(&mut opf_xml)
        .map_err(|e| format!("Failed to read OPF package: {e}"))?;

    let mut opf_reader = Reader::from_reader(opf_xml.as_slice());
    let mut cover_meta_id = None;
    let mut manifest_items: Vec<(String, String, String)> = Vec::new(); // (id, href, properties)

    loop {
        match opf_reader.read_event().map_err(|_| "Invalid OPF XML".to_string())? {
            Event::Start(tag) | Event::Empty(tag) => match tag.local_name().as_ref() {
                b"meta" => {
                    let name = attribute(&tag, b"name")?.unwrap_or_default();
                    if name.eq_ignore_ascii_case("cover")
                        && let Some(content) = attribute(&tag, b"content")?
                    {
                        cover_meta_id = Some(content);
                    }
                }
                b"item" => {
                    let id = attribute(&tag, b"id")?.unwrap_or_default();
                    let href = attribute(&tag, b"href")?.unwrap_or_default();
                    let properties = attribute(&tag, b"properties")?.unwrap_or_default();
                    if !id.is_empty() && !href.is_empty() {
                        manifest_items.push((id, href, properties));
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }

    // Try EPUB 3 cover-image property first
    let mut cover_href = None;
    for (_id, href, properties) in &manifest_items {
        if properties.split_whitespace().any(|p| p == "cover-image") {
            cover_href = Some(href.clone());
            break;
        }
    }

    // Fall back to EPUB 2 meta name="cover" referencing item id
    if cover_href.is_none()
        && let Some(meta_id) = &cover_meta_id
    {
        for (id, href, _properties) in &manifest_items {
            if id == meta_id {
                cover_href = Some(href.clone());
                break;
            }
        }
    }

    // Fall back to item id="cover" or href containing "cover"
    if cover_href.is_none() {
        for (id, href, _properties) in &manifest_items {
            if id.eq_ignore_ascii_case("cover")
                || href.to_ascii_lowercase().contains("cover")
            {
                let ext = Path::new(href).extension().and_then(|e| e.to_str()).unwrap_or("");
                if is_image_extension(ext) {
                    cover_href = Some(href.clone());
                    break;
                }
            }
        }
    }

    let cover_href = cover_href.ok_or("EPUB archive has no cover image".to_string())?;
    let resolved_path = normalize_zip_path(&opf_dir, &cover_href)?;

    let image_entry = archive
        .by_name(&resolved_path)
        .map_err(|_| format!("EPUB cover image entry missing: {resolved_path}"))?;

    let mut image_bytes = Vec::new();
    image_entry
        .take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut image_bytes)
        .map_err(|e| format!("Failed to read cover image: {e}"))?;

    if image_bytes.len() > MAX_IMAGE_BYTES {
        return Err("Cover image exceeds 8MB limit".to_string());
    }

    Ok(image_bytes)
}

fn extract_cbr_cover(input: &Path) -> Result<Vec<u8>, String> {
    let output = Command::new("bsdtar")
        .arg("-tf")
        .arg(input)
        .output()
        .map_err(|e| format!("Unable to invoke bsdtar for CBR listing: {e}"))?;

    if !output.status.success() {
        return Err("Invalid or corrupted CBR archive".to_string());
    }

    let listing = String::from_utf8_lossy(&output.stdout);
    let mut candidate_entries: Vec<&str> = listing
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.ends_with('/')
                && !line.contains("__MACOSX")
                && is_image_extension(
                    Path::new(line)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or_default(),
                )
        })
        .collect();

    if candidate_entries.is_empty() {
        return Err("CBR archive contains no images".to_string());
    }

    candidate_entries.sort_by(|a, b| compare_names(a, b));
    let chosen_entry = candidate_entries[0];

    let extract_output = Command::new("bsdtar")
        .args(["-xOf"])
        .arg(input)
        .arg(chosen_entry)
        .output()
        .map_err(|e| format!("Unable to invoke bsdtar for CBR extraction: {e}"))?;

    if !extract_output.status.success() || extract_output.stdout.is_empty() {
        return Err("Failed to extract CBR cover image".to_string());
    }

    if extract_output.stdout.len() > MAX_IMAGE_BYTES {
        return Err("CBR cover image exceeds 8MB limit".to_string());
    }

    Ok(extract_output.stdout)
}

pub(crate) fn render_cover(input: &Path, target_size: i32) -> Result<Vec<u8>, String> {
    let size = target_size.clamp(16, 1400);
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    let raw_bytes = match ext.as_str() {
        "epub" => extract_epub_cover(input)?,
        "cbz" => extract_cbz_cover(input)?,
        "cbr" => extract_cbr_cover(input)?,
        _ => return Err("Unsupported archive cover format".to_string()),
    };

    let loader = gdk_pixbuf::PixbufLoader::new();
    loader
        .write(&raw_bytes)
        .map_err(|e| format!("Unable to parse cover image: {e}"))?;
    loader
        .close()
        .map_err(|e| format!("Unable to close pixbuf loader: {e}"))?;

    let pixbuf = loader
        .pixbuf()
        .ok_or_else(|| "Failed to decode cover pixbuf".to_string())?;

    let width = pixbuf.width().max(1);
    let height = pixbuf.height().max(1);

    if u64::from(width as u32) * u64::from(height as u32) > MAX_PIXELS_CEILING {
        return Err("Cover image exceeds 16 MP guard".to_string());
    }

    let scale = (f64::from(size) / f64::from(width))
        .min(f64::from(size) / f64::from(height))
        .min(1.0);

    let scaled_w = (f64::from(width) * scale).round().max(1.0) as i32;
    let scaled_h = (f64::from(height) * scale).round().max(1.0) as i32;

    let scaled = pixbuf
        .scale_simple(scaled_w, scaled_h, gdk_pixbuf::InterpType::Bilinear)
        .ok_or_else(|| "Failed to scale cover image".to_string())?;

    scaled
        .save_to_bufferv("png", &[])
        .map_err(|e| format!("Failed to encode cover PNG: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_names_natural_sort() {
        assert_eq!(compare_names("page1.png", "page2.png"), std::cmp::Ordering::Less);
        assert_eq!(compare_names("page2.png", "page10.png"), std::cmp::Ordering::Less);
        assert_eq!(compare_names("page09.png", "page10.png"), std::cmp::Ordering::Less);
        assert_eq!(compare_names("cover.jpg", "page1.png"), std::cmp::Ordering::Less);
    }

    #[test]
    fn test_is_image_extension() {
        assert!(is_image_extension("png"));
        assert!(is_image_extension("JPG"));
        assert!(is_image_extension("webp"));
        assert!(!is_image_extension("xml"));
        assert!(!is_image_extension("txt"));
    }
}
