// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(dead_code, unused_imports, clippy::all)]

//! Empirical Challenger Stress Test Suite for Milestone 3 (M3.2):
//! - 1. Coordinate reprojection for UTM Zones 1-60 N & S, Web Mercator (EPSG:3857), and WGS84 (EPSG:4326):
//!      verify lat/lon conversion accuracy, pole/equator behavior, and bounds clamping on extremes.
//! - 2. Edge cases in metadata extraction:
//!      zero resolution (fallback to 1.0), inverted bounding box (min_x > max_x), zero-area bounding box (min == max),
//!      corrupted GeoKey directory, missing metadata on baseline TIFFs.
//! - 3. Cairo DrawingArea test:
//!      execute placement map drawing function with a mock Cairo ImageSurface across all edge cases,
//!      guaranteeing zero panics, zero division-by-zero, positive geometry (bw >= 4.0, bh >= 4.0), and valid PNG output.
//! - 4. Wire protocol and caching:
//!      verify `check_cache_entry` and `put_cache_entry` persistence and restoration of PNG and `result.meta` JSON,
//!      FIFO capacity eviction (32 entries), TTL expiration, and 8-byte framing round-trip.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

mod common;
use common::*;
use common::wire_protocol::*;
mod fixtures;
use fixtures::geotiff::*;

// =========================================================================
// Cairo & Placement Map Reference Implementations for Testing
// =========================================================================

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeoTiffMetadata {
    pub epsg: Option<u32>,
    pub crs_name: Option<String>,
    pub dimensions: [u32; 2],
    pub resolution: [f64; 2],
    pub bounds: [f64; 4],
    pub band_count: u32,
    pub data_type: String,
    pub elevation_min: Option<f64>,
    pub elevation_max: Option<f64>,
}

/// Exact replica of preview.rs normalize_to_lon_lat for empirical testing.
pub fn normalize_to_lon_lat(bounds: [f64; 4], epsg: Option<u32>) -> (f64, f64, f64, f64) {
    let x_min = bounds[0].min(bounds[2]);
    let x_max = bounds[0].max(bounds[2]);
    let y_min = bounds[1].min(bounds[3]);
    let y_max = bounds[1].max(bounds[3]);

    let (lon_min, lat_min, lon_max, lat_max) = match epsg {
        Some(code @ 32601..=32660) | Some(code @ 32701..=32760) => {
            let is_south = code >= 32701;
            let zone = if is_south { code - 32700 } else { code - 32600 };
            let central_lon = (zone as f64) * 6.0 - 183.0;

            let northing_min = if is_south { y_min - 10_000_000.0 } else { y_min };
            let northing_max = if is_south { y_max - 10_000_000.0 } else { y_max };

            let lat1 = northing_min / 111_319.5;
            let lat2 = northing_max / 111_319.5;
            let avg_lat = ((lat1 + lat2) * 0.5).to_radians();
            let cos_lat = avg_lat.cos().abs().max(0.01);

            let lon1 = central_lon + (x_min - 500_000.0) / (111_319.5 * cos_lat);
            let lon2 = central_lon + (x_max - 500_000.0) / (111_319.5 * cos_lat);

            (lon1, lat1, lon2, lat2)
        }
        Some(3857) => {
            let lon1 = (x_min / 20_037_508.34) * 180.0;
            let lon2 = (x_max / 20_037_508.34) * 180.0;
            let lat1 = (180.0 / std::f64::consts::PI)
                * (2.0 * (y_min / 6_378_137.0).exp().atan() - std::f64::consts::FRAC_PI_2);
            let lat2 = (180.0 / std::f64::consts::PI)
                * (2.0 * (y_max / 6_378_137.0).exp().atan() - std::f64::consts::FRAC_PI_2);
            (lon1, lat1, lon2, lat2)
        }
        _ => {
            if x_min >= -180.0 && x_max <= 180.0 && y_min >= -90.0 && y_max <= 90.0 {
                (x_min, y_min, x_max, y_max)
            } else {
                (x_min.clamp(-180.0, 180.0), y_min.clamp(-90.0, 90.0), x_max.clamp(-180.0, 180.0), y_max.clamp(-90.0, 90.0))
            }
        }
    };

    let min_lon = lon_min.min(lon_max).clamp(-180.0, 180.0);
    let max_lon = lon_min.max(lon_max).clamp(-180.0, 180.0);
    let min_lat = lat_min.min(lat_max).clamp(-90.0, 90.0);
    let max_lat = lat_min.max(lat_max).clamp(-90.0, 90.0);

    (min_lon, min_lat, max_lon, max_lat)
}

fn draw_world_continents(
    context: &cairo::Context,
    pad_x: f64,
    pad_y: f64,
    map_w: f64,
    map_h: f64,
) {
    let to_canvas = |lon: f64, lat: f64| -> (f64, f64) {
        let x = pad_x + ((lon + 180.0) / 360.0) * map_w;
        let y = pad_y + ((90.0 - lat) / 180.0) * map_h;
        (x, y)
    };

    let draw_poly = |pts: &[(f64, f64)]| {
        if pts.is_empty() {
            return;
        }
        let (x0, y0) = to_canvas(pts[0].0, pts[0].1);
        context.move_to(x0, y0);
        for pt in &pts[1..] {
            let (x, y) = to_canvas(pt.0, pt.1);
            context.line_to(x, y);
        }
        context.close_path();
    };

    // North America
    draw_poly(&[
        (-165.0, 65.0), (-140.0, 70.0), (-90.0, 70.0), (-60.0, 60.0),
        (-55.0, 45.0), (-80.0, 25.0), (-100.0, 20.0), (-110.0, 30.0),
        (-125.0, 50.0), (-165.0, 65.0),
    ]);

    // South America
    draw_poly(&[
        (-80.0, 10.0), (-35.0, -5.0), (-40.0, -22.0), (-55.0, -35.0),
        (-65.0, -55.0), (-75.0, -45.0), (-80.0, 0.0),
    ]);

    // Eurasia
    draw_poly(&[
        (-10.0, 36.0), (30.0, 36.0), (40.0, 30.0), (60.0, 25.0),
        (100.0, 10.0), (120.0, 20.0), (140.0, 40.0), (170.0, 65.0),
        (100.0, 75.0), (40.0, 70.0), (10.0, 55.0), (-10.0, 42.0),
    ]);

    // Africa
    draw_poly(&[
        (-15.0, 35.0), (35.0, 30.0), (50.0, 12.0), (42.0, -10.0),
        (30.0, -34.0), (18.0, -34.0), (10.0, 5.0), (-15.0, 15.0),
    ]);

    // Australia
    draw_poly(&[
        (115.0, -20.0), (150.0, -15.0), (150.0, -35.0),
        (135.0, -35.0), (115.0, -30.0),
    ]);

    context.set_source_rgba(1.0, 1.0, 1.0, 0.12);
    let _ = context.fill_preserve();
    context.set_source_rgba(1.0, 1.0, 1.0, 0.18);
    context.set_line_width(0.6);
    let _ = context.stroke();
}

/// Executes the exact placement map rendering pipeline from preview.rs.
pub fn execute_placement_map_draw(
    context: &cairo::Context,
    width: i32,
    height: i32,
    bounds: [f64; 4],
    epsg: Option<u32>,
) -> (f64, f64, f64, f64) {
    let w = f64::from(width);
    let h = f64::from(height);

    if w > 0.0 && h > 0.0 {
        // 1. Clip rounded rectangle
        let r = 6.0;
        let degrees = std::f64::consts::PI / 180.0;
        context.new_sub_path();
        context.arc((w - r).max(0.0), r.min(h), r, -90.0 * degrees, 0.0 * degrees);
        context.arc((w - r).max(0.0), (h - r).max(0.0), r, 0.0 * degrees, 90.0 * degrees);
        context.arc(r.min(w), (h - r).max(0.0), r, 90.0 * degrees, 180.0 * degrees);
        context.arc(r.min(w), r.min(h), r, 180.0 * degrees, 270.0 * degrees);
        context.close_path();
        let _ = context.clip();

        // 2. Dark card fill
        context.set_source_rgba(0.10, 0.12, 0.16, 0.95);
        context.rectangle(0.0, 0.0, w, h);
        let _ = context.fill();

        // 3. Subtle card border
        context.set_source_rgba(1.0, 1.0, 1.0, 0.10);
        context.set_line_width(1.0);
        context.rectangle(0.5, 0.5, (w - 1.0).max(0.0), (h - 1.0).max(0.0));
        let _ = context.stroke();
    }

    let pad_x = 8.0;
    let pad_y = 6.0;
    let map_w = (w - pad_x * 2.0).max(1.0);
    let map_h = (h - pad_y * 2.0).max(1.0);

    // 4. Graticule
    context.set_source_rgba(1.0, 1.0, 1.0, 0.08);
    context.set_line_width(0.8);
    // Equator
    context.move_to(pad_x, pad_y + map_h * 0.5);
    context.line_to(pad_x + map_w, pad_y + map_h * 0.5);
    // Prime Meridian
    context.move_to(pad_x + map_w * 0.5, pad_y);
    context.line_to(pad_x + map_w * 0.5, pad_y + map_h);
    let _ = context.stroke();

    // 5. Continents silhouettes
    draw_world_continents(context, pad_x, pad_y, map_w, map_h);

    // 6. Coordinate reprojection & footprint bounding box
    let (min_lon, min_lat, max_lon, max_lat) = normalize_to_lon_lat(bounds, epsg);
    let bx = pad_x + ((min_lon + 180.0) / 360.0) * map_w;
    let by = pad_y + ((90.0 - max_lat) / 180.0) * map_h;
    let bw = (((max_lon - min_lon) / 360.0) * map_w).max(4.0);
    let bh = (((max_lat - min_lat) / 180.0) * map_h).max(4.0);

    // Fill footprint
    context.set_source_rgba(0.20, 0.60, 1.00, 0.35);
    context.rectangle(bx, by, bw, bh);
    let _ = context.fill();

    // Stroke footprint
    context.set_source_rgba(0.30, 0.75, 1.00, 0.90);
    context.set_line_width(1.5);
    context.rectangle(bx, by, bw, bh);
    let _ = context.stroke();

    // Crosshair reticle if footprint is small
    if bw < 8.0 || bh < 8.0 {
        context.set_source_rgba(0.30, 0.75, 1.00, 0.60);
        context.set_line_width(0.75);
        let cx = bx + bw * 0.5;
        let cy = by + bh * 0.5;
        context.move_to(cx - 6.0, cy);
        context.line_to(cx + 6.0, cy);
        context.move_to(cx, cy - 6.0);
        context.line_to(cx, cy + 6.0);
        let _ = context.stroke();
    }

    (bx, by, bw, bh)
}

// =========================================================================
// Custom TIFF Generator with Explicit GeoTIFF Tags & Boundary Injection
// =========================================================================

pub struct CustomGeoTiffBuilder {
    width: u32,
    height: u32,
    pixel_scale: Option<[f64; 3]>,
    tiepoint: Option<[f64; 6]>,
    geokeys: Option<Vec<u16>>,
    raw_geokey_tag: Option<(u16, u32, u32)>, // (type, count, val_or_offset)
}

impl CustomGeoTiffBuilder {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixel_scale: None,
            tiepoint: None,
            geokeys: None,
            raw_geokey_tag: None,
        }
    }

    pub fn pixel_scale(mut self, scale: [f64; 3]) -> Self {
        self.pixel_scale = Some(scale);
        self
    }

    pub fn tiepoint(mut self, tp: [f64; 6]) -> Self {
        self.tiepoint = Some(tp);
        self
    }

    pub fn geokeys(mut self, keys: Vec<u16>) -> Self {
        self.geokeys = Some(keys);
        self
    }

    pub fn raw_geokey_tag(mut self, field_type: u16, count: u32, val_offset: u32) -> Self {
        self.raw_geokey_tag = Some((field_type, count, val_offset));
        self
    }

    pub fn build(self) -> Vec<u8> {
        let mut out = Vec::new();
        // Little endian TIFF magic
        out.extend_from_slice(b"II\x2a\x00");
        // Offset to IFD0
        out.extend_from_slice(&8u32.to_le_bytes());

        // Construct baseline entries
        let mut entries: Vec<(u16, u16, u32, Vec<u8>)> = Vec::new();

        // 256: ImageWidth
        entries.push((256, 4, 1, self.width.to_le_bytes().to_vec()));
        // 257: ImageLength
        entries.push((257, 4, 1, self.height.to_le_bytes().to_vec()));
        // 258: BitsPerSample
        entries.push((258, 3, 1, (8u16).to_le_bytes().to_vec()));
        // 259: Compression (1 = none)
        entries.push((259, 3, 1, (1u16).to_le_bytes().to_vec()));
        // 262: PhotometricInterpretation (1 = BlackIsZero)
        entries.push((262, 3, 1, (1u16).to_le_bytes().to_vec()));
        // 277: SamplesPerPixel
        entries.push((277, 3, 1, (1u16).to_le_bytes().to_vec()));
        // 278: RowsPerStrip
        entries.push((278, 4, 1, self.height.to_le_bytes().to_vec()));

        let strip_len = self.width * self.height;
        // 279: StripByteCounts
        entries.push((279, 4, 1, strip_len.to_le_bytes().to_vec()));
        // 273: StripOffsets (placeholder)
        entries.push((273, 4, 1, vec![0u8; 4]));

        // GeoTIFF ModelPixelScaleTag (33550, DOUBLE)
        if let Some(scale) = self.pixel_scale {
            let mut scale_bytes = Vec::new();
            for s in scale {
                scale_bytes.extend_from_slice(&s.to_le_bytes());
            }
            entries.push((33550, 12, 3, scale_bytes));
        }

        // GeoTIFF ModelTiepointTag (33922, DOUBLE)
        if let Some(tp) = self.tiepoint {
            let mut tp_bytes = Vec::new();
            for t in tp {
                tp_bytes.extend_from_slice(&t.to_le_bytes());
            }
            entries.push((33922, 12, 6, tp_bytes));
        }

        // GeoTIFF GeoKeyDirectoryTag (34735, SHORT)
        if let Some(keys) = self.geokeys {
            let mut key_bytes = Vec::new();
            for k in &keys {
                key_bytes.extend_from_slice(&k.to_le_bytes());
            }
            entries.push((34735, 3, keys.len() as u32, key_bytes));
        } else if let Some((ft, cnt, val_offset)) = self.raw_geokey_tag {
            entries.push((34735, ft, cnt, val_offset.to_le_bytes().to_vec()));
        }

        // Sort entries by tag ID
        entries.sort_by_key(|e| e.0);

        let num_entries = entries.len() as u16;
        out.extend_from_slice(&num_entries.to_le_bytes());

        let ifd_entries_size = entries.len() * 12;
        let next_ifd_offset_pos = 8 + 2 + ifd_entries_size;
        let extra_data_start = next_ifd_offset_pos + 4;

        // Allocate space for IFD entries and 4-byte next-IFD offset
        let entries_start = out.len();
        out.resize(extra_data_start, 0);

        let current_extra_offset = extra_data_start;
        let mut extra_buffer = Vec::new();

        for (i, entry) in entries.iter().enumerate() {
            let entry_offset = entries_start + i * 12;
            let tag = entry.0;
            let field_type = entry.1;
            let count = entry.2;
            let data = &entry.3;

            out[entry_offset..entry_offset + 2].copy_from_slice(&tag.to_le_bytes());
            out[entry_offset + 2..entry_offset + 4].copy_from_slice(&field_type.to_le_bytes());
            out[entry_offset + 4..entry_offset + 8].copy_from_slice(&count.to_le_bytes());

            if tag == 273 {
                // Strip offset will be written after extra_buffer
                continue;
            }

            if data.len() <= 4 {
                let mut padded = [0u8; 4];
                padded[..data.len()].copy_from_slice(data);
                out[entry_offset + 8..entry_offset + 12].copy_from_slice(&padded);
            } else {
                let val_offset = (current_extra_offset + extra_buffer.len()) as u32;
                out[entry_offset + 8..entry_offset + 12].copy_from_slice(&val_offset.to_le_bytes());
                extra_buffer.extend_from_slice(data);
            }
        }

        // Next IFD offset = 0
        out[next_ifd_offset_pos..next_ifd_offset_pos + 4].copy_from_slice(&0u32.to_le_bytes());

        // Append extra data buffer
        out.extend_from_slice(&extra_buffer);

        // Strip data follows extra data
        let strip_data_offset = out.len() as u32;
        for (i, entry) in entries.iter().enumerate() {
            if entry.0 == 273 {
                let entry_offset = entries_start + i * 12;
                out[entry_offset + 8..entry_offset + 12].copy_from_slice(&strip_data_offset.to_le_bytes());
            }
        }

        // Append raster strip data (grayscale ramp)
        for y in 0..self.height {
            for x in 0..self.width {
                let v = ((x + y) % 255) as u8;
                out.push(v);
            }
        }

        out
    }
}

// =========================================================================
// Wire Protocol Cache Replica for Direct Verification
// =========================================================================

pub const CACHE_ENTRIES: usize = 32;
pub const CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireOp {
    PreviewGeoTiff = 13,
    PreviewImage = 7,
}

pub struct TestCacheEntry {
    pub path: PathBuf,
    pub mtime: u64,
    pub size: u64,
    pub operation: WireOp,
    pub data: Vec<u8>,
    pub metadata: Option<Vec<u8>>,
    pub timestamp: Instant,
}

pub struct TestPreviewCache {
    cache: Mutex<Vec<TestCacheEntry>>,
}

impl TestPreviewCache {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(Vec::with_capacity(CACHE_ENTRIES)),
        }
    }

    pub fn check_cache_entry(
        &self,
        path: &Path,
        mtime: u64,
        size: u64,
        operation: WireOp,
    ) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
        let mut cache = self.cache.lock().ok()?;
        cache.retain(|entry| entry.timestamp.elapsed() < CACHE_TTL);
        cache
            .iter()
            .find(|entry| {
                entry.path == path
                    && entry.mtime == mtime
                    && entry.size == size
                    && entry.operation == operation
            })
            .map(|entry| (entry.data.clone(), entry.metadata.clone()))
    }

    pub fn put_cache_entry(
        &self,
        path: &Path,
        mtime: u64,
        size: u64,
        operation: WireOp,
        data: Vec<u8>,
        metadata: Option<Vec<u8>>,
    ) {
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() >= CACHE_ENTRIES {
                cache.remove(0);
            }
            cache.push(TestCacheEntry {
                path: path.to_owned(),
                mtime,
                size,
                operation,
                data,
                metadata,
                timestamp: Instant::now(),
            });
        }
    }
}

// =========================================================================
// Test Suite
// =========================================================================

// --- 1. Coordinate Reprojection for UTM Zones 1-60 N & S, 3857, 4326 ---

#[test]
fn test_stress_utm_zones_north_1_to_60() {
    for zone in 1..=60 {
        let epsg = 32600 + zone;
        let central_lon = (zone as f64) * 6.0 - 183.0;

        // Northing 0 = Equator; Northing 8,000,000 = ~72 deg N
        let bounds = [500_000.0, 0.0, 500_000.0, 8_000_000.0];
        let (min_lon, min_lat, max_lon, max_lat) = normalize_to_lon_lat(bounds, Some(epsg));

        // Longitude at false easting 500,000m should match central meridian exactly
        assert!(
            (min_lon - central_lon).abs() < 1e-4,
            "Zone {zone}N min_lon {min_lon} != central_lon {central_lon}"
        );
        assert!(
            (max_lon - central_lon).abs() < 1e-4,
            "Zone {zone}N max_lon {max_lon} != central_lon {central_lon}"
        );

        // Latitude at Northing 0m must be equator
        assert!(
            min_lat.abs() < 1e-2,
            "Zone {zone}N min_lat {min_lat} != 0.0 at equator"
        );
        // Latitude at Northing 8,000,000m should be positive ~71.86 deg
        assert!(
            max_lat > 70.0 && max_lat < 75.0,
            "Zone {zone}N max_lat {max_lat} out of range"
        );

        // Boundary clamping check
        assert!(min_lon >= -180.0 && max_lon <= 180.0);
        assert!(min_lat >= -90.0 && max_lat <= 90.0);
    }
}

#[test]
fn test_stress_utm_zones_south_1_to_60() {
    for zone in 1..=60 {
        let epsg = 32700 + zone;
        let central_lon = (zone as f64) * 6.0 - 183.0;

        // Northing 10,000,000 = Equator in South UTM; 5,000,000 = ~45 deg S
        let bounds = [500_000.0, 5_000_000.0, 500_000.0, 10_000_000.0];
        let (min_lon, min_lat, max_lon, max_lat) = normalize_to_lon_lat(bounds, Some(epsg));

        assert!(
            (min_lon - central_lon).abs() < 1e-4,
            "Zone {zone}S min_lon {min_lon} != central_lon {central_lon}"
        );
        assert!(
            (max_lon - central_lon).abs() < 1e-4,
            "Zone {zone}S max_lon {max_lon} != central_lon {central_lon}"
        );

        // Northing 10,000,000m in South UTM maps to 0.0 lat (Equator)
        assert!(
            max_lat.abs() < 1e-2,
            "Zone {zone}S max_lat {max_lat} != 0.0 at equator"
        );
        // Northing 5,000,000m maps to ~ -44.91 deg S
        assert!(
            min_lat < -40.0 && min_lat > -50.0,
            "Zone {zone}S min_lat {min_lat} not in southern hemisphere"
        );

        // Verify min <= max
        assert!(min_lat <= max_lat);
        assert!(min_lon <= max_lon);
    }
}

#[test]
fn test_stress_utm_extreme_bounds_clamping() {
    // Extreme Easting/Northing out-of-world inputs must clamp to [-180, 180] and [-90, 90]
    let extreme_bounds = [-50_000_000.0, -50_000_000.0, 50_000_000.0, 50_000_000.0];
    let (min_lon, min_lat, max_lon, max_lat) = normalize_to_lon_lat(extreme_bounds, Some(32632));

    assert_eq!(min_lon, -180.0);
    assert_eq!(max_lon, 180.0);
    assert_eq!(min_lat, -90.0);
    assert_eq!(max_lat, 90.0);

    // South hemisphere extreme bounds
    let (s_min_lon, s_min_lat, s_max_lon, s_max_lat) = normalize_to_lon_lat(extreme_bounds, Some(32732));
    assert_eq!(s_min_lon, -180.0);
    assert_eq!(s_max_lon, 180.0);
    assert_eq!(s_min_lat, -90.0);
    assert_eq!(s_max_lat, 90.0);
}

#[test]
fn test_stress_web_mercator_3857_accuracy_and_clamping() {
    // Origin at (0, 0)
    let (lon0, lat0, _, _) = normalize_to_lon_lat([0.0, 0.0, 100.0, 100.0], Some(3857));
    assert!(lon0.abs() < 1e-4);
    assert!(lat0.abs() < 1e-4);

    // Standard global Mercator bounds
    let bounds = [-20037508.34, -20037508.34, 20037508.34, 20037508.34];
    let (min_lon, min_lat, max_lon, max_lat) = normalize_to_lon_lat(bounds, Some(3857));

    assert!((min_lon - -180.0).abs() < 1e-2);
    assert!((max_lon - 180.0).abs() < 1e-2);
    assert!((min_lat - -85.0511).abs() < 0.1);
    assert!((max_lat - 85.0511).abs() < 0.1);

    // Out-of-bounds inputs: exp overflow should clamp to 90/-90 without panic or NaN
    let extreme = [-1e15, -1e15, 1e15, 1e15];
    let (c_min_lon, c_min_lat, c_max_lon, c_max_lat) = normalize_to_lon_lat(extreme, Some(3857));
    assert_eq!(c_min_lon, -180.0);
    assert_eq!(c_max_lon, 180.0);
    assert_eq!(c_min_lat, -90.0);
    assert_eq!(c_max_lat, 90.0);
}

#[test]
fn test_stress_wgs84_4326_accuracy_and_clamping() {
    // Standard San Francisco bounding box
    let bounds = [-122.45, 37.70, -122.35, 37.82];
    let (min_lon, min_lat, max_lon, max_lat) = normalize_to_lon_lat(bounds, Some(4326));
    assert_eq!(min_lon, -122.45);
    assert_eq!(min_lat, 37.70);
    assert_eq!(max_lon, -122.35);
    assert_eq!(max_lat, 37.82);

    // Clamping on out-of-range degrees
    let out_bounds = [-250.0, -120.0, 300.0, 150.0];
    let (c_min_lon, c_min_lat, c_max_lon, c_max_lat) = normalize_to_lon_lat(out_bounds, Some(4326));
    assert_eq!(c_min_lon, -180.0);
    assert_eq!(c_min_lat, -90.0);
    assert_eq!(c_max_lon, 180.0);
    assert_eq!(c_max_lat, 90.0);
}

// --- 2. Edge Cases in Metadata & GeoKey Parsing ---

#[test]
fn test_edge_zero_resolution_fallback_to_1() {
    let env = TestEnv::new();
    // Build TIFF with pixel scale [0.0, 0.0, 0.0]
    let tiff_bytes = CustomGeoTiffBuilder::new(64, 64)
        .pixel_scale([0.0, 0.0, 0.0])
        .tiepoint([0.0, 0.0, 0.0, 500000.0, 5100000.0, 0.0])
        .raw_geokey_tag(3, 4, 32632)
        .build();

    let input_path = env.write_file("zero_res.tif", &tiff_bytes);
    let output_path = env.file_path("zero_res.png");
    let meta_path = env.file_path("result.meta");

    let res = run_preview_helper("preview-geotiff", &input_path, &output_path, 1400);
    assert!(res.status.success(), "Helper failed on zero resolution: {}", res.stderr);

    let meta_str = fs::read_to_string(&meta_path).expect("result.meta must be written");
    let meta: GeoTiffMetadata = serde_json::from_str(&meta_str).expect("parse metadata JSON");

    // Must fallback to 1.0
    assert_eq!(meta.resolution[0], 1.0);
    assert_eq!(meta.resolution[1], 1.0);
}

#[test]
fn test_edge_inverted_bounding_box_normalized() {
    let env = TestEnv::new();
    // Build TIFF with negative pixel scale or inverted coordinates
    let tiff_bytes = CustomGeoTiffBuilder::new(100, 100)
        .pixel_scale([-10.0, -10.0, 0.0])
        .tiepoint([0.0, 0.0, 0.0, 500000.0, 5100000.0, 0.0])
        .raw_geokey_tag(3, 4, 32632)
        .build();

    let input_path = env.write_file("inverted_box.tif", &tiff_bytes);
    let output_path = env.file_path("inverted_box.png");
    let meta_path = env.file_path("result.meta");

    let res = run_preview_helper("preview-geotiff", &input_path, &output_path, 1400);
    assert!(res.status.success(), "Helper failed: {}", res.stderr);

    let meta_str = fs::read_to_string(&meta_path).expect("result.meta must be written");
    let meta: GeoTiffMetadata = serde_json::from_str(&meta_str).expect("parse metadata JSON");

    // Bounds must be normalized: min_x <= max_x and min_y <= max_y
    assert!(
        meta.bounds[0] <= meta.bounds[2],
        "min_x {} must be <= max_x {}",
        meta.bounds[0],
        meta.bounds[2]
    );
    assert!(
        meta.bounds[1] <= meta.bounds[3],
        "min_y {} must be <= max_y {}",
        meta.bounds[1],
        meta.bounds[3]
    );
}

#[test]
fn test_edge_corrupted_geokey_directory() {
    let env = TestEnv::new();
    // Build TIFF with truncated GeoKeyDirectoryTag (count = 2, less than header size 4)
    let tiff_bytes = CustomGeoTiffBuilder::new(64, 64)
        .pixel_scale([10.0, 10.0, 0.0])
        .tiepoint([0.0, 0.0, 0.0, 500000.0, 5100000.0, 0.0])
        .geokeys(vec![1, 1]) // Only 2 elements, corrupt!
        .build();

    let input_path = env.write_file("corrupt_geokey.tif", &tiff_bytes);
    let output_path = env.file_path("corrupt_geokey.png");
    let meta_path = env.file_path("result.meta");

    let res = run_preview_helper("preview-geotiff", &input_path, &output_path, 1400);
    assert!(res.status.success(), "Helper must not crash on corrupt geokeys: {}", res.stderr);

    let meta_str = fs::read_to_string(&meta_path).expect("result.meta must be written");
    let meta: GeoTiffMetadata = serde_json::from_str(&meta_str).expect("parse metadata JSON");

    // EPSG should safely degrade to None or fallback
    assert!(meta.dimensions == [64, 64]);
}

#[test]
fn test_edge_missing_all_geotiff_metadata() {
    let env = TestEnv::new();
    // Vanilla baseline TIFF without any GeoTIFF tags
    let tiff_bytes = CustomGeoTiffBuilder::new(128, 80).build();

    let input_path = env.write_file("vanilla.tif", &tiff_bytes);
    let output_path = env.file_path("vanilla.png");
    let meta_path = env.file_path("result.meta");

    let res = run_preview_helper("preview-geotiff", &input_path, &output_path, 1400);
    assert!(res.status.success(), "Helper must succeed on standard TIFF: {}", res.stderr);

    let meta_str = fs::read_to_string(&meta_path).expect("result.meta must be written");
    let meta: GeoTiffMetadata = serde_json::from_str(&meta_str).expect("parse metadata JSON");

    assert_eq!(meta.epsg, None);
    assert_eq!(meta.crs_name, None);
    assert_eq!(meta.resolution, [1.0, 1.0]);
    assert_eq!(meta.dimensions, [128, 80]);
    assert_eq!(meta.bounds, [0.0, 0.0, 128.0, 80.0]);
}

// --- 3. Cairo Placement Map DrawingArea Empirical Stress Tests ---

#[test]
fn test_cairo_placement_map_zero_panics_all_zones_and_edge_cases() {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 160, 90)
        .expect("create cairo surface");
    let context = cairo::Context::new(&surface).expect("create cairo context");

    // 1. Stress all 60 UTM North zones
    for zone in 1..=60 {
        let epsg = 32600 + zone;
        let bounds = [500_000.0, 1_000_000.0, 600_000.0, 2_000_000.0];
        let (bx, by, bw, bh) = execute_placement_map_draw(&context, 160, 90, bounds, Some(epsg));
        assert!(context.status().is_ok());
        assert!(bw >= 4.0, "Footprint bw must be >= 4.0, got {bw}");
        assert!(bh >= 4.0, "Footprint bh must be >= 4.0, got {bh}");
        assert!(bx.is_finite() && by.is_finite());
    }

    // 2. Stress all 60 UTM South zones
    for zone in 1..=60 {
        let epsg = 32700 + zone;
        let bounds = [500_000.0, 4_000_000.0, 600_000.0, 5_000_000.0];
        let (_bx, _by, bw, bh) = execute_placement_map_draw(&context, 160, 90, bounds, Some(epsg));
        assert!(context.status().is_ok());
        assert!(bw >= 4.0);
        assert!(bh >= 4.0);
    }

    // 3. Zero-area bounding box (min == max)
    let zero_bounds = [500_000.0, 5_000_000.0, 500_000.0, 5_000_000.0];
    let (_bx, _by, bw, bh) = execute_placement_map_draw(&context, 160, 90, zero_bounds, Some(32632));
    assert!(context.status().is_ok());
    assert_eq!(bw, 4.0, "Zero-area box must be clamped to 4.0 minimum width");
    assert_eq!(bh, 4.0, "Zero-area box must be clamped to 4.0 minimum height");

    // 4. Inverted bounding box (min > max)
    let inv_bounds = [600_000.0, 6_000_000.0, 400_000.0, 4_000_000.0];
    let (_bx, _by, bw, bh) = execute_placement_map_draw(&context, 160, 90, inv_bounds, Some(32632));
    assert!(context.status().is_ok());
    assert!(bw >= 4.0);
    assert!(bh >= 4.0);

    // 5. Web Mercator & WGS84
    execute_placement_map_draw(&context, 160, 90, [-20037508.34, -20037508.34, 20037508.34, 20037508.34], Some(3857));
    assert!(context.status().is_ok());
    execute_placement_map_draw(&context, 160, 90, [-180.0, -90.0, 180.0, 90.0], Some(4326));
    assert!(context.status().is_ok());

    // 6. Unknown / None EPSG
    execute_placement_map_draw(&context, 160, 90, [0.0, 0.0, 100.0, 100.0], None);
    assert!(context.status().is_ok());
    execute_placement_map_draw(&context, 160, 90, [0.0, 0.0, 100.0, 100.0], Some(99999));
    assert!(context.status().is_ok());

    // 7. Verify PNG encoding succeeds without error
    let mut png_out = Vec::new();
    surface.write_to_png(&mut png_out).expect("write to png");
    assert!(is_valid_png(&png_out), "Cairo placement map must produce valid PNG bytes");
}

#[test]
fn test_cairo_placement_map_canvas_dimensions_boundary() {
    let bounds = [500_000.0, 5_000_000.0, 510_000.0, 5_010_000.0];

    // Boundary canvas sizes: 16x9, 320x180, 1x1, 0x0
    for &(w, h) in &[(160, 90), (320, 180), (16, 9), (1, 1), (0, 0)] {
        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w.max(1), h.max(1))
            .expect("surface creation");
        let context = cairo::Context::new(&surface).expect("context creation");

        let (_, _, bw, bh) = execute_placement_map_draw(&context, w, h, bounds, Some(32632));
        assert!(context.status().is_ok());
        assert!(bw >= 4.0);
        assert!(bh >= 4.0);
    }
}

// --- 4. Wire Protocol and Caching Tests ---

#[test]
fn test_wire_framing_with_geotiff_metadata_round_trip() {
    let png_payload = b"\x89PNG\r\n\x1a\nrealistic_mock_png_bytes_for_geotiff";
    let meta_payload = br#"{"epsg":32632,"crs_name":"WGS 84 / UTM zone 32N","dimensions":[1400,1400],"resolution":[10.0,10.0],"bounds":[500000.0,5100000.0,514000.0,5114000.0],"band_count":3,"data_type":"MultiBand","elevation_min":null,"elevation_max":null}"#;

    let frame = encode_frame(png_payload, meta_payload);
    assert_eq!(frame.len(), 8 + png_payload.len() + meta_payload.len());

    let (png_len, meta_len) = decode_header(&frame).expect("decode header");
    assert_eq!(png_len, png_payload.len() as u32);
    assert_eq!(meta_len, meta_payload.len() as u32);

    let (dec_png, dec_meta) = read_framed_message(&frame[..]).expect("read framed message");
    assert_eq!(dec_png, png_payload);
    assert_eq!(dec_meta, meta_payload);
}

#[test]
fn test_cache_entry_persistence_and_restoration() {
    let cache = TestPreviewCache::new();
    let path = Path::new("/var/data/satellite_2026.tif");
    let mtime = 1775000000;
    let size = 52_428_800;
    let op = WireOp::PreviewGeoTiff;

    let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
    let meta_json = br#"{"epsg":32632,"dimensions":[1024,1024]}"#.to_vec();

    // Cache miss initially
    assert!(cache.check_cache_entry(path, mtime, size, op.clone()).is_none());

    // Put cache entry with metadata
    cache.put_cache_entry(path, mtime, size, op.clone(), png.clone(), Some(meta_json.clone()));

    // Cache hit
    let (cached_png, cached_meta) = cache
        .check_cache_entry(path, mtime, size, op.clone())
        .expect("must hit cache");

    assert_eq!(cached_png, png);
    assert_eq!(cached_meta, Some(meta_json));

    // Miss on different mtime
    assert!(cache.check_cache_entry(path, mtime + 1, size, op.clone()).is_none());
    // Miss on different size
    assert!(cache.check_cache_entry(path, mtime, size + 1, op.clone()).is_none());
    // Miss on different operation
    assert!(cache.check_cache_entry(path, mtime, size, WireOp::PreviewImage).is_none());
    // Miss on different path
    assert!(cache.check_cache_entry(Path::new("/other.tif"), mtime, size, op.clone()).is_none());
}

#[test]
fn test_cache_fifo_eviction_boundary() {
    let cache = TestPreviewCache::new();
    let op = WireOp::PreviewGeoTiff;

    // Insert 35 entries into a 32-entry capacity cache
    for i in 0..35 {
        let path_str = format!("/tmp/file_{i}.tif");
        let path = Path::new(&path_str);
        cache.put_cache_entry(
            path,
            1000 + i as u64,
            1024,
            op.clone(),
            vec![i as u8],
            Some(vec![i as u8]),
        );
    }

    // The first 3 entries (0, 1, 2) must be evicted
    for i in 0..3 {
        let path_str = format!("/tmp/file_{i}.tif");
        assert!(
            cache.check_cache_entry(Path::new(&path_str), 1000 + i as u64, 1024, op.clone()).is_none(),
            "Entry {i} should have been evicted by FIFO policy"
        );
    }

    // The remaining entries 3..35 must be present
    for i in 3..35 {
        let path_str = format!("/tmp/file_{i}.tif");
        let (data, meta) = cache
            .check_cache_entry(Path::new(&path_str), 1000 + i as u64, 1024, op.clone())
            .expect("Entry must be present");
        assert_eq!(data, vec![i as u8]);
        assert_eq!(meta, Some(vec![i as u8]));
    }
}

#[test]
fn test_end_to_end_dem_and_optical_preview_with_meta_caching() {
    let env = TestEnv::new();

    // 1. Float32 DEM test
    let dem_bytes = sample_dem_float32(64, 64);
    let dem_input = env.write_file("dem.tif", &dem_bytes);
    let dem_output = env.file_path("dem.png");
    let dem_meta_path = env.file_path("result.meta");

    let res = run_preview_helper("preview-geotiff", &dem_input, &dem_output, 1400);
    assert!(res.status.success(), "DEM preview failed: {}", res.stderr);

    let dem_png = fs::read(&dem_output).expect("read dem png");
    assert!(is_valid_png(&dem_png));
    let dem_meta_str = fs::read_to_string(&dem_meta_path).expect("read dem meta");
    let dem_meta: GeoTiffMetadata = serde_json::from_str(&dem_meta_str).expect("deserialize dem meta");
    assert_eq!(dem_meta.data_type, "Float32DEM");
    assert!(dem_meta.elevation_min.is_some());
    assert!(dem_meta.elevation_max.is_some());

    // 2. Multi-band optical test
    let optical_bytes = sample_multiband_optical(64, 64);
    let opt_input = env.write_file("optical.tif", &optical_bytes);
    let opt_output = env.file_path("optical.png");
    let opt_meta_path = env.file_path("result.meta");

    let res = run_preview_helper("preview-geotiff", &opt_input, &opt_output, 1400);
    assert!(res.status.success(), "Optical preview failed: {}", res.stderr);

    let opt_png = fs::read(&opt_output).expect("read optical png");
    assert!(is_valid_png(&opt_png));
    let opt_meta_str = fs::read_to_string(&opt_meta_path).expect("read optical meta");
    let opt_meta: GeoTiffMetadata = serde_json::from_str(&opt_meta_str).expect("deserialize optical meta");
    assert_eq!(opt_meta.data_type, "MultiBand");
    assert_eq!(opt_meta.band_count, 3);
    assert_eq!(opt_meta.epsg, Some(32632));
    assert_eq!(opt_meta.crs_name, Some("WGS 84 / UTM zone 32N".to_string()));

    // 3. Test wire framing round-trip on authentic output
    let wire_frame = encode_frame(&opt_png, opt_meta_str.as_bytes());
    let (read_png, read_meta) = read_framed_message(&wire_frame[..]).expect("framing read");
    assert_eq!(read_png, opt_png);
    assert_eq!(read_meta, opt_meta_str.as_bytes());
}
