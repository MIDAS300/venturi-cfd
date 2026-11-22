use crate::error::VenturiError;
use crate::models::{StlMeshPayload, StlSummary};
use std::path::Path;

const MAX_RENDER_TRIANGLES: usize = 80_000;

/// Parses either a Binary or ASCII STL file from disk, computing geometric metrics
/// (bounding box, surface area, signed volume) and returning triangle buffers for 3D rendering.
pub fn parse_stl_file(path: &Path) -> Result<StlMeshPayload, VenturiError> {
    let bytes = std::fs::read(path)?;
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("geometry.stl")
        .to_string();
    let mut payload = parse_stl_bytes(&bytes, &filename)?;
    payload.summary.source_path = Some(path.to_string_lossy().to_string());
    Ok(payload)
}

/// Reads an ASCII or Binary STL file, applies a uniform scale factor to all vertex coordinates,
/// and writes a clean Binary STL file ready for OpenFOAM's `constant/triSurface/`.
pub fn write_scaled_binary_stl(
    src_path: &Path,
    dst_path: &Path,
    scale: f64,
) -> Result<(), VenturiError> {
    let bytes = std::fs::read(src_path)?;
    let is_binary = detect_binary_stl(&bytes);
    let triangles = if is_binary {
        parse_binary_stl(&bytes)?
    } else {
        parse_ascii_stl(&bytes)?
    };
    if triangles.is_empty() {
        return Err(VenturiError::InvalidStl("STL contains zero triangles".into()));
    }

    let s = if scale.is_finite() && scale > 0.0 {
        scale as f32
    } else {
        1.0_f32
    };

    let mut out = Vec::with_capacity(84 + triangles.len() * 50);
    let mut header = [0u8; 80];
    let label = format!("Venturi CFD scaled STL (factor={:.6})", s);
    let label_bytes = label.as_bytes();
    let copy_len = label_bytes.len().min(80);
    header[..copy_len].copy_from_slice(&label_bytes[..copy_len]);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(triangles.len() as u32).to_le_bytes());

    for tri in &triangles {
        let n = ensure_unit_normal(tri.normal, &tri.verts);
        for c in n {
            out.extend_from_slice(&c.to_le_bytes());
        }
        for v in &tri.verts {
            out.extend_from_slice(&(v[0] * s).to_le_bytes());
            out.extend_from_slice(&(v[1] * s).to_le_bytes());
            out.extend_from_slice(&(v[2] * s).to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }

    std::fs::write(dst_path, out)?;
    Ok(())
}

pub fn parse_stl_bytes(bytes: &[u8], filename: &str) -> Result<StlMeshPayload, VenturiError> {
    if bytes.len() < 15 {
        return Err(VenturiError::InvalidStl("File is too small to be a valid STL".into()));
    }

    let is_binary = detect_binary_stl(bytes);
    let triangles = if is_binary {
        parse_binary_stl(bytes)?
    } else {
        parse_ascii_stl(bytes)?
    };

    if triangles.is_empty() {
        return Err(VenturiError::InvalidStl("STL contains zero triangles".into()));
    }

    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    let mut total_area = 0.0_f64;
    let mut signed_vol6 = 0.0_f64;

    for tri in &triangles {
        for v in &tri.verts {
            for axis in 0..3 {
                let c = v[axis] as f64;
                if c < min[axis] {
                    min[axis] = c;
                }
                if c > max[axis] {
                    max[axis] = c;
                }
            }
        }
        let a = [tri.verts[0][0] as f64, tri.verts[0][1] as f64, tri.verts[0][2] as f64];
        let b = [tri.verts[1][0] as f64, tri.verts[1][1] as f64, tri.verts[1][2] as f64];
        let c = [tri.verts[2][0] as f64, tri.verts[2][1] as f64, tri.verts[2][2] as f64];

        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        total_area += 0.5 * (cross[0].powi(2) + cross[1].powi(2) + cross[2].powi(2)).sqrt();

        // Signed volume of tetrahedron (O, a, b, c) * 6 = a . (b x c)
        let bxc = [
            b[1] * c[2] - b[2] * c[1],
            b[2] * c[0] - b[0] * c[2],
            b[0] * c[1] - b[1] * c[0],
        ];
        signed_vol6 += a[0] * bxc[0] + a[1] * bxc[1] + a[2] * bxc[2];
    }

    let stride = (triangles.len() / MAX_RENDER_TRIANGLES).max(1);
    let rendered_count = triangles.len().div_ceil(stride);
    let mut positions = Vec::with_capacity(rendered_count * 9);
    let mut normals = Vec::with_capacity(rendered_count * 9);

    for (idx, tri) in triangles.iter().enumerate() {
        if idx % stride != 0 {
            continue;
        }
        let n = ensure_unit_normal(tri.normal, &tri.verts);
        for v in &tri.verts {
            positions.extend_from_slice(v);
            normals.extend_from_slice(&n);
        }
    }

    Ok(StlMeshPayload {
        summary: StlSummary {
            filename: filename.to_string(),
            triangle_count: triangles.len(),
            is_binary,
            bounds_min: min,
            bounds_max: max,
            surface_area: total_area,
            volume_est: (signed_vol6 / 6.0).abs(),
            scale_factor: 1.0,
            source_path: None,
        },
        positions,
        normals,
    })
}

struct Triangle {
    normal: [f32; 3],
    verts: [[f32; 3]; 3],
}

fn detect_binary_stl(bytes: &[u8]) -> bool {
    if bytes.len() >= 84 {
        let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        if 84 + count.saturating_mul(50) == bytes.len() {
            return true;
        }
    }
    // Check if starts with "solid" and contains "facet"
    let head = std::str::from_utf8(&bytes[..bytes.len().min(512)]).unwrap_or("");
    !(head.trim_start().starts_with("solid") && head.contains("facet"))
}

fn parse_binary_stl(bytes: &[u8]) -> Result<Vec<Triangle>, VenturiError> {
    if bytes.len() < 84 {
        return Err(VenturiError::InvalidStl("Binary STL shorter than 84-byte header".into()));
    }
    let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
    let expected = 84 + count
        .checked_mul(50)
        .ok_or_else(|| VenturiError::InvalidStl("Triangle count overflow".into()))?;
    if bytes.len() < expected {
        return Err(VenturiError::InvalidStl(format!(
            "Binary STL truncated: expected {} bytes for {} triangles, found {}",
            expected,
            count,
            bytes.len()
        )));
    }

    let mut tris = Vec::with_capacity(count);
    let mut offset = 84;
    for _ in 0..count {
        let read_f32 = |o: usize| -> f32 {
            f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
        };
        let normal = [read_f32(offset), read_f32(offset + 4), read_f32(offset + 8)];
        let v0 = [read_f32(offset + 12), read_f32(offset + 16), read_f32(offset + 20)];
        let v1 = [read_f32(offset + 24), read_f32(offset + 28), read_f32(offset + 32)];
        let v2 = [read_f32(offset + 36), read_f32(offset + 40), read_f32(offset + 44)];
        tris.push(Triangle {
            normal,
            verts: [v0, v1, v2],
        });
        offset += 50;
    }
    Ok(tris)
}

fn parse_ascii_stl(bytes: &[u8]) -> Result<Vec<Triangle>, VenturiError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| VenturiError::InvalidStl("ASCII STL contains invalid UTF-8".into()))?;
    let mut tris = Vec::new();
    let mut current_normal = [0.0_f32; 3];
    let mut verts: Vec<[f32; 3]> = Vec::with_capacity(3);

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if let Some(rest) = line.strip_prefix("facet normal") {
            let nums: Vec<f32> = rest
                .split_whitespace()
                .filter_map(|s| s.parse::<f32>().ok())
                .collect();
            if nums.len() == 3 {
                current_normal = [nums[0], nums[1], nums[2]];
            }
            verts.clear();
        } else if let Some(rest) = line.strip_prefix("vertex") {
            let nums: Vec<f32> = rest
                .split_whitespace()
                .filter_map(|s| s.parse::<f32>().ok())
                .collect();
            if nums.len() == 3 {
                verts.push([nums[0], nums[1], nums[2]]);
            }
        } else if line.starts_with("endfacet") && verts.len() == 3 {
            tris.push(Triangle {
                normal: current_normal,
                verts: [verts[0], verts[1], verts[2]],
            });
            verts.clear();
        }
    }

    Ok(tris)
}

fn ensure_unit_normal(n: [f32; 3], verts: &[[f32; 3]; 3]) -> [f32; 3] {
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 1e-6 && len.is_finite() {
        return [n[0] / len, n[1] / len, n[2] / len];
    }
    let ab = [
        verts[1][0] - verts[0][0],
        verts[1][1] - verts[0][1],
        verts[1][2] - verts[0][2],
    ];
    let ac = [
        verts[2][0] - verts[0][0],
        verts[2][1] - verts[0][1],
        verts[2][2] - verts[0][2],
    ];
    let c = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let clen = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt().max(1e-8);
    [c[0] / clen, c[1] / clen, c[2] / clen]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ascii_stl_unit_triangle() {
        let ascii = r#"
solid unit
  facet normal 0 0 1
    outer loop
      vertex 0 0 0
      vertex 2 0 0
      vertex 0 3 0
    endloop
  endfacet
endsolid unit
"#;
        let payload = parse_stl_bytes(ascii.as_bytes(), "tri.stl").unwrap();
        assert_eq!(payload.summary.triangle_count, 1);
        assert!(!payload.summary.is_binary);
        assert!((payload.summary.surface_area - 3.0).abs() < 1e-6);
        assert_eq!(payload.positions.len(), 9);
    }

    #[test]
    fn parses_binary_stl() {
        let mut buf = vec![0u8; 80];
        buf.extend_from_slice(&1u32.to_le_bytes());
        // normal
        for v in [0.0f32, 0.0, 1.0] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        // vertices
        for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.extend_from_slice(&0u16.to_le_bytes());

        let payload = parse_stl_bytes(&buf, "bin.stl").unwrap();
        assert!(payload.summary.is_binary);
        assert_eq!(payload.summary.triangle_count, 1);
        assert!((payload.summary.surface_area - 0.5).abs() < 1e-6);
    }
}
