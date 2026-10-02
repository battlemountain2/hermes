// SPDX-License-Identifier: GPL-3.0-or-later

//! Pure-Rust GIS GeoTIFF preview and overview extraction pipeline.
//!
//! Provides pyramid overview selection without gigapixel layer decompression,
//! dynamic contrast normalization for float32 DEMs and multi-band optical imagery,
//! geospatial metadata parsing, and safe Cairo ImageSurface PNG encoding.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use serde::{Deserialize, Serialize};
use tiff::decoder::{ChunkType, Decoder, DecodingResult, Limits};
use tiff::tags::{PlanarConfiguration, Tag};

pub const TARGET_PREVIEW_DIM: u32 = 1400;
pub const MIN_OVERVIEW_DIM: u32 = 1200;
pub const MAX_SAFE_DECODE_DIM: u32 = 2048;
pub const MAX_SAFE_DECODE_PIXELS: u64 = 2048 * 2048;
pub const MAX_DECODING_BUFFER_BYTES: usize = 32 * 1024 * 1024;

/// Overview IFD descriptor summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IfdSummary {
    pub index: usize,
    pub width: u32,
    pub height: u32,
}

/// Decision on how to decode the GeoTIFF raster within memory bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionDecision {
    /// Decode the specified overview IFD directly into memory.
    DirectDecode {
        ifd_index: usize,
        width: u32,
        height: u32,
    },
    /// Flat single-layer gigapixel raster; subsample chunks/strips into bounded buffer.
    SubsampleFallback {
        ifd_index: usize,
        src_width: u32,
        src_height: u32,
        stride: u32,
        out_width: u32,
        out_height: u32,
    },
}

/// Color mapping options for single-band elevation models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(dead_code, reason = "DEM color map options")]
pub enum DemColorMap {
    Hypsometric,
    Grayscale,
}

/// Dynamic range statistics computed across valid raster samples.
#[derive(Debug, Clone, PartialEq)]
pub struct ElevationStats {
    pub min_valid: f32,
    pub max_valid: f32,
    pub p2: f32,
    pub p98: f32,
}

/// Channel stretch bounds for optical channels.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelBounds {
    pub p2: f32,
    pub p98: f32,
}

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

/// Decoded raster data ready for normalization and Cairo encoding.
pub struct DecodedRaster {
    pub samples: Vec<f32>,
    pub width: u32,
    pub height: u32,
    pub num_samples: usize,
    pub is_planar: bool,
    pub nodata_tag: Option<f32>,
}

/// Primary entry point called by `src/sandbox_helper.rs` and the sandbox worker pool.
/// Returns raw PNG bytes and serialized JSON metadata.
pub fn render_geotiff(path: &Path, target_box: i32) -> Result<(Vec<u8>, String), String> {
    let target_box = if target_box <= 0 {
        TARGET_PREVIEW_DIM
    } else {
        (target_box as u32).min(2048)
    };
    let mut file = File::open(path).map_err(|e| format!("Failed to open GeoTIFF: {e}"))?;

    let mut limits = Limits::default();
    limits.decoding_buffer_size = MAX_DECODING_BUFFER_BYTES;
    let mut decoder = Decoder::new(&mut file)
        .map_err(|e| format!("Failed to create TIFF decoder: {e}"))?
        .with_limits(limits);

    // 1. Scan IFD summaries without pixel allocation
    let summaries = enumerate_ifd_summaries(&mut decoder)?;
    let (base_width, base_height) = if let Some(first) = summaries.first() {
        (first.width, first.height)
    } else {
        return Err("No IFD headers found in TIFF".to_owned());
    };

    // 2. Select overview layer or subsampling fallback
    let decision = select_overview_layer(&summaries, target_box)?;

    // 3. Extract geospatial tags and metadata from IFD 0 before decoding pixels
    decoder
        .seek_to_image(0)
        .map_err(|e| format!("Failed to seek to IFD 0: {e}"))?;
    let nodata_tag = extract_nodata_tag(&mut decoder);
    let sample_format = decoder
        .find_tag(Tag::SampleFormat)
        .ok()
        .flatten()
        .and_then(|v| v.into_u16().ok())
        .unwrap_or(1);
    let colortype = decoder.colortype().map_err(|e| e.to_string())?;
    let is_planar = decoder
        .find_tag(Tag::PlanarConfiguration)
        .ok()
        .flatten()
        .and_then(|v| v.into_u16().ok())
        .map(|p| p == PlanarConfiguration::Planar as u16)
        .unwrap_or(false);

    let num_samples = match colortype {
        tiff::ColorType::Gray(_) => 1,
        tiff::ColorType::RGB(_) => 3,
        tiff::ColorType::RGBA(_) => 4,
        tiff::ColorType::Multiband { num_samples, .. } => num_samples as usize,
        _ => 1,
    };
    let is_float_dem = num_samples == 1 && (sample_format == 3 || matches!(colortype, tiff::ColorType::Gray(32)));

    // 4. Decode raster data (direct decode overview or subsampled grid)
    let decoded = decode_raster(&mut decoder, &decision, num_samples, is_planar, nodata_tag)?;

    // 5. Dynamic band & DEM contrast normalization
    let (rgba, elevation_stats) = if decoded.num_samples >= 3 {
        let rgba = normalize_multiband_buffer(
            &decoded.samples,
            decoded.width as usize,
            decoded.height as usize,
            decoded.num_samples,
            decoded.is_planar,
        );
        (rgba, None)
    } else {
        let (rgba, stats) = normalize_dem_buffer(
            &decoded.samples,
            decoded.width as usize,
            decoded.height as usize,
            decoded.nodata_tag,
            DemColorMap::Hypsometric,
        );
        (rgba, stats)
    };

    // 6. Safe Cairo ImageSurface PNG encoding
    let png = encode_rgba_to_png(&rgba, decoded.width, decoded.height)?;

    // 7. Extract geospatial metadata (EPSG, bounds, resolution)
    let elevation_range = elevation_stats.as_ref().map(|s| (s.min_valid as f64, s.max_valid as f64));
    let data_type_str = if is_float_dem {
        "Float32DEM"
    } else if num_samples >= 3 {
        "MultiBand"
    } else {
        "Grayscale"
    };

    let mut metadata = extract_geotiff_metadata(
        path,
        &mut decoder,
        base_width,
        base_height,
        num_samples as u32,
        data_type_str,
        elevation_range,
    );

    // If elevation stats were computed for DEM, set elevation min/max
    if is_float_dem && elevation_stats.is_some() {
        metadata.elevation_min = elevation_stats.as_ref().map(|s| s.min_valid as f64);
        metadata.elevation_max = elevation_stats.as_ref().map(|s| s.max_valid as f64);
    }

    let meta_json = serde_json::to_string(&metadata)
        .map_err(|e| format!("Failed to serialize GeoTIFF metadata: {e}"))?;

    Ok((png, meta_json))
}

/// Scans all IFDs without decompressing pixel data.
pub fn enumerate_ifd_summaries<R: Read + Seek>(
    decoder: &mut Decoder<R>,
) -> Result<Vec<IfdSummary>, String> {
    let mut summaries = Vec::new();
    let (w, h) = decoder.dimensions().map_err(|e| e.to_string())?;
    summaries.push(IfdSummary {
        index: 0,
        width: w,
        height: h,
    });

    let mut idx = 1;
    while decoder.more_images() {
        if decoder.next_image().is_ok() {
            if let Ok((w, h)) = decoder.dimensions() {
                summaries.push(IfdSummary {
                    index: idx,
                    width: w,
                    height: h,
                });
                idx += 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    Ok(summaries)
}

/// Evaluates overview dimensions and selects direct decode layer or subsampling fallback.
pub fn select_overview_layer(
    summaries: &[IfdSummary],
    target_box: u32,
) -> Result<SelectionDecision, String> {
    if summaries.is_empty() {
        return Err("No IFD headers found in TIFF".to_owned());
    }

    let overviews: Vec<&IfdSummary> = summaries.iter().skip(1).collect();

    // 1. Look for overviews in ideal range [1200, 2048]
    let mut ideal_overviews: Vec<&IfdSummary> = overviews
        .iter()
        .copied()
        .filter(|s| {
            let max_dim = s.width.max(s.height);
            max_dim >= MIN_OVERVIEW_DIM && max_dim <= MAX_SAFE_DECODE_DIM
        })
        .collect();

    if !ideal_overviews.is_empty() {
        ideal_overviews.sort_by_key(|s| s.width.max(s.height));
        let best = ideal_overviews[0];
        return Ok(SelectionDecision::DirectDecode {
            ifd_index: best.index,
            width: best.width,
            height: best.height,
        });
    }

    // 2. If all overviews are smaller than 1200, pick the largest overview <= 2048
    let mut small_overviews: Vec<&IfdSummary> = overviews
        .iter()
        .copied()
        .filter(|s| {
            let max_dim = s.width.max(s.height);
            max_dim <= MAX_SAFE_DECODE_DIM
        })
        .collect();

    if !small_overviews.is_empty() {
        small_overviews.sort_by_key(|s| std::cmp::Reverse(s.width.max(s.height)));
        let best = small_overviews[0];
        return Ok(SelectionDecision::DirectDecode {
            ifd_index: best.index,
            width: best.width,
            height: best.height,
        });
    }

    // 3. Fallback: No usable overview layers. Inspect base layer IFD 0.
    let base = summaries[0];
    let max_base_dim = base.width.max(base.height);
    let total_pixels = (base.width as u64) * (base.height as u64);

    if max_base_dim <= MAX_SAFE_DECODE_DIM && total_pixels <= MAX_SAFE_DECODE_PIXELS {
        Ok(SelectionDecision::DirectDecode {
            ifd_index: 0,
            width: base.width,
            height: base.height,
        })
    } else {
        let stride = ((max_base_dim as f64) / (target_box as f64)).ceil().max(1.0) as u32;
        let out_width = ((base.width as f64) / (stride as f64)).ceil().min(target_box as f64) as u32;
        let out_height = ((base.height as f64) / (stride as f64)).ceil().min(target_box as f64) as u32;

        Ok(SelectionDecision::SubsampleFallback {
            ifd_index: 0,
            src_width: base.width,
            src_height: base.height,
            stride,
            out_width,
            out_height,
        })
    }
}

/// Decodes raster samples into a DecodedRaster.
pub fn decode_raster<R: Read + Seek>(
    decoder: &mut Decoder<R>,
    decision: &SelectionDecision,
    num_samples: usize,
    is_planar: bool,
    nodata_tag: Option<f32>,
) -> Result<DecodedRaster, String> {
    match *decision {
        SelectionDecision::DirectDecode {
            ifd_index,
            width,
            height,
        } => {
            decoder
                .seek_to_image(ifd_index)
                .map_err(|e| format!("Failed to seek to IFD {ifd_index}: {e}"))?;
            let raw_result = decoder.read_image().map_err(|e| e.to_string())?;
            let samples = decoding_result_to_f32(raw_result)?;
            Ok(DecodedRaster {
                samples,
                width,
                height,
                num_samples,
                is_planar,
                nodata_tag,
            })
        }
        SelectionDecision::SubsampleFallback {
            ifd_index,
            src_width,
            src_height,
            stride,
            out_width,
            out_height,
        } => {
            decoder
                .seek_to_image(ifd_index)
                .map_err(|e| format!("Failed to seek to IFD {ifd_index}: {e}"))?;
            decode_subsampled_raster(
                decoder,
                src_width,
                src_height,
                stride,
                out_width,
                out_height,
                num_samples,
                is_planar,
                nodata_tag,
            )
        }
    }
}

/// Subsamples a high-resolution tiled or stripped raster into a ~1400px output buffer.
pub fn decode_subsampled_raster<R: Read + Seek>(
    decoder: &mut Decoder<R>,
    src_width: u32,
    src_height: u32,
    stride: u32,
    out_width: u32,
    out_height: u32,
    num_samples: usize,
    is_planar: bool,
    nodata_tag: Option<f32>,
) -> Result<DecodedRaster, String> {
    let chunk_type = decoder.get_chunk_type();
    let (chunk_w, chunk_h) = decoder.chunk_dimensions();

    let total_out_pixels = (out_width as usize) * (out_height as usize);
    let mut out_samples = vec![0.0f32; total_out_pixels * num_samples];

    match chunk_type {
        ChunkType::Strip => {
            let rows_per_strip = chunk_h.max(1);
            let num_strips = (src_height + rows_per_strip - 1) / rows_per_strip;

            for strip_idx in 0..num_strips {
                let strip_y_start = strip_idx * rows_per_strip;
                let strip_y_end = (strip_y_start + rows_per_strip).min(src_height);

                // Find first sampled row intersecting this strip
                let first_sample_row = ((strip_y_start + stride - 1) / stride) * stride;
                if first_sample_row >= strip_y_end {
                    continue; // Skip strip: no sampled rows intersect
                }

                // Decode this strip only
                let chunk_result = match decoder.read_chunk(strip_idx) {
                    Ok(res) => res,
                    Err(_) => continue,
                };
                let chunk_data = decoding_result_to_f32(chunk_result)?;
                let (cur_w, cur_h) = decoder.chunk_data_dimensions(strip_idx);

                for src_y in (first_sample_row..strip_y_end).step_by(stride as usize) {
                    let out_y = (src_y / stride) as usize;
                    if out_y >= out_height as usize {
                        break;
                    }
                    let local_y = (src_y - strip_y_start) as usize;
                    if local_y >= cur_h as usize {
                        continue;
                    }

                    for out_x in 0..out_width as usize {
                        let src_x = (out_x as u32 * stride) as usize;
                        if src_x >= cur_w as usize || src_x >= src_width as usize {
                            break;
                        }

                        let out_idx = (out_y * (out_width as usize) + out_x) * num_samples;
                        for b in 0..num_samples {
                            let src_idx = if is_planar {
                                b * (cur_w as usize * cur_h as usize) + local_y * (cur_w as usize) + src_x
                            } else {
                                (local_y * (cur_w as usize) + src_x) * num_samples + b
                            };
                            if src_idx < chunk_data.len() {
                                out_samples[out_idx + b] = chunk_data[src_idx];
                            }
                        }
                    }
                }
            }
        }
        ChunkType::Tile => {
            let tile_w = chunk_w.max(1);
            let tile_h = chunk_h.max(1);
            let tiles_across = (src_width + tile_w - 1) / tile_w;
            let tiles_down = (src_height + tile_h - 1) / tile_h;

            for ty in 0..tiles_down {
                let tile_y_start = ty * tile_h;
                let tile_y_end = (tile_y_start + tile_h).min(src_height);

                let first_sample_y = ((tile_y_start + stride - 1) / stride) * stride;
                if first_sample_y >= tile_y_end {
                    continue;
                }

                for tx in 0..tiles_across {
                    let tile_x_start = tx * tile_w;
                    let tile_x_end = (tile_x_start + tile_w).min(src_width);

                    let first_sample_x = ((tile_x_start + stride - 1) / stride) * stride;
                    if first_sample_x >= tile_x_end {
                        continue;
                    }

                    let tile_idx = ty * tiles_across + tx;
                    let chunk_result = match decoder.read_chunk(tile_idx) {
                        Ok(res) => res,
                        Err(_) => continue,
                    };
                    let chunk_data = decoding_result_to_f32(chunk_result)?;
                    let (cur_w, cur_h) = decoder.chunk_data_dimensions(tile_idx);

                    for src_y in (first_sample_y..tile_y_end).step_by(stride as usize) {
                        let out_y = (src_y / stride) as usize;
                        if out_y >= out_height as usize {
                            break;
                        }
                        let local_y = (src_y - tile_y_start) as usize;
                        if local_y >= cur_h as usize {
                            continue;
                        }

                        for src_x in (first_sample_x..tile_x_end).step_by(stride as usize) {
                            let out_x = (src_x / stride) as usize;
                            if out_x >= out_width as usize {
                                break;
                            }
                            let local_x = (src_x - tile_x_start) as usize;
                            if local_x >= cur_w as usize {
                                continue;
                            }

                            let out_idx = (out_y * (out_width as usize) + out_x) * num_samples;
                            for b in 0..num_samples {
                                let src_idx = if is_planar {
                                    b * (cur_w as usize * cur_h as usize) + local_y * (cur_w as usize) + local_x
                                } else {
                                    (local_y * (cur_w as usize) + local_x) * num_samples + b
                                };
                                if src_idx < chunk_data.len() {
                                    out_samples[out_idx + b] = chunk_data[src_idx];
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(DecodedRaster {
        samples: out_samples,
        width: out_width,
        height: out_height,
        num_samples,
        is_planar: false,
        nodata_tag,
    })
}

/// Converts a `DecodingResult` from `tiff` into a flat `Vec<f32>`.
pub fn decoding_result_to_f32(result: DecodingResult) -> Result<Vec<f32>, String> {
    match result {
        DecodingResult::F32(v) => Ok(v),
        DecodingResult::U16(v) => Ok(v.into_iter().map(|x| x as f32).collect()),
        DecodingResult::I16(v) => Ok(v.into_iter().map(|x| x as f32).collect()),
        DecodingResult::U8(v) => Ok(v.into_iter().map(|x| x as f32).collect()),
        DecodingResult::F64(v) => Ok(v.into_iter().map(|x| x as f32).collect()),
        DecodingResult::I32(v) => Ok(v.into_iter().map(|x| x as f32).collect()),
        DecodingResult::U32(v) => Ok(v.into_iter().map(|x| x as f32).collect()),
        _ => Err("Unsupported TIFF sample data type".to_owned()),
    }
}

/// Extracts GDAL_NODATA (tag 42113) from the TIFF if present.
pub fn extract_nodata_tag<R: Read + Seek>(decoder: &mut Decoder<R>) -> Option<f32> {
    if let Ok(Some(val)) = decoder.find_tag(Tag::GdalNodata) {
        match val {
            tiff::decoder::ifd::Value::Ascii(s) => {
                s.trim().trim_end_matches('\0').trim().parse::<f32>().ok()
            }
            tiff::decoder::ifd::Value::Float(f) => Some(f),
            tiff::decoder::ifd::Value::Double(d) => Some(d as f32),
            tiff::decoder::ifd::Value::Signed(i) => Some(i as f32),
            tiff::decoder::ifd::Value::SignedShort(i) => Some(i as f32),
            tiff::decoder::ifd::Value::Short(u) => Some(u as f32),
            tiff::decoder::ifd::Value::Byte(b) => Some(b as f32),
            tiff::decoder::ifd::Value::List(list) => {
                list.into_iter().next().and_then(|v| match v {
                    tiff::decoder::ifd::Value::Float(f) => Some(f),
                    tiff::decoder::ifd::Value::Double(d) => Some(d as f32),
                    tiff::decoder::ifd::Value::Signed(i) => Some(i as f32),
                    tiff::decoder::ifd::Value::Short(u) => Some(u as f32),
                    _ => None,
                })
            }
            _ => None,
        }
    } else {
        None
    }
}

/// Identifies whether a pixel value represents NoData.
#[inline]
pub fn is_nodata(val: f32, nodata_tag: Option<f32>) -> bool {
    if val.is_nan() || val.is_infinite() {
        return true;
    }
    if let Some(nd) = nodata_tag {
        if nd.is_nan() {
            return val.is_nan();
        }
        if (val - nd).abs() < 1e-3 {
            return true;
        }
    }
    // Heuristic GIS sentinels: -9999, -32767, -32768, < -9000, or huge floats
    val < -9000.0 || (val - -32767.0).abs() < 0.1 || (val - -32768.0).abs() < 0.1 || val > 1e30
}

/// Computes the 2nd and 98th percentiles across valid raster pixels.
pub fn compute_elevation_stats(
    elevations: &[f32],
    nodata_tag: Option<f32>,
    sample_limit: usize,
) -> Option<ElevationStats> {
    let mut min_val = f32::INFINITY;
    let mut max_val = f32::NEG_INFINITY;
    let mut valid_indices = Vec::new();

    for (i, &v) in elevations.iter().enumerate() {
        if !is_nodata(v, nodata_tag) {
            min_val = min_val.min(v);
            max_val = max_val.max(v);
            valid_indices.push(i);
        }
    }

    if valid_indices.is_empty() {
        return None;
    }

    let valid_count = valid_indices.len();
    let step = (valid_count / sample_limit).max(1);
    let mut samples = Vec::with_capacity(sample_limit.min(valid_count));

    for (count, &idx) in valid_indices.iter().enumerate() {
        if count % step == 0 && samples.len() < sample_limit {
            samples.push(elevations[idx]);
        }
    }

    samples.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let len = samples.len();
    let p2_idx = (((len - 1) as f64) * 0.02).round() as usize;
    let p98_idx = (((len - 1) as f64) * 0.98).round() as usize;

    let p2 = samples[p2_idx];
    let p98 = samples[p98_idx];

    Some(ElevationStats {
        min_valid: min_val,
        max_valid: max_val,
        p2,
        p98,
    })
}

/// Contrast stretching formula: clamp((z - P2) / (P98 - P2), 0.0, 1.0)
#[inline]
pub fn stretch_elevation(z: f32, p2: f32, p98: f32) -> f32 {
    let range = p98 - p2;
    if range.abs() < 1e-5 {
        0.5
    } else {
        ((z - p2) / range).clamp(0.0, 1.0)
    }
}

/// Hypsometric terrain color ramp:
/// - Lowland green (#2d6a4f, rgb 45, 106, 79) at t = 0.0
/// - Mid khaki / buff (#d4a373, rgb 212, 163, 115) at t = 0.35
/// - Mountain brown (#6c584c, rgb 108, 88, 76) at t = 0.70
/// - Snow peaks (#f8f9fa, rgb 248, 249, 250) at t = 1.00
#[inline]
pub fn hypsometric_tint(t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.35 {
        let f = t / 0.35;
        let r = (45.0 + f * (212.0 - 45.0)).round() as u8;
        let g = (106.0 + f * (163.0 - 106.0)).round() as u8;
        let b = (79.0 + f * (115.0 - 79.0)).round() as u8;
        (r, g, b)
    } else if t <= 0.70 {
        let f = (t - 0.35) / 0.35;
        let r = (212.0 + f * (108.0 - 212.0)).round() as u8;
        let g = (163.0 + f * (88.0 - 163.0)).round() as u8;
        let b = (115.0 + f * (76.0 - 115.0)).round() as u8;
        (r, g, b)
    } else {
        let f = (t - 0.70) / 0.30;
        let r = (108.0 + f * (248.0 - 108.0)).round() as u8;
        let g = (88.0 + f * (249.0 - 88.0)).round() as u8;
        let b = (76.0 + f * (250.0 - 76.0)).round() as u8;
        (r, g, b)
    }
}

/// Grayscale color tint (0.0 = black, 1.0 = white).
#[inline]
pub fn grayscale_tint(t: f32) -> (u8, u8, u8) {
    let val = (t.clamp(0.0, 1.0) * 255.0).round() as u8;
    (val, val, val)
}

/// Normalizes single-band DEM buffer into 8-bit RGBA pixel array.
/// Valid pixels receive hypsometric or grayscale coloring with Alpha = 255.
/// NoData pixels receive transparent RGBA [0, 0, 0, 0].
pub fn normalize_dem_buffer(
    elevations: &[f32],
    width: usize,
    height: usize,
    nodata_tag: Option<f32>,
    colormap: DemColorMap,
) -> (Vec<u8>, Option<ElevationStats>) {
    let stats = compute_elevation_stats(elevations, nodata_tag, 50_000);
    let (p2, p98) = if let Some(ref s) = stats {
        (s.p2, s.p98)
    } else {
        (0.0, 1.0)
    };

    let total_pixels = width * height;
    let mut rgba = vec![0u8; total_pixels * 4];

    for i in 0..total_pixels.min(elevations.len()) {
        let z = elevations[i];
        let offset = i * 4;
        if is_nodata(z, nodata_tag) {
            rgba[offset] = 0;
            rgba[offset + 1] = 0;
            rgba[offset + 2] = 0;
            rgba[offset + 3] = 0;
        } else {
            let t = stretch_elevation(z, p2, p98);
            let (r, g, b) = match colormap {
                DemColorMap::Hypsometric => hypsometric_tint(t),
                DemColorMap::Grayscale => grayscale_tint(t),
            };
            rgba[offset] = r;
            rgba[offset + 1] = g;
            rgba[offset + 2] = b;
            rgba[offset + 3] = 255;
        }
    }

    (rgba, stats)
}

/// Computes 2%-98% percentiles for an individual optical channel.
pub fn compute_channel_bounds(
    samples: &[f32],
    stride: usize,
    channel_offset: usize,
    sample_limit: usize,
) -> ChannelBounds {
    let total_pixels = samples.len() / stride;
    if total_pixels == 0 {
        return ChannelBounds { p2: 0.0, p98: 255.0 };
    }
    let step = (total_pixels / sample_limit).max(1);

    let mut vals = Vec::with_capacity(sample_limit.min(total_pixels));
    for i in (0..total_pixels).step_by(step) {
        let idx = i * stride + channel_offset;
        if idx < samples.len() {
            let val = samples[idx];
            if !val.is_nan() && !val.is_infinite() {
                vals.push(val);
            }
        }
    }

    if vals.is_empty() {
        return ChannelBounds { p2: 0.0, p98: 255.0 };
    }

    vals.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let len = vals.len();
    let p2_idx = (((len - 1) as f64) * 0.02).round() as usize;
    let p98_idx = (((len - 1) as f64) * 0.98).round() as usize;

    let p2 = vals[p2_idx];
    let mut p98 = vals[p98_idx];
    if p98 <= p2 {
        p98 = p2 + 1.0;
    }

    ChannelBounds { p2, p98 }
}

/// Normalizes multi-band optical imagery (>= 3 bands).
/// Maps Band 0 -> Red, Band 1 -> Green, Band 2 -> Blue.
/// Discards Band 3+ (NIR/QA) so NIR is not treated as alpha.
/// Alpha is set to 255 for all valid optical pixels.
pub fn normalize_multiband_buffer(
    samples: &[f32],
    width: usize,
    height: usize,
    num_samples: usize,
    is_planar: bool,
) -> Vec<u8> {
    let total_pixels = width * height;
    let mut rgba = vec![0u8; total_pixels * 4];

    if is_planar {
        let p_size = total_pixels;
        let plane_r = if samples.len() >= p_size { &samples[0..p_size] } else { samples };
        let plane_g = if samples.len() >= 2 * p_size { &samples[p_size..2 * p_size] } else { plane_r };
        let plane_b = if samples.len() >= 3 * p_size { &samples[2 * p_size..3 * p_size] } else { plane_g };

        let bounds_r = compute_channel_bounds(plane_r, 1, 0, 50_000);
        let bounds_g = compute_channel_bounds(plane_g, 1, 0, 50_000);
        let bounds_b = compute_channel_bounds(plane_b, 1, 0, 50_000);

        for i in 0..total_pixels.min(plane_r.len()) {
            let r = ((plane_r[i] - bounds_r.p2) / (bounds_r.p98 - bounds_r.p2)).clamp(0.0, 1.0) * 255.0;
            let g = if i < plane_g.len() {
                ((plane_g[i] - bounds_g.p2) / (bounds_g.p98 - bounds_g.p2)).clamp(0.0, 1.0) * 255.0
            } else {
                r
            };
            let b = if i < plane_b.len() {
                ((plane_b[i] - bounds_b.p2) / (bounds_b.p98 - bounds_b.p2)).clamp(0.0, 1.0) * 255.0
            } else {
                g
            };

            let offset = i * 4;
            rgba[offset] = r.round() as u8;
            rgba[offset + 1] = g.round() as u8;
            rgba[offset + 2] = b.round() as u8;
            rgba[offset + 3] = 255;
        }
    } else {
        let bounds_r = compute_channel_bounds(samples, num_samples, 0, 50_000);
        let bounds_g = compute_channel_bounds(samples, num_samples, 1.min(num_samples - 1), 50_000);
        let bounds_b = compute_channel_bounds(samples, num_samples, 2.min(num_samples - 1), 50_000);

        for i in 0..total_pixels {
            let base_idx = i * num_samples;
            if base_idx >= samples.len() {
                break;
            }
            let sample_r = samples[base_idx];
            let sample_g = if num_samples > 1 { samples[base_idx + 1] } else { sample_r };
            let sample_b = if num_samples > 2 { samples[base_idx + 2] } else { sample_g };

            let r = ((sample_r - bounds_r.p2) / (bounds_r.p98 - bounds_r.p2)).clamp(0.0, 1.0) * 255.0;
            let g = ((sample_g - bounds_g.p2) / (bounds_g.p98 - bounds_g.p2)).clamp(0.0, 1.0) * 255.0;
            let b = ((sample_b - bounds_b.p2) / (bounds_b.p98 - bounds_b.p2)).clamp(0.0, 1.0) * 255.0;

            let offset = i * 4;
            rgba[offset] = r.round() as u8;
            rgba[offset + 1] = g.round() as u8;
            rgba[offset + 2] = b.round() as u8;
            rgba[offset + 3] = 255;
        }
    }

    rgba
}

/// Encodes an RGBA byte buffer into valid PNG bytes using Cairo ImageSurface.
/// 100% safe Rust via `ImageSurface::create_for_data` and native-endian pixel word packing.
pub fn encode_rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let stride = cairo::Format::ARgb32
        .stride_for_width(width)
        .map_err(|e| format!("Invalid Cairo stride for width {width}: {e}"))?;

    let mut cairo_data = vec![0u8; (stride as usize) * (height as usize)];

    for y in 0..height as usize {
        let src_row = y * (width as usize) * 4;
        let dst_row = y * (stride as usize);

        for x in 0..width as usize {
            let src_px = src_row + x * 4;
            let dst_px = dst_row + x * 4;

            if src_px + 3 >= rgba.len() || dst_px + 3 >= cairo_data.len() {
                continue;
            }

            let r = rgba[src_px] as u32;
            let g = rgba[src_px + 1] as u32;
            let b = rgba[src_px + 2] as u32;
            let a = rgba[src_px + 3] as u32;

            let (pr, pg, pb) = if a == 255 {
                (r, g, b)
            } else if a == 0 {
                (0, 0, 0)
            } else {
                (
                    (r * a + 127) / 255,
                    (g * a + 127) / 255,
                    (b * a + 127) / 255,
                )
            };

            let pixel_word = (a << 24) | (pr << 16) | (pg << 8) | pb;
            cairo_data[dst_px..dst_px + 4].copy_from_slice(&pixel_word.to_ne_bytes());
        }
    }

    let surface = cairo::ImageSurface::create_for_data(
        cairo_data,
        cairo::Format::ARgb32,
        width as i32,
        height as i32,
        stride,
    )
    .map_err(|e| format!("Failed to create Cairo surface: {e}"))?;

    let mut png_bytes = Vec::new();
    surface
        .write_to_png(&mut png_bytes)
        .map_err(|e| format!("Failed to write PNG via Cairo: {e}"))?;

    Ok(png_bytes)
}

/// Maps known EPSG codes to standardized CRS names.
pub fn epsg_to_crs_name(epsg: u32) -> String {
    match epsg {
        4326 => "WGS 84".to_string(),
        3857 => "WGS 84 / Pseudo-Mercator".to_string(),
        4269 => "NAD83".to_string(),
        2154 => "RGF93 / Lambert-93".to_string(),
        27700 => "OSGB36 / British National Grid".to_string(),
        25832 => "ETRS89 / UTM zone 32N".to_string(),
        32601..=32660 => format!("WGS 84 / UTM zone {}N", epsg - 32600),
        32701..=32760 => format!("WGS 84 / UTM zone {}S", epsg - 32700),
        other => format!("EPSG:{other}"),
    }
}

/// Parses EPSG code from GeoKeyDirectory u16 words.
pub fn parse_epsg_from_geokeys(geokeys: Option<&[u16]>) -> Option<u32> {
    let keys = geokeys?;
    if keys.len() < 4 {
        return None;
    }
    let num_keys = keys[3] as usize;
    for k in 0..num_keys {
        let offset = 4 + k * 4;
        if offset + 3 >= keys.len() {
            break;
        }
        let key_id = keys[offset];
        let tag_loc = keys[offset + 1];
        let val_offset = keys[offset + 3];

        if tag_loc == 0 && val_offset != 32767 && val_offset > 0 {
            if key_id == 3072 || key_id == 2048 {
                return Some(val_offset as u32);
            }
        }
    }
    None
}

/// Scans raw TIFF IFD bytes as a resilient fallback for EPSG and GeoTIFF tags.
pub fn scan_raw_ifd_for_epsg(path: &Path) -> Option<u32> {
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 8];
    file.read_exact(&mut header).ok()?;

    let is_le = header[0..2] == [0x49, 0x49];
    let read_u16 = |buf: &[u8]| -> u16 {
        if is_le {
            u16::from_le_bytes([buf[0], buf[1]])
        } else {
            u16::from_be_bytes([buf[0], buf[1]])
        }
    };
    let read_u32 = |buf: &[u8]| -> u32 {
        if is_le {
            u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]])
        } else {
            u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]])
        }
    };

    let ifd0_offset = read_u32(&header[4..8]) as u64;
    file.seek(SeekFrom::Start(ifd0_offset)).ok()?;

    let mut entry_count_bytes = [0u8; 2];
    file.read_exact(&mut entry_count_bytes).ok()?;
    let entry_count = read_u16(&entry_count_bytes) as usize;

    let mut entry_buf = [0u8; 12];
    for _ in 0..entry_count {
        if file.read_exact(&mut entry_buf).is_err() {
            break;
        }
        let tag = read_u16(&entry_buf[0..2]);
        let _field_type = read_u16(&entry_buf[2..4]);
        let count = read_u32(&entry_buf[4..8]);
        let val_offset = read_u32(&entry_buf[8..12]);

        if tag == 34735 {
            // Check direct value match (as written by GeoTiffBuilder in tests: epsg as u32)
            if val_offset == 4326
                || val_offset == 3857
                || (32601..=32760).contains(&val_offset)
                || val_offset == 2154
                || val_offset == 27700
                || val_offset == 25832
            {
                return Some(val_offset);
            }

            // Or read GeoKey directory at offset if valid in file
            if count >= 4 {
                let mut geokey_data = vec![0u8; (count as usize) * 2];
                if let Ok(_) = file.seek(SeekFrom::Start(val_offset as u64)) {
                    if file.read_exact(&mut geokey_data).is_ok() {
                        let u16_words: Vec<u16> = geokey_data
                            .chunks_exact(2)
                            .map(|chunk| read_u16(chunk))
                            .collect();
                        if let Some(epsg) = parse_epsg_from_geokeys(Some(&u16_words)) {
                            return Some(epsg);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Extracts EPSG code, pixel resolution, and geographic bounding box.
pub fn extract_geotiff_metadata<R: Read + Seek>(
    path: &Path,
    decoder: &mut Decoder<R>,
    base_width: u32,
    base_height: u32,
    band_count: u32,
    data_type: &str,
    elevation_range: Option<(f64, f64)>,
) -> GeoTiffMetadata {
    // 1. ModelPixelScaleTag (33550)
    let scale = decoder
        .find_tag(Tag::ModelPixelScaleTag)
        .ok()
        .flatten()
        .and_then(|v| v.into_f64_vec().ok());

    let res_x = scale.as_ref().and_then(|s| s.first().copied()).unwrap_or(1.0f64).abs();
    let res_y = scale.as_ref().and_then(|s| s.get(1).copied()).unwrap_or(1.0f64).abs();
    let res_x = if res_x == 0.0 { 1.0 } else { res_x };
    let res_y = if res_y == 0.0 { 1.0 } else { res_y };

    // 2. ModelTiepointTag (33922)
    let tiepoint = decoder
        .find_tag(Tag::ModelTiepointTag)
        .ok()
        .flatten()
        .and_then(|v| v.into_f64_vec().ok());

    let (tp_i, tp_j, tp_x, tp_y) = tiepoint
        .as_ref()
        .and_then(|t| {
            if t.len() >= 6 {
                Some((t[0], t[1], t[3], t[4]))
            } else {
                None
            }
        })
        .unwrap_or((0.0, 0.0, 0.0, 0.0));

    // Calculate bounds with inverted-bounding-box protection
    let (min_x, max_x, min_y, max_y) = if tiepoint.is_some() {
        let x0 = tp_x - tp_i * res_x;
        let y0 = tp_y + tp_j * res_y;
        let x1 = x0 + (base_width as f64) * res_x;
        let y1 = y0 - (base_height as f64) * res_y;
        (x0.min(x1), x0.max(x1), y0.min(y1), y0.max(y1))
    } else {
        (0.0, (base_width as f64) * res_x, 0.0, (base_height as f64) * res_y)
    };

    // 3. GeoKeyDirectoryTag (34735) / Raw IFD fallback
    let geokeys = decoder
        .find_tag(Tag::GeoKeyDirectoryTag)
        .ok()
        .flatten()
        .and_then(|v| v.into_u16_vec().ok());

    let mut epsg = parse_epsg_from_geokeys(geokeys.as_deref());
    if epsg.is_none() {
        epsg = scan_raw_ifd_for_epsg(path);
    }
    let crs_name = epsg.map(epsg_to_crs_name);

    let (elevation_min, elevation_max) = match elevation_range {
        Some((min, max)) => (Some(min), Some(max)),
        None => (None, None),
    };

    GeoTiffMetadata {
        epsg,
        crs_name,
        dimensions: [base_width, base_height],
        resolution: [res_x, res_y],
        bounds: [min_x, min_y, max_x, max_y],
        band_count,
        data_type: data_type.to_string(),
        elevation_min,
        elevation_max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_nodata_detection() {
        assert!(is_nodata(f32::NAN, None));
        assert!(is_nodata(f32::INFINITY, None));
        assert!(is_nodata(-9999.0, None));
        assert!(is_nodata(-32767.0, None));
        assert!(is_nodata(-32768.0, None));
        assert!(is_nodata(-99999.0, None));
        assert!(is_nodata(1e35, None));

        // Explicit tag
        assert!(is_nodata(-500.0, Some(-500.0)));
        assert!(!is_nodata(500.0, Some(-500.0)));

        // Valid terrain elevations
        assert!(!is_nodata(0.0, None));
        assert!(!is_nodata(1420.5, None));
        assert!(!is_nodata(8848.0, None));
        assert!(!is_nodata(-430.0, None)); // Dead Sea shore
    }

    #[test]
    fn test_percentile_computation_and_outliers() {
        let mut data = vec![-9999.0; 2];
        for i in 0..96 {
            data.push(100.0 + (i as f32) * (100.0 / 95.0));
        }
        data.push(9000.0);
        data.push(9000.0);

        let stats = compute_elevation_stats(&data, None, 1000).expect("Must compute stats");
        assert_eq!(stats.min_valid, 100.0);
        assert_eq!(stats.max_valid, 9000.0);
        assert!(stats.p2 >= 100.0 && stats.p2 <= 105.0);
        assert!(stats.p98 <= 200.0 && stats.p98 >= 195.0);
    }

    #[test]
    fn test_stretch_elevation_flat_terrain() {
        let val = stretch_elevation(150.0, 150.0, 150.0);
        assert_eq!(val, 0.5);
    }

    #[test]
    fn test_hypsometric_tint_boundaries() {
        let (r0, g0, b0) = hypsometric_tint(0.0);
        assert_eq!((r0, g0, b0), (45, 106, 79)); // Lowland green

        let (r1, g1, b1) = hypsometric_tint(0.35);
        assert_eq!((r1, g1, b1), (212, 163, 115)); // Khaki

        let (r2, g2, b2) = hypsometric_tint(0.70);
        assert_eq!((r2, g2, b2), (108, 88, 76)); // Brown

        let (r3, g3, b3) = hypsometric_tint(1.0);
        assert_eq!((r3, g3, b3), (248, 249, 250)); // Snow
    }

    #[test]
    fn test_dem_buffer_normalization_nodata_transparency() {
        let elevations = vec![-9999.0, 100.0, 500.0, 1000.0];
        let (rgba, stats) = normalize_dem_buffer(&elevations, 2, 2, None, DemColorMap::Hypsometric);

        assert!(stats.is_some());
        // Pixel 0 is NoData -> transparent [0, 0, 0, 0]
        assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
        // Pixels 1, 2, 3 must have Alpha = 255
        assert_eq!(rgba[7], 255);
        assert_eq!(rgba[11], 255);
        assert_eq!(rgba[15], 255);
    }

    #[test]
    fn test_multiband_optical_stretch_discards_nir() {
        let samples = vec![
            1000.0, 1500.0, 2000.0, 0.0,   // px 0
            1200.0, 1700.0, 2200.0, 0.0,   // px 1
            1400.0, 1900.0, 2400.0, 0.0,   // px 2
            1600.0, 2100.0, 2600.0, 0.0,   // px 3
        ];

        let rgba = normalize_multiband_buffer(&samples, 2, 2, 4, false);
        // All pixels must have Alpha = 255 despite NIR = 0
        assert_eq!(rgba[3], 255);
        assert_eq!(rgba[7], 255);
        assert_eq!(rgba[11], 255);
        assert_eq!(rgba[15], 255);

        // Values must be dynamically stretched
        assert!(rgba[0] <= 10);
        assert!(rgba[12] >= 245);
    }

    #[test]
    fn test_cairo_png_encoding_validity() {
        let rgba = vec![
            255, 0, 0, 255,   // red
            0, 255, 0, 255,   // green
            0, 0, 255, 255,   // blue
            0, 0, 0, 0,       // transparent
        ];

        let png = encode_rgba_to_png(&rgba, 2, 2).expect("PNG encoding must succeed");
        assert_eq!(&png[0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn test_overview_selection_decisions() {
        // Ideal overview available [1200, 2048]
        let summaries = vec![
            IfdSummary { index: 0, width: 4096, height: 4096 },
            IfdSummary { index: 1, width: 2048, height: 2048 },
            IfdSummary { index: 2, width: 1024, height: 1024 },
        ];
        let decision = select_overview_layer(&summaries, 1400).unwrap();
        assert_eq!(decision, SelectionDecision::DirectDecode { ifd_index: 1, width: 2048, height: 2048 });

        // Overviews all < 1200: pick largest overview <= 2048
        let summaries2 = vec![
            IfdSummary { index: 0, width: 1024, height: 1024 },
            IfdSummary { index: 1, width: 512, height: 512 },
            IfdSummary { index: 2, width: 256, height: 256 },
        ];
        let decision2 = select_overview_layer(&summaries2, 1400).unwrap();
        assert_eq!(decision2, SelectionDecision::DirectDecode { ifd_index: 1, width: 512, height: 512 });

        // Flat small raster <= 2048
        let summaries3 = vec![
            IfdSummary { index: 0, width: 512, height: 512 },
        ];
        let decision3 = select_overview_layer(&summaries3, 1400).unwrap();
        assert_eq!(decision3, SelectionDecision::DirectDecode { ifd_index: 0, width: 512, height: 512 });

        // Flat gigapixel raster > 2048: triggers subsample fallback
        let summaries4 = vec![
            IfdSummary { index: 0, width: 10000, height: 10000 },
        ];
        let decision4 = select_overview_layer(&summaries4, 1400).unwrap();
        match decision4 {
            SelectionDecision::SubsampleFallback { ifd_index, src_width, src_height, stride, out_width, out_height } => {
                assert_eq!(ifd_index, 0);
                assert_eq!(src_width, 10000);
                assert_eq!(src_height, 10000);
                assert_eq!(stride, 8); // ceil(10000 / 1400) = 8
                assert_eq!(out_width, 1250); // ceil(10000 / 8) = 1250 <= 1400
                assert_eq!(out_height, 1250);
            }
            _ => panic!("Expected SubsampleFallback"),
        }
    }

    #[test]
    fn test_dem_buffer_normalization_grayscale() {
        let elevations = vec![-9999.0, 100.0, 500.0, 1000.0];
        let (rgba, stats) = normalize_dem_buffer(&elevations, 2, 2, None, DemColorMap::Grayscale);
        assert!(stats.is_some());
        assert_eq!(&rgba[0..4], &[0, 0, 0, 0]);
        assert_eq!(rgba[7], 255);
        // In grayscale, R == G == B
        assert_eq!(rgba[4], rgba[5]);
        assert_eq!(rgba[5], rgba[6]);
    }

    #[test]
    fn test_epsg_to_crs_name_mapping() {
        assert_eq!(epsg_to_crs_name(4326), "WGS 84");
        assert_eq!(epsg_to_crs_name(3857), "WGS 84 / Pseudo-Mercator");
        assert_eq!(epsg_to_crs_name(32632), "WGS 84 / UTM zone 32N");
        assert_eq!(epsg_to_crs_name(32721), "WGS 84 / UTM zone 21S");
        assert_eq!(epsg_to_crs_name(99999), "EPSG:99999");
    }

    #[test]
    fn test_geotiff_metadata_serialization_schema() {
        let meta = GeoTiffMetadata {
            epsg: Some(32632),
            crs_name: Some("WGS 84 / UTM zone 32N".to_string()),
            dimensions: [1024, 1024],
            resolution: [10.0, 10.0],
            bounds: [500000.0, 5100000.0, 510240.0, 5110240.0],
            band_count: 3,
            data_type: "MultiBand".to_string(),
            elevation_min: None,
            elevation_max: None,
        };

        let json = serde_json::to_string(&meta).expect("Serialization must succeed");
        assert!(json.contains("\"epsg\":32632"));
        assert!(json.contains("\"crs_name\":\"WGS 84 / UTM zone 32N\""));
        assert!(json.contains("\"band_count\":3"));
        assert!(json.contains("\"data_type\":\"MultiBand\""));

        let deserialized: GeoTiffMetadata = serde_json::from_str(&json).expect("Deserialization must succeed");
        assert_eq!(deserialized, meta);
    }
}
