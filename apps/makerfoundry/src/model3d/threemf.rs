//! 3MF files (MF-38), read from the 3MF Core Specification
//! (<https://github.com/3MFConsortium/spec_core/blob/master/3MF%20Core%20Specification.md>) and
//! its Production Extension (<https://github.com/3MFConsortium/spec_production>, the `p:path`
//! components Bambu Studio and OrcaSlicer write), on our own ZIP reader (`zip.rs`).
//!
//! The model XML is read by roxmltree with DTDs refused, so no entity is ever expanded or
//! loaded. Objects' meshes are placed through their components and the build's transforms, in
//! millimetres, under a budget of placed triangles and a depth limit (no cycles, no exponential
//! trees). Then the file is **rebuilt** with only the parts we know: the model parts we read,
//! their relationships, PNG/JPEG pictures decoded and written again, and slicer settings that read
//! as text. Everything else (scripts, programs, sliced G-code, unknown parts) is dropped, and the
//! rebuilt file is what people download (docs/3d-hub-plan.md § 3).

use std::collections::{HashMap, VecDeque};

use usvg::roxmltree::{Document, Node, ParsingOptions};

use super::mesh::{MAX_FILE_BYTES, Mesh};
use super::zip::{self, Archive};
use crate::photos;

const PRODUCTION: &str = "http://schemas.microsoft.com/3dmanufacturing/production/2015/06";
const RELATIONSHIPS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MODEL_REL: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";
const THUMBNAIL_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail";

/// The largest model part (XML) read; with roxmltree's own memory, a few times this.
const MAX_XML_BYTES: u64 = 256 * 1024 * 1024;
/// The largest relationships or settings file kept.
const MAX_TEXT_BYTES: u64 = 16 * 1024 * 1024;
/// The most vertices and triangles in all the objects of a file.
const MAX_VERTICES: usize = 10_000_000;
const MAX_OBJECT_TRIANGLES: usize = 10_000_000;
/// How deep components may nest.
const MAX_DEPTH: usize = 16;
/// The most triangles placed once components are expanded (work, not memory: the mesh keeps
/// at most `mesh::MAX_TRIANGLES`).
const MAX_PLACED: u64 = 50_000_000;

/// A 3MF read and rebuilt.
#[derive(Debug)]
pub struct ThreeMf {
    pub mesh: Mesh,
    /// The rebuilt file: what's kept and offered for download.
    pub file: Vec<u8>,
    /// The parts kept and the ones dropped, by name.
    pub kept: Vec<String>,
    pub dropped: Vec<String>,
    /// Slicer settings files kept (name, text), for the print profiles (MF-43, MF-44).
    pub settings: Vec<(String, String)>,
    /// Pictures kept (name, PNG or JPEG as written again), the package thumbnail first.
    pub pictures: Vec<(String, Vec<u8>)>,
}

/// A row-vector affine transform, as 3MF writes it: `m00 m01 m02 m10 m11 m12 m20 m21 m22 m30 m31
/// m32`.
type Transform = [f64; 12];
const IDENTITY: Transform = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];

/// `first` then `then`.
fn compose(first: &Transform, then: &Transform) -> Transform {
    let [a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11] = *first;
    let [b0, b1, b2, b3, b4, b5, b6, b7, b8, b9, b10, b11] = *then;
    [
        a0 * b0 + a1 * b3 + a2 * b6,
        a0 * b1 + a1 * b4 + a2 * b7,
        a0 * b2 + a1 * b5 + a2 * b8,
        a3 * b0 + a4 * b3 + a5 * b6,
        a3 * b1 + a4 * b4 + a5 * b7,
        a3 * b2 + a4 * b5 + a5 * b8,
        a6 * b0 + a7 * b3 + a8 * b6,
        a6 * b1 + a7 * b4 + a8 * b7,
        a6 * b2 + a7 * b5 + a8 * b8,
        a9 * b0 + a10 * b3 + a11 * b6 + b9,
        a9 * b1 + a10 * b4 + a11 * b7 + b10,
        a9 * b2 + a10 * b5 + a11 * b8 + b11,
    ]
}

fn apply(m: &Transform, [x, y, z]: [f32; 3], scale: f64) -> [f32; 3] {
    let (x, y, z) = (f64::from(x), f64::from(y), f64::from(z));
    let [m0, m1, m2, m3, m4, m5, m6, m7, m8, m9, m10, m11] = *m;
    // (an out-of-range result becomes infinite, which the mesh refuses)
    [
        ((x * m0 + y * m3 + z * m6 + m9) * scale) as f32,
        ((x * m1 + y * m4 + z * m7 + m10) * scale) as f32,
        ((x * m2 + y * m5 + z * m8 + m11) * scale) as f32,
    ]
}

fn transform(node: Node) -> Result<Transform, String> {
    let Some(text) = node.attribute("transform") else {
        return Ok(IDENTITY);
    };
    let mut m = IDENTITY;
    let mut numbers = text.split_ascii_whitespace();
    for n in &mut m {
        *n = numbers
            .next()
            .and_then(|w| w.parse::<f64>().ok())
            .filter(|n| n.is_finite())
            .ok_or_else(|| "a transform in the model isn't twelve numbers".to_string())?;
    }
    if numbers.next().is_some() {
        return Err("a transform in the model isn't twelve numbers".into());
    }
    Ok(m)
}

enum Object {
    Mesh {
        vertices: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
    },
    Components(Vec<(Option<String>, u32, Transform)>),
}

/// One model part: its objects by id, its build items, and how many millimetres its unit is.
struct Part {
    scale: f64,
    objects: HashMap<u32, Object>,
    build: Vec<(Option<String>, u32, Transform)>,
}

/// What's been read so far across the parts.
#[derive(Default)]
struct Counts {
    vertices: usize,
    triangles: usize,
}

fn options<'input>() -> ParsingOptions<'input> {
    ParsingOptions {
        allow_dtd: false,
        ..Default::default()
    }
}

fn text_of(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    Some(text.strip_prefix('\u{feff}').unwrap_or(text))
}

/// A part name as a path inside the package: without its leading `/`.
fn part_name(target: &str) -> String {
    target.trim_start_matches('/').to_string()
}

fn number<T: std::str::FromStr>(node: Node, name: &str) -> Result<T, String> {
    node.attribute(name)
        .and_then(|v| v.trim().parse::<T>().ok())
        .ok_or_else(|| format!("a {} in the model has no {name}", node.tag_name().name()))
}

/// A component's or item's `p:path`, lower-cased (part names ignore case).
fn path_of(node: Node) -> Option<String> {
    node.attribute((PRODUCTION, "path"))
        .map(|p| part_name(p).to_ascii_lowercase())
}

/// The child elements of `n` called `name`.
fn children<'a, 'input>(n: Node<'a, 'input>, name: &'static str) -> impl Iterator<Item = Node<'a, 'input>> {
    n.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn parse_part(xml: &[u8], counts: &mut Counts) -> Result<Part, String> {
    let text = text_of(xml).ok_or_else(|| "the model isn't UTF-8 text".to_string())?;
    let doc = Document::parse_with_options(text, options())
        .map_err(|_| "the model couldn't be read (MakerFoundry doesn't read DTDs or entities)".to_string())?;
    let model = doc.root_element();
    if model.tag_name().name() != "model" {
        return Err("the file isn't a 3MF model".into());
    }
    let scale = match model.attribute("unit").unwrap_or("millimeter") {
        "micron" => 0.001,
        "millimeter" => 1.0,
        "centimeter" => 10.0,
        "inch" => 25.4,
        "foot" => 304.8,
        "meter" => 1000.0,
        _ => return Err("the model's unit isn't one 3MF has".into()),
    };
    let mut part = Part {
        scale,
        objects: HashMap::new(),
        build: Vec::new(),
    };
    for resources in children(model, "resources") {
        for object in children(resources, "object") {
            let id: u32 = number(object, "id")?;
            let made = if let Some(mesh) = children(object, "mesh").next() {
                let mut vertices = Vec::new();
                for v in children(mesh, "vertices").flat_map(|vs| children(vs, "vertex")) {
                    counts.vertices += 1;
                    if counts.vertices > MAX_VERTICES {
                        return Err(format!("the model has more than {MAX_VERTICES} vertices"));
                    }
                    let n = |a: &str| {
                        number::<f32>(v, a).and_then(|n| {
                            if n.is_finite() {
                                Ok(n)
                            } else {
                                Err("the model has a number that isn't a number".to_string())
                            }
                        })
                    };
                    vertices.push([n("x")?, n("y")?, n("z")?]);
                }
                let mut triangles = Vec::new();
                for t in children(mesh, "triangles").flat_map(|ts| children(ts, "triangle")) {
                    counts.triangles += 1;
                    if counts.triangles > MAX_OBJECT_TRIANGLES {
                        return Err(format!("the model has more than {MAX_OBJECT_TRIANGLES} triangles"));
                    }
                    let corners = [
                        number::<u32>(t, "v1")?,
                        number::<u32>(t, "v2")?,
                        number::<u32>(t, "v3")?,
                    ];
                    if corners.iter().any(|&c| c as usize >= vertices.len()) {
                        return Err("a triangle in the model refers to a vertex that isn't there".into());
                    }
                    triangles.push(corners);
                }
                Object::Mesh { vertices, triangles }
            } else if let Some(components) = children(object, "components").next() {
                let mut list = Vec::new();
                for c in children(components, "component") {
                    list.push((path_of(c), number(c, "objectid")?, transform(c)?));
                }
                Object::Components(list)
            } else {
                // (other kinds of object, from extensions we don't read, have nothing to draw)
                Object::Components(Vec::new())
            };
            if part.objects.insert(id, made).is_some() {
                return Err("two objects in the model have the same id".into());
            }
        }
    }
    for build in children(model, "build") {
        for item in children(build, "item") {
            part.build
                .push((path_of(item), number(item, "objectid")?, transform(item)?));
        }
    }
    Ok(part)
}

/// Places an object's triangles into `mesh`, through its components.
struct Placer<'p> {
    parts: &'p HashMap<String, Part>,
    scale: f64,
    placed: u64,
    max_placed: u64,
}

impl Placer<'_> {
    fn place(&mut self, mesh: &mut Mesh, path: &str, id: u32, m: &Transform, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err("the model's components nest too deep (or refer to themselves)".into());
        }
        let object = self
            .parts
            .get(path)
            .and_then(|p| p.objects.get(&id))
            .ok_or_else(|| "the model refers to an object that isn't there".to_string())?;
        match object {
            Object::Mesh { vertices, triangles } => {
                for t in triangles {
                    self.placed += 1;
                    if self.placed > self.max_placed {
                        return Err("the model has too many triangles once its parts are placed".into());
                    }
                    let corner = |i: u32| {
                        vertices
                            .get(i as usize)
                            .map(|v| apply(m, *v, self.scale))
                            .ok_or_else(|| "a triangle refers to a vertex that isn't there".to_string())
                    };
                    mesh.push([corner(t[0])?, corner(t[1])?, corner(t[2])?])?;
                }
            }
            Object::Components(list) => {
                for (p, child, cm) in list {
                    let p = p.as_deref().unwrap_or(path);
                    self.place(mesh, p, *child, &compose(cm, m), depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

/// The relationships in a `.rels` part: (target, type).
fn relationships(xml: &[u8]) -> Result<Vec<(String, String)>, String> {
    let text = text_of(xml).ok_or_else(|| "a relationships part isn't text".to_string())?;
    let doc = Document::parse_with_options(text, options())
        .map_err(|_| "a relationships part couldn't be read".to_string())?;
    Ok(doc
        .descendants()
        .filter(|n| n.tag_name().name() == "Relationship")
        .filter_map(|n| Some((part_name(n.attribute("Target")?), n.attribute("Type")?.to_string())))
        .collect())
}

/// Whether a part is a slicer's settings file we keep (Bambu Studio and OrcaSlicer's
/// `Metadata/*.config`, PrusaSlicer's `Metadata/Slic3r_PE*.config`, Cura's `Cura/*`): text that
/// reads as what it says it is.
fn settings_text(name: &str, bytes: &[u8]) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let place = lower.starts_with("metadata/") || lower.starts_with("cura/");
    let kind = [".config", ".json", ".xml", ".cfg", ".ini"]
        .iter()
        .any(|e| lower.ends_with(e));
    if !place || !kind || bytes.len() as u64 > MAX_TEXT_BYTES || bytes.contains(&0) {
        return None;
    }
    let text = text_of(bytes)?;
    let start = text.trim_start();
    let reads = if start.starts_with('{') || start.starts_with('[') {
        serde_json::from_str::<serde_json::Value>(text).is_ok()
    } else if start.starts_with('<') {
        Document::parse_with_options(text, options()).is_ok()
    } else {
        true
    };
    reads.then(|| text.to_string())
}

fn picture(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")
}

fn escape_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Reads a 3MF file and rebuilds it.
pub fn read(bytes: &[u8]) -> Result<ThreeMf, String> {
    read_with(bytes, MAX_PLACED)
}

fn read_with(bytes: &[u8], max_placed: u64) -> Result<ThreeMf, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("the file is larger than {} MB", MAX_FILE_BYTES / (1024 * 1024)));
    }
    let archive = Archive::open(bytes)?;
    let read_small = |name: &str, max: u64| -> Result<Option<Vec<u8>>, String> {
        match archive.find(name) {
            Some(e) if e.size > max => Err(format!("{} in the file is too large", e.name)),
            Some(e) => archive.read(e).map(Some),
            None => Ok(None),
        }
    };
    // the root model, from the package's relationships (or where every 3MF writer puts it)
    let rels = match read_small("_rels/.rels", MAX_TEXT_BYTES)? {
        Some(x) => relationships(&x)?,
        None => Vec::new(),
    };
    let root = rels
        .iter()
        .find(|(_, t)| t == MODEL_REL)
        .map(|(target, _)| target.clone())
        .unwrap_or_else(|| "3D/3dmodel.model".to_string());
    let thumbnail = rels
        .iter()
        .find(|(_, t)| t == THUMBNAIL_REL)
        .map(|(target, _)| target.clone());

    // every model part the root refers to, read once each
    let mut parts: HashMap<String, Part> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut counts = Counts::default();
    let mut waiting = VecDeque::from([root.clone()]);
    while let Some(name) = waiting.pop_front() {
        let key = name.to_ascii_lowercase();
        if parts.contains_key(&key) {
            continue;
        }
        let entry = archive
            .find(&name)
            .ok_or_else(|| "the file isn't a 3MF: its model is missing".to_string())?;
        if entry.size > MAX_XML_BYTES {
            return Err(format!("the model is larger than {} MB unpacked", MAX_XML_BYTES >> 20));
        }
        let part = parse_part(&archive.read(entry)?, &mut counts)?;
        let refers = part.build.iter().chain(part.objects.values().flat_map(|o| match o {
            Object::Components(list) => list.as_slice(),
            Object::Mesh { .. } => &[],
        }));
        waiting.extend(refers.filter_map(|(p, _, _)| p.clone()));
        names.insert(key.clone(), entry.name.clone());
        parts.insert(key, part);
    }
    let root_key = root.to_ascii_lowercase();
    let root_part = parts
        .get(&root_key)
        .ok_or_else(|| "the file isn't a 3MF: its model is missing".to_string())?;
    let mut placer = Placer {
        parts: &parts,
        scale: root_part.scale,
        placed: 0,
        max_placed,
    };
    let mut mesh = Mesh::default();
    for (p, id, m) in &root_part.build {
        let p = p
            .as_ref()
            .map(|p| p.to_ascii_lowercase())
            .unwrap_or_else(|| root_key.clone());
        placer.place(&mut mesh, &p, *id, m, 0)?;
    }
    let mesh = mesh.checked()?;

    // the rebuild: only what we know
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let (mut kept, mut dropped) = (Vec::new(), Vec::new());
    let (mut settings, mut pictures) = (Vec::new(), Vec::new());
    for entry in archive.entries() {
        let name = entry.name.clone();
        let lower = name.to_ascii_lowercase();
        if name.ends_with('/') || lower == "[content_types].xml" || lower == "_rels/.rels" {
            continue;
        }
        let keep: Option<Vec<u8>> = if parts.contains_key(&lower) {
            Some(archive.read(entry)?)
        } else if lower.ends_with(".rels") && entry.size <= MAX_TEXT_BYTES {
            let x = archive.read(entry)?;
            relationships(&x).ok().map(|_| x)
        } else if picture(&name) && entry.size <= photos::MAX_FILE_BYTES as u64 {
            match photos::reencode(&archive.read(entry)?) {
                Ok(p) => {
                    pictures.push((name.clone(), p.clone()));
                    Some(p)
                }
                Err(_) => None,
            }
        } else if entry.size <= MAX_TEXT_BYTES {
            let x = archive.read(entry)?;
            settings_text(&name, &x).map(|t| {
                settings.push((name.clone(), t));
                x
            })
        } else {
            None
        };
        match keep {
            Some(x) => {
                kept.push(name.clone());
                files.push((name, x));
            }
            None => dropped.push(name),
        }
    }
    // the package thumbnail first
    let thumbnail = thumbnail.filter(|t| kept.iter().any(|k| k.eq_ignore_ascii_case(t)));
    if let Some(t) = &thumbnail {
        pictures.sort_by_key(|(n, _)| !n.eq_ignore_ascii_case(t));
    }
    let root_name = names.get(&root_key).cloned().unwrap_or(root);
    let mut rels_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{RELATIONSHIPS}"><Relationship Target="/{}" Id="rel0" Type="{MODEL_REL}"/>"#,
        escape_attr(&root_name)
    );
    if let Some(t) = &thumbnail {
        rels_xml.push_str(&format!(
            r#"<Relationship Target="/{}" Id="rel1" Type="{THUMBNAIL_REL}"/>"#,
            escape_attr(t)
        ));
    }
    rels_xml.push_str("</Relationships>");
    let mut extensions: Vec<String> = kept
        .iter()
        .filter_map(|n| n.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
        .filter(|e| e != "rels")
        .collect();
    extensions.sort_unstable();
    extensions.dedup();
    let mut types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
    );
    for e in &extensions {
        let content = match e.as_str() {
            "model" => "application/vnd.ms-package.3dmanufacturing-3dmodel+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "json" => "application/json",
            "xml" => "application/xml",
            _ => "text/plain",
        };
        types.push_str(&format!(
            r#"<Default Extension="{}" ContentType="{content}"/>"#,
            escape_attr(e)
        ));
    }
    types.push_str("</Types>");
    let mut package = vec![
        ("[Content_Types].xml".to_string(), types.into_bytes()),
        ("_rels/.rels".to_string(), rels_xml.into_bytes()),
    ];
    package.extend(files);
    let file = zip::write(&package)?;
    Ok(ThreeMf {
        mesh,
        file,
        kept,
        dropped,
        settings,
        pictures,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#;

    pub(crate) fn rels(extra: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="{RELATIONSHIPS}"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="{MODEL_REL}"/>{extra}</Relationships>"#
        )
    }

    /// A cube's mesh, `side` across, as 3MF XML.
    pub(crate) fn cube_mesh(side: f32) -> String {
        let mut v = String::new();
        for i in 0..8u8 {
            let c = |bit: u8| if i & bit != 0 { side } else { 0.0 };
            v.push_str(&format!(r#"<vertex x="{}" y="{}" z="{}"/>"#, c(1), c(2), c(4)));
        }
        // corners: bit 1 = x, 2 = y, 4 = z
        let faces = [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ];
        let mut t = String::new();
        for [a, b, c, d] in faces {
            t.push_str(&format!(
                r#"<triangle v1="{a}" v2="{b}" v3="{c}"/><triangle v1="{a}" v2="{c}" v3="{d}"/>"#
            ));
        }
        format!("<mesh><vertices>{v}</vertices><triangles>{t}</triangles></mesh>")
    }

    pub(crate) fn model(attrs: &str, resources: &str, build: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:p="{PRODUCTION}" {attrs}><resources>{resources}</resources><build>{build}</build></model>"#
        )
    }

    pub(crate) fn package(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let files: Vec<(String, Vec<u8>)> = parts.iter().map(|(n, d)| (n.to_string(), d.to_vec())).collect();
        zip::write(&files).unwrap()
    }

    /// A 3MF of one cube `side` mm across.
    pub(crate) fn cube_3mf(side: f32) -> Vec<u8> {
        let m = model(
            "",
            &format!(r#"<object id="1" type="model">{}</object>"#, cube_mesh(side)),
            r#"<item objectid="1"/>"#,
        );
        package(&[
            ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
            ("_rels/.rels", rels("").as_bytes()),
            ("3D/3dmodel.model", m.as_bytes()),
        ])
    }

    fn png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 0])))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn a_minimal_3mf_reads() {
        let t = read(&cube_3mf(10.0)).unwrap();
        assert_eq!(t.mesh.triangles.len(), 12);
        assert_eq!(t.mesh.size_mm(), [10.0, 10.0, 10.0]);
        assert_eq!(t.kept, ["3D/3dmodel.model"]);
        assert!(t.dropped.is_empty());
        // without relationships, where every writer puts the model
        let m = model(
            "",
            &format!(r#"<object id="1">{}</object>"#, cube_mesh(5.0)),
            r#"<item objectid="1"/>"#,
        );
        let t = read(&package(&[("3D/3dmodel.model", m.as_bytes())])).unwrap();
        assert_eq!(t.mesh.size_mm(), [5.0, 5.0, 5.0]);
    }

    #[test]
    fn units_become_millimetres() {
        let m = model(
            "",
            &format!(r#"<object id="1">{}</object>"#, cube_mesh(1.0)),
            r#"<item objectid="1"/>"#,
        )
        .replace(r#"unit="millimeter""#, r#"unit="inch""#);
        let t = read(&package(&[("3D/3dmodel.model", m.as_bytes())])).unwrap();
        assert!((t.mesh.size_mm()[0] - 25.4).abs() < 1e-4);
        let bad = m.replace(r#"unit="inch""#, r#"unit="furlong""#);
        assert!(read(&package(&[("3D/3dmodel.model", bad.as_bytes())])).is_err());
    }

    #[test]
    fn components_and_build_transforms_are_applied() {
        let resources = format!(
            r#"<object id="1">{}</object><object id="2"><components><component objectid="1"/><component objectid="1" transform="1 0 0 0 1 0 0 0 1 20 0 0"/></components></object>"#,
            cube_mesh(10.0)
        );
        let m = model(
            "",
            &resources,
            r#"<item objectid="2" transform="2 0 0 0 2 0 0 0 2 0 0 5"/>"#,
        );
        let t = read(&package(&[("3D/3dmodel.model", m.as_bytes())])).unwrap();
        assert_eq!(t.mesh.triangles.len(), 24);
        assert_eq!(t.mesh.size_mm(), [60.0, 20.0, 20.0]);
        let (lo, _) = t.mesh.bounds().unwrap();
        assert_eq!(lo, [0.0, 0.0, 5.0], "the item is moved after it's scaled");
    }

    #[test]
    fn production_paths_to_other_model_parts_are_followed() {
        // as Bambu Studio writes: the root's object is made of a component in another part
        let object = model("", &format!(r#"<object id="7">{}</object>"#, cube_mesh(10.0)), "");
        let root = model(
            "",
            r#"<object id="1"><components><component p:path="/3D/Objects/object_1.model" objectid="7" transform="1 0 0 0 1 0 0 0 1 0 0 0"/></components></object>"#,
            r#"<item objectid="1"/>"#,
        );
        let t = read(&package(&[
            ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
            ("_rels/.rels", rels("").as_bytes()),
            ("3D/3dmodel.model", root.as_bytes()),
            ("3D/Objects/object_1.model", object.as_bytes()),
        ]))
        .unwrap();
        assert_eq!(t.mesh.triangles.len(), 12);
        assert!(t.kept.contains(&"3D/Objects/object_1.model".to_string()));
        // a path to a part that isn't there
        let missing = root.replace("object_1", "object_9");
        assert!(
            read(&package(&[
                ("3D/3dmodel.model", missing.as_bytes()),
                ("3D/Objects/object_1.model", object.as_bytes())
            ]))
            .is_err()
        );
    }

    #[test]
    fn cycles_and_runaway_component_trees_are_refused() {
        let m = model(
            "",
            r#"<object id="1"><components><component objectid="1"/></components></object>"#,
            r#"<item objectid="1"/>"#,
        );
        assert!(
            read(&package(&[("3D/3dmodel.model", m.as_bytes())]))
                .unwrap_err()
                .contains("nest too deep")
        );
        // each level places the one below ten times: 10^4 cubes
        let mut resources = format!(r#"<object id="0">{}</object>"#, cube_mesh(1.0));
        for level in 1..=4 {
            let c = format!(r#"<component objectid="{}"/>"#, level - 1).repeat(10);
            resources.push_str(&format!(
                r#"<object id="{level}"><components>{c}</components></object>"#
            ));
        }
        let m = model("", &resources, r#"<item objectid="4"/>"#);
        let file = package(&[("3D/3dmodel.model", m.as_bytes())]);
        assert!(read_with(&file, 12 * 10_000).is_ok(), "at the budget");
        assert!(
            read_with(&file, 12 * 10_000 - 1)
                .unwrap_err()
                .contains("too many triangles")
        );
    }

    #[test]
    fn a_dtd_or_entity_in_the_model_is_refused() {
        let m = model(
            "",
            &format!(r#"<object id="1">{}</object>"#, cube_mesh(5.0)),
            r#"<item objectid="1"/>"#,
        );
        let evil = m.replacen(
            "<model",
            r#"<!DOCTYPE model [<!ENTITY x SYSTEM "file:///etc/passwd">]><model"#,
            1,
        );
        assert!(read(&package(&[("3D/3dmodel.model", evil.as_bytes())])).is_err());
        let rels_evil = format!(r#"<!DOCTYPE r [<!ENTITY a "aaaa">]>{}"#, rels(""));
        assert!(
            read(&package(&[
                ("_rels/.rels", rels_evil.as_bytes()),
                ("3D/3dmodel.model", m.as_bytes())
            ]))
            .is_err()
        );
    }

    #[test]
    fn broken_models_are_refused() {
        let with = |resources: &str| {
            let m = model("", resources, r#"<item objectid="1"/>"#);
            read(&package(&[("3D/3dmodel.model", m.as_bytes())]))
        };
        let bad_index = r#"<object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/></vertices><triangles><triangle v1="0" v2="0" v3="5"/></triangles></mesh></object>"#;
        assert!(with(bad_index).is_err());
        let nan = cube_mesh(5.0).replacen(r#"x="0""#, r#"x="NaN""#, 1);
        assert!(with(&format!(r#"<object id="1">{nan}</object>"#)).is_err());
        let huge = cube_mesh(5.0).replacen(r#"x="5""#, r#"x="1e30""#, 1);
        assert!(with(&format!(r#"<object id="1">{huge}</object>"#)).is_err());
        assert!(
            with(r#"<object id="2"></object>"#).is_err(),
            "the build refers to a missing object"
        );
        assert!(with("").is_err());
        assert!(read(&package(&[("readme.txt", b"hello")])).is_err(), "no model");
        assert!(read(b"PK\x03\x04 not really").is_err());
    }

    #[test]
    fn the_rebuild_keeps_only_known_parts() {
        let thumbnail = format!(r#"<Relationship Target="/Metadata/thumbnail.png" Id="rel1" Type="{THUMBNAIL_REL}"/>"#);
        let file = {
            let m = model(
                "",
                &format!(r#"<object id="1">{}</object>"#, cube_mesh(10.0)),
                r#"<item objectid="1"/>"#,
            );
            package(&[
                ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
                ("_rels/.rels", rels(&thumbnail).as_bytes()),
                ("3D/3dmodel.model", m.as_bytes()),
                ("Metadata/thumbnail.png", &png()),
                ("Metadata/plate_1.png", &png()),
                ("Metadata/not_really.png", b"<script>alert(1)</script>"),
                ("Metadata/project_settings.config", br#"{"layer_height": "0.2", "nozzle_diameter": ["0.4"]}"#),
                ("Metadata/model_settings.config", br#"<?xml version="1.0"?><config><object id="1"><metadata key="name" value="cube"/></object></config>"#),
                ("Metadata/Slic3r_PE.config", b"; generated by PrusaSlicer\nlayer_height = 0.2\n"),
                ("Metadata/broken.config", br#"{"layer_height": "#),
                ("Metadata/plate_1.gcode", b"G28\nG1 X10\n"),
                ("Metadata/plate_1.gcode.md5", b"abc"),
                ("Metadata/run.exe", b"MZ\x90\x00"),
                ("scripts/install.js", b"alert(1)"),
                ("3D/unused.model", b"<model/>"),
            ])
        };
        let t = read(&file).unwrap();
        let mut kept = t.kept.clone();
        kept.sort();
        assert_eq!(
            kept,
            [
                "3D/3dmodel.model",
                "Metadata/Slic3r_PE.config",
                "Metadata/model_settings.config",
                "Metadata/plate_1.png",
                "Metadata/project_settings.config",
                "Metadata/thumbnail.png",
            ]
        );
        for gone in [
            "Metadata/plate_1.gcode",
            "Metadata/plate_1.gcode.md5",
            "Metadata/run.exe",
            "scripts/install.js",
            "Metadata/not_really.png",
            "Metadata/broken.config",
            "3D/unused.model",
        ] {
            assert!(t.dropped.contains(&gone.to_string()), "{gone}");
        }
        assert_eq!(t.settings.len(), 3);
        assert_eq!(t.pictures[0].0, "Metadata/thumbnail.png", "the package thumbnail first");
        // the rebuilt file has exactly what was kept, plus its own package parts
        let a = Archive::open(&t.file).unwrap();
        let mut names: Vec<_> = a.entries().iter().map(|e| e.name.clone()).collect();
        names.sort();
        let mut expected: Vec<String> = kept.iter().map(|k| k.to_string()).collect();
        expected.extend(["[Content_Types].xml".to_string(), "_rels/.rels".to_string()]);
        expected.sort();
        assert_eq!(names, expected);
        let rels_out = String::from_utf8(a.read(a.find("_rels/.rels").unwrap()).unwrap()).unwrap();
        assert!(rels_out.contains("/Metadata/thumbnail.png") && rels_out.contains("/3D/3dmodel.model"));
        let types = String::from_utf8(a.read(a.find("[Content_Types].xml").unwrap()).unwrap()).unwrap();
        assert!(types.contains(r#"Extension="png""#) && types.contains(r#"Extension="model""#));
    }

    #[test]
    fn sliced_gcode_is_dropped_but_geometry_and_settings_kept() {
        let m = model(
            "",
            &format!(r#"<object id="1">{}</object>"#, cube_mesh(10.0)),
            r#"<item objectid="1"/>"#,
        );
        let t = read(&package(&[
            ("3D/3dmodel.model", m.as_bytes()),
            ("Metadata/plate_1.gcode", b"G28\n"),
            ("Metadata/project_settings.config", br#"{"layer_height": "0.2"}"#),
        ]))
        .unwrap();
        assert_eq!(t.dropped, ["Metadata/plate_1.gcode"]);
        assert_eq!(t.settings[0].0, "Metadata/project_settings.config");
        assert_eq!(t.mesh.triangles.len(), 12);
    }

    #[test]
    fn the_rebuild_reads_back_the_same_mesh() {
        let first = read(&cube_3mf(12.5)).unwrap();
        let again = read(&first.file).unwrap();
        assert_eq!(again.mesh, first.mesh);
        assert_eq!(again.kept, first.kept);
        let third = read(&again.file).unwrap();
        assert_eq!(third.file, again.file, "a rebuilt file rebuilds to itself");
    }

    #[test]
    fn a_zip_bomb_is_refused() {
        let zeros = vec![b' '; 4 * 1024 * 1024];
        let file = package(&[("3D/3dmodel.model", &zeros)]);
        assert!(read(&file).unwrap_err().contains("zip bomb"));
    }
}
