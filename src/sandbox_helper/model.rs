// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::Path,
};

use crate::services::model_preview::*;
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

type Point = [f32; 3];
type Matrix = [f32; 12];
const IDENTITY: Matrix = [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.];

fn triangle_limit_message() -> String {
    format!(
        "This model exceeds the {} million triangle preview limit. Try a lower-detail version.",
        MAX_MODEL_TRIANGLES as f64 / 1_000_000.
    )
}

#[derive(Default)]
struct Object {
    vertices: Vec<Point>,
    triangles: Vec<[usize; 3]>,
    components: Vec<(u32, Matrix)>,
}

fn attribute(tag: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>, String> {
    for attr in tag.attributes() {
        let attr = attr.map_err(|_| "Invalid 3MF attribute".to_string())?;
        if attr.key.local_name().as_ref() == name {
            return String::from_utf8(attr.value.into_owned())
                .map(Some)
                .map_err(|_| "Invalid 3MF attribute".to_string());
        }
    }
    Ok(None)
}

fn number(tag: &BytesStart<'_>, name: &[u8]) -> Result<f32, String> {
    let value = attribute(tag, name)?.ok_or("Missing 3MF coordinate")?;
    let num: f32 = value.parse().map_err(|_| "Invalid 3MF coordinate")?;
    if num.is_finite() {
        Ok(num)
    } else {
        Err("Invalid 3MF coordinate".to_string())
    }
}

fn index(tag: &BytesStart<'_>, name: &[u8]) -> Result<u32, String> {
    attribute(tag, name)?
        .ok_or("Missing 3MF index")?
        .parse()
        .map_err(|_| "Invalid 3MF index".to_string())
}

fn matrix(value: Option<String>) -> Result<Matrix, String> {
    let Some(value) = value else {
        return Ok(IDENTITY);
    };
    let parts: Vec<f32> = value
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| "Invalid 3MF transform".to_string())?;
    if parts.len() != 12 || parts.iter().any(|part| !part.is_finite()) {
        return Err("Invalid 3MF transform".to_string());
    }
    parts.try_into().map_err(|_| "Invalid 3MF transform".to_string())
}

fn transform(m: Matrix, p: Point) -> Point {
    [
        m[0] * p[0] + m[3] * p[1] + m[6] * p[2] + m[9],
        m[1] * p[0] + m[4] * p[1] + m[7] * p[2] + m[10],
        m[2] * p[0] + m[5] * p[1] + m[8] * p[2] + m[11],
    ]
}

fn combine(a: Matrix, b: Matrix) -> Matrix {
    let origin = transform(a, transform(b, [0., 0., 0.]));
    let mut result = [0.; 12];
    for axis in 0..3 {
        let mut basis = [0.; 3];
        basis[axis] = 1.;
        let point = transform(a, transform(b, basis));
        for component in 0..3 {
            result[axis * 3 + component] = point[component] - origin[component];
        }
    }
    result[9..].copy_from_slice(&origin);
    result
}

pub(crate) fn triangles_3mf(xml: &[u8]) -> Result<Vec<[Point; 3]>, String> {
    let mut reader = Reader::from_reader(xml);
    let mut objects = HashMap::<u32, Object>::new();
    let mut current = None;
    let mut items = Vec::new();
    let mut total_vertices = 0usize;
    let mut total_triangles = 0usize;
    let mut total_components = 0usize;

    loop {
        let event = reader.read_event().map_err(|_| "Invalid 3MF model XML".to_string())?;
        match event {
            Event::Start(tag) | Event::Empty(tag) => match tag.local_name().as_ref() {
                b"object" => {
                    let id = index(&tag, b"id")?;
                    if objects.len() >= MAX_3MF_OBJECTS
                        || objects.insert(id, Object::default()).is_some()
                    {
                        return Err("3MF object limit exceeded".to_string());
                    }
                    current = Some(id);
                }
                b"vertex" => {
                    let Some(id) = current else {
                        return Err("Unexpected vertex outside object".to_string());
                    };
                    let point = [
                        number(&tag, b"x")?,
                        number(&tag, b"y")?,
                        number(&tag, b"z")?,
                    ];
                    total_vertices = total_vertices
                        .checked_add(1)
                        .filter(|&n| n <= MAX_MODEL_VERTICES)
                        .ok_or_else(|| "3MF vertex limit exceeded".to_string())?;
                    let object = objects.get_mut(&id).ok_or("Unknown object")?;
                    object.vertices.push(point);
                }
                b"triangle" => {
                    let Some(id) = current else {
                        return Err("Unexpected triangle outside object".to_string());
                    };
                    let v1 = index(&tag, b"v1")? as usize;
                    let v2 = index(&tag, b"v2")? as usize;
                    let v3 = index(&tag, b"v3")? as usize;
                    total_triangles = total_triangles
                        .checked_add(1)
                        .filter(|&n| n <= MAX_MODEL_TRIANGLES)
                        .ok_or_else(triangle_limit_message)?;
                    let object = objects.get_mut(&id).ok_or("Unknown object")?;
                    object.triangles.push([v1, v2, v3]);
                }
                b"component" => {
                    let Some(id) = current else {
                        return Err("Unexpected component outside object".to_string());
                    };
                    let target_id = index(&tag, b"objectid")?;
                    let m = matrix(attribute(&tag, b"transform")?)?;
                    total_components = total_components
                        .checked_add(1)
                        .filter(|&n| n <= MAX_MODEL_COMPONENT_REFERENCES)
                        .ok_or_else(|| "3MF component reference limit exceeded".to_string())?;
                    let object = objects.get_mut(&id).ok_or("Unknown object")?;
                    object.components.push((target_id, m));
                }
                b"item" => {
                    let id = index(&tag, b"objectid")?;
                    let m = matrix(attribute(&tag, b"transform")?)?;
                    if items.len() >= MAX_3MF_BUILD_ITEMS {
                        return Err("3MF build item limit exceeded".to_string());
                    }
                    items.push((id, m));
                }
                _ => {}
            },
            Event::End(tag) => {
                if tag.local_name().as_ref() == b"object" {
                    current = None;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if items.is_empty() {
        return Err("3MF file contains no build items".to_string());
    }

    // Expand component tree DFS
    let mut faces = Vec::new();
    let mut stack: Vec<(u32, Matrix, usize)> = items.into_iter().map(|(id, m)| (id, m, 0)).collect();
    let mut expansions = 0usize;

    while let Some((id, m, depth)) = stack.pop() {
        if depth > MAX_3MF_COMPONENT_DEPTH {
            return Err("3MF component nesting depth exceeded".to_string());
        }
        expansions = expansions
            .checked_add(1)
            .filter(|&n| n <= MAX_MODEL_COMPONENT_EXPANSIONS)
            .ok_or_else(|| "3MF component expansion limit exceeded".to_string())?;

        let object = objects.get(&id).ok_or("Referenced 3MF object missing")?;
        for tri in &object.triangles {
            let p0 = object.vertices.get(tri[0]).ok_or("Invalid vertex index")?;
            let p1 = object.vertices.get(tri[1]).ok_or("Invalid vertex index")?;
            let p2 = object.vertices.get(tri[2]).ok_or("Invalid vertex index")?;
            faces.push([
                transform(m, *p0),
                transform(m, *p1),
                transform(m, *p2),
            ]);
            if faces.len() > MAX_MODEL_TRIANGLES {
                return Err(triangle_limit_message());
            }
        }
        for (sub_id, sub_m) in &object.components {
            stack.push((*sub_id, combine(m, *sub_m), depth + 1));
        }
    }

    if faces.is_empty() {
        return Err("Model contains no triangles".to_string());
    }
    Ok(faces)
}

pub(crate) fn stl(bytes: &[u8]) -> Result<Vec<[Point; 3]>, String> {
    if bytes.len() >= 84 {
        let count = u32::from_le_bytes(bytes[80..84].try_into().map_err(|_| "Invalid STL header".to_string())?) as usize;
        if count.checked_mul(50).and_then(|n| n.checked_add(84)) == Some(bytes.len()) {
            if count > MAX_MODEL_TRIANGLES {
                return Err(triangle_limit_message());
            }
            let mut faces = Vec::with_capacity(count);
            for chunk in bytes[84..].chunks_exact(50) {
                let v0 = [
                    f32::from_le_bytes(chunk[12..16].try_into().map_err(|_| "Invalid STL coordinate".to_string())?),
                    f32::from_le_bytes(chunk[16..20].try_into().map_err(|_| "Invalid STL coordinate".to_string())?),
                    f32::from_le_bytes(chunk[20..24].try_into().map_err(|_| "Invalid STL coordinate".to_string())?),
                ];
                let v1 = [
                    f32::from_le_bytes(chunk[24..28].try_into().map_err(|_| "Invalid STL coordinate".to_string())?),
                    f32::from_le_bytes(chunk[28..32].try_into().map_err(|_| "Invalid STL coordinate".to_string())?),
                    f32::from_le_bytes(chunk[32..36].try_into().map_err(|_| "Invalid STL coordinate".to_string())?),
                ];
                let v2 = [
                    f32::from_le_bytes(chunk[36..40].try_into().map_err(|_| "Invalid STL coordinate".to_string())?),
                    f32::from_le_bytes(chunk[40..44].try_into().map_err(|_| "Invalid STL coordinate".to_string())?),
                    f32::from_le_bytes(chunk[44..48].try_into().map_err(|_| "Invalid STL coordinate".to_string())?),
                ];
                if !v0.iter().chain(&v1).chain(&v2).all(|c| c.is_finite()) {
                    return Err("Invalid STL coordinate".to_string());
                }
                faces.push([v0, v1, v2]);
            }
            if faces.is_empty() {
                return Err("Model contains no triangles".to_string());
            }
            return Ok(faces);
        }
    }

    // ASCII STL parsing
    let text = std::str::from_utf8(bytes).map_err(|_| "Invalid STL encoding".to_string())?;
    let trimmed = text.trim();
    if !trimmed.to_ascii_lowercase().starts_with("solid") {
        return Err("Invalid STL file header".to_string());
    }

    let mut vertices = Vec::new();
    let mut faces = Vec::new();
    for line in text.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.to_ascii_lowercase().starts_with("vertex") {
            let parts: Vec<&str> = trimmed_line.split_whitespace().collect();
            if parts.len() == 4 {
                let x = parts[1].parse::<f32>().map_err(|_| "Invalid STL coordinate".to_string())?;
                let y = parts[2].parse::<f32>().map_err(|_| "Invalid STL coordinate".to_string())?;
                let z = parts[3].parse::<f32>().map_err(|_| "Invalid STL coordinate".to_string())?;
                if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                    return Err("Invalid STL coordinate".to_string());
                }
                vertices.push([x, y, z]);
                if vertices.len() == 3 {
                    faces.push([vertices[0], vertices[1], vertices[2]]);
                    vertices.clear();
                    if faces.len() > MAX_MODEL_TRIANGLES {
                        return Err(triangle_limit_message());
                    }
                }
            }
        }
    }

    if faces.is_empty() {
        return Err("Model contains no triangles".to_string());
    }
    Ok(faces)
}

fn project(p: Point) -> Point {
    [
        -0.83 * p[0] + 0.55 * p[1],
        0.35 * p[0] + 0.53 * p[1] + 0.77 * p[2],
        -0.43 * p[0] - 0.64 * p[1] + 0.64 * p[2],
    ]
}

pub(crate) fn shade(
    faces: &[[Point; 3]],
    width: u32,
    height: u32,
    accent: [u8; 3],
    surface_col: [u8; 3],
) -> Result<Vec<u8>, String> {
    if faces.is_empty() || width == 0 || height == 0 {
        return Err("Invalid model rendering parameters".to_string());
    }

    let projected: Vec<[Point; 3]> = faces
        .iter()
        .map(|tri| [project(tri[0]), project(tri[1]), project(tri[2])])
        .collect();

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for tri in &projected {
        for p in tri {
            min_x = min_x.min(p[0]);
            max_x = max_x.max(p[0]);
            min_y = min_y.min(p[1]);
            max_y = max_y.max(p[1]);
        }
    }

    let center = [(min_x + max_x) * 0.5, (min_y + max_y) * 0.5];
    let extent = (max_x - min_x).max(max_y - min_y);
    let scale = if extent > 1e-6 {
        (0.82 * (width.min(height) as f32)) / extent
    } else {
        1.0
    };

    let mut depths = vec![f32::NEG_INFINITY; (width * height) as usize];
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut work = 0u64;

    let edge = |a: Point, b: Point, p: Point| {
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
    };

    for tri in projected {
        let screen = [
            [
                width as f32 * 0.5 + (tri[0][0] - center[0]) * scale,
                height as f32 * 0.5 - (tri[0][1] - center[1]) * scale,
                tri[0][2],
            ],
            [
                width as f32 * 0.5 + (tri[1][0] - center[0]) * scale,
                height as f32 * 0.5 - (tri[1][1] - center[1]) * scale,
                tri[1][2],
            ],
            [
                width as f32 * 0.5 + (tri[2][0] - center[0]) * scale,
                height as f32 * 0.5 - (tri[2][1] - center[1]) * scale,
                tri[2][2],
            ],
        ];

        let area = edge(screen[0], screen[1], screen[2]);
        if area.abs() < 1e-8 {
            continue;
        }

        let normal = [
            (screen[1][1] - screen[0][1]) * (screen[2][2] - screen[0][2])
                - (screen[1][2] - screen[0][2]) * (screen[2][1] - screen[0][1]),
            (screen[1][2] - screen[0][2]) * (screen[2][0] - screen[0][0])
                - (screen[1][0] - screen[0][0]) * (screen[2][2] - screen[0][2]),
            (screen[1][0] - screen[0][0]) * (screen[2][1] - screen[0][1])
                - (screen[1][1] - screen[0][1]) * (screen[2][0] - screen[0][0]),
        ];
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        let light = if length > 1e-6 {
            (0.58 + 0.36 * (0.3 * normal[0] - 0.5 * normal[1] + 0.8 * normal[2]).abs() / length)
                .clamp(0.35, 0.94)
        } else {
            0.58
        };

        let r = (accent[0] as f32 * light + surface_col[0] as f32 * (1. - light) * 0.35) as u8;
        let g = (accent[1] as f32 * light + surface_col[1] as f32 * (1. - light) * 0.35) as u8;
        let b = (accent[2] as f32 * light + surface_col[2] as f32 * (1. - light) * 0.35) as u8;

        let left = screen
            .iter()
            .map(|p| p[0])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.) as u32;
        let right = screen
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .max(0.)
            .min(width as f32) as u32;
        let top = screen
            .iter()
            .map(|p| p[1])
            .fold(f32::INFINITY, f32::min)
            .floor()
            .max(0.) as u32;
        let bottom = screen
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil()
            .max(0.)
            .min(height as f32) as u32;

        work += u64::from(right.saturating_sub(left)) * u64::from(bottom.saturating_sub(top));
        if work > MAX_MODEL_RASTER_WORK {
            return Err("This model is too complex to draw within the preview rendering limit. Try a lower-detail version.".to_string());
        }

        for y in top..bottom {
            for x in left..right {
                let point = [x as f32 + 0.5, y as f32 + 0.5, 0.];
                let w0 = edge(screen[1], screen[2], point) / area;
                let w1 = edge(screen[2], screen[0], point) / area;
                let w2 = edge(screen[0], screen[1], point) / area;
                if w0 >= 0. && w1 >= 0. && w2 >= 0. {
                    let depth = w0 * screen[0][2] + w1 * screen[1][2] + w2 * screen[2][2];
                    let idx = (y * width + x) as usize;
                    if depth > depths[idx] {
                        depths[idx] = depth;
                        // Cairo ARgb32 byte order on little-endian: [B, G, R, A]
                        let offset = idx * 4;
                        pixels[offset] = b;
                        pixels[offset + 1] = g;
                        pixels[offset + 2] = r;
                        pixels[offset + 3] = 255;
                    }
                }
            }
        }
    }

    let surface = cairo::ImageSurface::create_for_data(
        pixels,
        cairo::Format::ARgb32,
        width as i32,
        height as i32,
        (width * 4) as i32,
    )
    .map_err(|e| format!("Failed to create Cairo surface: {e}"))?;

    let mut png_bytes = Vec::new();
    surface
        .write_to_png(&mut png_bytes)
        .map_err(|e| format!("Failed to write PNG: {e}"))?;

    Ok(png_bytes)
}

pub(crate) fn render_model(input: &Path, target_size: u32) -> Result<Vec<u8>, String> {
    let size = target_size.clamp(16, 800);
    let accent = [0x7a, 0xa2, 0xf7]; // TokyoNight blue
    let surface = [0x24, 0x28, 0x3b]; // Dark surface

    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if ext == "3mf" {
        let file = fs::File::open(input).map_err(|e| format!("Unable to open 3MF file: {e}"))?;
        let mut archive = zip::ZipArchive::new(file).map_err(|_| "Invalid or corrupted 3MF archive".to_string())?;
        if archive.len() > MAX_3MF_ARCHIVE_ENTRIES {
            return Err("3MF archive contains too many entries".to_string());
        }

        // Fast path: embedded thumbnail if available
        if let Ok(mut thumb_file) = archive.by_name("Metadata/thumbnail.png") {
            let mut thumb_bytes = Vec::new();
            if thumb_file.read_to_end(&mut thumb_bytes).is_ok()
                && thumb_bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            {
                return Ok(thumb_bytes);
            }
        }

        // Geometry extraction from 3D/3dmodel.model
        let model_file = archive
            .by_name("3D/3dmodel.model")
            .map_err(|_| "3MF archive missing 3D/3dmodel.model".to_string())?;
        let mut xml_bytes = Vec::new();
        model_file
            .take(MAX_MODEL_XML_BYTES + 1)
            .read_to_end(&mut xml_bytes)
            .map_err(|e| format!("Unable to read 3MF model XML: {e}"))?;
        if xml_bytes.len() as u64 > MAX_MODEL_XML_BYTES {
            return Err("3MF model XML exceeds size limit".to_string());
        }

        let faces = triangles_3mf(&xml_bytes)?;
        shade(&faces, size, size, accent, surface)
    } else {
        // STL (ASCII or binary)
        let mut bytes = Vec::new();
        let f = fs::File::open(input).map_err(|e| format!("Unable to open STL file: {e}"))?;
        f.take(MAX_MODEL_INPUT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Unable to read STL file: {e}"))?;
        if bytes.len() as u64 > MAX_MODEL_INPUT_BYTES {
            return Err("STL file exceeds size limit".to_string());
        }

        let faces = stl(&bytes)?;
        shade(&faces, size, size, accent, surface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_stl_parsing_and_rendering() {
        let ascii = b"solid cube\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid cube\n";
        let faces = stl(ascii).expect("parse ascii stl");
        assert_eq!(faces.len(), 1);
        let png = shade(&faces, 64, 64, [100, 150, 200], [30, 30, 30]).expect("shade png");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn test_binary_stl_parsing() {
        let mut binary = vec![0u8; 84 + 50];
        binary[0..5].copy_from_slice(b"solid"); // Even with "solid" header, binary check passes
        binary[80..84].copy_from_slice(&1u32.to_le_bytes()); // 1 triangle
        // Normal 0, 0, 1
        binary[84 + 8..84 + 12].copy_from_slice(&1.0f32.to_le_bytes());
        // Vertex 1: 0, 0, 0
        // Vertex 2: 1, 0, 0
        binary[84 + 24..84 + 28].copy_from_slice(&1.0f32.to_le_bytes());
        // Vertex 3: 0, 1, 0
        binary[84 + 40..84 + 44].copy_from_slice(&1.0f32.to_le_bytes());

        let faces = stl(&binary).expect("parse binary stl");
        assert_eq!(faces.len(), 1);
    }

    #[test]
    fn test_invalid_stl_rejected() {
        assert!(stl(b"not an stl").is_err());
        assert!(stl(b"").is_err());
    }
}
