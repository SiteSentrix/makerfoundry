//! STL files (MF-36), binary and ASCII, read by our own code from the format's public description
//! (<https://www.fabbers.com/tech/STL_Format>; also
//! <https://en.wikipedia.org/wiki/STL_(file_format)>). A binary STL is an 80-byte header, a
//! little-endian `u32` triangle count, then 50 bytes a triangle (a normal, three corners, as
//! `f32`s, and a 2-byte attribute). An ASCII STL is `solid … facet normal … outer loop vertex x y
//! z … endloop endfacet … endsolid`. Normals are ignored: they're worked out from the corners when
//! drawing. Units aren't in the file; millimetres are assumed, as slicers do.

use super::mesh::{MAX_FILE_BYTES, Mesh};

const HEADER: usize = 80;
const TRIANGLE: u64 = 50;

/// Reads an STL file, binary or ASCII.
pub fn read(bytes: &[u8]) -> Result<Mesh, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("the file is larger than {} MB", MAX_FILE_BYTES / (1024 * 1024)));
    }
    // A binary file whose count matches its size is binary, even if its header starts with
    // "solid" (some programs write that).
    if let Some(count) = binary_count(bytes) {
        let expected = (HEADER as u64 + 4).saturating_add(u64::from(count).saturating_mul(TRIANGLE));
        if expected == bytes.len() as u64 {
            return read_binary(bytes, count);
        }
    }
    let text_start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    if bytes.get(text_start..).is_some_and(|t| t.starts_with(b"solid")) {
        return read_ascii(bytes);
    }
    match binary_count(bytes) {
        Some(_) => Err("the file isn't an STL: its triangle count doesn't match its size".into()),
        None => Err("the file is too short to be an STL".into()),
    }
}

fn binary_count(bytes: &[u8]) -> Option<u32> {
    let c = bytes.get(HEADER..HEADER + 4)?;
    Some(u32::from_le_bytes([*c.first()?, *c.get(1)?, *c.get(2)?, *c.get(3)?]))
}

fn f32_at(b: &[u8], at: usize) -> f32 {
    match b.get(at..at + 4) {
        Some(&[a, b, c, d]) => f32::from_le_bytes([a, b, c, d]),
        _ => f32::NAN,
    }
}

fn read_binary(bytes: &[u8], count: u32) -> Result<Mesh, String> {
    if count == 0 {
        return Err("the file has no triangles".into());
    }
    let mut mesh = Mesh::default();
    // (the count was checked against the file's size, itself capped)
    mesh.triangles.reserve((count as usize).min(super::mesh::MAX_TRIANGLES));
    for chunk in bytes
        .get(HEADER + 4..)
        .unwrap_or_default()
        .chunks_exact(TRIANGLE as usize)
    {
        // skip the normal (12 bytes); three corners
        let v = |i: usize| {
            [
                f32_at(chunk, 12 + i * 12),
                f32_at(chunk, 16 + i * 12),
                f32_at(chunk, 20 + i * 12),
            ]
        };
        mesh.push([v(0), v(1), v(2)])?;
    }
    mesh.checked()
}

fn read_ascii(bytes: &[u8]) -> Result<Mesh, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "the file isn't a text STL".to_string())?;
    let mut mesh = Mesh::default();
    let mut corners: Vec<[f32; 3]> = Vec::with_capacity(3);
    let mut words = text.split_ascii_whitespace();
    while let Some(word) = words.next() {
        if !word.eq_ignore_ascii_case("vertex") {
            continue;
        }
        let mut v = [0.0f32; 3];
        for n in &mut v {
            *n = words
                .next()
                .and_then(|w| w.parse::<f32>().ok())
                .ok_or_else(|| "a vertex in the file doesn't have three numbers".to_string())?;
        }
        corners.push(v);
        if let [a, b, c] = corners[..] {
            mesh.push([a, b, c])?;
            corners.clear();
        }
    }
    if !corners.is_empty() {
        return Err("the file ends part-way through a triangle".into());
    }
    mesh.checked()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model3d::mesh::Triangle;

    /// A cube `side` mm across, as 12 triangles.
    pub(crate) fn cube(side: f32) -> Vec<Triangle> {
        let p = |x: u8, y: u8, z: u8| [f32::from(x) * side, f32::from(y) * side, f32::from(z) * side];
        let quads = [
            [p(0, 0, 0), p(1, 0, 0), p(1, 1, 0), p(0, 1, 0)],
            [p(0, 0, 1), p(1, 0, 1), p(1, 1, 1), p(0, 1, 1)],
            [p(0, 0, 0), p(1, 0, 0), p(1, 0, 1), p(0, 0, 1)],
            [p(0, 1, 0), p(1, 1, 0), p(1, 1, 1), p(0, 1, 1)],
            [p(0, 0, 0), p(0, 1, 0), p(0, 1, 1), p(0, 0, 1)],
            [p(1, 0, 0), p(1, 1, 0), p(1, 1, 1), p(1, 0, 1)],
        ];
        quads
            .iter()
            .flat_map(|[a, b, c, d]| [[*a, *b, *c], [*a, *c, *d]])
            .collect()
    }

    pub(crate) fn binary(header: &[u8], triangles: &[Triangle]) -> Vec<u8> {
        let mut out = vec![0u8; 80];
        out[..header.len()].copy_from_slice(header);
        out.extend((triangles.len() as u32).to_le_bytes());
        for t in triangles {
            out.extend([0u8; 12]);
            for v in t {
                for n in v {
                    out.extend(n.to_le_bytes());
                }
            }
            out.extend([0u8; 2]);
        }
        out
    }

    pub(crate) fn ascii(triangles: &[Triangle]) -> Vec<u8> {
        let mut out = String::from("solid cube\n");
        for t in triangles {
            out.push_str("  facet normal 0 0 0\n    outer loop\n");
            for v in t {
                out.push_str(&format!("      vertex {} {} {}\n", v[0], v[1], v[2]));
            }
            out.push_str("    endloop\n  endfacet\n");
        }
        out.push_str("endsolid cube\n");
        out.into_bytes()
    }

    #[test]
    fn a_binary_cube_reads() {
        let m = read(&binary(b"made by a test", &cube(10.0))).unwrap();
        assert_eq!(m.triangles.len(), 12);
        assert_eq!(m.total, 12);
        assert_eq!(m.size_mm(), [10.0, 10.0, 10.0]);
    }

    #[test]
    fn an_ascii_cube_reads() {
        let m = read(&ascii(&cube(10.0))).unwrap();
        assert_eq!(m.triangles.len(), 12);
        assert_eq!(m.size_mm(), [10.0, 10.0, 10.0]);
        // scientific notation and odd spacing
        let m = read(b"solid x\nfacet normal 0 0 1 outer loop\nvertex 0 0 0\nvertex 1e1 0 0\n  vertex 0 1.0E1 0\nendloop endfacet endsolid").unwrap();
        assert_eq!(m.size_mm(), [10.0, 10.0, 0.0]);
    }

    #[test]
    fn a_solid_word_in_a_binary_header_still_reads_as_binary() {
        let m = read(&binary(b"solid cube made by a program that says so", &cube(5.0))).unwrap();
        assert_eq!(m.triangles.len(), 12);
    }

    #[test]
    fn a_binary_stl_whose_count_doesnt_match_its_size_is_refused() {
        let mut b = binary(b"", &cube(10.0));
        b.pop();
        assert!(read(&b).is_err());
        let mut b = binary(b"", &cube(10.0));
        b.extend([0u8; 50]);
        assert!(read(&b).is_err());
    }

    #[test]
    fn a_huge_claimed_count_is_refused_without_allocating() {
        let mut b = binary(b"", &cube(10.0));
        b[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read(&b).unwrap_err().contains("doesn't match"));
    }

    #[test]
    fn nan_and_infinite_numbers_are_refused() {
        let mut t = cube(10.0);
        t[3][1][2] = f32::NAN;
        assert!(read(&binary(b"", &t)).is_err());
        t[3][1][2] = f32::INFINITY;
        assert!(read(&binary(b"", &t)).is_err());
        assert!(
            read(b"solid x facet outer loop vertex 0 0 0 vertex NaN 0 0 vertex 0 1 0 endloop endfacet endsolid")
                .is_err()
        );
        assert!(
            read(b"solid x facet outer loop vertex 0 0 0 vertex inf 0 0 vertex 0 1 0 endloop endfacet endsolid")
                .is_err()
        );
    }

    #[test]
    fn empty_and_tiny_files_are_refused() {
        assert!(read(b"").is_err());
        assert!(read(b"solid").is_err());
        assert!(read(b"solid x endsolid x").is_err());
        assert!(read(&binary(b"", &[])).is_err());
        assert!(
            read(b"solid x facet outer loop vertex 0 0 0 vertex 1 0 0 endloop").is_err(),
            "two corners"
        );
        assert!(
            read(b"solid x facet outer loop vertex 0 0 endloop").is_err(),
            "two numbers"
        );
    }

    #[test]
    fn an_html_file_named_stl_is_refused() {
        let page = b"<!doctype html><html><head><title>Not a model</title></head><body><script>alert(1)</script></body></html>";
        assert!(read(page).is_err());
    }

    #[test]
    fn a_model_too_big_for_any_printer_is_refused() {
        assert!(read(&binary(b"", &cube(5000.0))).is_ok());
        assert!(read(&binary(b"", &cube(5001.0))).unwrap_err().contains("5000 mm"));
    }
}
