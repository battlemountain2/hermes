// SPDX-License-Identifier: GPL-3.0-or-later

use std::{fs, path::Path, process::Command};

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

pub(crate) fn read_music_tags(input: &Path) -> Result<String, String> {
    let output = probe_audio(
        input,
        "format_tags=title,artist,album_artist,albumartist,album,track,tracknumber,disc,discnumber:stream_tags=title,artist,album_artist,albumartist,album,track,tracknumber,disc,discnumber",
        "65536",
        "0",
    )?;
    let tags = parse_music_tags(&output)?;
    serde_json::to_string(&tags).map_err(|error| error.to_string())
}

pub(crate) fn read_audio_metadata(input: &Path) -> Result<String, String> {
    let output = probe_audio(
        input,
        "format=format_name,duration,bit_rate:format_tags:stream=codec_name,codec_long_name,duration,sample_rate,channels,bit_rate,bits_per_raw_sample,bits_per_sample:stream_tags",
        "1048576",
        "2000000",
    )?;
    let metadata = parse_audio_details(&output)?;
    serde_json::to_string(&metadata).map_err(|error| error.to_string())
}

fn probe_audio(
    input: &Path,
    entries: &str,
    probe_size: &str,
    analyze_duration: &str,
) -> Result<Vec<u8>, String> {
    let file = fs::File::open(input).map_err(|error| error.to_string())?;
    let output = Command::new("ffprobe")
        .stdin(file)
        .args([
            "-v",
            "error",
            "-probesize",
            probe_size,
            "-analyzeduration",
            analyze_duration,
            "-select_streams",
            "a:0",
            "-show_entries",
            entries,
            "-of",
            "json",
            "-i",
            "/proc/self/fd/0",
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() || output.stdout.len() > 1024 * 1024 {
        return Err("Unable to read bounded music metadata".to_owned());
    }
    Ok(output.stdout)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioDetails {
    #[serde(flatten)]
    pub tags: crate::model::MusicTags,
    pub properties: AudioProperties,
    pub embedded_tags: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioProperties {
    pub container: Option<String>,
    pub codec: Option<String>,
    pub duration_seconds: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub bitrate: Option<u64>,
    pub bit_depth: Option<u16>,
}

fn bounded_tag(value: &str, limit: usize) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(limit)
        .collect()
}

fn parse_audio_details(data: &[u8]) -> Result<AudioDetails, String> {
    let parsed: serde_json::Value =
        serde_json::from_slice(data).map_err(|error| error.to_string())?;
    let format = parsed.get("format");
    let stream = parsed
        .get("streams")
        .and_then(|streams| streams.as_array())
        .and_then(|streams| streams.first());
    let text = |source: Option<&serde_json::Value>, key: &str| {
        source
            .and_then(|source| source.get(key))
            .and_then(|value| value.as_str())
            .map(|value| bounded_tag(value, 256))
            .filter(|value| !value.is_empty())
    };
    let integer = |source: Option<&serde_json::Value>, key: &str| {
        source
            .and_then(|source| source.get(key))
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| value.as_str()?.parse::<u64>().ok())
            })
            .filter(|value| *value > 0)
    };
    let duration = |source| {
        text(source, "duration")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0 && *value <= 31_536_000.0)
    };
    let properties = AudioProperties {
        container: text(format, "format_name"),
        codec: text(stream, "codec_name"),
        duration_seconds: duration(stream).or_else(|| duration(format)),
        sample_rate: integer(stream, "sample_rate")
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value <= 768_000),
        channels: integer(stream, "channels")
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value <= 64),
        bitrate: integer(stream, "bit_rate").or_else(|| integer(format, "bit_rate")),
        bit_depth: integer(stream, "bits_per_raw_sample")
            .or_else(|| integer(stream, "bits_per_sample"))
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| *value <= 64),
    };
    let mut embedded_tags = std::collections::BTreeMap::new();
    for source in [format, stream].into_iter().flatten() {
        if let Some(tags) = source.get("tags").and_then(|tags| tags.as_object()) {
            for (key, value) in tags {
                if embedded_tags.len() >= 64 {
                    break;
                }
                let key = bounded_tag(key, 64).to_ascii_lowercase();
                let Some(value) = value
                    .as_str()
                    .map(|value| bounded_tag(value, 1024))
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                if !key.is_empty() {
                    embedded_tags.entry(key).or_insert(value);
                }
            }
        }
    }
    Ok(AudioDetails {
        tags: parse_music_tags(data)?,
        properties,
        embedded_tags,
    })
}

fn parse_music_tags(data: &[u8]) -> Result<crate::model::MusicTags, String> {
    let parsed: serde_json::Value =
        serde_json::from_slice(data).map_err(|error| error.to_string())?;
    let format = parsed.get("format").and_then(|format| format.get("tags"));
    let stream = parsed
        .get("streams")
        .and_then(|streams| streams.as_array())
        .and_then(|streams| streams.first())
        .and_then(|stream| stream.get("tags"));
    let tag = |names: &[&str]| -> Option<String> {
        for source in [format, stream].into_iter().flatten() {
            if let Some(tags) = source.as_object() {
                for name in names {
                    if let Some(value) = tags
                        .iter()
                        .find(|(key, _)| key.eq_ignore_ascii_case(name))
                        .and_then(|(_, value)| value.as_str())
                    {
                        let value: String = value
                            .trim()
                            .chars()
                            .filter(|character| !character.is_control())
                            .take(256)
                            .collect();
                        if !value.is_empty() {
                            return Some(value);
                        }
                    }
                }
            }
        }
        None
    };
    Ok(crate::model::MusicTags {
        title: tag(&["title"]),
        artist: tag(&["artist"]),
        album_artist: tag(&["album_artist", "albumartist"]),
        album: tag(&["album"]),
        disc: tag(&["discnumber", "disc"]).as_deref().and_then(tag_number),
        track: tag(&["tracknumber", "track"])
            .as_deref()
            .and_then(tag_number),
    })
}

fn tag_number(value: &str) -> Option<u32> {
    let number = value.split('/').next()?.trim().parse::<u32>().ok()?;
    (number > 0 && number <= 1_000_000).then_some(number)
}

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
            bitrate: if byte_rate > 0 {
                Some(byte_rate as u64 * 8)
            } else {
                None
            },
        });
    }

    None
}
