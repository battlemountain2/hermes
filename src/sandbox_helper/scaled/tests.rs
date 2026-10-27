use super::*;
use gdk_pixbuf::prelude::*;

#[test]
fn scaled_output_rejects_invalid_sizes_headers_and_oversized_dimensions() {
    let header = |width: u32, height: u32| {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    };
    assert!(valid_output(&header(1400, 1050), 1400));
    assert!(!valid_output(&header(1401, 1050), 1400));
    assert!(!valid_output(&header(0, 1050), 1400));
    assert!(!valid_output(&header(10, 10), -1));
    assert!(!valid_output(&header(10, 10), 1500));
    assert!(!valid_output(b"not PNG", 1400));
}

#[test]
fn missing_optional_decoder_and_invalid_input_fail_without_changing_the_source() {
    let fixture = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    let path = fixture.0.join("invalid.jpg");
    std::fs::write(&path, b"generated invalid image").unwrap();
    assert!(render_with("/nonexistent/hermes-scaled-decoder", &path, 1400).is_err());
    assert!(render(&path, 0).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"generated invalid image");
}

fn decode(bytes: &[u8]) -> gdk_pixbuf::Pixbuf {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(bytes).unwrap();
    loader.close().unwrap();
    loader.pixbuf().unwrap()
}

#[test]
#[ignore = "Private generated 12MP corpus, vips/ImageMagick, bubblewrap and built helper"]
fn generated_heavy_photos_use_scaled_sandbox_path_with_full_size_and_colors() {
    let root = std::path::PathBuf::from(std::env::var_os("HERMES_PREVIEW_PROBE").unwrap());
    assert!(root.starts_with("/tmp"));
    assert!(
        std::path::PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap()).starts_with(&root)
    );
    for (name, operation) in [
        ("textured.jpg", crate::sandbox::ParseOperation::PreviewImage),
        ("textured.heic", crate::sandbox::ParseOperation::PreviewHeif),
    ] {
        let path = root.join("fixtures").join(name);
        if name.ends_with("heic") {
            assert!(super::super::heif::external_preview_safe(&path, 1400));
        } else {
            assert!(super::super::jpeg::external_preview_safe(&path));
        }
        let mut expected = None;
        for sample in 0..3 {
            let started = std::time::Instant::now();
            let scaled = render(&path, 1400).unwrap();
            let fast_time = started.elapsed();
            let started = std::time::Instant::now();
            let original = super::super::render_imagemagick(&path, 1400).unwrap();
            eprintln!(
                "Heavy {name} sample{sample}: scaled={fast_time:?}, previous={:?}",
                started.elapsed()
            );
            let pixels = decode(&scaled);
            let reference = decode(&original);
            assert_eq!((pixels.width(), pixels.height()), (1400, 1050));
            assert_eq!((reference.width(), reference.height()), (1400, 1050));
            let data = pixels.read_pixel_bytes();
            let reference_data = reference.read_pixel_bytes();
            let mut total = 0u64;
            for vertical in 0..1050usize {
                for horizontal in 0..1400usize {
                    for channel in 0..3 {
                        total += u64::from(
                            data[vertical * pixels.rowstride() as usize
                                + horizontal * pixels.n_channels() as usize
                                + channel]
                                .abs_diff(
                                    reference_data[vertical * reference.rowstride() as usize
                                        + horizontal * reference.n_channels() as usize
                                        + channel],
                                ),
                        );
                    }
                }
            }
            let error = total as f64 / (1400.0 * 1050.0 * 3.0);
            eprintln!("Representative image mean RGB difference: {error}");
            assert!(
                error < 5.0,
                "Scaled image must retain representative colors/detail"
            );
            expected = Some(scaled);
        }
        let expected = expected.unwrap();
        let pooled = crate::sandbox::parse(
            &path,
            operation,
            1400,
            &crate::sandbox::Cancellation::default(),
        )
        .unwrap();
        assert_eq!(pooled.data, expected);
        let one_shot = crate::sandbox::parse_one_shot_with_executable(
            &path,
            operation,
            1400,
            &crate::sandbox::Cancellation::default(),
            &crate::sandbox::helper_executable().unwrap(),
        )
        .unwrap();
        assert_eq!(one_shot.data, expected);
    }
    let source = std::fs::read(root.join("fixtures/textured.jpg")).unwrap();
    let fixture = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    for orientation in 1..=8 {
        let path = fixture.0.join(format!("oriented-{orientation}.jpg"));
        std::fs::write(
            &path,
            super::super::jpeg::tests::jpeg_fixture(orientation, &source, &[]),
        )
        .unwrap();
        let scaled = decode(&render(&path, 1400).unwrap());
        let reference = decode(&super::super::render_imagemagick(&path, 1400).unwrap());
        assert_eq!(
            (scaled.width(), scaled.height()),
            (reference.width(), reference.height())
        );
        let data = scaled.read_pixel_bytes();
        let reference_data = reference.read_pixel_bytes();
        let mut difference = 0u64;
        for vertical in 0..scaled.height() as usize {
            for horizontal in 0..scaled.width() as usize {
                for channel in 0..3 {
                    difference += u64::from(
                        data[vertical * scaled.rowstride() as usize
                            + horizontal * scaled.n_channels() as usize
                            + channel]
                            .abs_diff(
                                reference_data[vertical * reference.rowstride() as usize
                                    + horizontal * reference.n_channels() as usize
                                    + channel],
                            ),
                    );
                }
            }
        }
        assert!(
            difference as f64 / (f64::from(scaled.width()) * f64::from(scaled.height()) * 3.0)
                < 5.0
        );
    }
    let missing = fixture.0.join("tools-without-vips");
    std::fs::create_dir(&missing).unwrap();
    std::os::unix::fs::symlink("/usr/bin/magick", missing.join("magick")).unwrap();
    for (file, operation) in [
        ("textured.jpg", "preview-image"),
        ("textured.heic", "preview-heif"),
    ] {
        let path = root.join("fixtures").join(file);
        let output = fixture.0.join("fallback.png");
        let result = std::process::Command::new(crate::sandbox::helper_executable().unwrap())
            .env("PATH", &missing)
            .arg("--preview-helper")
            .arg(operation)
            .arg(&path)
            .arg(&output)
            .arg("1400")
            .output()
            .unwrap();
        assert!(result.status.success(), "{:?}", result);
        let fallback = decode(&std::fs::read(output).unwrap());
        assert_eq!((fallback.width(), fallback.height()), (1400, 1050));
    }
}
