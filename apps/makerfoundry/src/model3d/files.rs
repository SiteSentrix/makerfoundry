//! Which files a 3D model may have (MF-39), each checked by what it is, not what it's called:
//! meshes (STL, OBJ) and 3MF projects are read and their geometry measured; STEP sources are
//! checked to be ISO 10303-21 text (the standard's clear-text encoding, whose files start
//! `ISO-10303-21;` and end `END-ISO-10303-21;`:
//! <https://en.wikipedia.org/wiki/ISO_10303-21>); a few plain-text companions (an OBJ's `.mtl`,
//! OpenSCAD sources, notes) are checked to be text. Anything else is refused.
//!
//! STEP and the text companions aren't read further, so they're only ever offered as downloads
//! (`Content-Disposition: attachment`), never shown on our site. (A STEP file's preview, when the
//! converter service is set, is drawn from the STL the service makes of it: `step.rs`.)

use super::mesh::{MAX_FILE_BYTES, Mesh};
use super::{obj, stl, threemf};

/// What a file is for on a model's page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Geometry to print (STL, OBJ).
    Mesh,
    /// A slicer project with geometry and settings (3MF).
    Project,
    /// CAD source (STEP).
    Source,
    /// A text companion (`.mtl`, `.scad`, `.txt`, `.md`).
    Other,
}

impl Role {
    /// As kept in the database (`mf_model_files.role`).
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Mesh => "mesh",
            Role::Project => "project",
            Role::Source => "source",
            Role::Other => "other",
        }
    }
}

/// A file that passed its checks.
#[derive(Debug)]
pub struct Checked {
    /// `stl`, `obj`, `3mf`, `step`, `mtl`, `scad`, `txt` or `md`.
    pub format: &'static str,
    pub role: Role,
    /// The geometry read, for meshes and projects.
    pub mesh: Option<Mesh>,
    /// What's kept and offered: a 3MF as rebuilt, everything else as it came.
    pub bytes: Vec<u8>,
    /// A 3MF's slicer settings and pictures (`threemf::ThreeMf`).
    pub settings: Vec<(String, String)>,
    pub pictures: Vec<(String, Vec<u8>)>,
    /// The parts a 3MF lost in its rebuild, to tell the maker.
    pub dropped: Vec<String>,
    /// What a 3MF's slicer settings say (`profile`).
    pub profile: Option<super::profile::Profile>,
}

/// The extensions taken, with their format and role.
pub const ALLOWED: &[(&str, &str, Role)] = &[
    ("stl", "stl", Role::Mesh),
    ("obj", "obj", Role::Mesh),
    ("3mf", "3mf", Role::Project),
    ("step", "step", Role::Source),
    ("stp", "step", Role::Source),
    ("mtl", "mtl", Role::Other),
    ("scad", "scad", Role::Other),
    ("txt", "txt", Role::Other),
    ("md", "md", Role::Other),
];

/// The largest text companion.
const MAX_TEXT_BYTES: usize = 5 * 1024 * 1024;

/// What a file name's extension says it is, if it's one we take.
pub fn kind_of(name: &str) -> Option<(&'static str, Role)> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    ALLOWED
        .iter()
        .find(|(e, _, _)| *e == ext)
        .map(|(_, format, role)| (*format, *role))
}

/// Whether a format is only ever offered as a download, never shown.
pub fn attachment_only(format: &str) -> bool {
    !matches!(format, "stl" | "obj" | "3mf")
}

/// Checks an uploaded file by its name and contents.
pub fn check(name: &str, bytes: &[u8]) -> Result<Checked, String> {
    let (format, role) = kind_of(name).ok_or_else(|| {
        "MakerFoundry 3D takes STL, 3MF, OBJ and STEP files (and .mtl, .scad, .txt or .md beside them)".to_string()
    })?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("the file is larger than {} MB", MAX_FILE_BYTES / (1024 * 1024)));
    }
    let plain = |mesh: Option<Mesh>| Checked {
        format,
        role,
        mesh,
        bytes: bytes.to_vec(),
        settings: Vec::new(),
        pictures: Vec::new(),
        dropped: Vec::new(),
        profile: None,
    };
    match format {
        "stl" => Ok(plain(Some(stl::read(bytes)?))),
        "obj" => Ok(plain(Some(obj::read(bytes)?))),
        "3mf" => {
            let t = threemf::read(bytes)?;
            let profile = super::profile::read(&t.settings, t.application.as_deref());
            Ok(Checked {
                profile,
                format,
                role,
                mesh: Some(t.mesh),
                bytes: t.file,
                settings: t.settings,
                pictures: t.pictures,
                dropped: t.dropped,
            })
        }
        "step" => {
            check_step(bytes)?;
            Ok(plain(None))
        }
        _ => {
            if bytes.len() > MAX_TEXT_BYTES || bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
                return Err(format!("a .{format} file must be text of at most 5 MB"));
            }
            Ok(plain(None))
        }
    }
}

/// An ISO 10303-21 file: UTF-8 text, no NUL bytes, starting `ISO-10303-21;` with a `HEADER;`
/// section and ending `END-ISO-10303-21;`.
fn check_step(bytes: &[u8]) -> Result<(), String> {
    let not_step = || "the file isn't a STEP file (ISO 10303-21)".to_string();
    if bytes.contains(&0) {
        return Err(not_step());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| not_step())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).trim();
    let starts = text.starts_with("ISO-10303-21;");
    if !starts || !text.contains("HEADER;") || !text.ends_with("END-ISO-10303-21;") {
        return Err(not_step());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: &str = "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('a test part'),'2;1');\n\
FILE_NAME('part.step','2026-10-30T00:00:00',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\n\
ENDSEC;\nDATA;\n#1=CARTESIAN_POINT('',(0.,0.,0.));\nENDSEC;\nEND-ISO-10303-21;\n";

    #[test]
    fn a_step_header_passes() {
        let c = check("bracket.STEP", STEP.as_bytes()).unwrap();
        assert_eq!((c.format, c.role), ("step", Role::Source));
        assert!(c.mesh.is_none());
        assert_eq!(c.bytes, STEP.as_bytes(), "kept as it came");
        assert!(check("bracket.stp", format!("\u{feff}{STEP}").as_bytes()).is_ok());
    }

    #[test]
    fn a_binary_or_html_file_named_step_is_refused() {
        let html = "<!doctype html><html><body><script>alert(1)</script></body></html>";
        assert!(check("part.step", html.as_bytes()).is_err());
        assert!(check("part.step", &[0x50, 0x4b, 0x03, 0x04, 0, 0, 0xff]).is_err());
        let mut nul = STEP.as_bytes().to_vec();
        nul.insert(20, 0);
        assert!(check("part.step", &nul).is_err());
        assert!(
            check("part.step", STEP.replace("END-ISO-10303-21;", "").as_bytes()).is_err(),
            "cut short"
        );
        assert!(check("part.step", STEP.replace("HEADER;", "").as_bytes()).is_err());
    }

    #[test]
    fn an_unknown_extension_is_refused() {
        for name in [
            "model.exe",
            "model.stl.exe",
            "model.js",
            "model.svg",
            "model.html",
            "model",
            "model.gcode",
            "model.zip",
        ] {
            assert!(check(name, b"solid x").is_err(), "{name}");
        }
        assert_eq!(kind_of("Cube.STL"), Some(("stl", Role::Mesh)));
        assert_eq!(kind_of("x.3mf"), Some(("3mf", Role::Project)));
    }

    #[test]
    fn each_file_is_read_by_what_it_says_it_is() {
        let stl = crate::model3d::stl::tests::binary(b"", &crate::model3d::stl::tests::cube(10.0));
        let c = check("cube.stl", &stl).unwrap();
        assert_eq!(c.role, Role::Mesh);
        assert_eq!(c.mesh.unwrap().triangles.len(), 12);
        // an STL named .obj is read as OBJ, and isn't one
        assert!(check("cube.obj", &stl).is_err());
        let three = crate::model3d::threemf::tests::cube_3mf(10.0);
        let c = check("cube.3mf", &three).unwrap();
        assert_eq!(c.role, Role::Project);
        assert_eq!(
            c.bytes,
            crate::model3d::threemf::read(&three).unwrap().file,
            "a 3MF is kept as rebuilt"
        );
        assert!(check("cube.3mf", &stl).is_err());
        // text companions
        assert!(check("cube.mtl", b"newmtl red\nKd 1 0 0\n").is_ok());
        assert!(check("notes.md", b"# Print it on its side").is_ok());
        assert!(check("notes.txt", &[0xff, 0xfe, 0x00, 0x41]).is_err());
    }

    #[test]
    fn step_files_download_as_attachments() {
        for f in ["step", "mtl", "scad", "txt", "md"] {
            assert!(attachment_only(f), "{f}");
        }
        for f in ["stl", "obj", "3mf"] {
            assert!(!attachment_only(f), "{f}");
        }
    }
}
