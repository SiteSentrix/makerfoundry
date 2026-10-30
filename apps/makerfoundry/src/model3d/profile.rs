//! Print profiles (MF-43, MF-44): what a slicer project's settings say about how its maker printed
//! it: the printer, nozzle, layer height, infill, walls, supports, plates and the filaments
//! actually used. Read from the settings files a 3MF's rebuild kept (`threemf`), by their
//! documented layout (docs/3d-hub-plan.md § 4): Bambu Studio and OrcaSlicer's
//! `Metadata/project_settings.config` (JSON) and `Metadata/model_settings.config` (XML).
//!
//! Only the fields below are kept, never the files' other text, and every number must be in a
//! sensible range or it's left out. Settings that can't be read never fail an upload: the
//! profile says so and the geometry is kept.

use serde::Serialize;
use usvg::roxmltree::{Document, ParsingOptions};

/// One filament slot a project uses: its type (`PLA`) and colour (`#D03030`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Filament {
    #[serde(rename = "type")]
    pub kind: String,
    pub colour: String,
}

/// What a project's settings say, each part only when it could be read and is in range.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Profile {
    /// `bambu`, `orca`, `prusa`, `cura` or `unknown`.
    pub slicer: &'static str,
    /// The printer as the project names it, and its badges (`x1c`, `a1-mini`, `mk4`…).
    pub printer: Option<String>,
    pub printers: Vec<&'static str>,
    pub nozzle_mm: Option<f64>,
    pub layer_mm: Option<f64>,
    pub infill_percent: Option<f64>,
    pub walls: Option<i32>,
    pub supports: Option<bool>,
    pub plates: Option<i32>,
    /// The filaments used (only the slots the model's parts use, when that's known).
    pub filaments: Vec<Filament>,
    /// Whether the settings could be read at all.
    pub readable: bool,
}

/// Printers by what their names contain → badge id and name. Longer names first, so "A1 mini"
/// isn't taken for "A1".
pub const PRINTERS: &[(&str, &str, &str)] = &[
    ("x1 carbon", "x1c", "X1 Carbon"),
    ("x1e", "x1e", "X1E"),
    ("x1", "x1", "X1"),
    ("p1s", "p1s", "P1S"),
    ("p1p", "p1p", "P1P"),
    ("p2s", "p2s", "P2S"),
    ("a1 mini", "a1-mini", "A1 mini"),
    ("a1", "a1", "A1"),
    ("h2d", "h2d", "H2D"),
    ("h2s", "h2s", "H2S"),
    ("mk4s", "mk4s", "MK4S"),
    ("mk4", "mk4", "MK4"),
    ("mk3.9", "mk3-9", "MK3.9"),
    ("mk3s", "mk3s", "MK3S"),
    ("mini", "mini", "MINI"),
    ("core one", "core-one", "CORE One"),
    ("xl", "xl", "XL"),
    ("ender-3", "ender-3", "Ender-3"),
    ("k1", "k1", "K1"),
    ("voron", "voron", "Voron"),
];

/// The badge for a printer's name, if it's one we know.
pub fn badge(printer: &str) -> Option<&'static str> {
    let lower = printer.to_ascii_lowercase();
    PRINTERS
        .iter()
        .find(|(needle, _, _)| {
            // a whole word (or words): "a1" in "Bambu Lab A1" but not in "Bambu Lab A10"
            lower.match_indices(needle).any(|(at, _)| {
                let before = lower[..at].chars().next_back();
                let after = lower[at + needle.len()..].chars().next();
                !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_alphanumeric())
            })
        })
        .map(|(_, id, _)| *id)
}

/// A badge's name.
pub fn badge_name(id: &str) -> &'static str {
    PRINTERS.iter().find(|(_, b, _)| *b == id).map_or("", |(_, _, n)| n)
}

/// The slicer from the application that wrote the 3MF.
pub fn slicer_of(application: Option<&str>) -> &'static str {
    let app = application.unwrap_or("").to_ascii_lowercase();
    if app.contains("bambustudio") || app.contains("bambu studio") {
        "bambu"
    } else if app.contains("orcaslicer") || app.contains("orca slicer") {
        "orca"
    } else if app.contains("prusaslicer") || app.contains("slic3r") {
        "prusa"
    } else if app.contains("cura") {
        "cura"
    } else {
        "unknown"
    }
}

/// A number from a setting: `"0.2"`, `0.2`, `"15%"`, or the first of a list (`["0.4"]`).
fn number(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().trim_end_matches('%').trim().parse().ok(),
        serde_json::Value::Array(list) => list.first().and_then(number),
        _ => None,
    }
    .filter(|n: &f64| n.is_finite())
}

fn within(v: Option<f64>, lo: f64, hi: f64) -> Option<f64> {
    v.filter(|n| (lo..=hi).contains(n))
}

/// A list of strings from a setting (a list, or one string with `;` between).
fn strings(v: Option<&serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::Array(list)) => list.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        Some(serde_json::Value::String(s)) => s.split(';').map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// A filament type as kept: letters, digits, `-`, `+` and spaces, at most 20.
fn filament_type(t: &str) -> Option<String> {
    let t = t.trim();
    (!t.is_empty() && t.len() <= 20 && t.chars().all(|c| c.is_ascii_alphanumeric() || "-+ ".contains(c)))
        .then(|| t.to_ascii_uppercase())
}

/// A colour as kept: `#RRGGBB` (an alpha byte dropped), upper case.
fn colour(c: &str) -> Option<String> {
    let hex = c.trim().strip_prefix('#')?;
    let hex = match hex.len() {
        6 => hex,
        8 => hex.get(..6)?,
        _ => return None,
    };
    hex.chars()
        .all(|c| c.is_ascii_hexdigit())
        .then(|| format!("#{}", hex.to_ascii_uppercase()))
}

/// The most filament slots read.
const MAX_SLOTS: usize = 32;

/// Reads a project's profile from the settings files it kept (name, text) and what wrote it.
pub fn read(settings: &[(String, String)], application: Option<&str>) -> Option<Profile> {
    let find = |name: &str| {
        settings
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, t)| t.as_str())
    };
    let slicer = slicer_of(application);
    let project = find("Metadata/project_settings.config");
    if project.is_none() && slicer == "unknown" {
        return None;
    }
    let mut p = Profile {
        slicer,
        ..Profile::default()
    };
    let Some(json) = project.and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()) else {
        // a slicer's project whose settings can't be read: still a profile, saying so
        return Some(p);
    };
    let Some(map) = json.as_object() else {
        return Some(p);
    };
    p.readable = true;
    let get = |k: &str| map.get(k);
    p.printer = get("printer_model")
        .or_else(|| get("printer_settings_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.chars().filter(|c| !c.is_control()).take(80).collect::<String>())
        .filter(|s| !s.trim().is_empty());
    p.printers = p.printer.as_deref().and_then(badge).into_iter().collect();
    p.nozzle_mm = within(get("nozzle_diameter").and_then(number), 0.1, 2.0);
    p.layer_mm = within(get("layer_height").and_then(number), 0.01, 1.0);
    p.infill_percent = within(get("sparse_infill_density").and_then(number), 0.0, 100.0);
    p.walls = within(get("wall_loops").and_then(number), 0.0, 50.0).map(|n| n.round() as i32);
    p.supports = get("enable_support").and_then(|v| match v {
        serde_json::Value::Bool(b) => Some(*b),
        _ => number(v).filter(|n| *n == 0.0 || *n == 1.0).map(|n| n == 1.0),
    });
    let types = strings(get("filament_type"));
    let colours = strings(get("filament_colour"));
    let slots: Vec<Option<Filament>> = types
        .iter()
        .take(MAX_SLOTS)
        .enumerate()
        .map(|(i, t)| {
            Some(Filament {
                kind: filament_type(t)?,
                colour: colours.get(i).and_then(|c| colour(c)).unwrap_or_default(),
            })
        })
        .collect();
    // which slots the model's objects and parts use, and how many plates (model_settings.config)
    let used = find("Metadata/model_settings.config").and_then(|xml| {
        let doc = Document::parse_with_options(
            xml,
            ParsingOptions {
                allow_dtd: false,
                ..Default::default()
            },
        )
        .ok()?;
        let mut slots: Vec<usize> = Vec::new();
        let mut plates = 0;
        for node in doc.descendants() {
            match node.tag_name().name() {
                "plate" => plates += 1,
                "metadata" if node.attribute("key") == Some("extruder") => {
                    if let Some(n) = node.attribute("value").and_then(|v| v.trim().parse::<usize>().ok())
                        && (1..=MAX_SLOTS).contains(&n)
                        && !slots.contains(&n)
                    {
                        slots.push(n);
                    }
                }
                _ => {}
            }
        }
        Some((slots, plates))
    });
    let (used_slots, plates) = used.unwrap_or_default();
    p.plates = (plates > 0).then_some(plates).filter(|n| *n <= 200);
    p.filaments = if used_slots.is_empty() {
        slots.into_iter().flatten().collect()
    } else {
        let mut used_slots = used_slots;
        used_slots.sort_unstable();
        used_slots
            .iter()
            .filter_map(|n| slots.get(n - 1).cloned().flatten())
            .collect()
    };
    Some(p)
}

impl Profile {
    /// "0.2 mm layers, PLA, X1 Carbon": a short name for the profile.
    pub fn name(&self) -> String {
        let mut parts = Vec::new();
        if let Some(l) = self.layer_mm {
            parts.push(format!("{} mm layers", trim_number(l)));
        }
        let mut types: Vec<&str> = self.filaments.iter().map(|f| f.kind.as_str()).collect();
        types.dedup();
        if !types.is_empty() {
            parts.push(types.join(" + "));
        }
        if let Some(b) = self.printers.first() {
            parts.push(badge_name(b).to_string());
        }
        if parts.is_empty() {
            "Print settings".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// A number without trailing zeros: 0.20 → "0.2".
pub fn trim_number(n: f64) -> String {
    let s = format!("{n:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAMBU: &str = r##"{
        "from": "project",
        "printer_model": "Bambu Lab X1 Carbon",
        "printer_settings_id": "Bambu Lab X1 Carbon 0.4 nozzle",
        "nozzle_diameter": ["0.4"],
        "layer_height": "0.2",
        "sparse_infill_density": "15%",
        "wall_loops": "2",
        "enable_support": "1",
        "filament_type": ["PLA", "PLA", "PETG", "PLA"],
        "filament_colour": ["#FF0000", "#00AE42FF", "#FFFFFF", "#000000"],
        "api_key_for_something": "secret",
        "machine_start_gcode": "G28 ; home"
    }"##;

    const MODEL_SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="2"><metadata key="name" value="Base"/><metadata key="extruder" value="1"/>
    <part id="1" subtype="normal_part"><metadata key="extruder" value="4"/></part>
  </object>
  <object id="3"><metadata key="extruder" value="1"/></object>
  <plate><metadata key="plater_id" value="1"/></plate>
  <plate><metadata key="plater_id" value="2"/></plate>
</config>"#;

    fn files(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect()
    }

    #[test]
    fn a_bambu_3mf_with_ams_reads_printer_nozzle_layers_and_colours_used() {
        let p = read(
            &files(&[
                ("Metadata/project_settings.config", BAMBU),
                ("Metadata/model_settings.config", MODEL_SETTINGS),
            ]),
            Some("BambuStudio-02.00.00.00"),
        )
        .unwrap();
        assert_eq!(p.slicer, "bambu");
        assert!(p.readable);
        assert_eq!(p.printer.as_deref(), Some("Bambu Lab X1 Carbon"));
        assert_eq!(p.printers, ["x1c"]);
        assert_eq!(
            (p.nozzle_mm, p.layer_mm, p.infill_percent),
            (Some(0.4), Some(0.2), Some(15.0))
        );
        assert_eq!((p.walls, p.supports, p.plates), (Some(2), Some(true), Some(2)));
        // four filaments loaded, slots 1 and 4 used
        assert_eq!(
            p.filaments,
            [
                Filament {
                    kind: "PLA".into(),
                    colour: "#FF0000".into()
                },
                Filament {
                    kind: "PLA".into(),
                    colour: "#000000".into()
                },
            ]
        );
        assert_eq!(p.name(), "0.2 mm layers, PLA, X1 Carbon");
    }

    #[test]
    fn an_orca_3mf_reads() {
        let orca = BAMBU.replace("Bambu Lab X1 Carbon", "Voron 2.4 350");
        let p = read(
            &files(&[("Metadata/project_settings.config", &orca)]),
            Some("OrcaSlicer-2.2.0"),
        )
        .unwrap();
        assert_eq!(p.slicer, "orca");
        assert_eq!(p.printers, ["voron"]);
        assert_eq!(
            p.filaments.len(),
            4,
            "every loaded filament when the slots used aren't known"
        );
        assert_eq!(p.filaments[1].colour, "#00AE42", "the alpha byte is dropped");
    }

    #[test]
    fn printer_names_map_to_badges() {
        for (name, want) in [
            ("Bambu Lab X1 Carbon", Some("x1c")),
            ("Bambu Lab X1E", Some("x1e")),
            ("Bambu Lab P1S", Some("p1s")),
            ("Bambu Lab A1 mini", Some("a1-mini")),
            ("Bambu Lab A1", Some("a1")),
            ("Bambu Lab H2D", Some("h2d")),
            ("Original Prusa MK4S", Some("mk4s")),
            ("Original Prusa MK4", Some("mk4")),
            ("Prusa CORE One", Some("core-one")),
            ("Creality Ender-3 V3", Some("ender-3")),
            ("Bambu Lab A10", None),
            ("Some Printer", None),
        ] {
            assert_eq!(badge(name), want, "{name}");
        }
        assert_eq!(badge_name("a1-mini"), "A1 mini");
    }

    #[test]
    fn malformed_or_huge_settings_keep_the_geometry() {
        // unreadable JSON from a known slicer: a profile that says so
        let p = read(
            &files(&[("Metadata/project_settings.config", "{ not json")]),
            Some("BambuStudio-01.10"),
        )
        .unwrap();
        assert!(!p.readable);
        assert_eq!(p.name(), "Print settings");
        // a bad model_settings file only loses the slots used
        let p = read(
            &files(&[
                ("Metadata/project_settings.config", BAMBU),
                ("Metadata/model_settings.config", "<config><!DOCTYPE x>"),
            ]),
            Some("BambuStudio"),
        )
        .unwrap();
        assert_eq!(p.filaments.len(), 4);
        // many slots: only the first 32 read
        let many = format!(
            r#"{{"filament_type": [{}], "filament_colour": []}}"#,
            vec![r#""PLA""#; 5000].join(",")
        );
        let p = read(
            &files(&[("Metadata/project_settings.config", &many)]),
            Some("BambuStudio"),
        )
        .unwrap();
        assert_eq!(p.filaments.len(), MAX_SLOTS);
        // no settings and no known slicer: no profile
        assert!(read(&[], Some("Some CAD program")).is_none());
        assert!(
            read(
                &files(&[("Metadata/project_settings.config", "[1, 2]")]),
                Some("OrcaSlicer")
            )
            .is_some_and(|p| !p.readable)
        );
    }

    #[test]
    fn out_of_range_numbers_are_ignored() {
        let odd = r##"{"nozzle_diameter": ["40"], "layer_height": "-0.2", "sparse_infill_density": "250%",
            "wall_loops": "1e9", "enable_support": "7", "filament_type": ["PLA<script>", "PETG"],
            "filament_colour": ["red", "#12345"], "printer_model": 12}"##;
        let p = read(
            &files(&[("Metadata/project_settings.config", odd)]),
            Some("BambuStudio"),
        )
        .unwrap();
        assert!(p.readable);
        assert_eq!(
            (p.nozzle_mm, p.layer_mm, p.infill_percent, p.walls, p.supports),
            (None, None, None, None, None)
        );
        assert_eq!(p.printer, None);
        assert_eq!(
            p.filaments,
            [Filament {
                kind: "PETG".into(),
                colour: String::new()
            }]
        );
    }

    #[test]
    fn only_whitelisted_keys_are_kept() {
        let p = read(
            &files(&[("Metadata/project_settings.config", BAMBU)]),
            Some("BambuStudio"),
        )
        .unwrap();
        let kept = serde_json::to_string(&p).unwrap();
        assert!(!kept.contains("secret") && !kept.contains("G28"), "{kept}");
        let mut keys: Vec<String> = serde_json::to_value(&p)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "filaments",
                "infill_percent",
                "layer_mm",
                "nozzle_mm",
                "plates",
                "printer",
                "printers",
                "readable",
                "slicer",
                "supports",
                "walls"
            ]
        );
    }
}
