use super::*;

#[test]
fn test_is_nodata_detection() {
    assert!(is_nodata(f32::NAN, None));
    assert!(is_nodata(f32::INFINITY, None));
    assert!(is_nodata(-9999.0, None));
    assert!(is_nodata(-32767.0, None));
    assert!(is_nodata(-32768.0, None));
    assert!(is_nodata(-99999.0, None));
    assert!(is_nodata(1e35, None));

    // Explicit tag
    assert!(is_nodata(-500.0, Some(-500.0)));
    assert!(!is_nodata(500.0, Some(-500.0)));

    // Valid terrain elevations
    assert!(!is_nodata(0.0, None));
    assert!(!is_nodata(1420.5, None));
    assert!(!is_nodata(8848.0, None));
    assert!(!is_nodata(-430.0, None)); // Dead Sea shore
}

#[test]
fn test_percentile_computation_and_outliers() {
    let mut data = vec![-9999.0; 2];
    for i in 0..96 {
        data.push(100.0 + (i as f32) * (100.0 / 95.0));
    }
    data.push(9000.0);
    data.push(9000.0);

    let stats = compute_elevation_stats(&data, None, 1000).expect("Must compute stats");
    assert_eq!(stats.min_valid, 100.0);
    assert_eq!(stats.max_valid, 9000.0);
    assert!(stats.p2 >= 100.0 && stats.p2 <= 105.0);
    assert!(stats.p98 <= 200.0 && stats.p98 >= 195.0);
}

#[test]
fn test_stretch_elevation_flat_terrain() {
    let val = stretch_elevation(150.0, 150.0, 150.0);
    assert_eq!(val, 0.5);
}

#[test]
fn test_hypsometric_tint_boundaries() {
    let (r0, g0, b0) = hypsometric_tint(0.0);
    assert_eq!((r0, g0, b0), (45, 106, 79)); // Lowland green

    let (r1, g1, b1) = hypsometric_tint(0.35);
    assert_eq!((r1, g1, b1), (212, 163, 115)); // Khaki

    let (r2, g2, b2) = hypsometric_tint(0.70);
    assert_eq!((r2, g2, b2), (108, 88, 76)); // Brown

    let (r3, g3, b3) = hypsometric_tint(1.0);
    assert_eq!((r3, g3, b3), (248, 249, 250)); // Snow
}

#[test]
fn test_dem_buffer_normalization_nodata_transparency() {
    let elevations = vec![-9999.0, 100.0, 500.0, 1000.0];
    let (rgba, stats) = normalize_dem_buffer(&elevations, 2, 2, None, DemColorMap::Hypsometric);

    assert!(stats.is_some());
    // Pixel 0 is NoData -> transparent [0, 0, 0, 0]
    assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
    // Pixels 1, 2, 3 must have Alpha = 255
    assert_eq!(rgba[7], 255);
    assert_eq!(rgba[11], 255);
    assert_eq!(rgba[15], 255);
}

#[test]
fn test_multiband_optical_stretch_discards_nir() {
    let samples = vec![
        1000.0, 1500.0, 2000.0, 0.0, // px 0
        1200.0, 1700.0, 2200.0, 0.0, // px 1
        1400.0, 1900.0, 2400.0, 0.0, // px 2
        1600.0, 2100.0, 2600.0, 0.0, // px 3
    ];

    let rgba = normalize_multiband_buffer(&samples, 2, 2, 4, false);
    // All pixels must have Alpha = 255 despite NIR = 0
    assert_eq!(rgba[3], 255);
    assert_eq!(rgba[7], 255);
    assert_eq!(rgba[11], 255);
    assert_eq!(rgba[15], 255);

    // Values must be dynamically stretched
    assert!(rgba[0] <= 10);
    assert!(rgba[12] >= 245);
}

#[test]
fn test_cairo_png_encoding_validity() {
    let rgba = vec![
        255, 0, 0, 255, // red
        0, 255, 0, 255, // green
        0, 0, 255, 255, // blue
        0, 0, 0, 0, // transparent
    ];

    let png = encode_rgba_to_png(&rgba, 2, 2).expect("PNG encoding must succeed");
    assert_eq!(&png[0..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn test_overview_selection_decisions() {
    // Ideal overview available [1200, 2048]
    let summaries = vec![
        IfdSummary {
            index: 0,
            width: 4096,
            height: 4096,
        },
        IfdSummary {
            index: 1,
            width: 2048,
            height: 2048,
        },
        IfdSummary {
            index: 2,
            width: 1024,
            height: 1024,
        },
    ];
    let decision = select_overview_layer(&summaries, 1400)
        .expect("generated fixture must provide the expected value");
    assert_eq!(
        decision,
        SelectionDecision::DirectDecode {
            ifd_index: 1,
            width: 2048,
            height: 2048
        }
    );

    // Overviews all < 1200: pick largest overview <= 2048
    let summaries2 = vec![
        IfdSummary {
            index: 0,
            width: 1024,
            height: 1024,
        },
        IfdSummary {
            index: 1,
            width: 512,
            height: 512,
        },
        IfdSummary {
            index: 2,
            width: 256,
            height: 256,
        },
    ];
    let decision2 = select_overview_layer(&summaries2, 1400)
        .expect("generated fixture must provide the expected value");
    assert_eq!(
        decision2,
        SelectionDecision::DirectDecode {
            ifd_index: 1,
            width: 512,
            height: 512
        }
    );

    // Flat small raster <= 2048
    let summaries3 = vec![IfdSummary {
        index: 0,
        width: 512,
        height: 512,
    }];
    let decision3 = select_overview_layer(&summaries3, 1400)
        .expect("generated fixture must provide the expected value");
    assert_eq!(
        decision3,
        SelectionDecision::DirectDecode {
            ifd_index: 0,
            width: 512,
            height: 512
        }
    );

    // Flat gigapixel raster > 2048: triggers subsample fallback
    let summaries4 = vec![IfdSummary {
        index: 0,
        width: 10000,
        height: 10000,
    }];
    let decision4 = select_overview_layer(&summaries4, 1400)
        .expect("generated fixture must provide the expected value");
    match decision4 {
        SelectionDecision::SubsampleFallback {
            ifd_index,
            src_width,
            src_height,
            stride,
            out_width,
            out_height,
        } => {
            assert_eq!(ifd_index, 0);
            assert_eq!(src_width, 10000);
            assert_eq!(src_height, 10000);
            assert_eq!(stride, 8); // ceil(10000 / 1400) = 8
            assert_eq!(out_width, 1250); // ceil(10000 / 8) = 1250 <= 1400
            assert_eq!(out_height, 1250);
        }
        _ => panic!("Expected SubsampleFallback"),
    }
}

#[test]
fn test_dem_buffer_normalization_grayscale() {
    let elevations = vec![-9999.0, 100.0, 500.0, 1000.0];
    let (rgba, stats) = normalize_dem_buffer(&elevations, 2, 2, None, DemColorMap::Grayscale);
    assert!(stats.is_some());
    assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
    assert_eq!(rgba[7], 255);
    // In grayscale, R == G == B
    assert_eq!(rgba[4], rgba[5]);
    assert_eq!(rgba[5], rgba[6]);
}

#[test]
fn test_epsg_to_crs_name_mapping() {
    assert_eq!(epsg_to_crs_name(4326), "WGS 84");
    assert_eq!(epsg_to_crs_name(3857), "WGS 84 / Pseudo-Mercator");
    assert_eq!(epsg_to_crs_name(32632), "WGS 84 / UTM zone 32N");
    assert_eq!(epsg_to_crs_name(32721), "WGS 84 / UTM zone 21S");
    assert_eq!(epsg_to_crs_name(99999), "EPSG:99999");
}

#[test]
fn test_geotiff_metadata_serialization_schema() {
    let meta = GeoTiffMetadata {
        epsg: Some(32632),
        crs_name: Some("WGS 84 / UTM zone 32N".to_string()),
        dimensions: [1024, 1024],
        resolution: [10.0, 10.0],
        bounds: [500000.0, 5100000.0, 510240.0, 5110240.0],
        band_count: 3,
        data_type: "MultiBand".to_string(),
        elevation_min: None,
        elevation_max: None,
    };

    let json = serde_json::to_string(&meta).expect("Serialization must succeed");
    assert!(json.contains("\"epsg\":32632"));
    assert!(json.contains("\"crs_name\":\"WGS 84 / UTM zone 32N\""));
    assert!(json.contains("\"band_count\":3"));
    assert!(json.contains("\"data_type\":\"MultiBand\""));

    let deserialized: GeoTiffMetadata =
        serde_json::from_str(&json).expect("Deserialization must succeed");
    assert_eq!(deserialized, meta);
}
