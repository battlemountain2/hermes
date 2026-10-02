// SPDX-License-Identifier: GPL-3.0-or-later

use super::zip_util::create_zip;

/// Minimal 1x1 pixel PNG bytes for mock cover art.
pub fn minimal_png() -> &'static [u8] {
    &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, // PNG Signature
        0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, // IHDR chunk len & type
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1 width & height
        0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, // 8-bit RGBA, CRC
        0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, // IDAT chunk
        0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4,
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, // IEND chunk
        0xae, 0x42, 0x60, 0x82,
    ]
}

/// Generates a valid EPUB eBook containing a container.xml, content.opf, and cover.png.
pub fn sample_epub() -> Vec<u8> {
    let container_xml = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    let content_opf = r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Hermes Test Novel</dc:title>
    <dc:creator>Test Author</dc:creator>
    <meta name="cover" content="cover-image"/>
  </metadata>
  <manifest>
    <item id="cover-image" href="cover.png" media-type="image/png" properties="cover-image"/>
    <item id="chapter1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chapter1"/>
  </spine>
</package>"#;

    let ch1 = r#"<!DOCTYPE html><html><body><h1>Chapter 1</h1><p>Hermes E2E content.</p></body></html>"#;

    create_zip(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container_xml.as_bytes()),
        ("OEBPS/content.opf", content_opf.as_bytes()),
        ("OEBPS/cover.png", minimal_png()),
        ("OEBPS/ch1.xhtml", ch1.as_bytes()),
    ])
}

/// Generates a valid CBZ comic archive containing sequential PNG page images.
pub fn sample_cbz() -> Vec<u8> {
    create_zip(&[
        ("page_001.png", minimal_png()),
        ("page_002.png", minimal_png()),
        ("comicinfo.xml", b"<ComicInfo><Title>Hermes Comic</Title></ComicInfo>"),
    ])
}

/// Generates a simulated CBR (RAR 5.0) container with standard RAR magic bytes.
pub fn sample_cbr() -> Vec<u8> {
    let mut out = Vec::new();
    // RAR 5.0 signature: 0x52 0x61 0x72 0x21 0x1A 0x07 0x01 0x00
    out.extend_from_slice(&[0x52, 0x61, 0x72, 0x21, 0x1a, 0x07, 0x01, 0x00]);
    // Trailing mock header block
    out.extend_from_slice(&[0x00; 64]);
    out
}

/// Generates a corrupted archive fixture.
pub fn sample_corrupted_archive() -> Vec<u8> {
    let mut data = sample_cbz();
    data.truncate(24); // Cut mid-header
    data
}
