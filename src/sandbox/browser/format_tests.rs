use super::*;
use gtk::prelude::*;
use std::{fs, io::Write, path::PathBuf};

pub(crate) struct FixtureDirectory(pub(crate) PathBuf);

impl FixtureDirectory {
    pub(crate) fn new() -> Self {
        let name = format!(
            "hermes-pooled-formats-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(name);
        fs::create_dir(&path).expect("private fixture directory");
        Self(path)
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn cover_png(color: u32) -> Vec<u8> {
    let pixels = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 32, 24)
        .expect("fixture pixels");
    pixels.fill(color);
    pixels.save_to_bufferv("png", &[]).expect("fixture PNG")
}

pub(crate) fn write_package(path: &Path, members: &[(&str, &[u8])]) {
    let file = File::create(path).expect("fixture archive");
    let mut archive = zip::ZipWriter::new(file);
    for (name, data) in members {
        archive
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .expect("fixture member");
        archive.write_all(data).expect("fixture member data");
    }
    archive.finish().expect("finished fixture archive");
}

fn pooled_preview(path: &Path, operation: ParseOperation) -> Vec<u8> {
    preview(path, &operation, &Cancellation::default())
        .expect("fixture must use the real pooled path, not one-shot fallback")
        .expect("pooled format render")
        .0
}

pub(super) fn decoded_pixels(png: &[u8]) -> gtk::glib::Bytes {
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.write(png).expect("decoded PNG data");
    loader.close().expect("complete PNG");
    loader.pixbuf().expect("rendered pixels").read_pixel_bytes()
}

pub(super) fn write_pdf(path: &Path) {
    let surface = gtk::cairo::PdfSurface::new(600.0, 600.0, path).expect("fixture PDF");
    let drawing = gtk::cairo::Context::new(&surface).expect("fixture drawing");
    drawing.set_source_rgb(0.1, 0.3, 0.8);
    drawing.paint().expect("fixture color");
    drawing.set_source_rgb(1.0, 1.0, 1.0);
    drawing.set_font_size(32.0);
    drawing.move_to(40.0, 80.0);
    drawing
        .show_text("Hermes generated PDF")
        .expect("fixture text");
    drawing.show_page().expect("fixture page");
    surface.finish();
    surface.status().expect("finished PDF");
}

#[test]
#[ignore = "Requires bubblewrap and a built worker; generated PDF only"]
fn pooled_pdf_thumbnail_is_small_without_reducing_selected_page_or_text() {
    let directory = FixtureDirectory::new();
    let path = directory.0.join("page.pdf");
    write_pdf(&path);
    let thumbnail = thumbnail_parse(
        &path,
        ParseOperation::ThumbnailPdf,
        &Cancellation::default(),
    )
    .expect("PDF thumbnail uses pooled renderer")
    .expect("rendered thumbnail");
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.write(&thumbnail).unwrap();
    loader.close().unwrap();
    let pixels = loader.pixbuf().unwrap();
    assert_eq!((pixels.width(), pixels.height()), (256, 256));
    let full = crate::sandbox::parse(
        &path,
        ParseOperation::PreviewPdf,
        0,
        &Cancellation::default(),
    )
    .unwrap();
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.write(&full.data).unwrap();
    loader.close().unwrap();
    let pixels = loader.pixbuf().unwrap();
    assert_eq!((pixels.width(), pixels.height()), (1400, 1400));
    assert_eq!((full.page, full.pages), (0, 1));
    assert!(full.text_layer.is_some());
}

#[test]
#[ignore = "Requires bubblewrap and a built worker; generated 3MF only"]
fn pooled_three_mf_preserves_format_across_descriptor_transport() {
    let directory = FixtureDirectory::new();
    let path = directory.0.join("model.3MF");
    let xml = br#"<model><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="1" y="0" z="0"/><vertex x="0" y="1" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1"/></build></model>"#;
    write_package(&path, &[("3D/3dmodel.model", xml)]);
    let first = pooled_preview(&path, ParseOperation::PreviewModel);
    assert!(first.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(!decoded_pixels(&first).is_empty());
    assert_eq!(pooled_preview(&path, ParseOperation::PreviewModel), first);
    let stl = directory.0.join("model.STL");
    fs::write(&stl, b"solid fixture\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid fixture\n")
        .expect("generated STL fixture");
    let rendered = pooled_preview(&stl, ParseOperation::PreviewModel);
    assert!(!decoded_pixels(&rendered).is_empty());
}

#[test]
#[ignore = "Requires bubblewrap and a built worker; generated EPUB only"]
fn pooled_epub_preserves_format_across_descriptor_transport() {
    let directory = FixtureDirectory::new();
    let path = directory.0.join("book.EPUB");
    let cover = cover_png(0x2050d0ff);
    write_package(&path, &[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#),
        ("OEBPS/content.opf", br#"<package><manifest><item id="cover" href="images/cover.png" properties="cover-image"/></manifest></package>"#),
        ("OEBPS/images/cover.png", &cover),
    ]);
    let result = pooled_preview(&path, ParseOperation::PreviewArchiveCover);
    assert_eq!(decoded_pixels(&result), decoded_pixels(&cover));
    assert_eq!(
        pooled_preview(&path, ParseOperation::PreviewArchiveCover),
        result
    );
}

#[test]
#[ignore = "Requires bubblewrap and a built worker; generated CBZ only"]
fn pooled_cbz_preserves_format_across_descriptor_transport() {
    let directory = FixtureDirectory::new();
    let path = directory.0.join("comic.CBZ");
    let cover = cover_png(0xd05020ff);
    let second = cover_png(0x20d050ff);
    write_package(&path, &[("page10.png", &second), ("page2.png", &cover)]);
    let result = pooled_preview(&path, ParseOperation::PreviewArchiveCover);
    assert_eq!(decoded_pixels(&result), decoded_pixels(&cover));
    assert_eq!(
        pooled_preview(&path, ParseOperation::PreviewArchiveCover),
        result
    );
}

#[test]
#[ignore = "Requires local rar, bsdtar, bubblewrap and a built worker; generated CBR only"]
fn pooled_cbr_preserves_format_across_descriptor_transport() {
    match std::process::Command::new("rar").arg("-?").output() {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            eprintln!("SKIP real CBR fixture: local rar archive generator is unavailable");
            return;
        }
        Err(error) => panic!("Unable to inspect local rar: {error}"),
        Ok(_) => {}
    }
    let directory = FixtureDirectory::new();
    let cover = cover_png(0x8020d0ff);
    fs::write(directory.0.join("page1.png"), &cover).expect("fixture cover");
    let status = std::process::Command::new("rar")
        .current_dir(&directory.0)
        .args(["a", "-idq", "comic.cbr", "page1.png"])
        .status()
        .expect("local rar generator");
    assert!(status.success(), "real RAR fixture generation failed");
    let result = pooled_preview(
        &directory.0.join("comic.cbr"),
        ParseOperation::PreviewArchiveCover,
    );
    assert_eq!(decoded_pixels(&result), decoded_pixels(&cover));
}

#[test]
#[ignore = "Requires bubblewrap and a built worker; generated CBZ only"]
fn pooled_cover_format_keeps_identity_and_cancellation_guards() {
    let directory = FixtureDirectory::new();
    let path = directory.0.join("comic.cbz");
    let replacement = directory.0.join("replacement.cbz");
    let first_cover = cover_png(0x2050d0ff);
    let second_cover = cover_png(0xd05020ff);
    write_package(&path, &[("page1.png", &first_cover)]);
    let first = pooled_preview(&path, ParseOperation::PreviewArchiveCover);
    let identity = Identity::read(&path).expect("fixture identity");
    assert_eq!(
        preview_pool()
            .check_cache_entry(&path, identity, Operation::PreviewCbzCover)
            .expect("typed cover cache entry")
            .0,
        first
    );
    assert!(
        preview_pool()
            .check_cache_entry(&path, identity, Operation::PreviewEpubCover)
            .is_none()
    );
    write_package(&replacement, &[("page1.png", &second_cover)]);
    fs::rename(&replacement, &path).expect("fixture source replacement");
    let changed = pooled_preview(&path, ParseOperation::PreviewArchiveCover);
    assert_eq!(decoded_pixels(&changed), decoded_pixels(&second_cover));
    assert_ne!(changed, first);
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert!(
        preview(&path, &ParseOperation::PreviewArchiveCover, &cancellation)
            .expect("cancelled pooled request")
            .is_err()
    );
    fs::write(&path, b"corrupted archive").expect("invalid fixture");
    assert!(
        preview(
            &path,
            &ParseOperation::PreviewArchiveCover,
            &Cancellation::default()
        )
        .expect("invalid pooled request")
        .is_err()
    );
}

#[test]
fn pooled_format_mapping_rejects_unrecognized_names_before_opening() {
    for name in [
        "/proc/self/fd/42",
        "model.fcstd",
        "model.obj",
        "model.3mf/opaque",
    ] {
        assert!(map_format_preview_op(Path::new(name), &ParseOperation::PreviewModel).is_err());
    }
    for name in ["/proc/self/fd/42", "book.zip", "book.epub/opaque"] {
        assert!(
            map_format_preview_op(Path::new(name), &ParseOperation::PreviewArchiveCover).is_err()
        );
    }
    for (name, expected) in [
        ("model.STL", Operation::PreviewModel),
        ("model.3MF", Operation::PreviewThreeMf),
    ] {
        assert_eq!(
            map_format_preview_op(Path::new(name), &ParseOperation::PreviewModel)
                .expect("model format"),
            Some(expected)
        );
    }
    for (name, expected) in [
        ("book.EPUB", Operation::PreviewEpubCover),
        ("book.CBZ", Operation::PreviewCbzCover),
        ("book.CBR", Operation::PreviewCbrCover),
    ] {
        assert_eq!(
            map_format_preview_op(Path::new(name), &ParseOperation::PreviewArchiveCover)
                .expect("cover format"),
            Some(expected)
        );
    }
}
