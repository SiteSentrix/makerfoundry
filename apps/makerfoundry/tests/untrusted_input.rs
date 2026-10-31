//! Property tests of everything that reads what people send (MF-20; CLAUDE.md › Users' uploads
//! are untrusted input): embroidery, SVG, DXF and PDF files, photos, typed prices and thicknesses,
//! and Stripe's signature header. Each is given random bytes and damaged copies of good files, and
//! must answer `Ok` or `Err` without panicking; what it accepts must be within the limits.
//!
//! More cases: `PROPTEST_CASES=5000 cargo test -p makerfoundry --test untrusted_input --release`.

// The helpers below are test code too, but outside `#[test]` functions, where clippy.toml's
// allow-unwrap-in-tests does not reach.
#![allow(clippy::unwrap_used)]

use std::io::Cursor;

use image::{DynamicImage, ImageFormat, RgbImage};
use lopdf::{Document, Object, Stream, dictionary};
use makerfoundry::model3d::mesh::{self, Mesh};
use makerfoundry::model3d::{obj, profile, stl, threemf, zip};
use makerfoundry::{convert, embroidery, laser, money, photos, sizes, stripe};
use proptest::prelude::*;
use sf_core::{Cmd, Pattern, Point, Rgb, Thread};
use sf_formats::{Format, WriteOptions};

/// What may be done to a good file to damage it.
#[derive(Clone, Debug)]
enum Damage {
    /// Overwrite the byte at (a fraction of the length) with a value.
    Set(f64, u8),
    /// Put bytes in at a place.
    Insert(f64, Vec<u8>),
    /// Cut a run out.
    Remove(f64, usize),
    /// Keep only the start.
    Truncate(f64),
    /// Copy a run to elsewhere (repeats a header, a count, a block).
    Repeat(f64, usize, f64),
}

fn damage() -> impl Strategy<Value = Damage> {
    let at = 0.0..1.0f64;
    prop_oneof![
        4 => (at.clone(), any::<u8>()).prop_map(|(a, b)| Damage::Set(a, b)),
        2 => (at.clone(), prop::collection::vec(any::<u8>(), 1..16)).prop_map(|(a, b)| Damage::Insert(a, b)),
        1 => (at.clone(), 1..64usize).prop_map(|(a, n)| Damage::Remove(a, n)),
        1 => at.clone().prop_map(Damage::Truncate),
        1 => (at.clone(), 1..256usize, at).prop_map(|(a, n, b)| Damage::Repeat(a, n, b)),
    ]
}

fn damaged(good: &[u8], damages: &[Damage]) -> Vec<u8> {
    let mut v = good.to_vec();
    let place = |v: &Vec<u8>, at: f64| ((v.len() as f64) * at) as usize;
    for d in damages {
        match d {
            Damage::Set(at, b) => {
                let i = place(&v, *at);
                if let Some(x) = v.get_mut(i) {
                    *x = *b;
                }
            }
            Damage::Insert(at, bytes) => {
                let i = place(&v, *at).min(v.len());
                v.splice(i..i, bytes.iter().copied());
            }
            Damage::Remove(at, n) => {
                let i = place(&v, *at).min(v.len());
                let j = (i + n).min(v.len());
                v.drain(i..j);
            }
            Damage::Truncate(at) => {
                let i = place(&v, *at);
                v.truncate(i);
            }
            Damage::Repeat(from, n, to) => {
                let i = place(&v, *from).min(v.len());
                let run: Vec<u8> = v[i..(i + n).min(v.len())].to_vec();
                let j = place(&v, *to).min(v.len());
                v.splice(j..j, run);
            }
        }
    }
    v
}

fn damages() -> impl Strategy<Value = Vec<Damage>> {
    prop::collection::vec(damage(), 1..8)
}

/// A small two-colour design, written as `format` by StitchFoundry's own writer.
fn embroidery_sample(format: Format) -> Vec<u8> {
    let mut p = Pattern::new();
    for (r, name) in [(200, "Red"), (20, "Blue")] {
        p.threads.push(Thread {
            color: Rgb { r, g: 60, b: 90 },
            name: name.into(),
            code: "800".into(),
            chart: String::new(),
        });
    }
    for i in 0..=30 {
        p.add(Point::new(f64::from(i), f64::from(i % 2) * 3.0), Cmd::Stitch);
    }
    p.add(Point::new(30.0, 1.0), Cmd::Trim);
    p.add(Point::new(30.0, 1.0), Cmd::ColorChange);
    p.add(Point::new(0.0, 10.0), Cmd::Jump);
    for i in 0..=30 {
        p.add(Point::new(f64::from(i), 10.0 + f64::from(i % 2) * 3.0), Cmd::Stitch);
    }
    p.add(Point::new(30.0, 13.0), Cmd::End);
    sf_formats::write(format, &p, &WriteOptions::named("sample")).unwrap()
}

const EMBROIDERY: [(Format, &str); 7] = [
    (Format::Dst, "dst"),
    (Format::Exp, "exp"),
    (Format::Jef, "jef"),
    (Format::Pes, "pes"),
    (Format::Vp3, "vp3"),
    (Format::Xxx, "xxx"),
    (Format::U01, "u01"),
];

/// What a design that was accepted must be, whatever the file was.
fn check_embroidery(e: &embroidery::Embroidery) {
    assert!(
        (embroidery::MIN_STITCHES..=embroidery::MAX_STITCHES).contains(&e.stitches),
        "{}",
        e.stitches
    );
    for side in [e.width_mm, e.height_mm] {
        assert!(
            side.is_finite() && (0.0..=embroidery::MAX_SIZE_MM).contains(&side),
            "{side}"
        );
    }
    // the preview holds only numbers and colours
    let left = e
        .svg
        .replace(|c: char| c.is_ascii_alphanumeric() || " #=\"-/<>._'".contains(c), "");
    assert!(left.is_empty(), "{left:?}");
    for c in &e.colours {
        assert!(c.hex.len() == 7 && c.hex.starts_with('#'), "{}", c.hex);
    }
}

/// What a laser design that was accepted must be.
fn check_laser(l: &laser::Laser) {
    for side in [l.width_mm, l.height_mm] {
        assert!(side.is_finite() && (0.0..=laser::MAX_SIZE_MM).contains(&side), "{side}");
    }
    assert!(l.shapes <= laser::MAX_SHAPES && l.closed <= l.shapes);
    assert!(l.length_mm.is_finite() && l.length_mm >= 0.0, "{}", l.length_mm);
    for c in &l.colours {
        assert!(c.length_mm.is_finite(), "{}", c.length_mm);
    }
    let lower = l.svg.to_ascii_lowercase();
    for bad in ["script", "href", "javascript", "onload", "<image", "<foreign", "url("] {
        assert!(!lower.contains(bad), "{bad} in the preview");
    }
}

const SVG: &str = r##"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 100 50">
<g transform="translate(2 3) rotate(10)"><rect x="10" y="10" width="30" height="30" rx="2" fill="none" stroke="#ff0000"/></g>
<circle cx="70" cy="25" r="15" fill="none" stroke="#0000ff"/>
<ellipse cx="50" cy="25" rx="5" ry="9" stroke="green"/>
<polyline points="0,0 5,5 10,0 15,5" stroke="black" fill="none"/>
<path d="M 0 0 C 10 20 20 20 30 0 Q 40 -10 50 0 A 5 5 0 0 1 60 0 Z" stroke="black" fill="#333"/>
<use href="#p"/><defs><path id="p" d="m1 1h5v5z"/></defs>
<text x="5" y="45">Hi</text>
</svg>"##;

const DXF: &str = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\n0\n62\n1\n10\n0\n20\n0\n11\n20\n21\n0\n\
0\nCIRCLE\n8\n0\n62\n5\n10\n10\n20\n10\n40\n5\n\
0\nLWPOLYLINE\n8\n0\n90\n4\n70\n1\n10\n0\n20\n0\n10\n10\n20\n0\n42\n1\n10\n10\n20\n10\n10\n0\n20\n10\n\
0\nARC\n10\n0\n20\n0\n40\n10\n50\n0\n51\n90\n\
0\nELLIPSE\n10\n0\n20\n0\n11\n10\n21\n0\n40\n0.5\n41\n0\n42\n6.28\n\
0\nSPLINE\n71\n3\n72\n8\n73\n4\n40\n0\n40\n0\n40\n0\n40\n0\n40\n1\n40\n1\n40\n1\n40\n1\n\
10\n0\n20\n0\n10\n5\n20\n10\n10\n10\n20\n-10\n10\n15\n20\n0\n\
0\nPOLYLINE\n66\n1\n70\n1\n0\nVERTEX\n10\n0\n20\n0\n0\nVERTEX\n10\n5\n20\n5\n42\n0.5\n0\nVERTEX\n10\n0\n20\n5\n0\nSEQEND\n\
0\nINSERT\n2\nLOGO\n10\n0\n20\n0\n\
0\nENDSEC\n0\nEOF\n";

/// A one-page PDF drawing `content` (compressed if `compress`), with a form XObject `/F1`
/// drawing `form`, written by lopdf.
fn pdf(content: &[u8], form: &[u8], compress: bool) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut form_stream = Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()], "Matrix" => vec![1.into(), 0.into(), 0.into(), 1.into(), 72.into(), 0.into()] },
        form.to_vec(),
    );
    let mut page_stream = Stream::new(dictionary! {}, content.to_vec());
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
        "MediaBox" => vec![0.into(), 0.into(), 400.into(), 300.into()],
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

const PDF_CONTENT: &str = "1 0 0 RG 2 w 72 72 72 72 re S 0 0 1 rg 200 72 36 18 re f \
q 1 0 0 1 10 10 cm 0 0 m 50 50 l 60 0 70 10 80 0 c 90 10 100 0 v h S Q BT /F 12 Tf (Hi) Tj ET q /F1 Do Q";

/// PDF drawing operators with random operands: the content streams people's PDFs hold.
fn pdf_operations() -> impl Strategy<Value = String> {
    let number = prop_oneof![-1000.0..1000.0f64, Just(0.0), Just(1e30), Just(-1e30), Just(3.4e38),];
    let op = prop_oneof![
        Just("m"),
        Just("l"),
        Just("c"),
        Just("v"),
        Just("y"),
        Just("h"),
        Just("re"),
        Just("S"),
        Just("s"),
        Just("f"),
        Just("B"),
        Just("n"),
        Just("q"),
        Just("Q"),
        Just("cm"),
        Just("RG"),
        Just("rg"),
        Just("K"),
        Just("k"),
        Just("G"),
        Just("g"),
        Just("w"),
        Just("Do"),
        Just("BT"),
        Just("ET"),
        Just("Tj"),
        Just("/F1"),
        Just("(x)"),
        Just("["),
        Just("]"),
    ];
    prop::collection::vec((prop::collection::vec(number, 0..7), op), 0..200).prop_map(|ops| {
        let mut s = String::new();
        for (numbers, op) in ops {
            for n in numbers {
                s.push_str(&format!("{n} "));
            }
            s.push_str(op);
            s.push(' ');
        }
        s
    })
}

/// DXF group codes and values, valid and not, in any order.
fn dxf_pairs() -> impl Strategy<Value = String> {
    let code = prop_oneof![
        Just("0"),
        Just("2"),
        Just("8"),
        Just("9"),
        Just("10"),
        Just("20"),
        Just("11"),
        Just("21"),
        Just("40"),
        Just("41"),
        Just("42"),
        Just("50"),
        Just("51"),
        Just("62"),
        Just("66"),
        Just("70"),
        Just("71"),
        Just("72"),
        Just("73"),
        Just("90"),
        Just("x"),
        Just("-1"),
    ];
    let value = prop_oneof![
        Just("SECTION".to_string()),
        Just("ENTITIES".to_string()),
        Just("HEADER".to_string()),
        Just("$INSUNITS".to_string()),
        Just("ENDSEC".to_string()),
        Just("EOF".to_string()),
        Just("LINE".to_string()),
        Just("LWPOLYLINE".to_string()),
        Just("POLYLINE".to_string()),
        Just("VERTEX".to_string()),
        Just("SEQEND".to_string()),
        Just("CIRCLE".to_string()),
        Just("ARC".to_string()),
        Just("ELLIPSE".to_string()),
        Just("SPLINE".to_string()),
        Just("INSERT".to_string()),
        Just("nan".to_string()),
        Just("inf".to_string()),
        Just("1e308".to_string()),
        Just("-1e308".to_string()),
        Just("99999999999".to_string()),
        (-1000.0..1000.0f64).prop_map(|v| v.to_string()),
        (-5i64..300).prop_map(|v| v.to_string()),
    ];
    let header = "0\nSECTION\n2\nENTITIES\n";
    prop::collection::vec((code, value), 0..300).prop_map(move |pairs| {
        let mut s = String::from(header);
        for (c, v) in pairs {
            s.push_str(c);
            s.push('\n');
            s.push_str(&v);
            s.push('\n');
        }
        s
    })
}

/// SVG elements with random attribute values, nested in groups with random transforms.
fn svg_elements() -> impl Strategy<Value = String> {
    let number =
        prop_oneof![-500.0..500.0f64, Just(0.0), Just(1e30), Just(-1e30), Just(1e-30),].prop_map(|v| v.to_string());
    let element = prop_oneof![
        (number.clone(), number.clone(), number.clone(), number.clone())
            .prop_map(|(a, b, c, d)| format!(r#"<rect x="{a}" y="{b}" width="{c}" height="{d}" stroke="red"/>"#)),
        (number.clone(), number.clone(), number.clone())
            .prop_map(|(a, b, c)| format!(r#"<circle cx="{a}" cy="{b}" r="{c}" stroke="blue"/>"#)),
        prop::collection::vec(number.clone(), 0..20)
            .prop_map(|v| format!(r#"<polyline points="{}" stroke="black"/>"#, v.join(","))),
        (prop::collection::vec(
            (
                prop_oneof![
                    Just("M"),
                    Just("L"),
                    Just("C"),
                    Just("Q"),
                    Just("A"),
                    Just("Z"),
                    Just("h"),
                    Just("v"),
                    Just("s"),
                    Just("t")
                ],
                prop::collection::vec(number.clone(), 0..7)
            ),
            0..30
        ))
        .prop_map(|segs| {
            let d: Vec<String> = segs.into_iter().map(|(c, v)| format!("{c}{}", v.join(" "))).collect();
            format!(r#"<path d="{}" stroke="green" fill="none"/>"#, d.join(" "))
        }),
        (number.clone(), number.clone(), number.clone()).prop_map(|(a, b, c)| format!(
            r#"<g transform="translate({a} {b}) rotate({c}) scale({a})"><path d="M0 0L10 10"/></g>"#
        )),
        Just(r##"<use href="#a"/><g id="a"><use href="#a"/></g>"##.to_string()),
        Just(r#"<image href="data:image/png;base64,AAAA" width="5" height="5"/>"#.to_string()),
        Just(r#"<text>Hi<tspan>there</tspan></text>"#.to_string()),
    ];
    (prop::collection::vec(element, 0..60), number.clone(), number).prop_map(|(elements, w, h)| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" viewBox="0 0 {w} {h}">{}</svg>"#,
            elements.concat()
        )
    })
}

fn photo_sample(format: ImageFormat) -> Vec<u8> {
    let picture = DynamicImage::ImageRgb8(RgbImage::from_fn(40, 30, |x, y| {
        image::Rgb([(x * 6) as u8, (y * 8) as u8, 100])
    }));
    let mut out = Cursor::new(Vec::new());
    picture.write_to(&mut out, format).unwrap();
    out.into_inner()
}

/// What a 3D reader accepted is within the limits.
fn check_mesh(m: &Mesh) {
    assert!(!m.triangles.is_empty() && m.triangles.len() <= mesh::MAX_TRIANGLES);
    assert!(m.total >= m.triangles.len() as u64);
    assert!(m.triangles.iter().flatten().flatten().all(|n| n.is_finite()));
    assert!(m.size_mm().iter().all(|s| s.is_finite() && *s <= mesh::MAX_SIZE_MM));
}

/// A small OBJ: a square pyramid with a quad base, every index form.
const OBJ: &str = "v 0 0 0\nv 20 0 0\nv 20 20 0\nv 0 20 0\nv 10 10 15\nvt 0 0\nvn 0 0 1\n\
f 1 2 3 4\nf 1/1 2/1 5/1\nf 2//1 3//1 5//1\nf 3/1/1 4/1/1 5/1/1\nf -5 -2 -1\n";

/// OBJ lines with numbers and indexes from the edges of what's allowed.
fn obj_line() -> impl Strategy<Value = String> {
    let num = prop_oneof![
        Just("0".to_string()),
        Just("1e30".to_string()),
        Just("-1e38".to_string()),
        Just("nan".to_string()),
        (-1e4f32..1e4).prop_map(|n| n.to_string()),
    ];
    let index = prop_oneof![(-8i64..12).prop_map(|i| i.to_string()), Just("99999999999".to_string())];
    prop_oneof![
        (num.clone(), num.clone(), num).prop_map(|(a, b, c)| format!("v {a} {b} {c}\n")),
        prop::collection::vec(index, 0..6).prop_map(|i| format!("f {}\n", i.join(" "))),
        Just("vn 0 0 1\n".to_string()),
        Just("# note\n".to_string()),
    ]
}

/// A small 3MF: a tetrahedron placed twice through components, a thumbnail and settings.
fn threemf_sample(model_xml: &str) -> Vec<u8> {
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;
    let mut png = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(RgbImage::new(4, 4))
        .write_to(&mut png, ImageFormat::Png)
        .unwrap();
    let files = [
        ("_rels/.rels".to_string(), rels.as_bytes().to_vec()),
        ("3D/3dmodel.model".to_string(), model_xml.as_bytes().to_vec()),
        ("Metadata/plate_1.png".to_string(), png.into_inner()),
        (
            "Metadata/project_settings.config".to_string(),
            br#"{"layer_height": "0.2"}"#.to_vec(),
        ),
        ("Metadata/plate_1.gcode".to_string(), b"G28".to_vec()),
    ];
    zip::write(&files).unwrap()
}

const MODEL_3MF: &str = r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="20" y="0" z="0"/><vertex x="0" y="20" z="0"/><vertex x="0" y="0" z="20"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/><triangle v1="0" v2="1" v3="3"/><triangle v1="1" v2="2" v3="3"/><triangle v1="0" v2="2" v3="3"/></triangles></mesh></object><object id="2"><components><component objectid="1"/><component objectid="1" transform="1 0 0 0 1 0 0 0 1 30 0 0"/></components></object></resources><build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 0 0 1"/></build></model>"#;

/// Model XML from pieces near the edges of what's allowed.
fn model_pieces() -> impl Strategy<Value = String> {
    let num = prop_oneof![
        Just("0".to_string()),
        Just("1e30".to_string()),
        Just("NaN".to_string()),
        Just("-0.5".to_string()),
        (-1e3f32..1e3).prop_map(|n| n.to_string()),
    ];
    let index = prop_oneof![
        (0u32..8).prop_map(|i| i.to_string()),
        Just("4294967295".to_string()),
        Just("-1".to_string())
    ];
    let piece = prop_oneof![
        (num.clone(), num.clone(), num.clone()).prop_map(|(x, y, z)| format!(r#"<vertex x="{x}" y="{y}" z="{z}"/>"#)),
        (index.clone(), index.clone(), index.clone())
            .prop_map(|(a, b, c)| format!(r#"<triangle v1="{a}" v2="{b}" v3="{c}"/>"#)),
        (0u32..4, num)
            .prop_map(|(id, t)| format!(r#"<component objectid="{id}" transform="1 0 0 0 1 0 0 0 1 {t} 0 0"/>"#)),
        Just("</vertices><triangles>".to_string()),
        Just("</mesh></object><object id=\"2\"><components>".to_string()),
        Just("</components></object><object id=\"3\"><mesh><vertices>".to_string()),
    ];
    prop::collection::vec(piece, 0..40).prop_map(|pieces| {
        format!(
            r#"<model unit="millimeter"><resources><object id="1"><mesh><vertices>{}</vertices></mesh></object></resources><build><item objectid="2"/><item objectid="1"/></build></model>"#,
            pieces.concat()
        )
    })
}

/// A small STL: a tetrahedron, binary or ASCII.
fn stl_sample(ascii: bool) -> Vec<u8> {
    let p = [[0.0f32, 0.0, 0.0], [20.0, 0.0, 0.0], [0.0, 20.0, 0.0], [0.0, 0.0, 20.0]];
    let tris = [
        [p[0], p[1], p[2]],
        [p[0], p[1], p[3]],
        [p[1], p[2], p[3]],
        [p[0], p[2], p[3]],
    ];
    if ascii {
        let mut out = String::from("solid t\n");
        for t in tris {
            out.push_str("facet normal 0 0 0\nouter loop\n");
            for v in t {
                out.push_str(&format!("vertex {} {} {}\n", v[0], v[1], v[2]));
            }
            out.push_str("endloop\nendfacet\n");
        }
        out.push_str("endsolid t\n");
        return out.into_bytes();
    }
    let mut out = vec![0u8; 80];
    out.extend(4u32.to_le_bytes());
    for t in tris {
        out.extend([0u8; 12]);
        for n in t.iter().flatten() {
            out.extend(n.to_le_bytes());
        }
        out.extend([0u8; 2]);
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    #[test]
    fn stl_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096), solid in any::<bool>()) {
        let bytes = if solid { [b"solid ".as_slice(), &bytes].concat() } else { bytes };
        if let Ok(m) = stl::read(&bytes) {
            check_mesh(&m);
        }
    }

    #[test]
    fn obj_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        if let Ok(m) = obj::read(&bytes) {
            check_mesh(&m);
        }
    }

    #[test]
    fn obj_damaged(ds in damages()) {
        if let Ok(m) = obj::read(&damaged(OBJ.as_bytes(), &ds)) {
            check_mesh(&m);
        }
    }

    #[test]
    fn obj_generated(lines in prop::collection::vec(obj_line(), 0..64)) {
        if let Ok(m) = obj::read(lines.concat().as_bytes()) {
            check_mesh(&m);
        }
    }

    #[test]
    fn zip_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        if let Ok(a) = zip::Archive::open(&bytes) {
            for e in a.entries() {
                if let Ok(data) = a.read(e) {
                    assert_eq!(data.len() as u64, e.size);
                }
            }
        }
    }

    #[test]
    fn threemf_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        if let Ok(t) = threemf::read(&bytes) {
            check_mesh(&t.mesh);
        }
    }

    #[test]
    fn threemf_damaged(ds in damages()) {
        if let Ok(t) = threemf::read(&damaged(&threemf_sample(MODEL_3MF), &ds)) {
            check_mesh(&t.mesh);
            // what's rebuilt reads back the same
            assert_eq!(threemf::read(&t.file).unwrap().mesh, t.mesh);
        }
    }

    #[test]
    fn threemf_damaged_model(ds in damages()) {
        let xml = damaged(MODEL_3MF.as_bytes(), &ds);
        let xml = String::from_utf8_lossy(&xml);
        if let Ok(t) = threemf::read(&threemf_sample(&xml)) {
            check_mesh(&t.mesh);
            assert!(!t.kept.iter().any(|k| k.ends_with(".gcode")));
        }
    }

    #[test]
    fn threemf_generated(xml in model_pieces()) {
        if let Ok(t) = threemf::read(&threemf_sample(&xml)) {
            check_mesh(&t.mesh);
        }
    }

    #[test]
    fn print_settings_never_panic(
        text in ".{0,400}",
        numbers in prop::collection::vec(prop_oneof![Just("1e308".to_string()), Just("-5".to_string()), Just("NaN".to_string()), Just("\"40%\"".to_string()), any::<i64>().prop_map(|n| n.to_string())], 0..8),
        xml in ".{0,200}",
    ) {
        let keys = ["nozzle_diameter", "layer_height", "sparse_infill_density", "wall_loops", "enable_support", "filament_type", "filament_colour", "printer_model"];
        let json = format!("{{{}}}", keys.iter().zip(&numbers).map(|(k, v)| format!("\"{k}\": {v}")).collect::<Vec<_>>().join(", "));
        for project in [text.as_str(), json.as_str()] {
            let files = vec![
                ("Metadata/project_settings.config".to_string(), project.to_string()),
                ("Metadata/model_settings.config".to_string(), xml.clone()),
            ];
            if let Some(p) = profile::read(&files, Some("BambuStudio")) {
                assert!(p.nozzle_mm.is_none_or(|n| (0.1..=2.0).contains(&n)));
                assert!(p.layer_mm.is_none_or(|n| (0.01..=1.0).contains(&n)));
                assert!(p.infill_percent.is_none_or(|n| (0.0..=100.0).contains(&n)));
                assert!(p.filaments.len() <= 32);
                let _ = p.name();
            }
        }
    }

    #[test]
    fn stl_damaged(ascii in any::<bool>(), ds in damages()) {
        if let Ok(m) = stl::read(&damaged(&stl_sample(ascii), &ds)) {
            check_mesh(&m);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    #[test]
    fn embroidery_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096), which in 0..EMBROIDERY.len()) {
        let name = format!("x.{}", EMBROIDERY[which].1);
        if let Ok(e) = embroidery::read(&bytes, &name) {
            check_embroidery(&e);
        }
    }

    #[test]
    fn embroidery_damaged(which in 0..EMBROIDERY.len(), named_as in 0..EMBROIDERY.len(), ds in damages()) {
        let (format, _) = EMBROIDERY[which];
        let bytes = damaged(&embroidery_sample(format), &ds);
        let name = format!("x.{}", EMBROIDERY[named_as].1);
        if let Ok(e) = embroidery::read(&bytes, &name) {
            check_embroidery(&e);
            // what was accepted converts to every format without panicking
            for to in convert::offered(&[]) {
                let _ = convert::convert(&bytes, &name, to, "a label");
            }
        }
    }

    #[test]
    fn laser_kind_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..3000), name in ".{0,20}") {
        let _ = laser::kind(&bytes, &name);
    }

    #[test]
    fn laser_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096), kind in prop_oneof![Just("SVG"), Just("DXF"), Just("PDF")]) {
        if let Ok(l) = laser::read(&bytes, kind) {
            check_laser(&l);
        }
    }

    #[test]
    fn svg_damaged(ds in damages()) {
        if let Ok(l) = laser::read(&damaged(SVG.as_bytes(), &ds), "SVG") {
            check_laser(&l);
        }
    }

    #[test]
    fn svg_generated(svg in svg_elements()) {
        if let Ok(l) = laser::read(svg.as_bytes(), "SVG") {
            check_laser(&l);
        }
    }

    #[test]
    fn dxf_damaged(ds in damages()) {
        if let Ok(l) = laser::read(&damaged(DXF.as_bytes(), &ds), "DXF") {
            check_laser(&l);
        }
    }

    #[test]
    fn dxf_generated(dxf in dxf_pairs()) {
        if let Ok(l) = laser::read(dxf.as_bytes(), "DXF") {
            check_laser(&l);
        }
    }

    #[test]
    fn pdf_damaged(compress in any::<bool>(), ds in damages()) {
        let good = pdf(PDF_CONTENT.as_bytes(), b"0 1 0 RG 0 0 m 72 0 l S", compress);
        if let Ok(l) = laser::read(&damaged(&good, &ds), "PDF") {
            check_laser(&l);
        }
    }

    #[test]
    fn pdf_generated(content in pdf_operations(), form in pdf_operations(), compress in any::<bool>()) {
        if let Ok(l) = laser::read(&pdf(content.as_bytes(), form.as_bytes(), compress), "PDF") {
            check_laser(&l);
        }
    }

    #[test]
    fn photos_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = photos::read(&bytes);
    }

    #[test]
    fn photos_damaged(which in 0..3usize, ds in damages()) {
        let format = [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP][which];
        if let Ok(p) = photos::read(&damaged(&photo_sample(format), &ds)) {
            assert!(p.width <= photos::PHOTO_SIDE && p.height <= photos::PHOTO_SIDE);
            assert!(p.jpeg.starts_with(&[0xff, 0xd8]) && p.thumb.starts_with(&[0xff, 0xd8]));
        }
    }

    #[test]
    fn prices_as_typed(text in "\\PC{0,24}", digits in "[$]?[0-9,]{0,9}([.][0-9]{0,3})?") {
        for t in [text.as_str(), digits.as_str()] {
            if let Some(cents) = money::parse_dollars(t) {
                assert!((0..1_000_000_000).contains(&cents), "{t:?} → {cents}");
            }
        }
    }

    #[test]
    fn every_price_splits_exactly(price in any::<i64>()) {
        if let Some(s) = money::split(price) {
            assert_eq!(s.fee + s.processing + s.seller, s.price);
            assert!(s.fee > 0 && s.processing > 0 && s.seller > 0);
            assert!((money::MIN_PRICE..=money::MAX_PRICE).contains(&s.price));
        }
        let _ = money::dollars(price);
    }

    #[test]
    fn thicknesses_as_typed(text in "\\PC{0,24}", number in "[0-9]{0,4}[.,]?[0-9]{0,4} ?(mm)?") {
        for t in [text.as_str(), number.as_str()] {
            if let Ok(Some(mm)) = sizes::check_thickness(t) {
                assert!(mm.is_finite() && (0.0..=sizes::MAX_THICKNESS_MM).contains(&mm), "{t:?} → {mm}");
                let _ = sizes::thickness_text(mm);
            }
        }
    }

    #[test]
    fn stripe_signature_headers(header in "\\PC{0,200}", t in any::<i64>(), now in any::<i64>(), v1 in "[0-9a-f]{0,80}") {
        let s = stripe::Stripe::new("http://127.0.0.1:9", "sk_test_x", vec!["whsec_x".into()], "https://makerfoundry.app").unwrap();
        assert!(!s.verify(&header, b"{}", now));
        assert!(!s.verify(&format!("t={t},v1={v1}"), b"{}", now));
        // a right signature, at any time: good only within the tolerance
        let good = format!("t={t},v1={}", stripe::signature("whsec_x", t, b"{}"));
        let near = now.checked_sub(t).is_some_and(|d| d.abs() <= stripe::TOLERANCE_SECS);
        assert_eq!(s.verify(&good, b"{}", now), near);
    }
}

/// Pictures that say they are huge are refused before they are decoded.
#[test]
fn a_photo_claiming_to_be_huge_is_refused() {
    // a PNG header for a 60,000 × 60,000 picture, with a broken body
    let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    png.extend_from_slice(&60_000u32.to_be_bytes());
    png.extend_from_slice(&60_000u32.to_be_bytes());
    png.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
    png.extend_from_slice(&[0; 64]);
    assert!(photos::read(&png).is_err());
}

/// The good samples the damage starts from are good: otherwise the tests above test little.
#[test]
fn the_samples_read() {
    for (format, ext) in EMBROIDERY {
        let e = embroidery::read(&embroidery_sample(format), &format!("x.{ext}")).unwrap();
        check_embroidery(&e);
    }
    for (bytes, kind) in [
        (SVG.as_bytes().to_vec(), "SVG"),
        (DXF.as_bytes().to_vec(), "DXF"),
        (pdf(PDF_CONTENT.as_bytes(), b"0 1 0 RG 0 0 m 72 0 l S", true), "PDF"),
    ] {
        let l = laser::read(&bytes, kind).unwrap_or_else(|e| panic!("{kind}: {e}"));
        assert!(l.shapes >= 3, "{kind}: {}", l.shapes);
        check_laser(&l);
    }
    for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
        photos::read(&photo_sample(format)).unwrap();
    }
}

#[test]
fn the_3d_samples_read() {
    for ascii in [false, true] {
        assert_eq!(stl::read(&stl_sample(ascii)).unwrap().triangles.len(), 4);
    }
    assert_eq!(obj::read(OBJ.as_bytes()).unwrap().triangles.len(), 6);
    let t = threemf::read(&threemf_sample(MODEL_3MF)).unwrap();
    assert_eq!(t.mesh.triangles.len(), 8);
    assert_eq!(t.dropped, ["Metadata/plate_1.gcode"]);
    assert_eq!(t.pictures.len(), 1);
}

// ---------------------------------------------------------------- shop-order messages (MF-60)

/// The valid example messages shared with StitchHub and StitchShop.
fn shop_order_fixtures() -> Vec<Vec<u8>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/contract/shop-orders/valid");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    files.iter().map(|p| std::fs::read(p).unwrap()).collect()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn shop_order_random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = makerfoundry::shop_orders::contract::parse(&bytes);
    }

    #[test]
    fn shop_order_damaged(which in 0usize..7, ds in damages()) {
        let fixtures = shop_order_fixtures();
        let good = &fixtures[which % fixtures.len()];
        if let Ok((env, msg)) = makerfoundry::shop_orders::contract::parse(&damaged(good, &ds)) {
            // whatever is accepted is a message that would be accepted again as written
            prop_assert_eq!(env.kind.as_str(), msg.kind());
            prop_assert!(serde_json::to_vec(&env).unwrap().len() <= makerfoundry::shop_orders::contract::MAX_MESSAGE_BYTES);
        }
    }
}
