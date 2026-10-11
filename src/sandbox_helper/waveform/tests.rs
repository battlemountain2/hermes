use super::*;
use std::{
    fs,
    io::Cursor,
    sync::atomic::{AtomicU64, Ordering},
};

fn pcm(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

#[test]
fn exact_bucket_boundaries_and_normalization() {
    let mut samples = vec![0.0; BUCKETS * 4];
    samples[0] = -0.5;
    samples[4] = 0.25;
    samples[BUCKETS * 4 - 1] = -2.0;
    let peaks = read_pcm(Cursor::new(pcm(&samples)), samples.len() as u64).expect("peaks");
    assert_eq!(peaks.len(), BUCKETS);
    assert_eq!(peaks[0], 0.25);
    assert_eq!(peaks[1], 0.125);
    assert_eq!(peaks[BUCKETS - 1], 1.0);
    assert!(peaks[2..BUCKETS - 1].iter().all(|peak| *peak == 0.0));
}

#[test]
fn rebucketing_preserves_beginning_middle_and_tail() {
    let count = HISTOGRAM_BINS * 17 + 13;
    let mut histogram = Histogram::new();
    for index in 0..count {
        histogram
            .push(if index == 0 || index == count / 2 || index == count - 1 {
                1.0
            } else {
                0.0
            })
            .expect("sample");
    }
    assert_eq!(histogram.peaks.len(), HISTOGRAM_BINS);
    assert!(histogram.width > 1);
    let peaks = histogram.finish().expect("finish");
    assert_eq!(peaks[0], 1.0);
    assert_eq!(peaks[BUCKETS / 2], 1.0);
    assert_eq!(peaks[BUCKETS - 1], 1.0);
    assert!(peaks[100..400].iter().all(|peak| *peak == 0.0));
    assert!(peaks[600..900].iter().all(|peak| *peak == 0.0));
}

#[test]
fn silence_and_very_short_audio_are_finite() {
    for samples in [vec![0.0; 3], vec![0.0; HISTOGRAM_BINS + 1], vec![0.5]] {
        let peaks = read_pcm(Cursor::new(pcm(&samples)), MAX_SAMPLES).expect("peaks");
        assert_eq!(peaks.len(), BUCKETS);
        assert!(
            peaks
                .iter()
                .all(|peak| peak.is_finite() && (0.0..=1.0).contains(peak))
        );
        if samples[0] == 0.0 {
            assert!(peaks.iter().all(|peak| *peak == 0.0));
        } else {
            assert!(peaks.iter().all(|peak| *peak == 1.0));
        }
    }
}

#[test]
fn empty_incomplete_nonfinite_and_overbudget_pcm_are_rejected() {
    assert!(read_pcm(Cursor::new(Vec::<u8>::new()), 10).is_err());
    assert!(read_pcm(Cursor::new(vec![0_u8; 5]), 10).is_err());
    for sample in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(read_pcm(Cursor::new(pcm(&[sample])), 10).is_err());
    }
    let error = read_pcm(Cursor::new(pcm(&[0.1; 11])), 10).expect_err("limit");
    assert!(error.contains("Unsupported long track"));
}

struct Fragmented(Cursor<Vec<u8>>);

impl Read for Fragmented {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let count = bytes.len().min(3);
        self.0.read(&mut bytes[..count])
    }
}

#[test]
fn pcm_samples_can_cross_pipe_read_boundaries() {
    let samples = pcm(&[-0.5, 0.0, 1.0, 0.25]);
    let expected = read_pcm(Cursor::new(samples.clone()), 4).expect("aligned");
    assert_eq!(
        read_pcm(Fragmented(Cursor::new(samples)), 4).expect("fragmented"),
        expected
    );
}

#[test]
fn ffmpeg_decodes_beyond_thirty_seconds_without_source_writes() {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let path = std::env::temp_dir().join(format!(
        "strata-waveform-{}-{}.wav",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let sample_count = SAMPLE_RATE as usize * 40;
    let data_bytes = (sample_count * 2) as u32;
    let mut bytes = Vec::with_capacity(data_bytes as usize + 44);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(data_bytes + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for index in 0..sample_count {
        let sample: i16 = if index > sample_count - SAMPLE_RATE as usize {
            if index % 2 == 0 { 20_000 } else { -20_000 }
        } else {
            0
        };
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(&path, &bytes).expect("fixture");
    let result = analyze(&path);
    let unchanged = fs::read(&path).expect("source");
    fs::remove_file(&path).expect("cleanup");
    let peaks = result.expect("bounded decoder");
    assert_eq!(unchanged, bytes);
    assert!(peaks[..BUCKETS * 9 / 10].iter().all(|peak| *peak == 0.0));
    assert_eq!(peaks[BUCKETS - 1], 1.0);
}
