use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use gdk_pixbuf::prelude::*;

const PROFILE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry">
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop><prop oor:name="DisableMacrosExecution" oor:op="fuse"><value>true</value></prop><prop oor:name="DisableActiveContent" oor:op="fuse"><value>true</value></prop><prop oor:name="BlockUntrustedRefererLinks" oor:op="fuse"><value>true</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Misc"><prop oor:name="UseOpenCL" oor:op="fuse"><value>false</value></prop></item>
<item oor:path="/org.openoffice.Office.Writer/Content/Update"><prop oor:name="Link" oor:op="fuse"><value>2</value></prop></item>
<item oor:path="/org.openoffice.Office.Calc/Content/Update"><prop oor:name="Link" oor:op="fuse"><value>1</value></prop></item>
</oor:items>"#;

pub(crate) fn renderer() -> Option<PathBuf> {
    [
        "/usr/lib/libreoffice/program/soffice",
        "/usr/lib64/libreoffice/program/soffice",
        "/usr/bin/libreoffice",
        "/usr/bin/soffice",
        "/opt/libreoffice/program/soffice",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

pub(crate) fn convert(input: &Path) -> Result<Vec<u8>, String> {
    export(input, false)
}

pub(crate) fn thumbnail(input: &Path, size: i32) -> Result<Vec<u8>, String> {
    if let Ok(png) = poster(input, size) {
        return Ok(png);
    }
    let bytes = export(input, true)?;
    let pdf = Path::new("/tmp/hermes-office-first.pdf");
    fs::write(pdf, bytes).map_err(|error| error.to_string())?;
    if crate::services::classify_by_name(input.as_os_str())
        == crate::services::FormatFamily::Spreadsheet
    {
        return workbook_thumbnail(pdf, size);
    }
    super::super::render_pdf_thumbnail(pdf, size)
}

pub(crate) fn poster(input: &Path, size: i32) -> Result<Vec<u8>, String> {
    let mut package =
        super::Package::open(fs::File::open(input).map_err(|error| error.to_string())?)?;
    for name in [
        "docProps/thumbnail.jpeg",
        "docProps/thumbnail.jpg",
        "docProps/thumbnail.png",
        "Thumbnails/thumbnail.png",
    ] {
        if package.contains(name)
            && let Ok(bytes) = package.read(name)
            && let Ok(png) = embedded_thumbnail(&bytes, size)
        {
            return Ok(png);
        }
    }
    Err("No saved Office preview image is available".into())
}

fn workbook_thumbnail(pdf: &Path, size: i32) -> Result<Vec<u8>, String> {
    use gtk::gio::prelude::*;
    let uri = gtk::gio::File::for_path(pdf).uri();
    let document = poppler::Document::from_file(&uri, None).map_err(|error| error.to_string())?;
    let page = document
        .page(0)
        .ok_or("The workbook has no printable pages")?;
    let mut bounds = poppler::Rectangle::new();
    let (page_width, page_height) = page.size();
    if !page.get_bounding_box(&mut bounds)
        || ![
            bounds.x1(),
            bounds.y1(),
            bounds.x2(),
            bounds.y2(),
            page_width,
            page_height,
        ]
        .into_iter()
        .all(f64::is_finite)
        || page_width <= 0.0
        || page_height <= 0.0
    {
        return super::super::render_pdf_thumbnail(pdf, size);
    }
    let left = (bounds.x1() - 6.0).clamp(0.0, page_width);
    let top = (bounds.y1() - 6.0).clamp(0.0, page_height);
    let width = (bounds.x2() + 6.0).min(page_width) - left;
    let height = (bounds.y2() + 6.0).min(page_height) - top;
    if width <= 0.0 || height <= 0.0 {
        return super::super::render_pdf_thumbnail(pdf, size);
    }
    let scale = f64::from(size) / width.max(height);
    let surface = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        (width * scale).ceil().clamp(1.0, f64::from(size)) as i32,
        (height * scale).ceil().clamp(1.0, f64::from(size)) as i32,
    )
    .map_err(|error| error.to_string())?;
    let context = cairo::Context::new(&surface).map_err(|error| error.to_string())?;
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.paint().map_err(|error| error.to_string())?;
    context.scale(scale, scale);
    context.translate(-left, -top);
    page.render(&context);
    let mut png = Vec::new();
    surface
        .write_to_png(&mut png)
        .map_err(|error| error.to_string())?;
    Ok(png)
}

fn embedded_thumbnail(bytes: &[u8], size: i32) -> Result<Vec<u8>, String> {
    let dimensions = super::super::sniff::sniff_png_dimensions(bytes)
        .or_else(|| super::super::sniff::sniff_jpeg_dimensions(bytes))
        .ok_or("Unsupported Office thumbnail image")?;
    if dimensions.width == 0
        || dimensions.height == 0
        || super::super::sniff::exceeds_decoded_frame_budget(dimensions.width, dimensions.height)
    {
        return Err("Office thumbnail exceeds the image budget".into());
    }
    let (width, height) =
        super::super::sniff::image_target_dimensions(dimensions.width, dimensions.height, size);
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.set_size(width as i32, height as i32);
    loader.write(bytes).map_err(|error| error.to_string())?;
    loader.close().map_err(|error| error.to_string())?;
    loader
        .pixbuf()
        .ok_or("No embedded Office thumbnail was decoded")?
        .save_to_bufferv("png", &[])
        .map_err(|error| error.to_string())
}

fn export_filter(package: &mut super::Package, first_page: bool) -> Result<String, String> {
    let filter = if package.contains("word/document.xml") {
        "writer_pdf_Export"
    } else if package.contains("ppt/presentation.xml") {
        "impress_pdf_Export"
    } else if package.contains("xl/workbook.xml") {
        "calc_pdf_Export"
    } else if package.contains("content.xml") {
        if package.contains("mimetype")
            && package.read("mimetype")? == b"application/vnd.oasis.opendocument.spreadsheet"
        {
            "calc_pdf_Export"
        } else {
            "writer_pdf_Export"
        }
    } else {
        return Err("This Office layout format is not supported".into());
    };
    Ok(pdf_filter_options(filter, first_page))
}

fn pdf_filter_options(filter: &str, first_page: bool) -> String {
    if first_page {
        format!(
            "pdf:{filter}:{}",
            r#"{"PageRange":{"type":"string","value":"1"},"ReduceImageResolution":{"type":"boolean","value":"true"},"MaxImageResolution":{"type":"long","value":"75"}}"#
        )
    } else {
        format!(
            "pdf:{filter}:{}",
            r#"{"ReduceImageResolution":{"type":"boolean","value":"true"},"MaxImageResolution":{"type":"long","value":"150"}}"#
        )
    }
}

fn export(input: &Path, first_page: bool) -> Result<Vec<u8>, String> {
    let renderer = renderer().ok_or("LibreOffice is not installed")?;
    let mut package =
        super::Package::open(fs::File::open(input).map_err(|error| error.to_string())?)?;
    let filter = export_filter(&mut package, first_page)?;
    package.validate_spreadsheet_xml()?;
    let profile = Path::new("/tmp/hermes-office-profile/user");
    fs::create_dir_all(profile).map_err(|error| error.to_string())?;
    fs::write(profile.join("registrymodifications.xcu"), PROFILE)
        .map_err(|error| error.to_string())?;
    fs::create_dir_all("/tmp/hermes-office-pages").map_err(|error| error.to_string())?;
    let status = Command::new(renderer)
        .args([
            "-env:UserInstallation=file:///tmp/hermes-office-profile",
            "--headless",
            "--invisible",
            "--nologo",
            "--nodefault",
            "--norestore",
            "--nofirststartwizard",
            "--convert-to",
            &filter,
            "--outdir",
            "/tmp/hermes-office-pages",
        ])
        .arg(input)
        .env("SAL_USE_VCLPLUGIN", "svp")
        .env("SAL_DISABLE_OPENCL", "1")
        .env("OMP_NUM_THREADS", "2")
        .env("MALLOC_ARENA_MAX", "2")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(|error| format!("Unable to start Office renderer: {error}"))?;
    if !status.success() {
        return Err(format!(
            "LibreOffice could not render this document ({status})"
        ));
    }
    let name = input.file_stem().ok_or("Missing Office input name")?;
    let output = Path::new("/tmp/hermes-office-pages")
        .join(name)
        .with_extension("pdf");
    let metadata =
        fs::symlink_metadata(&output).map_err(|_| "LibreOffice did not produce a page preview")?;
    if !metadata.is_file() || metadata.len() > crate::sandbox::MAX_OUTPUT_BYTES {
        return Err("Office page preview exceeds the output limit".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(output)
        .map_err(|error| error.to_string())?
        .take(crate::sandbox::MAX_OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > crate::sandbox::MAX_OUTPUT_BYTES || !bytes.starts_with(b"%PDF-") {
        return Err("Invalid Office page preview".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
