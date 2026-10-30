//! Wavefront OBJ files (MF-37), read by our own code from the format's public description
//! (<https://paulbourke.net/dataformats/obj/>): only the geometry, `v x y z` corners and `f a b c …`
//! faces (each index `v`, `v/vt`, `v//vn` or `v/vt/vn`, counted from 1, or from the end when
//! negative). Faces with more than three corners are cut into triangles from their first corner.
//! Every other line (normals, texture coordinates, groups, materials, comments) is ignored; a
//! `.mtl` file is kept as a download, never read.

use super::mesh::{MAX_FILE_BYTES, Mesh};

/// The most corners (`v` lines) a file may have.
pub const MAX_VERTICES: usize = 5_000_000;

/// Reads an OBJ file.
pub fn read(bytes: &[u8]) -> Result<Mesh, String> {
    read_with(bytes, MAX_VERTICES)
}

fn read_with(bytes: &[u8], max_vertices: usize) -> Result<Mesh, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("the file is larger than {} MB", MAX_FILE_BYTES / (1024 * 1024)));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the file isn't a text OBJ file".to_string())?;
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut mesh = Mesh::default();
    for (n, line) in text.lines().enumerate() {
        let line_no = n + 1;
        let mut words = line.split_ascii_whitespace();
        match words.next() {
            Some("v") => {
                if vertices.len() >= max_vertices {
                    return Err(format!("the file has more than {max_vertices} vertices"));
                }
                let mut v = [0.0f32; 3];
                for x in &mut v {
                    *x = words
                        .next()
                        .and_then(|w| w.parse::<f32>().ok())
                        .filter(|x| x.is_finite())
                        .ok_or_else(|| format!("line {line_no}: a vertex needs three numbers"))?;
                }
                vertices.push(v);
            }
            Some("f") => {
                let corner = |w: &str| -> Result<[f32; 3], String> {
                    let i = w
                        .split('/')
                        .next()
                        .and_then(|i| i.parse::<i64>().ok())
                        .ok_or_else(|| format!("line {line_no}: a face has a corner that isn't a number"))?;
                    let count = vertices.len() as i64;
                    let at = if i < 0 { count + i } else { i - 1 };
                    usize::try_from(at)
                        .ok()
                        .filter(|_| i != 0)
                        .and_then(|at| vertices.get(at))
                        .copied()
                        .ok_or_else(|| format!("line {line_no}: a face refers to a vertex that isn't there"))
                };
                let (Some(a), Some(b)) = (words.next(), words.next()) else {
                    return Err(format!("line {line_no}: a face needs at least three corners"));
                };
                let first = corner(a)?;
                let mut prev = corner(b)?;
                let mut any = false;
                for w in words {
                    let next = corner(w)?;
                    mesh.push([first, prev, next])?;
                    prev = next;
                    any = true;
                }
                if !any {
                    return Err(format!("line {line_no}: a face needs at least three corners"));
                }
            }
            _ => {}
        }
    }
    mesh.checked()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUBE: &str = "# a 10 mm cube\nmtllib cube.mtl\no cube\nv 0 0 0\nv 10 0 0\nv 10 10 0\nv 0 10 0\n\
v 0 0 10\nv 10 0 10\nv 10 10 10\nv 0 10 10\nvn 0 0 1\nvt 0 0\ng sides\nusemtl red\ns off\n\
f 1 2 3 4\nf 5/1 6/1 7/1 8/1\nf 1//1 2//1 6//1 5//1\nf 4/1/1 3/1/1 7/1/1 8/1/1\nf 1 4 8 5\nf 2 3 7 6\n";

    #[test]
    fn an_obj_cube_with_quads_reads() {
        let m = read(CUBE.as_bytes()).unwrap();
        assert_eq!(m.triangles.len(), 12, "six quads, two triangles each");
        assert_eq!(m.size_mm(), [10.0, 10.0, 10.0]);
        // a pentagon is three triangles
        let m = read(b"v 0 0 0\nv 2 0 0\nv 3 1 0\nv 1 2 0\nv -1 1 0\nf 1 2 3 4 5\n").unwrap();
        assert_eq!(m.triangles.len(), 3);
    }

    #[test]
    fn negative_indexes_resolve() {
        let m = read(b"v 0 0 0\nv 5 0 0\nv 0 5 0\nf -3 -2 -1\nv 0 0 7\nf -4 -3 -1\n").unwrap();
        assert_eq!(m.triangles.len(), 2);
        assert_eq!(m.triangles[1][2], [0.0, 0.0, 7.0]);
        assert_eq!(m.size_mm(), [5.0, 5.0, 7.0]);
    }

    #[test]
    fn out_of_range_and_zero_indexes_are_refused() {
        assert!(read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 4\n").is_err());
        assert!(read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 1 2\n").is_err());
        assert!(read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf -4 1 2\n").is_err());
        assert!(read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 99999999999999999999\n").is_err());
        assert!(read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2\n").is_err(), "two corners");
        assert!(read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 x\n").is_err());
        // a face before its vertices
        assert!(read(b"f 1 2 3\nv 0 0 0\nv 1 0 0\nv 0 1 0\n").is_err());
    }

    #[test]
    fn too_many_vertices_are_refused() {
        let file = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        assert!(read_with(file.as_bytes(), 3).is_ok(), "at the limit");
        assert!(read_with(format!("{file}v 0 0 1\n").as_bytes(), 3).is_err(), "one over");
    }

    #[test]
    fn other_lines_are_ignored() {
        let m = read(b"# comment\nvp 0.5\nl 1 2\ncstype bezier\nv 0 0 0\nv 1 0 0\nv 0 1 0\n\nf 1 2 3\n").unwrap();
        assert_eq!(m.triangles.len(), 1);
    }

    #[test]
    fn bad_numbers_and_non_models_are_refused() {
        assert!(read(b"v 0 0 nan\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").is_err());
        assert!(read(b"v 0 0 inf\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").is_err());
        assert!(read(b"v 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").is_err());
        assert!(read(b"").is_err());
        assert!(read(b"v 0 0 0\n").is_err(), "no faces");
        assert!(read(b"<html><body>hello</body></html>").is_err());
        assert!(read(&[0xff, 0xfe, 0x00]).is_err());
    }
}
