// SPDX-License-Identifier: GPL-3.0-or-later

/// Minimal standard uncompressed (Store) ZIP file generator in pure Rust.
pub fn create_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd_records = Vec::new();

    for (name, data) in entries {
        let local_header_offset = out.len() as u32;
        let crc = crc32(data);
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len() as u16;
        let size = data.len() as u32;

        // Local file header signature 0x04034b50
        out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed to extract (2.0)
        out.extend_from_slice(&0u16.to_le_bytes());  // general purpose bit flag
        out.extend_from_slice(&0u16.to_le_bytes());  // compression method (0 = Store)
        out.extend_from_slice(&0u16.to_le_bytes());  // last mod file time
        out.extend_from_slice(&0u16.to_le_bytes());  // last mod file date
        out.extend_from_slice(&crc.to_le_bytes());   // crc-32
        out.extend_from_slice(&size.to_le_bytes());  // compressed size
        out.extend_from_slice(&size.to_le_bytes());  // uncompressed size
        out.extend_from_slice(&name_len.to_le_bytes()); // file name length
        out.extend_from_slice(&0u16.to_le_bytes());  // extra field length
        out.extend_from_slice(name_bytes);           // file name
        out.extend_from_slice(data);                 // file data

        // Central directory entry
        let mut cd = Vec::new();
        cd.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]); // central file header signature
        cd.extend_from_slice(&20u16.to_le_bytes()); // version made by
        cd.extend_from_slice(&20u16.to_le_bytes()); // version needed to extract
        cd.extend_from_slice(&0u16.to_le_bytes());  // flag
        cd.extend_from_slice(&0u16.to_le_bytes());  // method (0 = Store)
        cd.extend_from_slice(&0u16.to_le_bytes());  // mod time
        cd.extend_from_slice(&0u16.to_le_bytes());  // mod date
        cd.extend_from_slice(&crc.to_le_bytes());   // crc-32
        cd.extend_from_slice(&size.to_le_bytes());  // compressed size
        cd.extend_from_slice(&size.to_le_bytes());  // uncompressed size
        cd.extend_from_slice(&name_len.to_le_bytes()); // file name length
        cd.extend_from_slice(&0u16.to_le_bytes());  // extra field length
        cd.extend_from_slice(&0u16.to_le_bytes());  // file comment length
        cd.extend_from_slice(&0u16.to_le_bytes());  // disk number start
        cd.extend_from_slice(&0u16.to_le_bytes());  // internal file attributes
        cd.extend_from_slice(&0u32.to_le_bytes());  // external file attributes
        cd.extend_from_slice(&local_header_offset.to_le_bytes()); // relative offset of local header
        cd.extend_from_slice(name_bytes);

        cd_records.push(cd);
    }

    let cd_offset = out.len() as u32;
    let mut cd_size = 0u32;
    for cd in &cd_records {
        out.extend_from_slice(cd);
        cd_size += cd.len() as u32;
    }

    // End of central directory record (EOCD)
    out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]); // EOCD signature
    out.extend_from_slice(&0u16.to_le_bytes()); // number of this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // disk with start of CD
    let entry_count = entries.len() as u16;
    out.extend_from_slice(&entry_count.to_le_bytes()); // total entries on this disk
    out.extend_from_slice(&entry_count.to_le_bytes()); // total entries in CD
    out.extend_from_slice(&cd_size.to_le_bytes());     // size of central directory
    out.extend_from_slice(&cd_offset.to_le_bytes());   // offset of start of CD
    out.extend_from_slice(&0u16.to_le_bytes());        // comment length

    out
}

/// Standard IEEE 802.3 CRC-32 checksum calculation.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = -( (crc & 1) as i32 ) as u32;
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
