use std::{fs::File, path::Path, process::Command};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Details {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_seconds: Option<f64>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub audio_codec: Option<String>,
    pub video_codec: Option<String>,
}

pub(crate) fn image(path: &Path) -> Result<Vec<u8>, String> {
    let dimensions = super::sniff::image_dimensions(path).or_else(|| {
        let (_, width, height) = gdk_pixbuf::Pixbuf::file_info(path)?;
        Some((u32::try_from(width).ok()?, u32::try_from(height).ok()?))
    });
    serde_json::to_vec(&Details {
        width: dimensions.map(|value| value.0),
        height: dimensions.map(|value| value.1),
        ..Details::default()
    }).map_err(|error| error.to_string())
}

pub(crate) fn media(path: &Path) -> Result<Vec<u8>, String> {
    let output = Command::new("ffprobe")
        .stdin(File::open(path).map_err(|error| error.to_string())?)
        .args(["-v", "error", "-threads", "1", "-probesize", "1048576", "-analyzeduration", "0",
            "-show_entries", "format=duration:format_tags=title,artist,album_artist,albumartist,album:stream=codec_type,codec_name,width,height,duration:stream_tags=title,artist,album_artist,albumartist,album:stream_disposition=attached_pic",
            "-of", "json", "-i", "/proc/self/fd/0"])
        .output().map_err(|error| error.to_string())?;
    if !output.status.success() || output.stdout.len() > 65536 {
        return Err("Quick media details unavailable".into());
    }
    serde_json::to_vec(&parse_media(&output.stdout)?).map_err(|error| error.to_string())
}

fn parse_media(data: &[u8]) -> Result<Details, String> {
    if data.len() > 65536 { return Err("Quick media details exceeded their budget".into()); }
    let value: serde_json::Value = serde_json::from_slice(data).map_err(|error| error.to_string())?;
    let tags = super::audio::parse_music_tags(data)?;
    let streams = value.get("streams").and_then(|value| value.as_array());
    let video = streams.and_then(|streams| streams.iter().find(|stream| stream["codec_type"] == "video" && stream["disposition"]["attached_pic"].as_u64() != Some(1)));
    let audio = streams.and_then(|streams| streams.iter().find(|stream| stream["codec_type"] == "audio"));
    let duration = |value: &serde_json::Value| value.as_str().and_then(|value| value.parse::<f64>().ok()).or_else(|| value.as_f64()).filter(|value| value.is_finite() && *value >= 0.0 && *value < 315_360_000.0);
    let dimension = |name: &str| video.and_then(|stream| stream[name].as_u64()).and_then(|value| u32::try_from(value).ok()).filter(|value| *value > 0 && *value <= 1_000_000);
    Ok(Details {
        width: dimension("width"), height: dimension("height"),
        duration_seconds: duration(&value["format"]["duration"]).or_else(|| video.and_then(|stream| duration(&stream["duration"]))).or_else(|| audio.and_then(|stream| duration(&stream["duration"]))),
        title: tags.title, artist: tags.artist.or(tags.album_artist), album: tags.album,
        audio_codec: audio.and_then(|stream| stream["codec_name"].as_str()).map(|value| value.chars().take(32).collect()),
        video_codec: video.and_then(|stream| stream["codec_name"].as_str()).map(|value| value.chars().take(32).collect()),
    })
}

#[cfg(test)]
mod tests;
