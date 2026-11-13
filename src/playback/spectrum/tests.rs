use super::*;

#[test]
fn spectrum_analyzer_initializes_with_valid_coefficients_and_zero_bands() {
    let analyzer = SpectrumAnalyzer::new();
    assert_eq!(analyzer.current_bands.len(), SPECTRUM_BANDS);
    assert!(analyzer.current_bands.iter().all(|&b| b == 0.0));
    assert!(analyzer.coeffs.iter().all(|&c| c.is_finite()));
    assert!(
        analyzer
            .eq_weights
            .iter()
            .all(|&w| w > 0.0 && w.is_finite())
    );
}

#[test]
fn spectrum_analyzer_detects_sine_frequency_in_matching_band() {
    let mut analyzer = SpectrumAnalyzer::new();
    let target_freq = 1000.0_f32;
    let mut pcm = Vec::with_capacity(ANALYSIS_SAMPLES * 8);
    for n in 0..ANALYSIS_SAMPLES {
        let sample = (2.0 * PI * target_freq * n as f32 / SAMPLE_RATE).sin() * 0.8;
        pcm.extend_from_slice(&sample.to_le_bytes());
        pcm.extend_from_slice(&sample.to_le_bytes());
    }

    analyzer.push_pcm(&pcm);
    let bands = analyzer.analyze();

    // Band near 1000 Hz should have highest magnitude
    let peak_idx = bands
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(idx, _)| idx)
        .expect("generated fixture must provide the expected value");

    let ratio = peak_idx as f32 / (SPECTRUM_BANDS - 1) as f32;
    let detected_freq = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(ratio);

    // Center should be close to 1000 Hz (within 1-2 bands)
    assert!(
        (detected_freq - target_freq).abs() < 400.0,
        "Target {target_freq} Hz, detected at band {peak_idx} ({detected_freq} Hz)"
    );
    assert!(bands[peak_idx] > 0.3, "Peak magnitude should be noticeable");
}

#[test]
fn spectrum_analyzer_decays_cleanly_to_zero_on_silence() {
    let mut analyzer = SpectrumAnalyzer::new();
    analyzer.current_bands.fill(0.9);
    for _ in 0..60 {
        analyzer.decay();
    }
    assert!(
        analyzer.current_bands.iter().all(|&b| b == 0.0),
        "Bands should decay to zero: {:?}",
        analyzer.current_bands
    );
}
