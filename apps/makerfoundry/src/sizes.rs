//! What a design fits (MF-8): the embroidery hoops it fits in, and the material a laser design is
//! made for, so people can find designs for their own machine.
//!
//! Hoop sizes are the usual names for embroidery fields; a machine's actual field can be a little
//! smaller than its hoop's name, so the sizes used are the common sewing fields (in mm).

/// An embroidery hoop: its id in addresses, its name, and its sewing field (mm).
#[derive(Debug)]
pub struct Hoop {
    pub id: &'static str,
    pub name: &'static str,
    pub width_mm: f64,
    pub height_mm: f64,
}

pub const HOOPS: [Hoop; 5] = [
    Hoop {
        id: "4x4",
        name: "4×4 in (100 × 100 mm)",
        width_mm: 100.0,
        height_mm: 100.0,
    },
    Hoop {
        id: "5x7",
        name: "5×7 in (130 × 180 mm)",
        width_mm: 130.0,
        height_mm: 180.0,
    },
    Hoop {
        id: "6x10",
        name: "6×10 in (160 × 260 mm)",
        width_mm: 160.0,
        height_mm: 260.0,
    },
    Hoop {
        id: "8x8",
        name: "8×8 in (200 × 200 mm)",
        width_mm: 200.0,
        height_mm: 200.0,
    },
    Hoop {
        id: "8x12",
        name: "8×12 in (200 × 300 mm)",
        width_mm: 200.0,
        height_mm: 300.0,
    },
];

pub fn hoop(id: &str) -> Option<&'static Hoop> {
    HOOPS.iter().find(|h| h.id == id)
}

impl Hoop {
    /// Whether a design `w` × `h` mm fits, either way round.
    pub fn fits(&self, w: f64, h: f64) -> bool {
        w.is_finite()
            && h.is_finite()
            && ((w <= self.width_mm && h <= self.height_mm) || (w <= self.height_mm && h <= self.width_mm))
    }
}

/// The smallest hoop (by field area) a design `w` × `h` mm fits.
pub fn smallest_hoop(w: f64, h: f64) -> Option<&'static Hoop> {
    HOOPS
        .iter()
        .filter(|hp| hp.fits(w, h))
        .min_by(|a, b| (a.width_mm * a.height_mm).total_cmp(&(b.width_mm * b.height_mm)))
}

/// Materials a laser design can be made for: id and name.
pub const MATERIALS: [(&str, &str); 10] = [
    ("plywood", "Plywood"),
    ("mdf", "MDF"),
    ("hardwood", "Hardwood"),
    ("acrylic", "Acrylic"),
    ("leather", "Leather"),
    ("card", "Paper and card"),
    ("fabric", "Fabric"),
    ("felt", "Felt"),
    ("cork", "Cork"),
    ("other", "Other"),
];

pub fn material_name(id: &str) -> Option<&'static str> {
    MATERIALS.iter().find(|(m, _)| *m == id).map(|(_, n)| *n)
}

/// The thickest material a design may name (mm).
pub const MAX_THICKNESS_MM: f64 = 50.0;

/// A thickness as typed (`3`, `3.2`, `3 mm`) → mm, `None` when empty; or why not.
pub fn check_thickness(text: &str) -> Result<Option<f64>, String> {
    let t = text.trim().trim_end_matches("mm").trim();
    if t.is_empty() {
        return Ok(None);
    }
    match t.replace(',', ".").parse::<f64>() {
        Ok(v) if v.is_finite() && v > 0.0 && v <= MAX_THICKNESS_MM => Ok(Some((v * 100.0).round() / 100.0)),
        _ => Err(format!(
            "Give the material's thickness in mm, up to {MAX_THICKNESS_MM:.0}, or leave it empty."
        )),
    }
}

/// A thickness as shown: `3 mm`, `1.5 mm`.
pub fn thickness_text(mm: f64) -> String {
    let s = format!("{mm:.2}");
    format!("{} mm", s.trim_end_matches('0').trim_end_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn designs_fit_hoops_either_way_round() {
        let h57 = hoop("5x7").unwrap();
        assert!(h57.fits(130.0, 180.0) && h57.fits(180.0, 130.0) && !h57.fits(131.0, 181.0));
        assert!(!h57.fits(f64::NAN, 10.0));
        assert_eq!(smallest_hoop(60.0, 60.0).map(|h| h.id), Some("4x4"));
        assert_eq!(smallest_hoop(170.0, 120.0).map(|h| h.id), Some("5x7"));
        assert_eq!(smallest_hoop(190.0, 190.0).map(|h| h.id), Some("8x8"));
        assert_eq!(smallest_hoop(250.0, 150.0).map(|h| h.id), Some("6x10"));
        assert!(smallest_hoop(400.0, 100.0).is_none());
    }

    #[test]
    fn thicknesses_are_checked() {
        assert_eq!(check_thickness(""), Ok(None));
        assert_eq!(check_thickness("3"), Ok(Some(3.0)));
        assert_eq!(check_thickness(" 3,2 mm"), Ok(Some(3.2)));
        for bad in ["0", "-1", "51", "nan", "inf", "three"] {
            assert!(check_thickness(bad).is_err(), "{bad}");
        }
        assert_eq!(thickness_text(3.0), "3 mm");
        assert_eq!(thickness_text(1.5), "1.5 mm");
        assert_eq!(material_name("mdf"), Some("MDF"));
        assert_eq!(material_name("gold"), None);
    }
}
