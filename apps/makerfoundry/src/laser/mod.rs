//! Laser-cut and engrave files (MF-2): SVG and DXF read into shapes in millimetres, checked, and
//! drawn. As with embroidery, the preview is an SVG MakerFoundry writes itself from the shapes
//! (numbers and colours only), never the uploaded file: an uploaded SVG can hold scripts and
//! links, and is never served from our site as it is.
//!
//! - SVG: parsed and resolved (units, `viewBox`, transforms) by `usvg`, after refusing XML entity
//!   declarations (entity expansion is a memory bomb) and with no outside files loaded. Text isn't
//!   drawn (laser files convert text to paths) and embedded pictures aren't either; both are counted.
//! - DXF (ASCII, the DXF reference: https://help.autodesk.com/view/OARX/2024/ENU/?guid=GUID-235B22E0-A567-4CF6-92D3-38A2306D73F3):
//!   read here, group code by group code: LINE, LWPOLYLINE (with bulges), POLYLINE, CIRCLE, ARC,
//!   ELLIPSE and SPLINE, in the units `$INSUNITS` gives. Block references (INSERT) are counted,
//!   not drawn.
//! - PDF (`pdf`): the first page's drawing, read with lopdf under limits of our own.

mod pdf;

use std::collections::BTreeMap;
use std::f64::consts::TAU;
use std::fmt::Write as _;

/// The largest laser file accepted.
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
/// The most shapes a design may have, and the most points drawn in all.
pub const MAX_SHAPES: usize = 100_000;
pub const MAX_POINTS: usize = 2_000_000;
/// The largest design accepted (mm, either side): bigger than any laser bed.
pub const MAX_SIZE_MM: f64 = 5000.0;
/// Pieces a curve is flattened into (at most).
pub(crate) const CURVE_STEPS: usize = 48;

/// A colour of a design's lines (laser software uses colours for layers: cut, score, engrave).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LaserColour {
    /// `#rrggbb`.
    pub hex: String,
    pub shapes: usize,
    /// Length of its lines (mm).
    pub length_mm: f64,
}

/// A laser file, read and checked.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Laser {
    /// `SVG`, `DXF` or `PDF`.
    pub format: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub shapes: usize,
    /// Shapes that close on themselves (what a cut frees from the sheet).
    pub closed: usize,
    /// Length of every line (mm): about what the laser travels cutting it all.
    pub length_mm: f64,
    pub colours: Vec<LaserColour>,
    /// What was in the file but isn't drawn: text, pictures, block references.
    pub not_drawn: Vec<String>,
    /// The design drawn, as an SVG element.
    #[serde(skip)]
    pub svg: String,
}

/// One shape: a line through `points` (mm, y down), maybe closed, in a colour.
#[derive(Clone, Debug)]
pub(crate) struct Shape {
    pub(crate) points: Vec<(f64, f64)>,
    pub(crate) closed: bool,
    /// Filled (an engraved area) rather than only a line.
    pub(crate) filled: bool,
    pub(crate) rgb: (u8, u8, u8),
}

/// Shapes as they are read, with the caps kept.
#[derive(Default)]
pub(crate) struct Shapes {
    list: Vec<Shape>,
    points: usize,
}

impl Shapes {
    pub(crate) fn push(&mut self, mut shape: Shape) -> Result<(), String> {
        if shape.points.len() < 2 {
            return Ok(());
        }
        // a line that ends where it began closes, said so or not (a cut frees what's inside)
        if let (Some(a), Some(b)) = (shape.points.first(), shape.points.last())
            && shape.points.len() > 2
            && (a.0 - b.0).abs() < 0.01
            && (a.1 - b.1).abs() < 0.01
        {
            shape.closed = true;
        }
        if shape.points.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
            return Err("the file has lines at impossible positions".into());
        }
        self.points += shape.points.len();
        if self.list.len() >= MAX_SHAPES || self.points > MAX_POINTS {
            return Err(format!(
                "the design has more than {MAX_SHAPES} shapes or {MAX_POINTS} points"
            ));
        }
        self.list.push(shape);
        Ok(())
    }
}

/// Which kind of laser file `bytes` (named `file_name`) is, if it is one.
pub fn kind(bytes: &[u8], file_name: &str) -> Option<&'static str> {
    let ext = file_name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "pdf" || bytes.starts_with(b"%PDF-") {
        return Some("PDF");
    }
    let head = String::from_utf8_lossy(bytes.get(..bytes.len().min(2048)).unwrap_or_default()).to_ascii_lowercase();
    if ext == "svg"
        || head.trim_start_matches('\u{feff}').trim_start().starts_with("<?xml") && head.contains("<svg")
        || head.trim_start().starts_with("<svg")
    {
        return Some("SVG");
    }
    let first = head
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(2)
        .collect::<Vec<_>>();
    if ext == "dxf" || first == ["0", "section"] || head.starts_with("autocad binary dxf") {
        return Some("DXF");
    }
    None
}

/// Reads a laser file of the given `kind` (`SVG`, `DXF` or `PDF`).
pub fn read(bytes: &[u8], kind: &str) -> Result<Laser, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("the file is larger than {} MB", MAX_FILE_BYTES / (1024 * 1024)));
    }
    if bytes.is_empty() {
        return Err("the file is empty".into());
    }
    let (shapes, not_drawn) = match kind {
        "SVG" => read_svg(bytes)?,
        "DXF" => read_dxf(bytes)?,
        "PDF" => pdf::read(bytes)?,
        _ => return Err("this isn't an SVG, DXF or PDF file".into()),
    };
    summarise(kind, shapes.list, not_drawn)
}

// ---------------------------------------------------------------- SVG

fn read_svg(bytes: &[u8]) -> Result<(Shapes, Vec<String>), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "this SVG isn't UTF-8 text".to_string())?;
    let lower = text.to_ascii_lowercase();
    if lower.contains("<!entity") {
        return Err("this SVG declares XML entities, which MakerFoundry doesn't read".into());
    }
    let tree =
        usvg::Tree::from_str(text, &usvg::Options::default()).map_err(|_| "this SVG couldn't be read".to_string())?;
    let mut shapes = Shapes::default();
    let mut counts = BTreeMap::new();
    svg_group(tree.root(), &mut shapes, &mut counts)?;
    // usvg leaves out text it has no font for: say so from the source
    if lower.contains("<text") {
        *counts.entry("text").or_insert(0usize) += 1;
    }
    let mut not_drawn = Vec::new();
    if counts.get("text").is_some_and(|n| *n > 0) {
        not_drawn.push("text (convert it to paths to cut or engrave it)".to_string());
    }
    if let Some(n) = counts.get("image").filter(|n| **n > 0) {
        not_drawn.push(format!("{n} embedded picture{}", if *n == 1 { "" } else { "s" }));
    }
    Ok((shapes, not_drawn))
}

fn svg_rgb(paint: &usvg::Paint) -> (u8, u8, u8) {
    match paint {
        usvg::Paint::Color(c) => (c.red, c.green, c.blue),
        // gradients and patterns: a mid grey stands in
        _ => (128, 128, 128),
    }
}

fn svg_group(
    group: &usvg::Group,
    shapes: &mut Shapes,
    counts: &mut BTreeMap<&'static str, usize>,
) -> Result<(), String> {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => svg_group(g, shapes, counts)?,
            usvg::Node::Path(p) => {
                if !p.is_visible() {
                    continue;
                }
                let (rgb, filled) = match (p.stroke(), p.fill()) {
                    (Some(s), _) => (svg_rgb(s.paint()), p.fill().is_some()),
                    (None, Some(f)) => (svg_rgb(f.paint()), true),
                    (None, None) => continue,
                };
                let ts = p.abs_transform();
                let mut current: Vec<(f64, f64)> = Vec::new();
                let mut last = (0.0f64, 0.0f64);
                // to the page, then from CSS pixels (96 to the inch) into mm
                let px = |x: f32, y: f32| {
                    let mut pt = usvg::tiny_skia_path::Point::from_xy(x, y);
                    ts.map_points(std::slice::from_mut(&mut pt));
                    (f64::from(pt.x) * 25.4 / 96.0, f64::from(pt.y) * 25.4 / 96.0)
                };
                for seg in p.data().segments() {
                    use usvg::tiny_skia_path::PathSegment as S;
                    match seg {
                        S::MoveTo(a) => {
                            shapes.push(Shape {
                                points: std::mem::take(&mut current),
                                closed: false,
                                filled,
                                rgb,
                            })?;
                            last = px(a.x, a.y);
                            current.push(last);
                        }
                        S::LineTo(a) => {
                            last = px(a.x, a.y);
                            current.push(last);
                        }
                        S::QuadTo(c, a) => {
                            let (p0, p1, p2) = (last, px(c.x, c.y), px(a.x, a.y));
                            for i in 1..=CURVE_STEPS {
                                let t = i as f64 / CURVE_STEPS as f64;
                                let u = 1.0 - t;
                                current.push((
                                    u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0,
                                    u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1,
                                ));
                            }
                            last = p2;
                        }
                        S::CubicTo(c1, c2, a) => {
                            let (p0, p1, p2, p3) = (last, px(c1.x, c1.y), px(c2.x, c2.y), px(a.x, a.y));
                            for i in 1..=CURVE_STEPS {
                                let t = i as f64 / CURVE_STEPS as f64;
                                let u = 1.0 - t;
                                let (a0, a1, a2, a3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                                current.push((
                                    a0 * p0.0 + a1 * p1.0 + a2 * p2.0 + a3 * p3.0,
                                    a0 * p0.1 + a1 * p1.1 + a2 * p2.1 + a3 * p3.1,
                                ));
                            }
                            last = p3;
                        }
                        S::Close => {
                            let start = current.first().copied();
                            shapes.push(Shape {
                                points: std::mem::take(&mut current),
                                closed: true,
                                filled,
                                rgb,
                            })?;
                            if let Some(s) = start {
                                last = s;
                                current.push(s);
                            }
                        }
                    }
                }
                shapes.push(Shape {
                    points: current,
                    closed: false,
                    filled,
                    rgb,
                })?;
            }
            usvg::Node::Image(_) => *counts.entry("image").or_insert(0) += 1,
            usvg::Node::Text(_) => *counts.entry("text").or_insert(0) += 1,
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- DXF

/// AutoCAD Color Index colours 1 to 7 (red, yellow, green, cyan, blue, magenta, black/white);
/// others are drawn grey.
fn aci(index: i64) -> (u8, u8, u8) {
    match index {
        1 => (220, 30, 30),
        2 => (210, 170, 0),
        3 => (20, 150, 50),
        4 => (0, 160, 180),
        5 => (30, 60, 220),
        6 => (190, 30, 170),
        _ => (40, 40, 40),
    }
}

/// One entity's group codes and values, in order.
struct Entity<'a> {
    kind: &'a str,
    codes: Vec<(i32, &'a str)>,
}

impl Entity<'_> {
    fn f(&self, code: i32) -> Option<f64> {
        self.codes
            .iter()
            .find(|(c, _)| *c == code)
            .and_then(|(_, v)| v.trim().parse().ok())
            .filter(|v: &f64| v.is_finite())
    }

    fn i(&self, code: i32) -> Option<i64> {
        self.codes
            .iter()
            .find(|(c, _)| *c == code)
            .and_then(|(_, v)| v.trim().parse().ok())
    }

    fn all(&self, code: i32) -> Vec<f64> {
        self.codes
            .iter()
            .filter(|(c, _)| *c == code)
            .filter_map(|(_, v)| v.trim().parse().ok())
            .filter(|v: &f64| v.is_finite())
            .collect()
    }

    fn rgb(&self) -> (u8, u8, u8) {
        aci(self.i(62).unwrap_or(7))
    }
}

/// Points along an arc (radians, counter-clockwise from `a0` to `a1`).
fn arc_points(cx: f64, cy: f64, r: f64, a0: f64, a1: f64) -> Vec<(f64, f64)> {
    let mut sweep = a1 - a0;
    while sweep <= 0.0 {
        sweep += TAU;
    }
    let sweep = sweep.min(TAU);
    let steps = ((sweep / TAU * CURVE_STEPS as f64 * 2.0).ceil() as usize).clamp(2, CURVE_STEPS * 2);
    (0..=steps)
        .map(|i| a0 + sweep * i as f64 / steps as f64)
        .map(|a| (cx + r * a.cos(), cy + r * a.sin()))
        .collect()
}

/// The points of a polyline's vertices, with each bulge (the tangent of a quarter of the arc's
/// angle, from that vertex to the next) drawn as its arc.
fn bulged(vertices: &[(f64, f64, f64)], closed: bool) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let n = vertices.len();
    for i in 0..n {
        let Some(&(x0, y0, bulge)) = vertices.get(i) else { break };
        out.push((x0, y0));
        let next = if i + 1 < n {
            vertices.get(i + 1)
        } else if closed {
            vertices.first()
        } else {
            None
        };
        let Some(&(x1, y1, _)) = next else { continue };
        if bulge.abs() < 1e-9 {
            continue;
        }
        let angle = 4.0 * bulge.atan();
        let chord = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        let half_sin = (angle / 2.0).sin();
        if chord < 1e-12 || half_sin.abs() < 1e-12 {
            continue;
        }
        let r = chord / (2.0 * half_sin);
        let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let d = r * (angle / 2.0).cos();
        let (nx, ny) = (-(y1 - y0) / chord, (x1 - x0) / chord);
        let (cx, cy) = (mx + nx * d, my + ny * d);
        let a0 = (y0 - cy).atan2(x0 - cx);
        let steps = ((angle.abs() / TAU * CURVE_STEPS as f64 * 2.0).ceil() as usize).clamp(2, CURVE_STEPS * 2);
        for s in 1..steps {
            let a = a0 + angle * s as f64 / steps as f64;
            out.push((cx + r.abs() * a.cos(), cy + r.abs() * a.sin()));
        }
    }
    out
}

/// A B-spline of `degree` through `control` points (with `weights`, maybe) over `knots`, by de
/// Boor's algorithm; the control polygon when the knots don't fit.
fn spline(degree: usize, knots: &[f64], control: &[(f64, f64)], weights: &[f64]) -> Vec<(f64, f64)> {
    let n = control.len();
    if degree == 0 || degree > 10 || n <= degree || knots.len() != n + degree + 1 {
        return control.to_vec();
    }
    let w = |i: usize| weights.get(i).copied().filter(|w| *w > 0.0).unwrap_or(1.0);
    let (Some(&lo), Some(&hi)) = (knots.get(degree), knots.get(n)) else {
        return control.to_vec();
    };
    if hi <= lo {
        return control.to_vec();
    }
    let steps = (n * 8).clamp(CURVE_STEPS, 4096);
    let mut out = Vec::with_capacity(steps + 1);
    for s in 0..=steps {
        let t = (lo + (hi - lo) * s as f64 / steps as f64).min(hi - 1e-12 * (hi - lo));
        let Some(k) = (degree..n).rev().find(|&k| knots.get(k).is_some_and(|kv| *kv <= t)) else {
            continue;
        };
        // homogeneous points for the rational case
        let mut d: Vec<(f64, f64, f64)> = (0..=degree)
            .filter_map(|j| {
                control
                    .get(j + k - degree)
                    .map(|&(x, y)| (x * w(j + k - degree), y * w(j + k - degree), w(j + k - degree)))
            })
            .collect();
        for r in 1..=degree {
            for j in (r..=degree).rev() {
                let (Some(&a_lo), Some(&a_hi)) = (knots.get(j + k - degree), knots.get(j + 1 + k - r)) else {
                    continue;
                };
                let alpha = if a_hi > a_lo { (t - a_lo) / (a_hi - a_lo) } else { 0.0 };
                if let (Some(&p), Some(&q)) = (d.get(j - 1), d.get(j))
                    && let Some(slot) = d.get_mut(j)
                {
                    *slot = (
                        (1.0 - alpha) * p.0 + alpha * q.0,
                        (1.0 - alpha) * p.1 + alpha * q.1,
                        (1.0 - alpha) * p.2 + alpha * q.2,
                    );
                }
            }
        }
        if let Some(&(x, y, wt)) = d.get(degree)
            && wt.abs() > 1e-12
        {
            out.push((x / wt, y / wt));
        }
    }
    out
}

/// Millimetres in one drawing unit, from `$INSUNITS` (unitless drawings are taken as mm, as laser
/// software does).
fn unit_mm(insunits: i64) -> f64 {
    match insunits {
        1 => 25.4,
        2 => 304.8,
        5 => 10.0,
        6 => 1000.0,
        8 => 0.0000254,
        9 => 0.0254,
        10 => 914.4,
        _ => 1.0,
    }
}

fn read_dxf(bytes: &[u8]) -> Result<(Shapes, Vec<String>), String> {
    if bytes.starts_with(b"AutoCAD Binary DXF") {
        return Err("this is a binary DXF: save it as an ASCII DXF (any version)".into());
    }
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().collect();
    let mut pairs: Vec<(i32, &str)> = Vec::with_capacity(lines.len() / 2);
    for chunk in lines.chunks(2) {
        let [code, value] = chunk else { break };
        let code: i32 = code
            .trim()
            .parse()
            .map_err(|_| "this DXF couldn't be read (a group code isn't a number)".to_string())?;
        pairs.push((code, value.trim_end_matches('\r')));
    }
    // $INSUNITS in the header
    let mut unit = 1.0;
    if let Some(i) = pairs.iter().position(|(c, v)| *c == 9 && v.trim() == "$INSUNITS")
        && let Some((70, v)) = pairs.get(i + 1)
    {
        unit = unit_mm(v.trim().parse().unwrap_or(0));
    }
    // the ENTITIES section's entities, each its group codes until the next 0
    let start = pairs
        .windows(2)
        .position(|w| matches!(w, [(0, s), (2, e)] if s.trim() == "SECTION" && e.trim() == "ENTITIES"));
    let Some(start) = start else {
        return Err("this DXF has no ENTITIES section".into());
    };
    let mut entities: Vec<Entity<'_>> = Vec::new();
    for &(code, value) in pairs.iter().skip(start + 2) {
        if code == 0 {
            let kind = value.trim();
            if kind == "ENDSEC" {
                break;
            }
            if entities.len() >= MAX_SHAPES * 4 {
                return Err(format!("the design has more than {MAX_SHAPES} shapes"));
            }
            entities.push(Entity {
                kind,
                codes: Vec::new(),
            });
        } else if let Some(e) = entities.last_mut() {
            e.codes.push((code, value));
        }
    }
    let mut shapes = Shapes::default();
    let mut inserts = 0usize;
    let mut skipped = BTreeMap::<String, usize>::new();
    let mm = |(x, y): (f64, f64)| (x * unit, -y * unit);
    let mut i = 0;
    while let Some(e) = entities.get(i) {
        i += 1;
        let rgb = e.rgb();
        let mut push = |points: Vec<(f64, f64)>, closed: bool| {
            shapes.push(Shape {
                points: points.into_iter().map(mm).collect(),
                closed,
                filled: false,
                rgb,
            })
        };
        match e.kind {
            "LINE" => {
                if let (Some(x0), Some(y0), Some(x1), Some(y1)) = (e.f(10), e.f(20), e.f(11), e.f(21)) {
                    push(vec![(x0, y0), (x1, y1)], false)?;
                }
            }
            "CIRCLE" => {
                if let (Some(cx), Some(cy), Some(r)) = (e.f(10), e.f(20), e.f(40)) {
                    push(arc_points(cx, cy, r.abs(), 0.0, TAU), true)?;
                }
            }
            "ARC" => {
                if let (Some(cx), Some(cy), Some(r), Some(a0), Some(a1)) = (e.f(10), e.f(20), e.f(40), e.f(50), e.f(51))
                {
                    push(arc_points(cx, cy, r.abs(), a0.to_radians(), a1.to_radians()), false)?;
                }
            }
            "ELLIPSE" => {
                if let (Some(cx), Some(cy), Some(mx), Some(my), Some(ratio)) =
                    (e.f(10), e.f(20), e.f(11), e.f(21), e.f(40))
                {
                    let (t0, t1) = (e.f(41).unwrap_or(0.0), e.f(42).unwrap_or(TAU));
                    let mut sweep = t1 - t0;
                    while sweep <= 0.0 {
                        sweep += TAU;
                    }
                    let sweep = sweep.min(TAU);
                    let rot = my.atan2(mx);
                    let (a, b) = ((mx * mx + my * my).sqrt(), (mx * mx + my * my).sqrt() * ratio);
                    let steps = CURVE_STEPS * 2;
                    let pts = (0..=steps)
                        .map(|s| t0 + sweep * s as f64 / steps as f64)
                        .map(|t| {
                            (
                                cx + a * t.cos() * rot.cos() - b * t.sin() * rot.sin(),
                                cy + a * t.cos() * rot.sin() + b * t.sin() * rot.cos(),
                            )
                        })
                        .collect();
                    push(pts, (sweep - TAU).abs() < 1e-9)?;
                }
            }
            "LWPOLYLINE" => {
                let closed = e.i(70).unwrap_or(0) & 1 == 1;
                let mut vertices: Vec<(f64, f64, f64)> = Vec::new();
                let mut x = None;
                for &(code, value) in &e.codes {
                    let v: Option<f64> = value.trim().parse().ok().filter(|v: &f64| v.is_finite());
                    match code {
                        10 => x = v,
                        20 => {
                            if let (Some(x), Some(y)) = (x.take(), v) {
                                vertices.push((x, y, 0.0));
                            }
                        }
                        42 => {
                            if let (Some(last), Some(b)) = (vertices.last_mut(), v) {
                                last.2 = b;
                            }
                        }
                        _ => {}
                    }
                }
                push(bulged(&vertices, closed), closed)?;
            }
            "POLYLINE" => {
                let closed = e.i(70).unwrap_or(0) & 1 == 1;
                let mut vertices = Vec::new();
                while let Some(v) = entities.get(i) {
                    if v.kind != "VERTEX" {
                        break;
                    }
                    i += 1;
                    if let (Some(x), Some(y)) = (v.f(10), v.f(20)) {
                        vertices.push((x, y, v.f(42).unwrap_or(0.0)));
                    }
                }
                if entities.get(i).is_some_and(|s| s.kind == "SEQEND") {
                    i += 1;
                }
                push(bulged(&vertices, closed), closed)?;
            }
            "SPLINE" => {
                let degree = usize::try_from(e.i(71).unwrap_or(3)).unwrap_or(3);
                let (xs, ys) = (e.all(10), e.all(20));
                let control: Vec<(f64, f64)> = xs.iter().copied().zip(ys.iter().copied()).collect();
                let (fx, fy) = (e.all(11), e.all(21));
                let pts = if control.is_empty() {
                    fx.into_iter().zip(fy).collect()
                } else {
                    spline(degree, &e.all(40), &control, &e.all(41))
                };
                push(pts, e.i(70).unwrap_or(0) & 1 == 1)?;
            }
            "INSERT" => inserts += 1,
            "VERTEX" | "SEQEND" => {}
            other => *skipped.entry(other.to_string()).or_insert(0) += 1,
        }
    }
    let mut not_drawn = Vec::new();
    if inserts > 0 {
        not_drawn.push(format!(
            "{inserts} block reference{} (explode blocks before saving)",
            if inserts == 1 { "" } else { "s" }
        ));
    }
    for (kind, n) in skipped
        .iter()
        .filter(|(k, _)| matches!(k.as_str(), "TEXT" | "MTEXT" | "HATCH" | "SOLID" | "IMAGE" | "DIMENSION"))
    {
        not_drawn.push(format!("{n} {}", kind.to_lowercase()));
    }
    Ok((shapes, not_drawn))
}

// ---------------------------------------------------------------- summary and preview

fn length(points: &[(f64, f64)], closed: bool) -> f64 {
    let mut l: f64 = points
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .sum();
    if closed && let (Some(a), Some(b)) = (points.first(), points.last()) {
        l += ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    }
    l
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn summarise(format: &str, shapes: Vec<Shape>, not_drawn: Vec<String>) -> Result<Laser, String> {
    if shapes.is_empty() {
        return Err(if not_drawn.is_empty() {
            "the file has no lines or shapes".into()
        } else {
            format!(
                "the file has nothing MakerFoundry can draw, only {}",
                not_drawn.join(", ")
            )
        });
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in shapes.iter().flat_map(|s| s.points.iter()) {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    let (w, h) = (x1 - x0, y1 - y0);
    if !(w.is_finite() && h.is_finite()) || w > MAX_SIZE_MM || h > MAX_SIZE_MM {
        return Err(format!("the design is larger than {MAX_SIZE_MM} mm (check its units)"));
    }
    let mut by_colour: BTreeMap<String, LaserColour> = BTreeMap::new();
    let mut total = 0.0;
    for s in &shapes {
        let l = length(&s.points, s.closed);
        total += l;
        let c = by_colour.entry(hex(s.rgb)).or_insert_with(|| LaserColour {
            hex: hex(s.rgb),
            shapes: 0,
            length_mm: 0.0,
        });
        c.shapes += 1;
        c.length_mm += l;
    }
    let mut colours: Vec<LaserColour> = by_colour.into_values().collect();
    colours.sort_by(|a, b| b.length_mm.total_cmp(&a.length_mm));
    Ok(Laser {
        format: format.to_string(),
        width_mm: w,
        height_mm: h,
        shapes: shapes.len(),
        closed: shapes.iter().filter(|s| s.closed).count(),
        length_mm: total,
        colours,
        not_drawn,
        svg: preview_svg(&shapes, (x0, y0, w, h)),
    })
}

/// The shapes as an SVG: hundredths of a millimetre as whole numbers, lines a constant width
/// whatever the size (`.lines path` in the stylesheet: `vector-effect` isn't inherited), filled areas lightly shaded, on a sheet-coloured ground.
fn preview_svg(shapes: &[Shape], (x0, y0, w, h): (f64, f64, f64, f64)) -> String {
    let u = |v: f64| (v * 100.0).round() as i64;
    let pad = (w.max(h) * 0.04).max(1.0);
    let (vx, vy, vw, vh) = (
        u(x0 - pad),
        u(y0 - pad),
        u(w + 2.0 * pad).max(1),
        u(h + 2.0 * pad).max(1),
    );
    let mut body = String::new();
    for s in shapes {
        let mut d = String::new();
        for (i, (x, y)) in s.points.iter().enumerate() {
            let _ = write!(d, "{}{} {}", if i == 0 { 'M' } else { 'L' }, u(*x), u(*y));
        }
        if s.closed {
            d.push('Z');
        }
        let colour = hex(s.rgb);
        let fill = if s.filled && s.closed {
            format!(r#" fill="{colour}" fill-opacity="0.25""#)
        } else {
            String::new()
        };
        let _ = write!(body, r#"<path stroke="{colour}"{fill} d="{d}"/>"#);
    }
    format!(
        r##"<svg class="stitches lines" viewBox="{vx} {vy} {vw} {vh}" role="img" aria-label="The design's lines"><rect x="{vx}" y="{vy}" width="{vw}" height="{vh}" fill="#fbfaf6"/><g fill="none" stroke-linejoin="round" stroke-linecap="round">{body}</g></svg>"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const SQUARE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 100 50">
<rect x="10" y="10" width="30" height="30" fill="none" stroke="#ff0000"/>
<circle cx="70" cy="25" r="15" fill="none" stroke="#0000ff"/>
<path d="M 0 0 C 10 20 20 20 30 0" stroke="black" fill="none"/>
<text x="5" y="45">Hi</text>
</svg>"##;

    #[test]
    fn an_svg_reads_in_millimetres_with_its_colours() {
        assert_eq!(kind(SQUARE_SVG.as_bytes(), "box.svg"), Some("SVG"));
        let l = read(SQUARE_SVG.as_bytes(), "SVG").unwrap();
        assert_eq!(l.shapes, 3);
        assert_eq!(l.closed, 2);
        // the square is 30 mm a side (the viewBox is in mm), the circle 15 mm across
        let red = l.colours.iter().find(|c| c.hex == "#ff0000").unwrap();
        assert!((red.length_mm - 120.0).abs() < 0.5, "{}", red.length_mm);
        let blue = l.colours.iter().find(|c| c.hex == "#0000ff").unwrap();
        assert!((blue.length_mm - TAU * 15.0).abs() < 1.0, "{}", blue.length_mm);
        assert!(l.not_drawn.iter().any(|n| n.starts_with("text")), "{:?}", l.not_drawn);
        assert!((l.width_mm - 85.0).abs() < 0.5, "{}", l.width_mm);
    }

    #[test]
    fn hostile_svgs_are_refused_or_defused() {
        let bomb = r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;">]><svg xmlns="http://www.w3.org/2000/svg"><text>&b;</text></svg>"#;
        assert!(read(bomb.as_bytes(), "SVG").unwrap_err().contains("entities"));
        let script = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm" viewBox="0 0 10 10" onload="alert(1)"><script>alert(2)</script><a href="javascript:alert(3)"><path d="M0 0L10 10" stroke="red"/></a><image href="file:///etc/passwd" width="5" height="5"/></svg>"#;
        let l = read(script.as_bytes(), "SVG").unwrap();
        for bad in ["script", "alert", "javascript", "onload", "href", "passwd"] {
            assert!(!l.svg.contains(bad), "{bad} in {}", l.svg);
        }
        assert!(read(b"<svg", "SVG").is_err());
        assert!(read(&[0xff, 0xfe, 0x00], "SVG").is_err());
    }

    const DXF: &str = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n1\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\n0\n62\n1\n10\n0\n20\n0\n11\n2\n21\n0\n\
0\nCIRCLE\n8\n0\n62\n5\n10\n1\n20\n1\n40\n0.5\n\
0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n10\n0\n20\n0\n10\n1\n20\n0\n42\n1\n10\n1\n20\n1\n10\n0\n20\n1\n\
0\nARC\n10\n0\n20\n0\n40\n1\n50\n0\n51\n90\n\
0\nINSERT\n2\nLOGO\n10\n0\n20\n0\n\
0\nENDSEC\n0\nEOF\n";

    #[test]
    fn a_dxf_reads_in_its_units() {
        assert_eq!(kind(DXF.as_bytes(), "part.dxf"), Some("DXF"));
        assert_eq!(kind(DXF.as_bytes(), "part"), Some("DXF"));
        let l = read(DXF.as_bytes(), "DXF").unwrap();
        assert_eq!(l.shapes, 4);
        // inches: the 2-inch line is 50.8 mm, red
        let red = l.colours.iter().find(|c| c.hex == "#dc1e1e").unwrap();
        assert!((red.length_mm - 50.8).abs() < 0.01, "{}", red.length_mm);
        // the circle, blue, is 1 inch across
        let blue = l.colours.iter().find(|c| c.hex == "#1e3cdc").unwrap();
        assert!((blue.length_mm - PI * 25.4).abs() < 0.2, "{}", blue.length_mm);
        assert!(
            l.not_drawn.iter().any(|n| n.contains("block reference")),
            "{:?}",
            l.not_drawn
        );
        assert!((l.width_mm - 50.8).abs() < 0.01, "{}", l.width_mm);
    }

    #[test]
    fn a_bulge_of_one_is_a_half_circle() {
        let pts = bulged(&[(0.0, 0.0, 1.0), (2.0, 0.0, 0.0)], false);
        let l = length(&pts, false);
        assert!((l - PI).abs() < 0.01, "{l}");
    }

    #[test]
    fn a_clamped_quadratic_spline_ends_on_its_end_points() {
        let pts = spline(
            2,
            &[0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            &[(0.0, 0.0), (1.0, 2.0), (2.0, 0.0)],
            &[],
        );
        let (first, last) = (pts.first().unwrap(), pts.last().unwrap());
        assert!(
            first.0.abs() < 1e-6 && (last.0 - 2.0).abs() < 1e-3,
            "{first:?} {last:?}"
        );
        assert!(pts.iter().any(|p| (p.1 - 1.0).abs() < 0.05), "peaks at 1");
    }

    #[test]
    fn broken_dxfs_are_refused_with_a_reason() {
        assert!(
            read(b"AutoCAD Binary DXF\r\n\x1a\x00", "DXF")
                .unwrap_err()
                .contains("binary")
        );
        assert!(
            read(b"0\nSECTION\n2\nHEADER\n0\nENDSEC\n0\nEOF\n", "DXF")
                .unwrap_err()
                .contains("ENTITIES")
        );
        assert!(
            read(
                b"0\nSECTION\n2\nENTITIES\n0\nLINE\n10\nnan\n20\n0\n11\n1\n21\n1\n0\nENDSEC\n",
                "DXF"
            )
            .is_err()
        );
        assert!(
            read(b"0\nSECTION\n2\nENTITIES\n0\nTEXT\n1\nHello\n0\nENDSEC\n", "DXF")
                .unwrap_err()
                .contains("text")
        );
        assert!(read(b"x\ny\n", "DXF").is_err());
    }

    #[test]
    fn random_and_damaged_input_never_panics() {
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for i in 0..3000 {
            let base: &[u8] = if i % 2 == 0 {
                DXF.as_bytes()
            } else {
                SQUARE_SVG.as_bytes()
            };
            let mut bytes = base.to_vec();
            for _ in 0..(next() % 12) {
                let at = (next() as usize) % bytes.len().max(1);
                if let Some(b) = bytes.get_mut(at) {
                    *b = b"0123456789-.e\nABCZ<>\"/ "[(next() % 23) as usize];
                }
            }
            bytes.truncate((next() as usize) % (bytes.len() + 1));
            let _ = read(&bytes, if i % 2 == 0 { "DXF" } else { "SVG" });
        }
    }
}
