// SPDX-License-Identifier: GPL-3.0-or-later

use super::zip_util::create_zip;

/// Generates a valid ASCII STL model (a single tetrahedron / 4 triangles).
pub fn sample_ascii_stl() -> Vec<u8> {
    let content = r#"solid tetrahedron
  facet normal 0.0 0.0 1.0
    outer loop
      vertex 0.0 0.0 0.0
      vertex 1.0 0.0 0.0
      vertex 0.5 0.866 0.0
    endloop
  endfacet
  facet normal 0.0 -0.866 0.5
    outer loop
      vertex 0.0 0.0 0.0
      vertex 1.0 0.0 0.0
      vertex 0.5 0.288 0.816
    endloop
  endfacet
  facet normal -0.75 0.433 0.5
    outer loop
      vertex 0.0 0.0 0.0
      vertex 0.5 0.866 0.0
      vertex 0.5 0.288 0.816
    endloop
  endfacet
  facet normal 0.75 0.433 0.5
    outer loop
      vertex 1.0 0.0 0.0
      vertex 0.5 0.866 0.0
      vertex 0.5 0.288 0.816
    endloop
  endfacet
endsolid tetrahedron
"#;
    content.as_bytes().to_vec()
}

/// Generates a valid binary STL model (80-byte header, triangle count, 50 bytes per triangle).
pub fn sample_binary_stl() -> Vec<u8> {
    let mut out = Vec::with_capacity(84 + 50 * 2);
    // 80-byte header
    out.extend_from_slice(&[b' '; 80]);
    // Number of triangles: 2
    out.extend_from_slice(&2u32.to_le_bytes());

    for _ in 0..2 {
        // Normal vector (nx, ny, nz)
        out.extend_from_slice(&0.0f32.to_le_bytes());
        out.extend_from_slice(&0.0f32.to_le_bytes());
        out.extend_from_slice(&1.0f32.to_le_bytes());

        // Vertex 1 (x, y, z)
        out.extend_from_slice(&0.0f32.to_le_bytes());
        out.extend_from_slice(&0.0f32.to_le_bytes());
        out.extend_from_slice(&0.0f32.to_le_bytes());

        // Vertex 2 (x, y, z)
        out.extend_from_slice(&10.0f32.to_le_bytes());
        out.extend_from_slice(&0.0f32.to_le_bytes());
        out.extend_from_slice(&0.0f32.to_le_bytes());

        // Vertex 3 (x, y, z)
        out.extend_from_slice(&5.0f32.to_le_bytes());
        out.extend_from_slice(&10.0f32.to_le_bytes());
        out.extend_from_slice(&0.0f32.to_le_bytes());

        // Attribute byte count (u16)
        out.extend_from_slice(&0u16.to_le_bytes());
    }

    out
}

/// Generates a valid 3MF 3D model (ZIP container with 3D/3dmodel.model and [Content_Types].xml).
pub fn sample_3mf_model() -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
</Types>"#;

    let model_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
  <resources>
    <object id="1" type="model">
      <mesh>
        <vertices>
          <vertex x="0.0" y="0.0" z="0.0" />
          <vertex x="10.0" y="0.0" z="0.0" />
          <vertex x="10.0" y="10.0" z="0.0" />
          <vertex x="0.0" y="10.0" z="0.0" />
          <vertex x="5.0" y="5.0" z="10.0" />
        </vertices>
        <triangles>
          <triangle v1="0" v2="1" v3="4" />
          <triangle v1="1" v2="2" v3="4" />
          <triangle v1="2" v2="3" v3="4" />
          <triangle v1="3" v2="0" v3="4" />
        </triangles>
      </mesh>
    </object>
  </resources>
  <build>
    <item objectid="1" />
  </build>
</model>"#;

    create_zip(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("3D/3dmodel.model", model_xml.as_bytes()),
    ])
}

pub fn sample_corrupted_stl() -> Vec<u8> {
    let mut data = sample_binary_stl();
    // Claim 1,000,000 triangles but only supply 10 bytes of data
    data[80..84].copy_from_slice(&1_000_000u32.to_le_bytes());
    data.truncate(100);
    data
}
