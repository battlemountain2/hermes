use super::*;

#[test]
fn thumbnail_selection_rejects_undersized_and_unbounded_images() {
    assert!(adequate_thumbnail(320, 240, (256, 192)));
    assert!(!adequate_thumbnail(160, 120, (256, 192)));
    assert!(!adequate_thumbnail(320, 120, (256, 192)));
    assert!(!adequate_thumbnail(512, 384, (192, 256)));
    assert!(!adequate_thumbnail(0, 0, (0, 0)));
    assert!(!bounded_dimensions(u32::MAX, u32::MAX));
}

#[test]
fn invalid_input_is_rejected_without_decoding() {
    let path = std::env::temp_dir().join(format!("hermes-invalid-heif-{}", std::process::id()));
    std::fs::write(&path, b"not a HEIF image").unwrap();
    assert!(thumbnail(&path, 256).is_err());
    std::fs::remove_file(path).unwrap();
}

#[test]
#[ignore = "Requires HEVC encoder/decoder plugins and ImageMagick; generated HEIF only"]
fn generated_embedded_heif_thumbnail_orientation_fallback_and_cache() {
    let decoder = LibHeif::new_checked().unwrap();
    let mut encoder = decoder
        .encoder_for_format(libheif_rs::CompressionFormat::Hevc)
        .unwrap();
    encoder
        .set_quality(libheif_rs::EncoderQuality::Lossy(70))
        .unwrap();
    let mut image = libheif_rs::Image::new(4032, 3024, ColorSpace::Rgb(RgbChroma::Rgb)).unwrap();
    image
        .create_plane(libheif_rs::Channel::Interleaved, 4032, 3024, 8)
        .unwrap();
    let planes = image.planes_mut();
    let plane = planes.interleaved.unwrap();
    for vertical in 0..plane.height as usize {
        for horizontal in 0..plane.width as usize {
            let offset = vertical * plane.stride + horizontal * 3;
            plane.data[offset..offset + 3].copy_from_slice(&[
                if horizontal < 2016 { 220 } else { 30 },
                if vertical < 1512 { 200 } else { 20 },
                60,
            ]);
        }
    }
    let mut options = libheif_rs::EncodingOptions::new().unwrap();
    options.set_image_orientation(libheif_rs::ImageOrientation::Rotate90Cw);
    let mut context = HeifContext::new().unwrap();
    let primary = context
        .encode_image(&image, &mut encoder, Some(options))
        .unwrap();
    let mut thumbnail_options = libheif_rs::EncodingOptions::new().unwrap();
    thumbnail_options.set_image_orientation(libheif_rs::ImageOrientation::Rotate90Cw);
    context
        .encode_thumbnail(&image, &primary, 320, &mut encoder, Some(thumbnail_options))
        .unwrap()
        .unwrap();
    let directory =
        std::env::temp_dir().join(format!("hermes-generated-heif-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("generated.heic");
    context.write_to_file(path.to_str().unwrap()).unwrap();
    drop(context);
    let mut direct_times = Vec::new();
    let mut fallback_times = Vec::new();
    for _ in 0..3 {
        let start = std::time::Instant::now();
        let png = thumbnail(&path, 256).unwrap();
        direct_times.push(start.elapsed());
        let loader = gdk_pixbuf::PixbufLoader::new();
        use gdk_pixbuf::prelude::*;
        loader.write(&png).unwrap();
        loader.close().unwrap();
        let pixels = loader.pixbuf().unwrap();
        assert_eq!((pixels.width(), pixels.height()), (192, 256));
        let start = std::time::Instant::now();
        let fallback = super::super::render_imagemagick(&path, 256).unwrap();
        fallback_times.push(start.elapsed());
        let loader = gdk_pixbuf::PixbufLoader::new();
        loader.write(&fallback).unwrap();
        loader.close().unwrap();
        let full_pixels = loader.pixbuf().unwrap();
        assert_eq!((full_pixels.width(), full_pixels.height()), (192, 256));
        let direct_pixels = pixels.read_pixel_bytes();
        let reference_pixels = full_pixels.read_pixel_bytes();
        for (horizontal, vertical) in [(48, 64), (144, 64), (48, 192), (144, 192)] {
            let direct =
                vertical * pixels.rowstride() as usize + horizontal * pixels.n_channels() as usize;
            let reference = vertical * full_pixels.rowstride() as usize
                + horizontal * full_pixels.n_channels() as usize;
            for channel in 0..3 {
                assert!(
                    direct_pixels[direct + channel].abs_diff(reference_pixels[reference + channel])
                        < 25
                );
            }
        }
    }
    eprintln!(
        "HEIF generated 4032x3024 direct-thumbnail times: {direct_times:?}; full ImageMagick: {fallback_times:?}"
    );
    let cancelled = crate::sandbox::Cancellation::default();
    cancelled.cancel();
    assert!(
        crate::sandbox::parse(
            &path,
            crate::sandbox::ParseOperation::ThumbnailHeif,
            256,
            &cancelled
        )
        .is_err()
    );
    for operation in [
        crate::sandbox::ParseOperation::ThumbnailHeif,
        crate::sandbox::ParseOperation::PreviewHeif,
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
        let second = crate::sandbox::parse(
            &path,
            operation,
            256,
            &crate::sandbox::Cancellation::default(),
        )
        .unwrap();
        let warm = start.elapsed();
        assert_eq!(first.data, second.data);
        eprintln!("HEIF sandbox {operation:?}: cold={cold:?}, warm={warm:?}");
    }
    let mut no_thumbnail = HeifContext::new().unwrap();
    no_thumbnail
        .encode_image(&image, &mut encoder, None)
        .unwrap();
    let replacement = directory.join("without-thumbnail.heic");
    no_thumbnail
        .write_to_file(replacement.to_str().unwrap())
        .unwrap();
    assert!(thumbnail(&replacement, 256).is_err());
    assert!(
        !super::super::render_heif_thumbnail(&replacement, 256)
            .unwrap()
            .is_empty()
    );
    let before = crate::sandbox::parse(
        &path,
        crate::sandbox::ParseOperation::ThumbnailHeif,
        256,
        &crate::sandbox::Cancellation::default(),
    )
    .unwrap();
    std::fs::copy(&replacement, &path).unwrap();
    let after = crate::sandbox::parse(
        &path,
        crate::sandbox::ParseOperation::ThumbnailHeif,
        256,
        &crate::sandbox::Cancellation::default(),
    )
    .unwrap();
    assert_ne!(before.data, after.data);
    std::fs::remove_dir_all(directory).unwrap();
}
