//! Pictures of a model drawn on the server (MF-40), with no JavaScript: our own small renderer,
//! orthographic, with a depth buffer and flat shading, written to PNG with a clear background.
//! A model with more triangles than `BUDGET` is drawn from an even sample of them, and the
//! pixels filled are capped too, so any mesh draws in bounded time.
//!
//! A 3MF's own thumbnail (re-encoded when it was read) is preferred for the main picture: it's
//! what its maker chose.

use std::io::Cursor;

use image::{ImageFormat, Rgba, RgbaImage};

use super::mesh::Mesh;

/// The most triangles drawn.
pub const BUDGET: usize = 300_000;
/// The most pixels visited while filling triangles (bounding boxes), for one picture.
const MAX_PIXEL_WORK: u64 = 400_000_000;
/// The largest picture drawn, a side.
pub const MAX_SIDE: u32 = 2048;
/// The model's colour, before shading: a blue-grey that reads on light and dark pages.
const COLOUR: [f32; 3] = [96.0, 132.0, 176.0];

/// Where the model is seen from. Models stand on z = 0, z up, as slicers have them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum View {
    /// From the front, to the right and above: the main picture.
    Hero,
    Front,
    Side,
    Top,
}

impl View {
    pub const ALL: [View; 4] = [View::Hero, View::Front, View::Side, View::Top];

    pub fn name(self) -> &'static str {
        match self {
            View::Hero => "hero",
            View::Front => "front",
            View::Side => "side",
            View::Top => "top",
        }
    }

    /// Turn around z, then tilt down, in degrees.
    fn angles(self) -> (f32, f32) {
        match self {
            View::Hero => (-35.0, 30.0),
            // (each a little turned and tilted, so faces are shaded apart rather than a flat outline)
            View::Front => (-12.0, 12.0),
            View::Side => (-102.0, 12.0),
            View::Top => (-20.0, 72.0),
        }
    }
}

/// A point seen from a view: across, up, and how far away.
fn project(v: [f32; 3], (sin_y, cos_y, sin_p, cos_p): (f32, f32, f32, f32)) -> [f32; 3] {
    let [x, y, z] = v;
    let across = x * cos_y - y * sin_y;
    let into = x * sin_y + y * cos_y;
    let up = z * cos_p + into * sin_p;
    let depth = into * cos_p - z * sin_p;
    [across, up, depth]
}

/// Draws `mesh` from `view` on a `side`-pixel square → PNG.
pub fn render(mesh: &Mesh, view: View, side: u32) -> Result<Vec<u8>, String> {
    let side = side.clamp(16, MAX_SIDE);
    let (yaw, pitch) = view.angles();
    let (sin_y, cos_y) = yaw.to_radians().sin_cos();
    let (sin_p, cos_p) = pitch.to_radians().sin_cos();
    let turn = (sin_y, cos_y, sin_p, cos_p);
    let step = mesh.triangles.len().div_ceil(BUDGET).max(1);
    let seen: Vec<[[f32; 3]; 3]> = mesh
        .triangles
        .iter()
        .step_by(step)
        .map(|t| t.map(|v| project(v, turn)))
        .filter(|t| t.iter().flatten().all(|n| n.is_finite()))
        .collect();
    let mut image = RgbaImage::new(side, side);
    if let Some(first) = seen.first().and_then(|t| t.first()) {
        // fit the model in the square with a margin
        let (mut lo, mut hi) = ([first[0], first[1]], [first[0], first[1]]);
        for p in seen.iter().flatten() {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        let usable = side as f32 * 0.9;
        let scale = if extent > f32::EPSILON { usable / extent } else { 1.0 };
        let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        let half = side as f32 / 2.0;
        let to_screen = |p: [f32; 3]| {
            [
                half + (p[0] - centre[0]) * scale,
                half - (p[1] - centre[1]) * scale,
                p[2],
            ]
        };
        let mut depths = vec![f32::INFINITY; (side as usize) * (side as usize)];
        let light = normalise([-0.45, 0.6, -0.65]);
        let mut work: u64 = 0;
        for t in &seen {
            let [a, b, c] = *t;
            let normal = normalise(cross(sub(b, a), sub(c, a)));
            let shade = 0.3 + 0.7 * dot(normal, light).abs();
            let colour = Rgba([
                (COLOUR[0] * shade).clamp(0.0, 255.0) as u8,
                (COLOUR[1] * shade).clamp(0.0, 255.0) as u8,
                (COLOUR[2] * shade).clamp(0.0, 255.0) as u8,
                255,
            ]);
            let [a, b, c] = [to_screen(a), to_screen(b), to_screen(c)];
            let area = edge(a, b, c);
            if !area.is_normal() {
                continue;
            }
            let max = side as f32 - 1.0;
            let x0 = a[0].min(b[0]).min(c[0]).floor().clamp(0.0, max) as u32;
            let x1 = a[0].max(b[0]).max(c[0]).ceil().clamp(0.0, max) as u32;
            let y0 = a[1].min(b[1]).min(c[1]).floor().clamp(0.0, max) as u32;
            let y1 = a[1].max(b[1]).max(c[1]).ceil().clamp(0.0, max) as u32;
            work += u64::from(x1 - x0 + 1) * u64::from(y1 - y0 + 1);
            if work > MAX_PIXEL_WORK {
                break;
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let p = [x as f32 + 0.5, y as f32 + 0.5, 0.0];
                    let (w0, w1, w2) = (edge(b, c, p) / area, edge(c, a, p) / area, edge(a, b, p) / area);
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let depth = w0 * a[2] + w1 * b[2] + w2 * c[2];
                    let at = y as usize * side as usize + x as usize;
                    if let Some(d) = depths.get_mut(at)
                        && depth < *d
                    {
                        *d = depth;
                        image.put_pixel(x, y, colour);
                    }
                }
            }
        }
    }
    let mut out = Cursor::new(Vec::new());
    image
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|e| format!("the picture couldn't be made: {e}"))?;
    Ok(out.into_inner())
}

/// The main picture of a model file → (bytes, content type): its own thumbnail when it came with
/// one (a 3MF's, already re-encoded), else drawn.
pub fn hero(mesh: Option<&Mesh>, pictures: &[(String, Vec<u8>)], side: u32) -> Option<(Vec<u8>, &'static str)> {
    if let Some((name, bytes)) = pictures.first() {
        let lower = name.to_ascii_lowercase();
        let kind = if lower.ends_with(".png") {
            "image/png"
        } else {
            "image/jpeg"
        };
        return Some((bytes.clone(), kind));
    }
    render(mesh?, View::Hero, side).ok().map(|png| (png, "image/png"))
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalise(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    if length.is_normal() {
        [v[0] / length, v[1] / length, v[2] / length]
    } else {
        [0.0, 0.0, 0.0]
    }
}

/// Twice the signed area of a, b, p on screen.
fn edge(a: [f32; 3], b: [f32; 3], p: [f32; 3]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model3d::stl::tests::cube;

    fn mesh(triangles: Vec<[[f32; 3]; 3]>) -> Mesh {
        Mesh {
            total: triangles.len() as u64,
            triangles,
        }
    }

    fn decode(png: &[u8]) -> RgbaImage {
        image::load_from_memory_with_format(png, ImageFormat::Png)
            .unwrap()
            .into_rgba8()
    }

    #[test]
    fn a_cube_renders_to_an_image_of_the_right_size_with_ink_on_it() {
        let m = mesh(cube(10.0));
        for view in View::ALL {
            let picture = decode(&render(&m, view, 200).unwrap());
            assert_eq!(picture.dimensions(), (200, 200));
            let inked: Vec<_> = picture.pixels().filter(|p| p[3] == 255).collect();
            let share = inked.len() as f32 / (200.0 * 200.0);
            assert!(share > 0.2 && share < 0.95, "{view:?}: {share}");
            assert_eq!(picture.get_pixel(0, 0)[3], 0, "the corners are clear");
            let mut shades: Vec<_> = inked.iter().map(|p| p[2]).collect();
            shades.sort_unstable();
            shades.dedup();
            if view == View::Hero {
                assert!(shades.len() >= 3, "three faces seen, each shaded: {shades:?}");
            }
        }
    }

    #[test]
    fn a_degenerate_mesh_renders_without_panicking() {
        let point = [[1.0, 1.0, 1.0]; 3];
        let line = [[0.0, 0.0, 0.0], [5.0, 0.0, 0.0], [10.0, 0.0, 0.0]];
        for m in [
            mesh(vec![point]),
            mesh(vec![line]),
            mesh(Vec::new()),
            mesh(vec![point, line]),
        ] {
            for view in View::ALL {
                let picture = decode(&render(&m, view, 64).unwrap());
                assert_eq!(picture.dimensions(), (64, 64));
            }
        }
        // a flat square seen edge-on from the front
        let flat = vec![[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0]]];
        assert!(render(&mesh(flat), View::Front, 64).is_ok());
        // sides out of range are brought in
        assert_eq!(decode(&render(&mesh(cube(1.0)), View::Hero, 0).unwrap()).width(), 16);
        assert_eq!(
            decode(&render(&mesh(cube(1.0)), View::Hero, 99_999).unwrap()).width(),
            MAX_SIDE
        );
    }

    #[test]
    fn a_huge_mesh_is_reduced_and_renders_in_time() {
        // a million small triangles: a 1000 × 500 grid of squares, gently waved
        let mut triangles = Vec::with_capacity(1_000_000);
        for i in 0..1000 {
            for j in 0..500 {
                let (x, y) = (i as f32 * 0.1, j as f32 * 0.1);
                let z = |x: f32, y: f32| (x * 0.3).sin() + (y * 0.2).cos();
                let p = |x: f32, y: f32| [x, y, z(x, y)];
                triangles.push([p(x, y), p(x + 0.1, y), p(x + 0.1, y + 0.1)]);
                triangles.push([p(x, y), p(x + 0.1, y + 0.1), p(x, y + 0.1)]);
            }
        }
        let m = mesh(triangles);
        let started = std::time::Instant::now();
        let picture = decode(&render(&m, View::Hero, 400).unwrap());
        assert!(picture.pixels().any(|p| p[3] == 255));
        assert!(started.elapsed().as_secs() < 30, "{:?}", started.elapsed());
    }

    #[test]
    fn huge_overlapping_triangles_stop_at_the_pixel_budget() {
        let big = [[0.0, 0.0, 0.0], [1000.0, 0.0, 0.0], [0.0, 0.0, 1000.0]];
        let m = mesh(vec![big; 50_000]);
        let started = std::time::Instant::now();
        assert!(render(&m, View::Front, MAX_SIDE).is_ok());
        assert!(started.elapsed().as_secs() < 60, "{:?}", started.elapsed());
    }

    #[test]
    fn an_embedded_thumbnail_is_preferred() {
        let m = mesh(cube(10.0));
        let own = vec![("Metadata/plate_1.png".to_string(), vec![1, 2, 3])];
        assert_eq!(hero(Some(&m), &own, 100), Some((vec![1, 2, 3], "image/png")));
        let jpeg = vec![("Metadata/thumb.JPG".to_string(), vec![4])];
        assert_eq!(hero(Some(&m), &jpeg, 100).map(|h| h.1), Some("image/jpeg"));
        let (drawn, kind) = hero(Some(&m), &[], 100).unwrap();
        assert_eq!(kind, "image/png");
        assert_eq!(decode(&drawn).width(), 100);
        assert!(hero(None, &[], 100).is_none(), "a STEP file has no picture");
    }
}
