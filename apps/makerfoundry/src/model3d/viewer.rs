//! The mesh the 3D viewer draws (MF-46): made by the server when a file is uploaded, from the mesh
//! it read, at most `BUDGET` triangles (an even sample beyond), with each corner quantized to 16
//! bits a coordinate inside the model's box. `static/viewer.js` reads it.
//!
//! Layout (little-endian): `MFM1`, the triangle count (u32), the box's smallest and largest corners
//! (6 × f32), then 9 × u16 a triangle.

use super::mesh::Mesh;

/// The most triangles the viewer is sent.
pub const BUDGET: usize = 100_000;
const MAGIC: &[u8; 4] = b"MFM1";

/// The viewer's mesh for `mesh`.
pub fn encode(mesh: &Mesh) -> Vec<u8> {
    let step = mesh.triangles.len().div_ceil(BUDGET).max(1);
    let kept: Vec<_> = mesh.triangles.iter().step_by(step).collect();
    let (lo, hi) = mesh
        .bounds()
        .map(|(lo, hi)| (lo.map(|v| v as f32), hi.map(|v| v as f32)))
        .unwrap_or(([0.0; 3], [0.0; 3]));
    let mut out = Vec::with_capacity(32 + kept.len() * 18);
    out.extend_from_slice(MAGIC);
    out.extend(u32::try_from(kept.len()).unwrap_or(u32::MAX).to_le_bytes());
    for v in lo.iter().chain(hi.iter()) {
        out.extend(v.to_le_bytes());
    }
    for t in kept {
        for corner in t {
            for axis in 0..3 {
                let (l, h) = (lo[axis], hi[axis]);
                let extent = h - l;
                let q = if extent > 0.0 {
                    ((corner[axis] - l) / extent * 65535.0).round().clamp(0.0, 65535.0)
                } else {
                    0.0
                };
                out.extend((q as u16).to_le_bytes());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What viewer.js does, in Rust: the triangles back, in millimetres.
    fn decode(bytes: &[u8]) -> Vec<[[f32; 3]; 3]> {
        assert_eq!(&bytes[..4], MAGIC);
        let n = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let f = |i: usize| f32::from_le_bytes(bytes[8 + i * 4..12 + i * 4].try_into().unwrap());
        let (lo, hi) = ([f(0), f(1), f(2)], [f(3), f(4), f(5)]);
        let body = &bytes[32..];
        assert_eq!(body.len(), n * 18);
        (0..n)
            .map(|t| {
                let mut tri = [[0f32; 3]; 3];
                for (c, corner) in tri.iter_mut().enumerate() {
                    for (a, v) in corner.iter_mut().enumerate() {
                        let at = (t * 9 + c * 3 + a) * 2;
                        let q = u16::from_le_bytes([body[at], body[at + 1]]);
                        *v = lo[a] + f32::from(q) / 65535.0 * (hi[a] - lo[a]);
                    }
                }
                tri
            })
            .collect()
    }

    #[test]
    fn the_mesh_format_round_trips() {
        let triangles = crate::model3d::stl::tests::cube(37.5);
        let mesh = Mesh {
            total: 12,
            triangles: triangles.clone(),
        };
        let back = decode(&encode(&mesh));
        assert_eq!(back.len(), 12);
        for (a, b) in triangles.iter().zip(&back) {
            for (p, q) in a.iter().flatten().zip(b.iter().flatten()) {
                assert!((p - q).abs() < 37.5 / 65535.0 + 1e-4, "{p} {q}");
            }
        }
        // flat in one axis, and empty
        let flat = Mesh {
            total: 1,
            triangles: vec![[[0.0, 0.0, 5.0], [10.0, 0.0, 5.0], [0.0, 10.0, 5.0]]],
        };
        assert_eq!(decode(&encode(&flat))[0][1], [10.0, 0.0, 5.0]);
        assert!(decode(&encode(&Mesh::default())).is_empty());
    }

    #[test]
    fn a_huge_mesh_is_sampled_to_the_budget() {
        let one = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let mesh = Mesh {
            total: 250_000,
            triangles: vec![one; 250_000],
        };
        let bytes = encode(&mesh);
        let n = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert!(n <= BUDGET && n > BUDGET / 2, "{n}");
    }
}
