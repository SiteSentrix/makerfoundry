//! PDF laser files: the first page's vector drawing, read from its content stream (ISO 32000-1,
//! § 8.5 path construction and painting, § 8.4 graphics state, § 8.6.8 colour operators, § 8.10
//! form XObjects; https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf).
//!
//! lopdf parses the file. Untrusted PDFs can hide a decompression bomb in any stream, so: each
//! stream may decompress to at most `MAX_STREAM`, a file may hold at most `MAX_OBJECT_STREAMS`
//! object streams, and the page's drawing to at most `MAX_CONTENT` in all; forms nest at most
//! `MAX_DEPTH` deep and at most `MAX_OPERATIONS` operations are read. Reading also runs under a
//! panic guard: lopdf is a third-party parser, and a bug in it must not take the site down.

use lopdf::content::Content;
use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId};

use super::{CURVE_STEPS, Shape, Shapes};

const MAX_STREAM: usize = 4 * 1024 * 1024;
const MAX_OBJECT_STREAMS: usize = 64;
const MAX_CONTENT: usize = 32 * 1024 * 1024;
const MAX_DEPTH: usize = 8;
const MAX_OPERATIONS: usize = 2_000_000;
/// Graphics states saved (`q`) at once.
const MAX_SAVED: usize = 64;

/// A PDF unit (a point, 1/72 inch) in mm.
const PT_MM: f64 = 25.4 / 72.0;

type Matrix = [f64; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` then `n`: the matrix that applies `m` first (PDF's `m × n`).
fn concat(m: &Matrix, n: &Matrix) -> Matrix {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

#[derive(Clone)]
struct State {
    ctm: Matrix,
    stroke: (u8, u8, u8),
    fill: (u8, u8, u8),
}

/// What reading a page found besides shapes.
#[derive(Default)]
struct Found {
    text: usize,
    images: usize,
    operations: usize,
    content_bytes: usize,
}

pub(crate) fn read(bytes: &[u8]) -> Result<(Shapes, Vec<String>), String> {
    if !bytes.starts_with(b"%PDF-") {
        return Err("this isn't a PDF".into());
    }
    // a file with many object streams could decompress many times over: refuse it up front
    if count(bytes, b"/ObjStm") > MAX_OBJECT_STREAMS {
        return Err("this PDF is packed in an unusual way MakerFoundry doesn't read".into());
    }
    // (lopdf is someone else's parser: a bug in it is caught here, not left to end the request)
    std::panic::catch_unwind(|| read_guarded(bytes)).unwrap_or_else(|_| Err("this PDF couldn't be read".into()))
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack.windows(needle.len()).filter(|w| *w == needle).count()
}

fn read_guarded(bytes: &[u8]) -> Result<(Shapes, Vec<String>), String> {
    let options = LoadOptions {
        max_decompressed_size: Some(MAX_STREAM),
        ..Default::default()
    };
    let doc = Document::load_mem_with_options(bytes, options).map_err(|_| "this PDF couldn't be read".to_string())?;
    if doc.is_encrypted() {
        return Err("this PDF is password-protected: save it without a password".into());
    }
    let pages = doc.get_pages();
    let Some((_, &page_id)) = pages.iter().next() else {
        return Err("this PDF has no pages".into());
    };
    // the page's top edge, to turn PDF's y-up into y-down
    let media = page_box(&doc, page_id);
    let top = media.map_or(0.0, |b| b[3]);
    let content = doc
        .get_page_content_with_limit(page_id, MAX_CONTENT)
        .map_err(|_| "this PDF's drawing is too large or couldn't be read".to_string())?;
    let mut shapes = Shapes::default();
    let mut found = Found {
        content_bytes: content.len(),
        ..Found::default()
    };
    let resources = page_resources(&doc, page_id);
    let flip: Matrix = [PT_MM, 0.0, 0.0, -PT_MM, 0.0, top * PT_MM];
    let state = State {
        ctm: IDENTITY,
        stroke: (0, 0, 0),
        fill: (0, 0, 0),
    };
    run(&doc, &content, &resources, state, &flip, 0, &mut shapes, &mut found)?;
    let mut not_drawn = Vec::new();
    if found.text > 0 {
        not_drawn.push("text (convert it to outlines to cut or engrave it)".to_string());
    }
    if found.images > 0 {
        not_drawn.push(format!(
            "{} picture{}",
            found.images,
            if found.images == 1 { "" } else { "s" }
        ));
    }
    if pages.len() > 1 {
        not_drawn.push(format!("pages 2 to {} (only the first page is drawn)", pages.len()));
    }
    Ok((shapes, not_drawn))
}

/// The page's MediaBox (inherited from its parents if it hasn't one).
fn page_box(doc: &Document, page_id: ObjectId) -> Option<[f64; 4]> {
    let mut id = page_id;
    for _ in 0..16 {
        let dict = doc.get_dictionary(id).ok()?;
        if let Ok(obj) = dict.get(b"MediaBox") {
            let arr = doc.dereference(obj).ok()?.1.as_array().ok()?;
            let v: Vec<f64> = arr
                .iter()
                .filter_map(|o| doc.dereference(o).ok()?.1.as_float().ok())
                .map(f64::from)
                .collect();
            if let [x0, y0, x1, y1] = v[..] {
                return Some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]);
            }
            return None;
        }
        id = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// The resource dictionaries a page's names are looked up in, nearest first.
fn page_resources(doc: &Document, page_id: ObjectId) -> Vec<Dictionary> {
    let Ok((inline, ids)) = doc.get_page_resources(page_id) else {
        return Vec::new();
    };
    let mut out: Vec<Dictionary> = inline.cloned().into_iter().collect();
    out.extend(ids.into_iter().filter_map(|id| doc.get_dictionary(id).ok().cloned()));
    out
}

/// The XObject named `name` in `resources`, if there is one.
fn xobject<'a>(doc: &'a Document, resources: &'a [Dictionary], name: &[u8]) -> Option<&'a lopdf::Stream> {
    resources.iter().find_map(|res| {
        let xobjects = doc.dereference(res.get(b"XObject").ok()?).ok()?.1.as_dict().ok()?;
        doc.dereference(xobjects.get(name).ok()?).ok()?.1.as_stream().ok()
    })
}

fn nums(operands: &[Object]) -> Vec<f64> {
    operands
        .iter()
        .filter_map(|o| o.as_float().ok())
        .map(f64::from)
        .filter(|v| v.is_finite())
        .collect()
}

/// A colour from 1 (grey), 3 (RGB) or 4 (CMYK) components.
fn colour(c: &[f64]) -> Option<(u8, u8, u8)> {
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    match c {
        [g] => Some((byte(*g), byte(*g), byte(*g))),
        [r, g, b] => Some((byte(*r), byte(*g), byte(*b))),
        [c, m, y, k] => Some((
            byte((1.0 - c) * (1.0 - k)),
            byte((1.0 - m) * (1.0 - k)),
            byte((1.0 - y) * (1.0 - k)),
        )),
        _ => None,
    }
}

/// Runs one content stream (a page's or a form's), adding what it paints to `shapes`.
#[allow(clippy::too_many_arguments)]
fn run(
    doc: &Document,
    content: &[u8],
    resources: &[Dictionary],
    start: State,
    flip: &Matrix,
    depth: usize,
    shapes: &mut Shapes,
    found: &mut Found,
) -> Result<(), String> {
    let content = Content::decode(content).map_err(|_| "this PDF's drawing couldn't be read".to_string())?;
    let mut state = start;
    let mut saved: Vec<State> = Vec::new();
    // the path being built, in page units: its subpaths, each with whether it closes
    let mut path: Vec<(Vec<(f64, f64)>, bool)> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut at = (0.0, 0.0);
    let mut in_text = false;
    for op in &content.operations {
        found.operations += 1;
        if found.operations > MAX_OPERATIONS {
            return Err("this PDF's drawing is too large".into());
        }
        let n = nums(&op.operands);
        // a point through the current matrix, in page units
        let to_page = |x: f64, y: f64, m: &Matrix| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        let ctm = state.ctm;
        match (op.operator.as_str(), n.as_slice()) {
            ("q", _) => {
                if saved.len() >= MAX_SAVED {
                    return Err("this PDF's drawing nests too deep".into());
                }
                saved.push(state.clone());
            }
            ("Q", _) => {
                if let Some(s) = saved.pop() {
                    state = s;
                }
            }
            ("cm", [a, b, c, d, e, f]) => state.ctm = concat(&[*a, *b, *c, *d, *e, *f], &state.ctm),
            ("RG" | "SC" | "SCN", c) | ("G", c @ [_]) | ("K", c @ [_, _, _, _]) => {
                if let Some(rgb) = colour(c) {
                    state.stroke = rgb;
                }
            }
            ("rg" | "sc" | "scn", c) | ("g", c @ [_]) | ("k", c @ [_, _, _, _]) => {
                if let Some(rgb) = colour(c) {
                    state.fill = rgb;
                }
            }
            ("m", [x, y]) => {
                if current.len() > 1 {
                    path.push((std::mem::take(&mut current), false));
                }
                current.clear();
                at = to_page(*x, *y, &ctm);
                current.push(at);
            }
            ("l", [x, y]) => {
                at = to_page(*x, *y, &ctm);
                current.push(at);
            }
            ("c", [x1, y1, x2, y2, x3, y3]) => {
                let (p1, p2, p3) = (
                    to_page(*x1, *y1, &ctm),
                    to_page(*x2, *y2, &ctm),
                    to_page(*x3, *y3, &ctm),
                );
                cubic(&mut current, at, p1, p2, p3);
                at = p3;
            }
            ("v", [x2, y2, x3, y3]) => {
                let (p2, p3) = (to_page(*x2, *y2, &ctm), to_page(*x3, *y3, &ctm));
                cubic(&mut current, at, at, p2, p3);
                at = p3;
            }
            ("y", [x1, y1, x3, y3]) => {
                let (p1, p3) = (to_page(*x1, *y1, &ctm), to_page(*x3, *y3, &ctm));
                cubic(&mut current, at, p1, p3, p3);
                at = p3;
            }
            ("h", _) => {
                if current.len() > 1 {
                    let start = current.first().copied();
                    path.push((std::mem::take(&mut current), true));
                    if let Some(s) = start {
                        at = s;
                    }
                }
            }
            ("re", [x, y, w, h]) => {
                if current.len() > 1 {
                    path.push((std::mem::take(&mut current), false));
                }
                current.clear();
                let corners = [(*x, *y), (x + w, *y), (x + w, y + h), (*x, y + h)];
                path.push((corners.iter().map(|(cx, cy)| to_page(*cx, *cy, &ctm)).collect(), true));
                at = to_page(*x, *y, &ctm);
            }
            // painting: stroke, fill, both, or neither (`n`, a clipping path)
            (paint @ ("S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n"), _) => {
                if current.len() > 1 {
                    let close = matches!(paint, "s" | "b" | "b*");
                    path.push((std::mem::take(&mut current), close));
                }
                current.clear();
                let stroked = matches!(paint, "S" | "s" | "B" | "B*" | "b" | "b*");
                let filled = matches!(paint, "f" | "F" | "f*" | "B" | "B*" | "b" | "b*");
                for (points, closed) in path.drain(..) {
                    if !stroked && !filled {
                        continue;
                    }
                    let closed = closed || filled;
                    let rgb = if stroked { state.stroke } else { state.fill };
                    let points = points.into_iter().map(|(x, y)| to_page(x, y, flip)).collect();
                    shapes.push(Shape {
                        points,
                        closed,
                        filled,
                        rgb,
                    })?;
                }
            }
            ("BT", _) => in_text = true,
            ("ET", _) => in_text = false,
            ("Tj" | "TJ" | "'" | "\"", _) => {
                if in_text {
                    found.text += 1;
                }
            }
            ("Do", _) => {
                let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) else {
                    continue;
                };
                let Some(stream) = xobject(doc, resources, name) else {
                    continue;
                };
                let subtype = stream
                    .dict
                    .get(b"Subtype")
                    .and_then(Object::as_name)
                    .unwrap_or_default();
                if subtype == b"Image" {
                    found.images += 1;
                } else if subtype == b"Form" {
                    if depth + 1 >= MAX_DEPTH {
                        return Err("this PDF's drawing nests too deep".into());
                    }
                    let budget = MAX_CONTENT.saturating_sub(found.content_bytes);
                    let inner = stream
                        .decompressed_content_with_limit(budget.min(MAX_STREAM))
                        .or_else(|_| {
                            if stream.dict.get(b"Filter").is_err() {
                                Ok(stream.content.clone())
                            } else {
                                Err(())
                            }
                        })
                        .map_err(|_| "this PDF's drawing is too large or couldn't be read".to_string())?;
                    found.content_bytes += inner.len();
                    let matrix = stream
                        .dict
                        .get(b"Matrix")
                        .and_then(Object::as_array)
                        .map(|a| nums(a))
                        .ok()
                        .and_then(|v| <[f64; 6]>::try_from(v).ok())
                        .unwrap_or(IDENTITY);
                    let own: Vec<Dictionary> = stream
                        .dict
                        .get(b"Resources")
                        .ok()
                        .and_then(|r| doc.dereference(r).ok())
                        .and_then(|(_, r)| r.as_dict().ok().cloned())
                        .into_iter()
                        .chain(resources.iter().cloned())
                        .collect();
                    let inner_state = State {
                        ctm: concat(&matrix, &state.ctm),
                        ..state.clone()
                    };
                    run(doc, &inner, &own, inner_state, flip, depth + 1, shapes, found)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn cubic(out: &mut Vec<(f64, f64)>, p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64)) {
    for i in 1..=CURVE_STEPS {
        let t = i as f64 / CURVE_STEPS as f64;
        let u = 1.0 - t;
        let (a0, a1, a2, a3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push((
            a0 * p0.0 + a1 * p1.0 + a2 * p2.0 + a3 * p3.0,
            a0 * p0.1 + a1 * p1.1 + a2 * p2.1 + a3 * p3.1,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::super::read as read_laser;
    use super::*;
    use lopdf::{Stream, dictionary};

    /// A one-page PDF, `w` × `h` points, drawing `content` (compressed if `compress`), with a form
    /// XObject `/F1` drawing `form`.
    fn pdf(w: f64, h: f64, content: &str, form: &str, compress: bool) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let mut form_stream = Stream::new(
            dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()], "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 72.into(), 0.into()] },
            form.as_bytes().to_vec(),
        );
        let mut page_stream = Stream::new(dictionary! {}, content.as_bytes().to_vec());
        if compress {
            let _ = page_stream.compress();
            let _ = form_stream.compress();
        }
        let form_id = doc.add_object(form_stream);
        let content_id = doc.add_object(page_stream);
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => dictionary! { "XObject" => dictionary! { "F1" => form_id } },
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(w as f32), Object::Real(h as f32)],
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn a_laser_pdf_reads_in_millimetres_with_its_colours() {
        // a red 1-inch square stroked, a blue filled rectangle, text, and a green line in a form
        // moved an inch right by its matrix
        let content = "1 0 0 RG 72 72 72 72 re S 0 0 1 rg 200 72 36 18 re f BT /F 12 Tf (Hi) Tj ET q /F1 Do Q";
        let bytes = pdf(400.0, 300.0, content, "0 1 0 RG 0 0 m 72 0 l S", true);
        let l = read_laser(&bytes, "PDF").unwrap();
        assert_eq!(l.closed, 2, "the square and the rectangle; the green line is open");
        assert_eq!(l.format, "PDF");
        assert_eq!(l.shapes, 3);
        let red = l.colours.iter().find(|c| c.hex == "#ff0000").unwrap();
        assert!((red.length_mm - 4.0 * 25.4).abs() < 0.01, "{}", red.length_mm);
        let green = l.colours.iter().find(|c| c.hex == "#00ff00").unwrap();
        assert!((green.length_mm - 25.4).abs() < 0.01, "{}", green.length_mm);
        assert!(l.not_drawn.iter().any(|n| n.starts_with("text")), "{:?}", l.not_drawn);
        // from the square's left (1 in) to the rectangle's right (236 pt)
        assert!((l.width_mm - (236.0 - 72.0) * PT_MM).abs() < 0.01, "{}", l.width_mm);
    }

    #[test]
    fn transforms_and_curves_are_followed() {
        // a circle of radius 36 pt from four curves, drawn after scaling by 2 and rotating 90°
        let k = 36.0 * 0.5523;
        let circle = format!(
            "36 0 m 36 {k} {k} 36 0 36 c -{k} 36 -36 {k} -36 0 c -36 -{k} -{k} -36 0 -36 c {k} -36 36 -{k} 36 0 c S"
        );
        let content = format!("0 0 0 RG q 2 0 0 2 200 150 cm 0 1 -1 0 0 0 cm {circle} Q");
        let l = read_laser(&pdf(400.0, 300.0, &content, "", false), "PDF").unwrap();
        assert_eq!(l.closed, 1, "a circle ends where it starts, with or without h");
        let expected = 2.0 * std::f64::consts::PI * 72.0 * PT_MM;
        assert!((l.length_mm - expected).abs() < 1.0, "{} vs {expected}", l.length_mm);
        assert!((l.width_mm - 144.0 * PT_MM).abs() < 0.5, "{}", l.width_mm);
    }

    #[test]
    fn bombs_and_broken_files_are_refused() {
        // a page whose drawing inflates past the limit
        let huge = "0 0 m ".repeat(MAX_CONTENT / 6 + 10);
        let r = read_laser(&pdf(100.0, 100.0, &huge, "", true), "PDF");
        assert!(r.is_err(), "a bomb");
        assert!(read_laser(b"%PDF-1.7\nnot really", "PDF").is_err());
        assert!(read_laser(b"%PDF-1.4\n%%EOF", "PDF").is_err());
        let mut packed = b"%PDF-1.5\n".to_vec();
        for _ in 0..=MAX_OBJECT_STREAMS {
            packed.extend_from_slice(b"1 0 obj << /Type /ObjStm >> endobj\n");
        }
        assert!(read_laser(&packed, "PDF").unwrap_err().contains("unusual"));
        // nothing drawable
        assert!(
            read_laser(&pdf(100.0, 100.0, "BT (x) Tj ET", "", false), "PDF")
                .unwrap_err()
                .contains("text")
        );
    }

    #[test]
    fn damaged_pdfs_never_panic() {
        let base = pdf(
            400.0,
            300.0,
            "1 0 0 RG 72 72 72 72 re S q /F1 Do Q 0 0 m 10 10 20 0 30 30 c S",
            "0 1 0 RG 0 0 m 72 0 l S",
            false,
        );
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..1500 {
            let mut bytes = base.clone();
            for _ in 0..(next() % 10) {
                let at = (next() as usize) % bytes.len();
                if let Some(b) = bytes.get_mut(at) {
                    *b = b"0123456789 /<>[]()RSmlcqQ\nobj"[(next() % 29) as usize];
                }
            }
            bytes.truncate((next() as usize) % (bytes.len() + 1));
            let _ = read_laser(&bytes, "PDF");
        }
    }
}
