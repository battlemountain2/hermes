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
    std::fs::write(&path, b"not a HEIF image").expect("write generated fixture");
    assert!(thumbnail(&path, 256).is_err());
    assert!(preview(&path, 1400).is_err());
    assert!(!external_preview_safe(&path, 1400));
    std::fs::remove_file(path).expect("remove generated fixture");
}

#[test]
fn native_preview_defers_hdr_and_wide_gamut_profiles_to_the_existing_renderer() {
    use libheif_rs::{ColorPrimaries, TransferCharacteristics};
    assert!(native_color_profile(
        ColorPrimaries::ITU_R_BT_709_5,
        TransferCharacteristics::IEC_61966_2_1
    ));
    assert!(!native_color_profile(
        ColorPrimaries::ITU_R_BT_709_5,
        TransferCharacteristics::ITU_R_BT_2100_0_PQ
    ));
    assert!(!native_color_profile(
        ColorPrimaries::SMPTE_EG_432_1,
        TransferCharacteristics::IEC_61966_2_1
    ));
}

#[test]
#[ignore = "Generated native HEIF benchmark requires HEVC codecs, ImageMagick, bubblewrap and built helper"]
fn generated_native_heif_preview_preserves_orientation_and_uses_primary_image() {
    use gdk_pixbuf::prelude::*;
    let directory = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    let decoder = LibHeif::new_checked().expect("expected test value from LibHeif::new_checked()");
    let mut encoder = decoder
        .encoder_for_format(libheif_rs::CompressionFormat::Hevc)
        .expect("expected test value from decoder .encoder_for_format(libheif_rs::CompressionFormat::Hevc)");
    encoder
        .set_quality(libheif_rs::EncoderQuality::Lossy(90))
        .expect(
            "expected test value from encoder .set_quality(libheif_rs::EncoderQuality::Lossy(90))",
        );
    let mut image = libheif_rs::Image::new(1600, 1200, ColorSpace::Rgb(RgbChroma::Rgb)).expect("expected test value from libheif_rs::Image::new(1600, 1200, ColorSpace::Rgb(RgbChroma::Rgb))");
    image
        .create_plane(libheif_rs::Channel::Interleaved, 1600, 1200, 8)
        .expect("expected test value from image .create_plane(libheif_rs::Channel::Interleaved, 1600, 1200, 8)");
    let plane = image
        .planes_mut()
        .interleaved
        .expect("expected test value from image.planes_mut().interleaved");
    for vertical in 0..1200usize {
        for horizontal in 0..1600usize {
            let offset = vertical * plane.stride + horizontal * 3;
            plane.data[offset..offset + 3].copy_from_slice(if horizontal < 800 {
                &[220, 30, 60]
            } else {
                &[30, 220, 60]
            });
        }
    }
    let mut options = libheif_rs::EncodingOptions::new()
        .expect("expected test value from libheif_rs::EncodingOptions::new()");
    options.set_image_orientation(libheif_rs::ImageOrientation::Rotate90Cw);
    let mut context = HeifContext::new().expect("expected test value from HeifContext::new()");
    context
        .encode_image(&image, &mut encoder, Some(options))
        .expect(
            "expected test value from context .encode_image(&image, &mut encoder, Some(options))",
        );
    let path = directory.0.join("primary.heic");
    context
        .write_to_file(
            path.to_str()
                .expect("expected test value from path.to_str()"),
        )
        .expect("expected test value from context.write_to_file(path.to_str().unwrap())");
    let expected = preview(&path, 1400).expect("expected test value from preview(&path, 1400)");
    assert!(external_preview_safe(&path, 1400));
    let pooled = crate::sandbox::parse(
        &path,
        crate::sandbox::ParseOperation::PreviewHeif,
        1400,
        &crate::sandbox::Cancellation::default(),
    )
    .expect("expected test value from crate::sandbox::parse( &path, crate::sandbox::ParseOperation::PreviewHeif, 1400, &crate::sandbox::Cancellation::default(), )");
    assert_eq!(pooled.data, expected);
    let one_shot = crate::sandbox::parse_one_shot_with_executable(
        &path,
        crate::sandbox::ParseOperation::PreviewHeif,
        1400,
        &crate::sandbox::Cancellation::default(),
        &crate::sandbox::helper_executable().expect("expected test value from crate::sandbox::helper_executable()"),
    )
    .expect("expected test value from crate::sandbox::parse_one_shot_with_executable( &path, crate::sandbox::ParseOperation::PreviewHeif, 1400, &crate::sandbox::Canc...");
    assert_eq!(one_shot.data, expected);
    for _ in 0..3 {
        let started = std::time::Instant::now();
        let native = preview(&path, 1400).expect("expected test value from preview(&path, 1400)");
        let native_time = started.elapsed();
        let started = std::time::Instant::now();
        let old = super::super::render_imagemagick(&path, 1400)
            .expect("expected test value from super::super::render_imagemagick(&path, 1400)");
        eprintln!(
            "Native HEIF primary: {native_time:?}, previous={:?}",
            started.elapsed()
        );
        let decode = |data: &[u8]| {
            let loader = gdk_pixbuf::PixbufLoader::new();
            loader
                .write(data)
                .expect("expected test value from loader.write(data)");
            loader
                .close()
                .expect("expected test value from loader.close()");
            loader
                .pixbuf()
                .expect("expected test value from loader.pixbuf()")
        };
        let native = decode(&native);
        let old = decode(&old);
        assert_eq!((native.width(), native.height()), (1050, 1400));
        assert_eq!(
            (native.width(), native.height()),
            (old.width(), old.height())
        );
        let native_bytes = native.read_pixel_bytes();
        let old_bytes = old.read_pixel_bytes();
        for (horizontal, vertical) in [(262, 350), (788, 1050)] {
            let native_offset =
                vertical * native.rowstride() as usize + horizontal * native.n_channels() as usize;
            let old_offset =
                vertical * old.rowstride() as usize + horizontal * old.n_channels() as usize;
            for channel in 0..3 {
                assert!(
                    native_bytes[native_offset + channel].abs_diff(old_bytes[old_offset + channel])
                        <= 8
                );
            }
        }
    }
}

#[test]
#[ignore = "Requires HEVC encoder/decoder plugins and ImageMagick; generated HEIF only"]
fn generated_embedded_heif_thumbnail_orientation_fallback_and_cache() {
    let decoder = LibHeif::new_checked().expect("expected test value from LibHeif::new_checked()");
    let mut encoder = decoder
        .encoder_for_format(libheif_rs::CompressionFormat::Hevc)
        .expect("expected test value from decoder .encoder_for_format(libheif_rs::CompressionFormat::Hevc)");
    encoder
        .set_quality(libheif_rs::EncoderQuality::Lossy(70))
        .expect(
            "expected test value from encoder .set_quality(libheif_rs::EncoderQuality::Lossy(70))",
        );
    let mut image = libheif_rs::Image::new(4032, 3024, ColorSpace::Rgb(RgbChroma::Rgb)).expect("expected test value from libheif_rs::Image::new(4032, 3024, ColorSpace::Rgb(RgbChroma::Rgb))");
    image
        .create_plane(libheif_rs::Channel::Interleaved, 4032, 3024, 8)
        .expect("expected test value from image .create_plane(libheif_rs::Channel::Interleaved, 4032, 3024, 8)");
    let planes = image.planes_mut();
    let plane = planes
        .interleaved
        .expect("expected test value from planes.interleaved");
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
    let mut options = libheif_rs::EncodingOptions::new()
        .expect("expected test value from libheif_rs::EncodingOptions::new()");
    options.set_image_orientation(libheif_rs::ImageOrientation::Rotate90Cw);
    let mut context = HeifContext::new().expect("expected test value from HeifContext::new()");
    let primary = context
        .encode_image(&image, &mut encoder, Some(options))
        .expect(
            "expected test value from context .encode_image(&image, &mut encoder, Some(options))",
        );
    let mut thumbnail_options = libheif_rs::EncodingOptions::new()
        .expect("expected test value from libheif_rs::EncodingOptions::new()");
    thumbnail_options.set_image_orientation(libheif_rs::ImageOrientation::Rotate90Cw);
    context
        .encode_thumbnail(&image, &primary, 320, &mut encoder, Some(thumbnail_options))
        .expect("expected test value from context .encode_thumbnail(&image, &primary, 320, &mut encoder, Some(thumbnail_options))")
        .expect("expected test value from context .encode_thumbnail(&image, &primary, 320, &mut encoder, Some(thumbnail_options)) .unwrap()");
    let directory =
        std::env::temp_dir().join(format!("hermes-generated-heif-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create generated fixture directory");
    let path = directory.join("generated.heic");
    context
        .write_to_file(
            path.to_str()
                .expect("expected test value from path.to_str()"),
        )
        .expect("expected test value from context.write_to_file(path.to_str().unwrap())");
    assert!(external_preview_safe(&path, 1400));
    assert!(!external_preview_safe(&path, 256));
    drop(context);
    let mut direct_times = Vec::new();
    let mut fallback_times = Vec::new();
    for _ in 0..3 {
        let start = std::time::Instant::now();
        let png = thumbnail(&path, 256).expect("expected test value from thumbnail(&path, 256)");
        direct_times.push(start.elapsed());
        let loader = gdk_pixbuf::PixbufLoader::new();
        use gdk_pixbuf::prelude::*;
        loader
            .write(&png)
            .expect("expected test value from loader.write(&png)");
        loader
            .close()
            .expect("expected test value from loader.close()");
        let pixels = loader
            .pixbuf()
            .expect("expected test value from loader.pixbuf()");
        assert_eq!((pixels.width(), pixels.height()), (192, 256));
        let start = std::time::Instant::now();
        let fallback = super::super::render_imagemagick(&path, 256)
            .expect("expected test value from super::super::render_imagemagick(&path, 256)");
        fallback_times.push(start.elapsed());
        let loader = gdk_pixbuf::PixbufLoader::new();
        loader
            .write(&fallback)
            .expect("expected test value from loader.write(&fallback)");
        loader
            .close()
            .expect("expected test value from loader.close()");
        let full_pixels = loader
            .pixbuf()
            .expect("expected test value from loader.pixbuf()");
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
        .expect("expected test value from crate::sandbox::parse( &path, operation, 256, &crate::sandbox::Cancellation::default(), )");
        let cold = start.elapsed();
        let start = std::time::Instant::now();
        let second = crate::sandbox::parse(
            &path,
            operation,
            256,
            &crate::sandbox::Cancellation::default(),
        )
        .expect("expected test value from crate::sandbox::parse( &path, operation, 256, &crate::sandbox::Cancellation::default(), )");
        let warm = start.elapsed();
        assert_eq!(first.data, second.data);
        eprintln!("HEIF sandbox {operation:?}: cold={cold:?}, warm={warm:?}");
    }
    let mut no_thumbnail = HeifContext::new().expect("expected test value from HeifContext::new()");
    no_thumbnail
        .encode_image(&image, &mut encoder, None)
        .expect("expected test value from no_thumbnail .encode_image(&image, &mut encoder, None)");
    let replacement = directory.join("without-thumbnail.heic");
    no_thumbnail
        .write_to_file(
            replacement
                .to_str()
                .expect("expected test value from replacement.to_str()"),
        )
        .expect(
            "expected test value from no_thumbnail .write_to_file(replacement.to_str().unwrap())",
        );
    assert!(thumbnail(&replacement, 256).is_err());
    assert!(
        !super::super::render_heif_thumbnail(&replacement, 256)
            .expect(
                "expected test value from super::super::render_heif_thumbnail(&replacement, 256)"
            )
            .is_empty()
    );
    let before = crate::sandbox::parse(
        &path,
        crate::sandbox::ParseOperation::ThumbnailHeif,
        256,
        &crate::sandbox::Cancellation::default(),
    )
    .expect("expected test value from crate::sandbox::parse( &path, crate::sandbox::ParseOperation::ThumbnailHeif, 256, &crate::sandbox::Cancellation::default(), )");
    std::fs::copy(&replacement, &path)
        .expect("expected test value from std::fs::copy(&replacement, &path)");
    let after = crate::sandbox::parse(
        &path,
        crate::sandbox::ParseOperation::ThumbnailHeif,
        256,
        &crate::sandbox::Cancellation::default(),
    )
    .expect("expected test value from crate::sandbox::parse( &path, crate::sandbox::ParseOperation::ThumbnailHeif, 256, &crate::sandbox::Cancellation::default(), )");
    assert_ne!(before.data, after.data);
    std::fs::remove_dir_all(directory).expect("remove generated fixture");
}
