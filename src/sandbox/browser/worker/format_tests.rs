use super::*;
use crate::sandbox::browser::format_tests::{
    FixtureDirectory, cover_png, decoded_pixels, write_package,
};

#[test]
fn worker_uses_declared_cover_format_on_extensionless_descriptors() {
    let directory = FixtureDirectory::new();
    let cover = cover_png(0x2050d0ff);
    for operation in [Operation::PreviewEpubCover, Operation::PreviewCbzCover] {
        let path = directory.0.join("opaque.fd");
        let members = if operation == Operation::PreviewEpubCover {
            vec![
                ("META-INF/container.xml", br#"<container><rootfile full-path="content.opf"/></container>"#.as_slice()),
                ("content.opf", br#"<package><manifest><item id="cover" href="cover.png" properties="cover-image"/></manifest></package>"#.as_slice()),
                ("cover.png", cover.as_slice()),
            ]
        } else {
            vec![("page1.png", cover.as_slice())]
        };
        write_package(&path, &members);
        let descriptor = OwnedFd::from(File::open(&path).expect("fixture descriptor"));
        let result = render_job(operation, &descriptor).expect("declared cover format");
        assert_eq!(decoded_pixels(&result.png), decoded_pixels(&cover));
    }
}

#[test]
fn worker_uses_declared_three_mf_format_on_extensionless_descriptors() {
    let directory = FixtureDirectory::new();
    let path = directory.0.join("opaque.fd");
    let cover = cover_png(0x2050d0ff);
    write_package(&path, &[("Metadata/thumbnail.png", &cover)]);
    let descriptor = OwnedFd::from(File::open(&path).expect("fixture descriptor"));
    let result = render_job(Operation::PreviewThreeMf, &descriptor).expect("declared 3MF format");
    assert_eq!(decoded_pixels(&result.png), decoded_pixels(&cover));
    assert!(render_job(Operation::PreviewModel, &descriptor).is_err());
}

#[test]
fn worker_rejects_cover_requests_without_an_explicit_format() {
    let descriptor = OwnedFd::from(File::open("/dev/null").expect("empty input descriptor"));
    assert_eq!(
        render_job(Operation::PreviewArchiveCover, &descriptor).expect_err("untyped cover request"),
        "Archive cover request requires an explicit format"
    );
}

#[test]
fn pdf_worker_renders_a_thumbnail_not_a_full_page_or_text_layer() {
    let directory = FixtureDirectory::new();
    let path = directory.0.join("opaque.fd");
    crate::sandbox::browser::format_tests::write_pdf(&path);
    let descriptor = OwnedFd::from(File::open(&path).unwrap());
    let result = render_job(Operation::Pdf, &descriptor).unwrap();
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    gtk::prelude::PixbufLoaderExt::write(&loader, &result.png).unwrap();
    gtk::prelude::PixbufLoaderExt::close(&loader).unwrap();
    let pixels = gtk::prelude::PixbufLoaderExt::pixbuf(&loader).unwrap();
    assert_eq!((pixels.width(), pixels.height()), (256, 256));
    assert!(result.metadata.is_empty());
}
