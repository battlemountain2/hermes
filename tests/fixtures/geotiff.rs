// SPDX-License-Identifier: GPL-3.0-or-later

/// Dynamic pure-Rust generator for GeoTIFF, COG, DEM, and multi-band optical rasters.
pub struct GeoTiffBuilder {
    width: u32,
    height: u32,
    bands: u16,
    is_float32: bool,
    pyramid_levels: usize,
    epsg_code: Option<u16>,
    nodata: Option<f32>,
}

impl GeoTiffBuilder {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            bands: 1,
            is_float32: false,
            pyramid_levels: 1,
            epsg_code: None,
            nodata: None,
        }
    }

    pub fn bands(mut self, bands: u16) -> Self {
        self.bands = bands;
        self
    }

    pub fn float32_dem(mut self, nodata: Option<f32>) -> Self {
        self.is_float32 = true;
        self.bands = 1;
        self.nodata = nodata;
        self
    }

    pub fn pyramid_levels(mut self, levels: usize) -> Self {
        self.pyramid_levels = levels.max(1);
        self
    }

    pub fn epsg(mut self, epsg: u16) -> Self {
        self.epsg_code = Some(epsg);
        self
    }

    pub fn build(self) -> Vec<u8> {
        let mut out = Vec::new();
        // Little-endian header
        out.extend_from_slice(b"II\x2a\x00");
        // Offset to IFD0 is 8
        out.extend_from_slice(&8u32.to_le_bytes());

        let mut current_w = self.width;
        let mut current_h = self.height;

        for level in 0..self.pyramid_levels {
            let is_last = level + 1 == self.pyramid_levels;
            let mut entries = Vec::new();

            // Tag 256: ImageWidth
            entries.push((256u16, 4u16, 1u32, current_w));
            // Tag 257: ImageLength
            entries.push((257u16, 4u16, 1u32, current_h));
            // Tag 258: BitsPerSample
            let bits = if self.is_float32 { 32u32 } else { 8u32 };
            entries.push((258u16, 3u16, 1u32, bits));
            // Tag 259: Compression (1 = No compression)
            entries.push((259u16, 3u16, 1u32, 1u32));
            // Tag 262: PhotometricInterpretation (1 = BlackIsZero, 2 = RGB)
            let photometric = if self.bands >= 3 { 2u32 } else { 1u32 };
            entries.push((262u16, 3u16, 1u32, photometric));
            // Tag 277: SamplesPerPixel
            entries.push((277u16, 3u16, 1u32, self.bands as u32));
            // Tag 278: RowsPerStrip
            entries.push((278u16, 4u16, 1u32, current_h));

            if self.is_float32 {
                // Tag 339: SampleFormat (3 = IEEE floating point)
                entries.push((339u16, 3u16, 1u32, 3u32));
            }

            // Tag 279: StripByteCounts
            let bytes_per_sample = if self.is_float32 { 4 } else { 1 };
            let strip_len = current_w * current_h * (self.bands as u32) * bytes_per_sample;
            entries.push((279u16, 4u16, 1u32, strip_len));

            // Tag 273: StripOffsets (Placeholder, will patch)
            entries.push((273u16, 4u16, 1u32, 0u32));

            // GeoKeyDirectoryTag (34735) if EPSG is present on IFD0
            if level == 0 && self.epsg_code.is_some() {
                // Placeholder for GeoKey tag
                entries.push((34735u16, 3u16, 4u32, self.epsg_code.unwrap() as u32));
            }

            // Sort entries by tag ID as required by TIFF standard
            entries.sort_by_key(|e| e.0);

            let num_entries = entries.len() as u16;
            let ifd_start = out.len();
            out.extend_from_slice(&num_entries.to_le_bytes());

            let entries_bytes_len = entries.len() * 12;
            let next_ifd_offset_pos = ifd_start + 2 + entries_bytes_len;
            let strip_data_pos = next_ifd_offset_pos + 4;

            // Write entries with placeholder strip offset
            for entry in &entries {
                out.extend_from_slice(&entry.0.to_le_bytes());
                out.extend_from_slice(&entry.1.to_le_bytes());
                out.extend_from_slice(&entry.2.to_le_bytes());
                if entry.0 == 273 {
                    out.extend_from_slice(&(strip_data_pos as u32).to_le_bytes());
                } else {
                    out.extend_from_slice(&entry.3.to_le_bytes());
                }
            }

            let next_ifd_offset = if is_last {
                0u32
            } else {
                (strip_data_pos + strip_len as usize) as u32
            };
            out.extend_from_slice(&next_ifd_offset.to_le_bytes());

            // Write raster strip data
            if self.is_float32 {
                // Generate gradient elevation values (e.g. 500.0m to 2500.0m)
                for y in 0..current_h {
                    for x in 0..current_w {
                        let val: f32 = if let Some(nodata) = self.nodata {
                            if x == 0 && y == 0 { nodata } else { 500.0 + (x + y) as f32 * 5.0 }
                        } else {
                            500.0 + (x + y) as f32 * 5.0
                        };
                        out.extend_from_slice(&val.to_le_bytes());
                    }
                }
            } else {
                // Write 8-bit optical or grayscale data
                for y in 0..current_h {
                    for x in 0..current_w {
                        for b in 0..self.bands {
                            let val = ((x * 255 / current_w.max(1)) + (y * 255 / current_h.max(1)) + (b as u32 * 40)) as u8;
                            out.push(val);
                        }
                    }
                }
            }

            // Overview downsampling for next pyramid level
            current_w = (current_w / 2).max(1);
            current_h = (current_h / 2).max(1);
        }

        out
    }
}

/// Convenience functions for common GeoTIFF fixtures
pub fn sample_dem_float32(w: u32, h: u32) -> Vec<u8> {
    GeoTiffBuilder::new(w, h).float32_dem(Some(-9999.0)).epsg(4326).build()
}

pub fn sample_multiband_optical(w: u32, h: u32) -> Vec<u8> {
    GeoTiffBuilder::new(w, h).bands(3).epsg(32632).build()
}

pub fn sample_cog_pyramidal(w: u32, h: u32, levels: usize) -> Vec<u8> {
    GeoTiffBuilder::new(w, h).bands(3).pyramid_levels(levels).epsg(32632).build()
}

pub fn sample_corrupted_geotiff() -> Vec<u8> {
    let mut data = sample_multiband_optical(64, 64);
    // Corrupt the header magic and IFD count
    data[2] = 0xFF;
    data[3] = 0xFF;
    data.truncate(20);
    data
}
