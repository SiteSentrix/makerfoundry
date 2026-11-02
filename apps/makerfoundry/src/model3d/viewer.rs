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

/// The cells along a coarse mesh's longest side (`coarse`).
pub const COARSE_GRID: u32 = 40;
/// The most triangles a coarse mesh has.
pub const COARSE_BUDGET: usize = 4_000;

/// A paid model's mesh for people who haven't bought it (the owner's decision, 2026-11-01): the
/// viewer's mesh `bytes` (as `encode` makes it) with every corner moved to the middle of its cell
/// in a grid `COARSE_GRID` cells along the model's longest side, the triangles that collapse
/// dropped, and at most `COARSE_BUDGET` kept. It turns like the model but is far too rough to
/// print. `None` if `bytes` isn't a viewer mesh.
pub fn coarse(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.get(..4)? != MAGIC {
        return None;
    }
    let n = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize;
    let f = |i: usize| -> Option<f32> {
        let at = 8 + i * 4;
        Some(f32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let (lo, hi) = ([f(0)?, f(1)?, f(2)?], [f(3)?, f(4)?, f(5)?]);
    let body = bytes.get(32..)?;
    let n = n.min(body.len() / 18);
    let extent: Vec<f64> = (0..3)
        .map(|a| {
            let e = f64::from(hi.get(a).copied().unwrap_or(0.0)) - f64::from(lo.get(a).copied().unwrap_or(0.0));
            if e.is_finite() && e > 0.0 { e } else { 0.0 }
        })
        .collect();
    let longest = extent.iter().copied().fold(0.0, f64::max);
    let cell = longest / f64::from(COARSE_GRID);
    // a quantized coordinate on `axis` → its cell and the cell's middle, quantized again
    let snap = |axis: usize, q: u16| -> (u32, u16) {
        let e = extent.get(axis).copied().unwrap_or(0.0);
        if e <= 0.0 || cell <= 0.0 {
            return (0, 0);
        }
        let cells = (e / cell).ceil().max(1.0);
        let at = (f64::from(q) / 65535.0 * e / cell).floor().clamp(0.0, cells - 1.0);
        let middle = ((at + 0.5) * cell).min(e);
        (at as u32, (middle / e * 65535.0).round().clamp(0.0, 65535.0) as u16)
    };
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<[u16; 9]> = Vec::new();
    for t in 0..n {
        let mut cells = [(0u32, 0u32, 0u32); 3];
        let mut out = [0u16; 9];
        for c in 0..3 {
            let mut key = [0u32; 3];
            for a in 0..3 {
                let at = (t * 9 + c * 3 + a) * 2;
                let q = u16::from_le_bytes([*body.get(at)?, *body.get(at + 1)?]);
                let (k, v) = snap(a, q);
                if let (Some(slot), Some(o)) = (key.get_mut(a), out.get_mut(c * 3 + a)) {
                    *slot = k;
                    *o = v;
                }
            }
            if let Some(slot) = cells.get_mut(c) {
                *slot = (key[0], key[1], key[2]);
            }
        }
        let [a, b, c] = cells;
        if a == b || b == c || a == c {
            continue;
        }
        let mut sorted = cells;
        sorted.sort_unstable();
        if seen.insert(sorted) {
            kept.push(out);
        }
    }
    let step = kept.len().div_ceil(COARSE_BUDGET).max(1);
    let kept: Vec<_> = kept.iter().step_by(step).collect();
    let mut out = Vec::with_capacity(32 + kept.len() * 18);
    out.extend_from_slice(MAGIC);
    out.extend(u32::try_from(kept.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(bytes.get(8..32)?);
    for t in kept {
        for v in t {
            out.extend(v.to_le_bytes());
        }
    }
    Some(out)
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

    #[test]
    fn a_coarse_mesh_is_rough_small_and_still_a_mesh() {
        // a finely divided sphere-ish surface: 160 000 triangles over a 50 mm box
        let mut triangles = Vec::new();
        let steps = 200;
        for i in 0..steps {
            for j in 0..steps {
                let p = |i: usize, j: usize| {
                    let (u, v) = (
                        i as f32 / steps as f32 * std::f32::consts::TAU,
                        j as f32 / steps as f32 * std::f32::consts::PI,
                    );
                    [25.0 * u.cos() * v.sin(), 25.0 * u.sin() * v.sin(), 25.0 * v.cos()]
                };
                triangles.push([p(i, j), p(i + 1, j), p(i, j + 1)]);
                triangles.push([p(i + 1, j), p(i + 1, j + 1), p(i, j + 1)]);
            }
        }
        let mesh = Mesh {
            total: triangles.len() as u64,
            triangles,
        };
        let full = encode(&mesh);
        let rough = coarse(&full).unwrap();
        let back = decode(&rough);
        assert!(!back.is_empty() && back.len() <= COARSE_BUDGET, "{}", back.len());
        assert!(rough.len() * 20 < full.len(), "{} vs {}", rough.len(), full.len());
        // every corner sits in the middle of a cell (50 mm / 40 = 1.25 mm), never where it was
        let cell = 50.0 / COARSE_GRID as f32;
        for t in &back {
            for corner in t {
                for v in corner {
                    let r = ((v + 25.0) / cell).fract();
                    assert!((r - 0.5).abs() < 0.01 || *v >= 24.9, "{v}");
                }
            }
        }
        assert_eq!(&rough[8..32], &full[8..32], "the same box");
    }

    #[test]
    fn a_coarse_mesh_of_anything_else_is_none_and_never_panics() {
        assert!(coarse(b"").is_none());
        assert!(coarse(b"MFM2xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx").is_none());
        let mut lying = b"MFM1".to_vec();
        lying.extend(u32::MAX.to_le_bytes());
        lying.extend([0u8; 24]);
        lying.extend([7u8; 40]);
        assert_eq!(decode(&coarse(&lying).unwrap()).len(), 0);
        let mut nan = b"MFM1".to_vec();
        nan.extend(1u32.to_le_bytes());
        for v in [f32::NAN, 0.0, 0.0, f32::INFINITY, 1.0, 1.0] {
            nan.extend(v.to_le_bytes());
        }
        nan.extend([1u8; 18]);
        let _ = coarse(&nan);
    }
}
