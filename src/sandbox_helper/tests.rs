// SPDX-License-Identifier: GPL-3.0-or-later

use super::office_xml_text;

#[test]
fn office_xml_is_reduced_to_readable_bounded_text() {
    let xml = r#"<w:document><w:p><w:r><w:t>Hello &amp; welcome</w:t></w:r></w:p><w:p><w:t>Hermes</w:t></w:p></w:document>"#;
    let text = office_xml_text(xml, 1024);
    assert!(text.contains("Hello & welcome"));
    assert!(text.contains("Hermes"));
    assert!(!text.contains("<w:"));

    assert!(office_xml_text(xml, 12).len() <= 12);
}

use super::sniff::*;

#[test]
fn test_sniff_png_dimensions() {
    let mut png = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
        0x00, 0x00, 0x00, 0x0D, // IHDR length
        b'I', b'H', b'D', b'R', // IHDR chunk type
        0x00, 0x00, 0x01, 0x00, // width: 256
        0x00, 0x00, 0x00, 0x80, // height: 128
        0x08, 0x06, 0x00, 0x00, 0x00, // bit depth, color type, etc.
    ];
    let dims = sniff_png_dimensions(&png).expect("valid png dimensions");
    assert_eq!(dims.width, 256);
    assert_eq!(dims.height, 128);

    // Truncated
    png.truncate(20);
    assert!(sniff_png_dimensions(&png).is_none());

    // Bad signature
    png[0] = 0x00;
    assert!(sniff_png_dimensions(&png).is_none());
}

#[test]
fn test_sniff_gif_dimensions() {
    let mut gif = vec![
        b'G', b'I', b'F', b'8', b'9', b'a',
        0x40, 0x01, // width: 320
        0xF0, 0x00, // height: 240
    ];
    let dims = sniff_gif_dimensions(&gif).expect("valid gif dimensions");
    assert_eq!(dims.width, 320);
    assert_eq!(dims.height, 240);

    // Truncated
    gif.truncate(8);
    assert!(sniff_gif_dimensions(&gif).is_none());
}

#[test]
fn test_sniff_jpeg_dimensions() {
    // Construct JPEG with APP0 and SOF0
    let jpeg = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xE0, // APP0
        0x00, 0x04, // length: 4
        0x00, 0x00,
        0xFF, 0xC0, // SOF0
        0x00, 0x0B, // length: 11
        0x08,       // precision
        0x01, 0xE0, // height: 480
        0x02, 0x80, // width: 640
        0x03, 0x01, 0x11, 0x00,
        0xFF, 0xDA, // SOS
    ];
    let dims = sniff_jpeg_dimensions(&jpeg).expect("valid jpeg dimensions");
    assert_eq!(dims.width, 640);
    assert_eq!(dims.height, 480);

    // Missing SOF
    let truncated_jpeg = vec![0xFF, 0xD8, 0xFF, 0xDA];
    assert!(sniff_jpeg_dimensions(&truncated_jpeg).is_none());
}

#[test]
fn test_sniff_tiff_dimensions() {
    // Little-endian Standard TIFF
    let mut tiff = vec![
        b'I', b'I', 0x2A, 0x00, // magic 42
        0x08, 0x00, 0x00, 0x00, // IFD0 offset: 8
        0x02, 0x00,             // 2 entries
        // Entry 1: tag 256 (ImageWidth), type 3 (SHORT), count 1, value 800
        0x00, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x20, 0x03, 0x00, 0x00,
        // Entry 2: tag 257 (ImageLength), type 3 (SHORT), count 1, value 600
        0x01, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, 0x58, 0x02, 0x00, 0x00,
    ];
    let dims = sniff_tiff_dimensions(&tiff).expect("valid tiff dimensions");
    assert_eq!(dims.width, 800);
    assert_eq!(dims.height, 600);

    // Corrupted IFD offset
    tiff[4] = 0xFF;
    tiff[5] = 0xFF;
    assert!(sniff_tiff_dimensions(&tiff).is_none());
}

#[test]
fn test_decoded_frame_budget_checks() {
    // 1920x1080 = 2,073,600 pixels * 4 = 8,294,400 bytes (< 32MB)
    assert!(!exceeds_decoded_frame_budget(1920, 1080));

    // 2896x2896 = 8,386,816 pixels * 4 = 33,547,264 bytes (< 32MB)
    assert!(!exceeds_decoded_frame_budget(2896, 2896));

    // 2897x2897 = 8,392,609 pixels * 4 = 33,570,436 bytes (> 32MB)
    assert!(exceeds_decoded_frame_budget(2897, 2897));

    // 12000x12000 = 144,000,000 pixels (> 134 MP ceiling)
    assert!(exceeds_decoded_frame_budget(12000, 12000));
}

#[test]
fn test_oversized_embedded_thumbnail_rejected() {
    // Construct JPEG header with 20000x20000 dimensions
    let huge_thumb_jpeg = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xC0, // SOF0
        0x00, 0x0B, // length: 11
        0x08,       // precision
        0x4E, 0x20, // height: 20000
        0x4E, 0x20, // width: 20000
        0x03, 0x01, 0x11, 0x00,
        0xFF, 0xD9, // EOI
    ];
    let result = scale_embedded_thumbnail(&huge_thumb_jpeg, 256);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("frame budget"));
}

