// SPDX-License-Identifier: GPL-3.0-or-later

/// Generates a valid PCM 16-bit mono 44.1kHz WAV audio stream with 200ms of a sine tone.
pub fn sample_wav() -> Vec<u8> {
    let sample_rate = 44100u32;
    let num_samples = 44100u32 / 5; // 200ms
    let data_len: u32 = num_samples * 2; // 16-bit = 2 bytes per sample
    let file_len: u32 = 36 + data_len;

    let mut out = Vec::with_capacity((file_len + 8) as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&file_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");

    // fmt subchunk
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size (16 for PCM)
    out.extend_from_slice(&1u16.to_le_bytes());  // audio format (1 = PCM)
    out.extend_from_slice(&1u16.to_le_bytes());  // num channels (1 = mono)
    out.extend_from_slice(&sample_rate.to_le_bytes()); // sample rate
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes());  // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    // data subchunk
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());

    for i in 0..num_samples {
        // 440 Hz sine wave
        let t = i as f64 / sample_rate as f64;
        let sample = ( (2.0 * std::f64::consts::PI * 440.0 * t).sin() * 16000.0 ) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }

    out
}

/// Generates a valid minimal FLAC container with STREAMINFO metadata block.
pub fn sample_flac() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"fLaC"); // 4-byte marker

    // Metadata block header: bit 0 is last block flag (1), bits 1-7 is block type (0 = STREAMINFO), 3 bytes length (34)
    out.push(0x80); // Last-metadata-block flag + block type 0
    out.extend_from_slice(&[0x00, 0x00, 0x22]); // 34 bytes length

    // STREAMINFO: min block size (u16), max block size (u16), min frame size (3 bytes), max frame size (3 bytes)
    out.extend_from_slice(&1024u16.to_be_bytes());
    out.extend_from_slice(&1024u16.to_be_bytes());
    out.extend_from_slice(&[0x00, 0x00, 0x00]);
    out.extend_from_slice(&[0x00, 0x00, 0x00]);

    // Sample rate (20 bits), channels (3 bits), bits per sample (5 bits), total samples (36 bits)
    // 44100 Hz = 0x0AC44
    out.extend_from_slice(&[0x0A, 0xC4, 0x41, 0x00, 0x00, 0x00, 0x00, 0x00]);
    // 16-byte MD5 signature
    out.extend_from_slice(&[0u8; 16]);

    out
}

/// Generates a valid simulated MP3 stream (ID3v2.3 tag + MPEG-1 Audio Layer III frame sync).
pub fn sample_mp3() -> Vec<u8> {
    let mut out = Vec::new();
    // ID3v2.3 header: "ID3", version 3.0, flags 0, syncsafe integer length 10
    out.extend_from_slice(b"ID3");
    out.extend_from_slice(&[0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0a]);
    out.extend_from_slice(&[0x00; 10]); // padding

    // MP3 Frame sync: 11 bits 0x7FF, MPEG-1 (1), Layer III (01), No CRC (1) -> 0xFFFB
    // Bitrate 128kbps (1001), 44100Hz (00), Padding 0, Private 0 -> 0x90
    // Channel mode: Joint stereo (01), etc. -> 0x64
    for _ in 0..4 {
        out.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
        out.extend_from_slice(&[0x00; 414]); // 418 bytes per frame at 128kbps 44.1kHz
    }

    out
}

/// Generates a valid minimal Ogg Vorbis/Opus container page header.
pub fn sample_ogg() -> Vec<u8> {
    let mut out = Vec::new();
    // OggS magic: 0x4f 0x67 0x67 0x53
    out.extend_from_slice(b"OggS");
    out.push(0x00); // Structure version
    out.push(0x02); // Header type: Beginning of stream (BOS)
    out.extend_from_slice(&0u64.to_le_bytes()); // Granule position
    out.extend_from_slice(&12345u32.to_le_bytes()); // Bitstream serial number
    out.extend_from_slice(&0u32.to_le_bytes()); // Page sequence number
    out.extend_from_slice(&0u32.to_le_bytes()); // Checksum placeholder
    out.push(1); // Page segments count
    out.push(30); // Segment length

    // Vorbis identification packet header
    out.push(0x01);
    out.extend_from_slice(b"vorbis");
    out.extend_from_slice(&[0u8; 23]);

    out
}

pub fn sample_corrupted_audio() -> Vec<u8> {
    let mut data = sample_wav();
    data.truncate(16); // Cut in the middle of format chunk
    data
}
