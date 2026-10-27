use super::*;

fn png(color: u32) -> Vec<u8> {
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 64, 64).unwrap();
    pixels.fill(color);
    pixels.save_to_bufferv("png", &[]).unwrap()
}

#[test]
fn artwork_palette_follows_warm_and_cool_covers_without_theme_blue() {
    let (blurred, warm) = presentation(&png(0xf08020ff)).unwrap();
    assert!(warm.0.0 > warm.0.1 && warm.0.1 > warm.0.2);
    let (_, cool) = presentation(&png(0x2080f0ff)).unwrap();
    assert!(cool.0.2 > cool.0.1 && cool.0.1 > cool.0.0);
    assert_ne!(warm, cool);
    let loader = PixbufLoader::with_type("png").unwrap();
    loader.write(&blurred).unwrap();
    loader.close().unwrap();
    let pixels = loader.pixbuf().unwrap();
    assert_eq!((pixels.width(), pixels.height()), (64, 64));
}

#[test]
fn artwork_analysis_rejects_bad_bounds_and_fully_transparent_pixels() {
    assert!(presentation(b"not a PNG").is_none());
    assert!(presentation(&png(0xff000000)).is_none());
    assert!(presentation(&vec![0; 16 * 1024 * 1024 + 1]).is_none());
    let mut oversized = png(0xf08020ff);
    oversized[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(presentation(&oversized).is_none());
    assert!(palette(&[0; 32], 96, 3).is_none());
    assert!(palette(&[0; 4096], 96, 2).is_none());
    assert!(palette(&[0; 4096], 1, 4).is_none());
}

#[test]
fn opaque_grayscale_artwork_remains_a_valid_neutral_palette() {
    let (_, colors) = presentation(&png(0x808080ff)).unwrap();
    for color in [colors.0, colors.1] {
        assert_eq!(color.0, color.1);
        assert_eq!(color.1, color.2);
        assert!((0.0..=1.0).contains(&color.0));
    }
}

#[test]
fn artwork_tints_the_base_instead_of_retaining_a_blue_theme_hue() {
    for theme in [(0.05, 0.1, 0.16), (0.96, 0.96, 0.95)] {
        let warm = background(((0.9, 0.4, 0.1), (0.9, 0.4, 0.1)), theme);
        let cool = background(((0.1, 0.4, 0.9), (0.1, 0.4, 0.9)), theme);
        assert!(warm.0 > warm.1 && warm.1 > warm.2);
        assert!(cool.2 > cool.1 && cool.1 > cool.0);
        assert!((contrast(warm, theme) - 1.0).abs() < 0.0001);
        assert!((contrast(cool, theme) - 1.0).abs() < 0.0001);
    }
}

#[test]
fn extreme_covers_keep_visualizer_and_metadata_legible_in_light_and_dark_themes() {
    let color = |value: &str| {
        let parsed = gtk::gdk::RGBA::parse(value).unwrap();
        (
            f64::from(parsed.red()),
            f64::from(parsed.green()),
            f64::from(parsed.blue()),
        )
    };
    for source in [
        include_str!("../../../../../data/themes/graphite.toml"),
        include_str!("../../../../../data/themes/paper.toml"),
    ] {
        let tokens: crate::ui::theme::ThemeTokens = toml::from_str(source).unwrap();
        let background = color(&tokens.background);
        let foreground = color(&tokens.text);
        for cover in [(0.0, 0.0, 0.0), (1.0, 1.0, 1.0)] {
            let tinted = super::background((cover, cover), background);
            let visible = legible_palette((cover, cover), tinted, foreground);
            let behind = |scrim: f64| {
                let layer = |base: f64, artwork: f64, theme: f64| {
                    (base * (1.0 - f64::from(BACKDROP_OPACITY))
                        + artwork * f64::from(BACKDROP_OPACITY))
                        * (1.0 - scrim)
                        + theme * scrim
                };
                (
                    layer(tinted.0, cover.0, background.0),
                    layer(tinted.1, cover.1, background.1),
                    layer(tinted.2, cover.2, background.2),
                )
            };
            assert!(contrast(visible.0, behind(0.0)) >= 3.0);
            assert!(contrast(visible.1, behind(0.0)) >= 3.0);
            assert!(contrast(color(&tokens.dim_text), behind(0.25)) >= 4.5);
        }
    }
}
