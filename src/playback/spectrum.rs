use std::f32::consts::PI;

pub const SPECTRUM_BANDS: usize = 32;
pub const ANALYSIS_SAMPLES: usize = 1024;
const SAMPLE_RATE: f32 = 48000.0;
const MIN_FREQ: f32 = 50.0;
const MAX_FREQ: f32 = 16000.0;
const ATTACK_RATE: f32 = 0.95;
const DECAY_RATE: f32 = 0.82;

pub(crate) struct SpectrumAnalyzer {
    ring: [f32; ANALYSIS_SAMPLES],
    ring_pos: usize,
    samples_since_analysis: usize,
    coeffs: [f32; SPECTRUM_BANDS],
    eq_weights: [f32; SPECTRUM_BANDS],
    hann_window: [f32; ANALYSIS_SAMPLES],
    current_bands: [f32; SPECTRUM_BANDS],
}

impl SpectrumAnalyzer {
    pub fn new() -> Self {
        let mut coeffs = [0.0; SPECTRUM_BANDS];
        let mut eq_weights = [0.0; SPECTRUM_BANDS];
        for i in 0..SPECTRUM_BANDS {
            let ratio = i as f32 / (SPECTRUM_BANDS - 1) as f32;
            let freq = MIN_FREQ * (MAX_FREQ / MIN_FREQ).powf(ratio);
            let omega = 2.0 * PI * freq / SAMPLE_RATE;
            coeffs[i] = 2.0 * omega.cos();
            // Equalization curve: compensation for musical spectral rolloff (~1/f pink noise)
            eq_weights[i] = (1.0 + (freq / 350.0).powf(0.48)) * 1.8;
        }

        let mut hann_window = [0.0; ANALYSIS_SAMPLES];
        for n in 0..ANALYSIS_SAMPLES {
            hann_window[n] = 0.5 * (1.0 - (2.0 * PI * n as f32 / ANALYSIS_SAMPLES as f32).cos());
        }

        Self {
            ring: [0.0; ANALYSIS_SAMPLES],
            ring_pos: 0,
            samples_since_analysis: 0,
            coeffs,
            eq_weights,
            hann_window,
            current_bands: [0.0; SPECTRUM_BANDS],
        }
    }

    pub fn push_pcm(&mut self, data: &[u8]) {
        for frame in data.chunks_exact(8) {
            let left = f32::from_le_bytes(frame[0..4].try_into().unwrap_or_default());
            let right = f32::from_le_bytes(frame[4..8].try_into().unwrap_or_default());
            let mono = if left.is_finite() && right.is_finite() {
                ((left + right) * 0.5).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            self.ring[self.ring_pos] = mono;
            self.ring_pos = (self.ring_pos + 1) % ANALYSIS_SAMPLES;
            self.samples_since_analysis += 1;
        }
    }

    pub fn analyze(&mut self) -> [f32; SPECTRUM_BANDS] {
        if self.samples_since_analysis < 128 {
            return self.current_bands;
        }
        self.samples_since_analysis = 0;

        let start = self.ring_pos;
        for i in 0..SPECTRUM_BANDS {
            let coeff = self.coeffs[i];
            let mut s1 = 0.0_f32;
            let mut s2 = 0.0_f32;

            for n in 0..ANALYSIS_SAMPLES {
                let sample_idx = (start + n) % ANALYSIS_SAMPLES;
                let sample = self.ring[sample_idx] * self.hann_window[n];
                let s0 = sample + coeff * s1 - s2;
                s2 = s1;
                s1 = s0;
            }

            let power = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0);
            let mag = (power.sqrt() / ANALYSIS_SAMPLES as f32) * self.eq_weights[i];
            
            // Logarithmic dynamic range mapping (~ -45 dB to 0 dB full scale)
            let db = 20.0 * (mag + 1e-4).log10();
            let raw_level = ((db + 45.0) / 45.0).clamp(0.0, 1.0);

            // Responsive attack, smooth decay
            let prev = self.current_bands[i];
            if raw_level > prev {
                self.current_bands[i] = prev + (raw_level - prev) * ATTACK_RATE;
            } else {
                self.current_bands[i] = (prev * DECAY_RATE).max(0.0);
            }
        }

        self.current_bands
    }

    pub fn decay(&mut self) -> [f32; SPECTRUM_BANDS] {
        for band in &mut self.current_bands {
            *band = (*band * DECAY_RATE).max(0.0);
            if *band < 0.005 {
                *band = 0.0;
            }
        }
        self.current_bands
    }

    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.ring_pos = 0;
        self.samples_since_analysis = 0;
        self.current_bands.fill(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrum_analyzer_initializes_with_valid_coefficients_and_zero_bands() {
        let analyzer = SpectrumAnalyzer::new();
        assert_eq!(analyzer.current_bands.len(), SPECTRUM_BANDS);
        assert!(analyzer.current_bands.iter().all(|&b| b == 0.0));
        assert!(analyzer.coeffs.iter().all(|&c| c.is_finite()));
        assert!(analyzer.eq_weights.iter().all(|&w| w > 0.0 && w.is_finite()));
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
            .unwrap();

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
}
