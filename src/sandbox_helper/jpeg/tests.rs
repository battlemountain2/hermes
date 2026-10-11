use super::*;

#[test]
fn embedded_jpeg_must_be_large_enough_and_match_the_requested_aspect() {
    assert!(adequate((320, 240), (256, 192)));
    assert!(!adequate((160, 120), (256, 192)));
    assert!(!adequate((320, 120), (256, 192)));
    assert!(!adequate((320, 240), (192, 256)));
    assert!(!adequate((0, 0), (0, 0)));
    assert_eq!(oriented_dimensions(320, 240, 6), (240, 320));
}

#[test]
fn malformed_exif_and_invalid_orientation_fall_back_safely() {
    assert!(exif(b"not a jpeg").is_none());
    assert!(exif(&[0xff, 0xd8, 0xff, 0xe1, 0, 1]).is_none());
    let mut tiff = tiff_header(6, &[]);
    let primary = directory(&tiff, 8).unwrap();
    assert_eq!(orientation(&tiff, primary.0, primary.1), Some(6));
    assert_eq!(directory(&tiff, usize::MAX), None);
    tiff[18..20].copy_from_slice(&9u16.to_le_bytes());
    assert_eq!(orientation(&tiff, primary.0, primary.1), None);
    for length in 0..26 {
        assert!(directory(&tiff[..length], 8).is_none());
    }
}

fn tiff_header(orientation: u16, thumbnail: &[u8]) -> Vec<u8> {
    let mut tiff = b"II*\0".to_vec();
    tiff.extend_from_slice(&8u32.to_le_bytes());
    tiff.extend_from_slice(&1u16.to_le_bytes());
    tiff.extend_from_slice(&0x0112u16.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&0u16.to_le_bytes());
    tiff.extend_from_slice(&26u32.to_le_bytes());
    tiff.extend_from_slice(&3u16.to_le_bytes());
    for (tag, kind, value) in [
        (0x0112u16, 3u16, orientation as u32),
        (0x0201, 4, 68),
        (0x0202, 4, thumbnail.len() as u32),
    ] {
        tiff.extend_from_slice(&tag.to_le_bytes());
        tiff.extend_from_slice(&kind.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&value.to_le_bytes());
    }
    tiff.extend_from_slice(&0u32.to_le_bytes());
    tiff.extend_from_slice(thumbnail);
    tiff
}

fn jpeg_fixture(orientation: u16, original: &[u8], thumbnail: &[u8]) -> Vec<u8> {
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&tiff_header(orientation, thumbnail));
    let mut data = vec![0xff, 0xd8, 0xff, 0xe1];
    data.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    data.extend_from_slice(&payload);
    data.extend_from_slice(&original[2..]);
    data
}

fn pixels(data: &[u8]) -> Pixbuf {
    let loader = PixbufLoader::new();
    loader.write(data).unwrap();
    loader.close().unwrap();
    loader.pixbuf().unwrap()
}

#[test]
#[ignore = "Generated JPEG fixture requires JPEG loader and ImageMagick"]
fn generated_jpeg_thumbnail_orientation_quality_fallback_and_timing() {
    let directory = std::env::temp_dir().join(format!("hermes-jpeg-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("photo.jpg");
    let original = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 2400, 1600).unwrap();
    original.fill(0x2040d0ff);
    let source = original
        .save_to_bufferv("jpeg", &[("quality", "90")])
        .unwrap();
    let thumbnail_pixels = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 384, 256).unwrap();
    thumbnail_pixels.fill(0xd02040ff);
    let corner = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 192, 128).unwrap();
    corner.fill(0x20d040ff);
    corner.copy_area(0, 0, 192, 128, &thumbnail_pixels, 0, 0);
    let thumbnail_data = thumbnail_pixels
        .save_to_bufferv("jpeg", &[("quality", "90")])
        .unwrap();
    let matching_original = thumbnail_pixels
        .scale_simple(2400, 1600, gdk_pixbuf::InterpType::Bilinear)
        .unwrap()
        .save_to_bufferv("jpeg", &[("quality", "90")])
        .unwrap();
    for orientation in 1..=8 {
        std::fs::write(
            &path,
            jpeg_fixture(orientation, &matching_original, &thumbnail_data),
        )
        .unwrap();
        let result = pixels(&thumbnail(&path, 256).expect("Adequate embedded JPEG"));
        let expected = pixels(&super::super::render_imagemagick(&path, 256).unwrap());
        let dimensions =
            sniff::image_target_dimensions(expected.width() as u32, expected.height() as u32, 256);
        assert_eq!(
            (result.width(), result.height()),
            (dimensions.0 as i32, dimensions.1 as i32)
        );
        let expected = expected
            .scale_simple(
                result.width(),
                result.height(),
                gdk_pixbuf::InterpType::Bilinear,
            )
            .unwrap();
        let actual_bytes = result.read_pixel_bytes();
        let expected_bytes = expected.read_pixel_bytes();
        for (horizontal, vertical) in [
            (10, 10),
            (result.width() - 10, 10),
            (10, result.height() - 10),
            (result.width() - 10, result.height() - 10),
        ] {
            let actual =
                (vertical * result.rowstride() + horizontal * result.n_channels()) as usize;
            let expected_offset =
                (vertical * expected.rowstride() + horizontal * expected.n_channels()) as usize;
            for channel in 0..3 {
                assert!(
                    (i16::from(actual_bytes[actual + channel])
                        - i16::from(expected_bytes[expected_offset + channel]))
                    .abs()
                        < 12,
                    "Orientation {orientation}"
                );
            }
        }
    }
    std::fs::write(&path, jpeg_fixture(1, &source, &thumbnail_data)).unwrap();
    let mut fast = Vec::new();
    let mut fallback = Vec::new();
    for _ in 0..3 {
        let start = std::time::Instant::now();
        super::super::render_image(&path, 256).unwrap();
        fast.push(start.elapsed());
        let start = std::time::Instant::now();
        super::super::render_imagemagick(&path, 256).unwrap();
        fallback.push(start.elapsed());
    }
    eprintln!("JPEG embedded {fast:?}; original ImageMagick {fallback:?}");
    let preview = pixels(&super::super::render_image(&path, 1400).unwrap());
    assert_eq!(preview.width(), 1400);
    assert_eq!(preview.height(), 933);
    let bytes = preview.read_pixel_bytes();
    assert!(
        bytes[2] > 150 && bytes[0] < 80,
        "Full preview uses the original, not the deliberately red thumbnail"
    );
    let cancelled = crate::sandbox::Cancellation::default();
    cancelled.cancel();
    assert!(
        crate::sandbox::parse(
            &path,
            crate::sandbox::ParseOperation::ThumbnailImage,
            256,
            &cancelled
        )
        .is_err()
    );
    let mut cached_thumbnail = Vec::new();
    for operation in [
        crate::sandbox::ParseOperation::ThumbnailImage,
        crate::sandbox::ParseOperation::PreviewImage,
    ] {
        let start = std::time::Instant::now();
        let first = crate::sandbox::parse(
            &path,
            operation,
            256,
            &crate::sandbox::Cancellation::default(),
        )
        .unwrap();
        let cold = start.elapsed();
        let start = std::time::Instant::now();
        let warm = crate::sandbox::parse(
            &path,
            operation,
            256,
            &crate::sandbox::Cancellation::default(),
        )
        .unwrap();
        assert_eq!(first.data, warm.data);
        eprintln!(
            "JPEG sandbox {operation:?}: cold={cold:?}, warm={:?}",
            start.elapsed()
        );
        if operation == crate::sandbox::ParseOperation::ThumbnailImage {
            cached_thumbnail = first.data;
        }
    }
    let tiny = thumbnail_pixels
        .scale_simple(96, 64, gdk_pixbuf::InterpType::Bilinear)
        .unwrap()
        .save_to_bufferv("jpeg", &[])
        .unwrap();
    std::fs::write(&path, jpeg_fixture(1, &source, &tiny)).unwrap();
    assert!(thumbnail(&path, 256).is_none());
    let fallback = pixels(&super::super::render_image(&path, 256).unwrap());
    assert!(fallback.read_pixel_bytes()[2] > 150);
    let refreshed = crate::sandbox::parse(
        &path,
        crate::sandbox::ParseOperation::ThumbnailImage,
        256,
        &crate::sandbox::Cancellation::default(),
    )
    .unwrap();
    assert_ne!(
        cached_thumbnail, refreshed.data,
        "Replacing the source invalidates cached thumbnails"
    );
    std::fs::write(&path, &source).unwrap();
    assert!(thumbnail(&path, 256).is_none());
    let large = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 3600, 2400).unwrap();
    large.fill(0x2040d0ff);
    let large_data = large.save_to_bufferv("jpeg", &[("quality", "90")]).unwrap();
    std::fs::write(&path, jpeg_fixture(1, &large_data, &thumbnail_data)).unwrap();
    assert!(sniff::exceeds_decoded_frame_budget(3600, 2400));
    assert!(thumbnail(&path, 256).is_some());
    let large_preview = pixels(&super::super::render_image(&path, 1400).unwrap());
    assert_eq!(large_preview.width(), 1400);
    assert!(
        large_preview.read_pixel_bytes()[2] > 150,
        "Over-budget JPEG previews still use bounded original-image decoding"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
