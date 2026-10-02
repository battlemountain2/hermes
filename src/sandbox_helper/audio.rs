// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    path::Path,
    process::Command,
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioMetadata {
    pub format: String,
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate: Option<u64>,
}

pub(crate) fn render_waveform_and_meta(input: &Path) -> Result<(Vec<u8>, String), String> {
    let file = fs::File::open(input).map_err(|e| format!("Unable to open audio input: {e}"))?;
    let output = Command::new("ffmpeg")
        .stdin(file)
        .args([
            "-v",
            "error",
            "-threads",
            "2",
            "-i",
            "pipe:0",
            "-filter_complex",
            "showwavespic=s=800x240:colors=0x7aa2f7:filter=peak:scale=cbrt",
            "-frames:v",
            "1",
            "-c:v",
            "png",
            "-f",
            "image2",
            "-",
        ])
        .output()
        .map_err(|e| format!("Unable to invoke ffmpeg: {e}"))?;

    if !output.status.success() {
        return Err("FFmpeg waveform generation failed".to_string());
    }

    let png_bytes = output.stdout;
    if !png_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Invalid waveform PNG produced by FFmpeg".to_string());
    }

    let metadata = extract_metadata(input).unwrap_or_else(|| {
        let ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("audio")
            .to_uppercase();
        AudioMetadata {
            format: ext,
            duration_seconds: 0.0,
            sample_rate: 44100,
            channels: 2,
            bitrate: None,
        }
    });

    let meta_json = serde_json::to_string(&metadata)
        .map_err(|e| format!("Failed to serialize audio metadata: {e}"))?;

    Ok((png_bytes, meta_json))
}

fn extract_metadata(input: &Path) -> Option<AudioMetadata> {
    // 1. Try ffprobe for rich probe
    if let Ok(file) = fs::File::open(input)
        && let Ok(output) = Command::new("ffprobe")
            .stdin(file)
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "format=format_name,duration,bit_rate:stream=sample_rate,channels",
                "-of",
                "json",
                "-i",
                "pipe:0",
            ])
            .output()
        && output.status.success()
        && let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&output.stdout)
    {
        let format_obj = parsed.get("format");
        let stream_obj = parsed
            .get("streams")
            .and_then(|s| s.as_array())
            .and_then(|a| a.first());

        let format_name = format_obj
            .and_then(|f| f.get("format_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("audio")
            .to_uppercase();

        let duration = format_obj
            .and_then(|f| f.get("duration"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        let bitrate = format_obj
            .and_then(|f| f.get("bit_rate"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u64>().ok());

        let sample_rate = stream_obj
            .and_then(|s| s.get("sample_rate"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(44100);

        let channels = stream_obj
            .and_then(|s| s.get("channels"))
            .and_then(|v| v.as_u64())
            .map(|c| c as u16)
            .unwrap_or(2);

        return Some(AudioMetadata {
            format: format_name,
            duration_seconds: duration,
            sample_rate,
            channels,
            bitrate,
        });
    }

    // 2. Fallback: Parse RIFF WAV header directly if applicable
    if let Ok(bytes) = fs::read(input)
        && bytes.len() >= 44
        && &bytes[0..4] == b"RIFF"
        && &bytes[8..12] == b"WAVE"
    {
        let channels = u16::from_le_bytes(bytes[22..24].try_into().ok()?);
        let sample_rate = u32::from_le_bytes(bytes[24..28].try_into().ok()?);
        let byte_rate = u32::from_le_bytes(bytes[28..32].try_into().ok()?);
        let data_len = bytes.len().saturating_sub(44) as u64;
        let duration = if byte_rate > 0 {
            data_len as f64 / byte_rate as f64
        } else {
            0.0
        };
        return Some(AudioMetadata {
            format: "WAV".to_string(),
            duration_seconds: duration,
            sample_rate,
            channels,
            bitrate: if byte_rate > 0 { Some(byte_rate as u64 * 8) } else { None },
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_metadata_serialization() {
        let meta = AudioMetadata {
            format: "FLAC".to_string(),
            duration_seconds: 184.2,
            sample_rate: 96000,
            channels: 2,
            bitrate: Some(1500000),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let decoded: AudioMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, decoded);
    }
}
