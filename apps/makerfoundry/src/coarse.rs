//! The lower-detail picture of a paid design (the owner's decision, 2026-11-01): people who
//! haven't bought a paid embroidery or laser design see a small PNG drawn on the server, never
//! the preview SVG, whose stitches (every one at 0.1 mm, with its colour blocks) and lines (every
//! point at 0.01 mm) are enough to rebuild the file without buying it. The picture looks like
//! the design but is at most `MAX_SIDE` pixels across and a quarter of a millimetre a pixel
//! (`MAX_PX_PER_MM`): stitches run together into areas of colour,
//! and nothing in it is a stitch or a point to copy.
//!
//! It is drawn from the preview SVG MakerFoundry wrote itself (`embroidery::preview_svg`,
//! `laser::preview_svg`: a `viewBox`, a ground `rect`, then `path`s of `M`/`L`/`Z` and whole
//! numbers), never from an uploaded file; anything else in it is skipped.

use std::io::Cursor;

use image::{ImageFormat, Rgb, RgbImage};

/// The longest side of the picture, in pixels.
pub const MAX_SIDE: u32 = 560;
/// The most pixels a millimetre of the design gets: a quarter of a millimetre a pixel, so a
/// small design isn't drawn large enough to show where each stitch goes in (the page scales the
/// picture up instead).
pub const MAX_PX_PER_MM: f64 = 4.0;
/// The most line pieces drawn: more than any design has (`embroidery::MAX_STITCHES`), so a
/// strange SVG can't make a picture take long.
const MAX_SEGMENTS: usize = 1_000_000;
/// Line pieces longer than this (pixels) are drawn in parts, so each covers few pixels.
const PIECE: f64 = 8.0;

/// One path of the preview: its stroke, its fill (with opacity) if any, its points (a pen lift
/// before each `true`), and whether it closes.
struct PathSpec {
    stroke: Rgb<u8>,
    fill: Option<Rgb<u8>>,
    points: Vec<(bool, f64, f64)>,
    closed: bool,
}

/// The value of attribute `name` in `tag` (the text of one element).
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let rest = tag.get(start..)?;
    rest.get(..rest.find('"')?)
}

/// `#rrggbb` → a colour.
fn colour(hex: &str) -> Option<Rgb<u8>> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let byte = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok());
    Some(Rgb([byte(0)?, byte(2)?, byte(4)?]))
}

/// The points of a `d` made of `M`, `L` and `Z` with whole numbers → (points, closed).
fn points(d: &str) -> (Vec<(bool, f64, f64)>, bool) {
    let mut out = Vec::new();
    let mut closed = false;
    let mut lift = true;
    let mut numbers: Vec<f64> = Vec::with_capacity(2);
    let mut word = String::new();
    let take = |word: &mut String, numbers: &mut Vec<f64>, out: &mut Vec<(bool, f64, f64)>, lift: &mut bool| {
        if let Ok(v) = word.parse::<f64>()
            && v.is_finite()
        {
            numbers.push(v);
        }
        word.clear();
        if let [x, y] = numbers[..] {
            out.push((*lift, x, y));
            *lift = false;
            numbers.clear();
        }
    };
    for c in d.chars() {
        match c {
            'M' => {
                take(&mut word, &mut numbers, &mut out, &mut lift);
                numbers.clear();
                lift = true;
            }
            'L' => {
                take(&mut word, &mut numbers, &mut out, &mut lift);
                numbers.clear();
            }
            'Z' => {
                take(&mut word, &mut numbers, &mut out, &mut lift);
                closed = true;
            }
            ' ' | ',' => take(&mut word, &mut numbers, &mut out, &mut lift),
            '-' | '0'..='9' | '.' => word.push(c),
            _ => {}
        }
    }
    take(&mut word, &mut numbers, &mut out, &mut lift);
    (out, closed)
}

/// The preview SVG read back: its view box, ground colour, whether it is a laser design's lines,
/// and its paths.
struct Drawing {
    view: (f64, f64, f64, f64),
    ground: Rgb<u8>,
    lines: bool,
    paths: Vec<PathSpec>,
}

fn read(svg: &str) -> Option<Drawing> {
    let head = svg.get(..svg.find('>')?)?;
    if !head.starts_with("<svg") {
        return None;
    }
    let view: Vec<f64> = attr(head, "viewBox")?
        .split_whitespace()
        .filter_map(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .collect();
    let [vx, vy, vw, vh] = view[..] else { return None };
    if vw <= 0.0 || vh <= 0.0 {
        return None;
    }
    let lines = attr(head, "class").is_some_and(|c| c.split_whitespace().any(|w| w == "lines"));
    let ground = svg
        .find("<rect")
        .and_then(|at| svg.get(at..))
        .and_then(|rest| rest.get(..rest.find('>')?))
        .and_then(|rect| attr(rect, "fill"))
        .and_then(colour)
        .unwrap_or(Rgb([0xf4, 0xf0, 0xe8]));
    let mut paths = Vec::new();
    let mut rest = svg;
    while let Some(at) = rest.find("<path ") {
        let Some(tail) = rest.get(at..) else { break };
        let Some(end) = tail.find('>') else { break };
        let tag = tail.get(..end).unwrap_or("");
        rest = tail.get(end..).unwrap_or("");
        let Some(stroke) = attr(tag, "stroke").and_then(colour) else {
            continue;
        };
        let fill = attr(tag, "fill").and_then(colour);
        let (pts, closed) = points(attr(tag, "d").unwrap_or(""));
        paths.push(PathSpec {
            stroke,
            fill,
            points: pts,
            closed,
        });
    }
    Some(Drawing {
        view: (vx, vy, vw, vh),
        ground,
        lines,
        paths,
    })
}

/// The picture's size in pixels for the preview `svg` (`None` if it isn't one of ours).
pub fn size(svg: &str) -> Option<(u32, u32)> {
    let d = read(svg)?;
    Some(fit(&d).0)
}

/// The picture's size and its pixels a unit of the view box: at most `MAX_SIDE` pixels
/// across, and at most `MAX_PX_PER_MM` (embroidery units are tenths of a millimetre, laser ones
/// hundredths).
fn fit(d: &Drawing) -> ((u32, u32), f64) {
    let (w, h) = (d.view.2, d.view.3);
    let unit_mm = if d.lines { 0.01 } else { 0.1 };
    let scale = (f64::from(MAX_SIDE) / w.max(h)).min(MAX_PX_PER_MM * unit_mm);
    let px = |v: f64| ((v * scale).round() as u32).clamp(1, MAX_SIDE);
    ((px(w), px(h)), scale)
}

/// Blends `c` over pixel (`x`, `y`) by `a` (0 to 1).
fn blend(img: &mut RgbImage, x: u32, y: u32, c: Rgb<u8>, a: f64) {
    if let Some(p) = img.get_pixel_mut_checked(x, y) {
        for i in 0..3 {
            let under = f64::from(p.0[i]);
            p.0[i] = (under + (f64::from(c.0[i]) - under) * a).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// A line from `a` to `b` (pixels), `half` pixels either side, smoothed at its edges.
fn line(img: &mut RgbImage, a: (f64, f64), b: (f64, f64), half: f64, c: Rgb<u8>) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let reach = half + 1.0;
    let (w, h) = (f64::from(img.width()), f64::from(img.height()));
    let x0 = (a.0.min(b.0) - reach).floor().max(0.0);
    let x1 = (a.0.max(b.0) + reach).ceil().min(w - 1.0);
    let y0 = (a.1.min(b.1) - reach).floor().max(0.0);
    let y1 = (a.1.max(b.1) + reach).ceil().min(h - 1.0);
    if x0 > x1 || y0 > y1 {
        return;
    }
    let (x0, x1, y0, y1) = (x0 as u32, x1 as u32, y0 as u32, y1 as u32);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let t = if len2 > 0.0 {
                (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (qx, qy) = (a.0 + t * dx - px, a.1 + t * dy - py);
            let cover = (half + 0.5 - (qx * qx + qy * qy).sqrt()).clamp(0.0, 1.0);
            if cover > 0.0 {
                blend(img, x, y, c, cover);
            }
        }
    }
}

/// A closed shape filled (even-odd) with `c` at `a`.
fn fill(img: &mut RgbImage, poly: &[(f64, f64)], c: Rgb<u8>, a: f64) {
    if poly.len() < 3 {
        return;
    }
    let lo = poly.iter().map(|p| p.1).fold(f64::INFINITY, f64::min).floor().max(0.0);
    let hi = poly
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .min(f64::from(img.height()));
    if lo.partial_cmp(&hi) != Some(std::cmp::Ordering::Less) {
        return;
    }
    let mut xs: Vec<f64> = Vec::new();
    for y in (lo as u32)..(hi as u32) {
        let py = f64::from(y) + 0.5;
        xs.clear();
        for (i, p) in poly.iter().enumerate() {
            let q = poly.get((i + 1) % poly.len()).unwrap_or(p);
            if (p.1 <= py) != (q.1 <= py) {
                xs.push(p.0 + (py - p.1) / (q.1 - p.1) * (q.0 - p.0));
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks_exact(2) {
            let [from, to] = pair else { continue };
            let (from, to) = (from.round().max(0.0), to.round().min(f64::from(img.width())));
            if from < to {
                for x in (from as u32)..(to as u32) {
                    blend(img, x, y, c, a);
                }
            }
        }
    }
}

/// The lower-detail picture of the preview `svg`, as PNG bytes (`None` if it isn't one of ours).
pub fn picture(svg: &str) -> Option<Vec<u8>> {
    let d = read(svg)?;
    let (vx, vy) = (d.view.0, d.view.1);
    let ((w, h), scale) = fit(&d);
    let at = |x: f64, y: f64| ((x - vx) * scale, (y - vy) * scale);
    let mut img = RgbImage::from_pixel(w, h, d.ground);
    // embroidery: the thread's own width (0.4 mm, 4 units), at least a little more than a pixel;
    // laser: lines a pixel and a half wide, as the page draws them
    let half = if d.lines { 0.75 } else { (2.0 * scale).max(0.6) };
    let mut budget = MAX_SEGMENTS;
    for p in &d.paths {
        if let Some(c) = p.fill
            && p.closed
        {
            let poly: Vec<(f64, f64)> = p.points.iter().map(|&(_, x, y)| at(x, y)).collect();
            fill(&mut img, &poly, c, 0.25);
        }
        let mut last: Option<(f64, f64)> = None;
        let mut first: Option<(f64, f64)> = None;
        for &(lift, x, y) in &p.points {
            let here = at(x, y);
            if lift {
                first = Some(here);
            } else if let Some(from) = last {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                pieces(&mut img, from, here, half, p.stroke);
            }
            last = Some(here);
        }
        if p.closed
            && let (Some(from), Some(to)) = (last, first)
        {
            pieces(&mut img, from, to, half, p.stroke);
        }
    }
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// A line drawn in pieces no longer than `PIECE` pixels.
fn pieces(img: &mut RgbImage, a: (f64, f64), b: (f64, f64), half: f64, c: Rgb<u8>) {
    let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    if !len.is_finite() {
        return;
    }
    let n = (len / PIECE).ceil().clamp(1.0, 4096.0) as u32;
    let step = |i: u32| {
        let t = f64::from(i) / f64::from(n);
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    };
    for i in 0..n {
        line(img, step(i), step(i + 1), half, c);
    }
}

/// The figure's contents for a paid design `id` seen by someone who hasn't bought it: the
/// lower-detail picture of its preview `svg` (`kind` embroidery or laser), and a line saying so.
pub fn figure_html(id: i64, kind: &str, svg: &str, title: &str) -> String {
    let (w, h) = size(svg).unwrap_or((MAX_SIDE, MAX_SIDE));
    let what = if kind == "laser" { "every line" } else { "every stitch" };
    format!(
        r#"<img class="stitches" src="/d/{id}/preview.png" width="{w}" height="{h}" alt="A picture of {alt}"><figcaption>A lower-detail picture. Buyers see {what}.</figcaption>"#,
        alt = crate::pages::escape(title)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const STITCHES: &str = r##"<svg class="stitches" viewBox="-20 -20 240 140" role="img" aria-label="The design's stitches"><rect x="-20" y="-20" width="240" height="140" fill="#f4f0e8"/><g fill="none" stroke-width="4"><path stroke="#c81e28" d="M0 0L200 0L200 100"/><path stroke="#143cc8" d="M0 100L100 50M150 50L160 60"/></g></svg>"##;
    const LINES: &str = r##"<svg class="stitches lines" viewBox="-100 -100 10200 5200" role="img" aria-label="The design's lines"><rect x="-100" y="-100" width="10200" height="5200" fill="#fbfaf6"/><g fill="none"><path stroke="#ff0000" fill="#ff0000" fill-opacity="0.25" d="M0 0L10000 0L10000 5000L0 5000Z"/></g></svg>"##;

    fn decode(png: &[u8]) -> RgbImage {
        image::load_from_memory_with_format(png, ImageFormat::Png)
            .unwrap()
            .to_rgb8()
    }

    #[test]
    fn stitches_are_drawn_small_in_their_colours() {
        let img = decode(&picture(STITCHES).unwrap());
        // 24 × 14 mm at four pixels a millimetre
        assert_eq!((img.width(), img.height()), (96, 56));
        assert_eq!(size(STITCHES), Some((96, 56)));
        let red = img.pixels().filter(|p| p.0[0] > 150 && p.0[2] < 100).count();
        let blue = img.pixels().filter(|p| p.0[2] > 150 && p.0[0] < 100).count();
        assert!(red > 50 && blue > 20, "{red} {blue}");
        // the ground
        assert_eq!(img.get_pixel(5, 5), &Rgb([0xf4, 0xf0, 0xe8]));
        // a pen lift isn't drawn: between the blue runs (125, 50) is ground
        let (x, y) = (((125.0 + 20.0) * 0.4) as u32, ((50.0 + 20.0) * 0.4) as u32);
        assert_eq!(img.get_pixel(x, y), &Rgb([0xf4, 0xf0, 0xe8]));
    }

    #[test]
    fn laser_lines_and_fills_are_drawn() {
        let img = decode(&picture(LINES).unwrap());
        assert_eq!((img.width(), img.height()), (408, 208));
        let inside = img.get_pixel(204, 104);
        assert!(inside.0[0] > 240 && inside.0[1] < 220, "lightly shaded: {inside:?}");
        let edge = (0..img.width()).map(|x| img.get_pixel(x, 104).0[1]).min().unwrap();
        assert!(edge < 120, "the outline is drawn: {edge}");
    }

    #[test]
    fn a_large_design_is_at_most_max_side_across() {
        let big = STITCHES.replace("-20 -20 240 140", "0 0 12000 3000");
        assert_eq!(size(&big), Some((MAX_SIDE, 140)));
    }

    #[test]
    fn anything_else_gives_no_picture_and_never_panics() {
        for bad in [
            "",
            "<svg>",
            r#"<svg viewBox="0 0 0 10">"#,
            r#"<svg viewBox="0 0 nan 10">"#,
            r##"<svg viewBox="0 0 10 10"><path stroke="#zzzzzz" d="M1 1L2 2"/></svg>"##,
            r##"<svg viewBox="0 0 10 10"><path stroke="#ffffff" d="M1e400 1L-1e400 2 L 3"/></svg>"##,
            r##"<svg viewBox="0 0 1e-300 10"><path stroke="#ffffff" d="M0 0L1e300 1e300"/></svg>"##,
            "<p>not an svg</p>",
        ] {
            let _ = picture(bad);
            let _ = size(bad);
        }
        assert!(picture("").is_none());
        assert!(picture("<p>not an svg</p>").is_none());
    }
}
