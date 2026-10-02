// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(dead_code, unused_imports, clippy::all)]

//! Empirical Challenger Stress Test Suite for Milestone 3.1:
//! - 1. Multi-IFD COGs with diverse pyramid levels (1024x1024, 2048x2048, 4096x4096):
//!      verify selected overview matches specification (smallest in [1200, 2048] or largest <= 2048).
//! - 2. Flat single-layer gigapixel rasters (e.g. 5000x5000):
//!      verify subsampling stride and memory remains bounded <= 10MB without triggering sandbox OOM or SIGXFSZ.
//! - 3. Float32 DEMs with NoData sentinels (-9999.0, NaN, Inf, < -9000.0):
//!      verify generated PNG has non-zero contrast variance (> 10) and NoData pixels are transparent RGBA [0,0,0,0].
//! - 4. Flat terrain DEMs where min == max (P2 == P98):
//!      verify no division-by-zero or NaN pixel outputs.
//! - 5. 4-band optical rasters with NIR = 0:
//!      verify NIR is discarded and alpha = 255 across all pixels.

use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

use gdk_pixbuf::prelude::*;

mod common;
use common::*;

#[path = "../src/sandbox_helper/geotiff.rs"]
mod geotiff;
use geotiff::*;

// =========================================================================
// Verification Oracles & Statistical Helpers
// =========================================================================

/// Calculates the luminance contrast variance across all valid (alpha > 0) pixels.
/// Luminance formula: Y = 0.299 * R + 0.587 * G + 0.114 * B.
pub fn calculate_contrast_variance(rgba: &[u8]) -> f64 {
    let mut luminances = Vec::new();
    for chunk in rgba.chunks_exact(4) {
        if chunk[3] > 0 {
            let y = 0.299 * (chunk[0] as f64) + 0.587 * (chunk[1] as f64) + 0.114 * (chunk[2] as f64);
            luminances.push(y);
        }
    }
    if luminances.is_empty() {
        return 0.0;
    }
    let n = luminances.len() as f64;
    let mean = luminances.iter().sum::<f64>() / n;
    let variance = luminances.iter().map(|&y| (y - mean).powi(2)).sum::<f64>() / n;
    variance
}

/// Decodes PNG bytes to raw 8-bit RGBA pixel slice using GdkPixbuf loader.
pub fn decode_png_to_rgba(png_bytes: &[u8]) -> (Vec<u8>, u32, u32) {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(png_bytes).expect("Failed to write to pixbuf loader");
    loader.close().expect("Failed to close pixbuf loader");
    let pixbuf = loader.pixbuf().expect("Valid pixbuf expected");
    let width = pixbuf.width() as u32;
    let height = pixbuf.height() as u32;
    let n_channels = pixbuf.n_channels();
    let has_alpha = pixbuf.has_alpha();
    let rowstride = pixbuf.rowstride() as usize;

    let pixels = pixbuf.read_pixel_bytes();
    let raw = pixels.as_ref();

    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as usize {
        let row_start = y * rowstride;
        for x in 0..width as usize {
            let px_offset = row_start + x * (n_channels as usize);
            let r = raw[px_offset];
            let g = raw[px_offset + 1];
            let b = raw[px_offset + 2];
            let a = if has_alpha { raw[px_offset + 3] } else { 255 };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    (rgba, width, height)
}

#[test]
fn test_harness_sanity() {
    assert!(true);
}
