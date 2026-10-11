use std::io::{self, Read, Write};

pub(super) const MAX_FRAME: usize = 1280 * 720 * 4;
pub(super) const MAX_AUDIO: usize = 64 * 1024;
const MAX_HEADER: usize = 4096;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum Event {
    Prepared {
        duration: i64,
        audio: bool,
        video: bool,
        seekable: bool,
    },
    Frame {
        epoch: u64,
        timestamp: i64,
        width: u32,
        height: u32,
    },
    Audio {
        epoch: u64,
    },
    Position {
        epoch: u64,
        timestamp: i64,
    },
    Seeked {
        epoch: u64,
        success: bool,
    },
    Ended {
        epoch: u64,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(super) enum Command {
    Play,
    Pause,
    Seek { epoch: u64, timestamp: i64 },
    Stop,
}

pub(super) fn write_event(writer: &mut impl Write, event: &Event, data: &[u8]) -> io::Result<()> {
    validate(event, data.len())?;
    let header = serde_json::to_vec(event)?;
    if header.len() > MAX_HEADER {
        return Err(invalid("Playback header too large"));
    }
    writer.write_all(&(header.len() as u32).to_le_bytes())?;
    writer.write_all(&(data.len() as u32).to_le_bytes())?;
    writer.write_all(&header)?;
    writer.write_all(data)?;
    writer.flush()
}

pub(super) fn read_event(reader: &mut impl Read) -> io::Result<(Event, Vec<u8>)> {
    let mut sizes = [0_u8; 8];
    reader.read_exact(&mut sizes)?;
    let header_size = u32::from_le_bytes(sizes[..4].try_into().unwrap()) as usize;
    let data_size = u32::from_le_bytes(sizes[4..].try_into().unwrap()) as usize;
    if header_size == 0 || header_size > MAX_HEADER || data_size > MAX_FRAME {
        return Err(invalid("Invalid playback packet size"));
    }
    let mut header = vec![0; header_size];
    reader.read_exact(&mut header)?;
    let event: Event = serde_json::from_slice(&header)?;
    validate(&event, data_size)?;
    let mut data = vec![0; data_size];
    reader.read_exact(&mut data)?;
    Ok((event, data))
}

fn validate(event: &Event, bytes: usize) -> io::Result<()> {
    let valid = match event {
        Event::Frame {
            width,
            height,
            timestamp,
            ..
        } => {
            *width > 0
                && *width <= 1280
                && *height > 0
                && *height <= 720
                && *timestamp >= 0
                && bytes == *width as usize * *height as usize * 4
        }
        Event::Audio { .. } => bytes > 0 && bytes <= MAX_AUDIO && bytes.is_multiple_of(8),
        Event::Prepared { duration, .. } => *duration >= 0 && bytes == 0,
        Event::Position { timestamp, .. } => *timestamp >= 0 && bytes == 0,
        Event::Error { message } => message.len() <= 2048 && bytes == 0,
        _ => bytes == 0,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid("Invalid playback packet"))
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests;
