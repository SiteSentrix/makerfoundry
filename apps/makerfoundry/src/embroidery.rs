//! Machine-embroidery files (MF-2): read with StitchFoundry's readers (`sf-formats`, which never
//! panic and cap what they allocate on any bytes), checked, and drawn as stitches. The preview is
//! an SVG MakerFoundry writes itself from the stitches (numbers and colours only), never the
//! uploaded file, so it is safe to show inline.

use std::fmt::Write as _;

use sf_core::stats::pattern_stats;
use sf_core::{Cmd, DesignSettings, Pattern, Rgb};

/// The largest embroidery file accepted. Real designs are well under a megabyte.
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
/// The fewest stitches a design may have.
pub const MIN_STITCHES: usize = 10;
/// The most stitches a design may have: more than any hoop sews in one go, and the most the
/// preview draws.
pub const MAX_STITCHES: usize = 300_000;
/// The largest design accepted (mm, either side): the biggest multi-needle fields are about 1.2 m.
pub const MAX_SIZE_MM: f64 = 1500.0;

/// One thread colour of a design.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Colour {
    /// `#rrggbb`.
    pub hex: String,
    /// The thread's name and number as the file gives them (may be empty).
    pub name: String,
    pub stitches: usize,
}

/// An embroidery file, read and checked.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Embroidery {
    /// The format's usual extension, in capitals (`PES`).
    pub format: String,
    pub stitches: usize,
    pub jumps: usize,
    pub trims: usize,
    pub color_changes: usize,
    pub width_mm: f64,
    pub height_mm: f64,
    pub colours: Vec<Colour>,
    /// The stitches drawn, as an SVG element.
    #[serde(skip)]
    pub svg: String,
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// Reads `bytes` (named `file_name` by the uploader; its extension is only a hint).
pub fn read(bytes: &[u8], file_name: &str) -> Result<Embroidery, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("the file is larger than {} MB", MAX_FILE_BYTES / (1024 * 1024)));
    }
    if bytes.is_empty() {
        return Err("the file is empty".into());
    }
    let hint = file_name.rsplit_once('.').map(|(_, ext)| ext);
    let (format, pattern) = sf_formats::read_any(bytes, hint)
        .map_err(|_| "this isn't an embroidery file MakerFoundry can read (DST, PES, PEC, JEF, EXP, VP3, XXX, U01, PCS, TAP, DSB or DSZ)".to_string())?;
    if pattern.stitches.len() > MAX_STITCHES {
        return Err(format!("the design has more than {MAX_STITCHES} stitches"));
    }
    if pattern.stitches.iter().any(|s| !s.x.is_finite() || !s.y.is_finite()) {
        return Err("the file has stitches at impossible positions".into());
    }
    let stats = pattern_stats(&pattern, &DesignSettings::default());
    if stats.stitches == 0 {
        return Err("the file has no stitches".into());
    }
    // (formats without a header, such as EXP, read any few bytes as a few stitches)
    if stats.stitches < MIN_STITCHES {
        return Err(format!(
            "the file has only {} stitches: too few to be a design",
            stats.stitches
        ));
    }
    if !(stats.width_mm.is_finite() && stats.height_mm.is_finite())
        || stats.width_mm > MAX_SIZE_MM
        || stats.height_mm > MAX_SIZE_MM
    {
        return Err(format!("the design is larger than {MAX_SIZE_MM} mm"));
    }
    let colours = stats
        .colors
        .iter()
        .map(|c| {
            let name = [c.thread.name.as_str(), c.thread.code.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            Colour {
                hex: hex(c.thread.color),
                name,
                stitches: c.stitches,
            }
        })
        .collect();
    Ok(Embroidery {
        format: format.extension().trim_start_matches('.').to_uppercase(),
        stitches: stats.stitches,
        jumps: stats.jumps,
        trims: stats.trims,
        color_changes: stats.color_changes,
        width_mm: stats.width_mm,
        height_mm: stats.height_mm,
        colours,
        svg: preview_svg(&pattern),
    })
}

/// The stitches as an SVG: one path a colour block, a pen lift at each jump and trim, on a
/// fabric-coloured ground. Positions in tenths of a millimetre, as whole numbers.
fn preview_svg(pattern: &Pattern) -> String {
    let tenth = |v: f64| (v * 10.0).round() as i64;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for s in pattern.stitches.iter().filter(|s| s.cmd == Cmd::Stitch) {
        let (x, y) = (tenth(s.x), tenth(s.y));
        (min_x, min_y, max_x, max_y) = (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y));
    }
    if min_x > max_x {
        return String::new();
    }
    let pad = ((max_x - min_x).max(max_y - min_y) / 20).max(20);
    let (vx, vy, vw, vh) = (
        min_x - pad,
        min_y - pad,
        max_x - min_x + 2 * pad,
        max_y - min_y + 2 * pad,
    );
    let mut paths = String::new();
    let mut d = String::new();
    let mut colour = 0usize;
    let mut pen_down = false;
    let flush = |d: &mut String, colour: usize, paths: &mut String| {
        if d.contains('L') {
            let _ = write!(
                paths,
                r#"<path stroke="{}" d="{d}"/>"#,
                hex(pattern.thread(colour).color)
            );
        }
        d.clear();
    };
    for s in &pattern.stitches {
        match s.cmd {
            Cmd::Stitch => {
                let _ = write!(d, "{}{} {}", if pen_down { 'L' } else { 'M' }, tenth(s.x), tenth(s.y));
                pen_down = true;
            }
            Cmd::Jump | Cmd::Trim => pen_down = false,
            Cmd::ColorChange => {
                flush(&mut d, colour, &mut paths);
                colour += 1;
                pen_down = false;
            }
            Cmd::Stop | Cmd::End => {}
        }
    }
    flush(&mut d, colour, &mut paths);
    format!(
        r##"<svg class="stitches" viewBox="{vx} {vy} {vw} {vh}" role="img" aria-label="The design's stitches"><rect x="{vx}" y="{vy}" width="{vw}" height="{vh}" fill="#f4f0e8"/><g fill="none" stroke-width="4" stroke-linecap="round" stroke-linejoin="round">{paths}</g></svg>"##
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_few_bytes_are_not_a_design() {
        // a headerless format reads anything; four bytes make two EXP stitches
        let r = read(b"junk", "x.jef");
        assert!(
            r.as_ref().is_err_and(|e| e.contains("too few")),
            "{:?}",
            r.map(|e| e.stitches)
        );
    }

    use super::*;
    use sf_core::{Point, Thread};
    use sf_formats::{Format, WriteOptions};

    /// A small two-colour design written as `format` by StitchFoundry's own writer.
    fn sample(format: Format) -> Vec<u8> {
        let mut p = Pattern::new();
        p.threads = vec![
            Thread {
                color: Rgb { r: 200, g: 30, b: 40 },
                name: "Red".into(),
                code: "800".into(),
                chart: String::new(),
            },
            Thread {
                color: Rgb { r: 20, g: 60, b: 200 },
                name: "Blue".into(),
                code: String::new(),
                chart: String::new(),
            },
        ];
        for i in 0..=20 {
            p.add(Point::new(f64::from(i), f64::from(i % 2) * 3.0), Cmd::Stitch);
        }
        p.add(Point::new(20.0, 1.0), Cmd::Trim);
        p.add(Point::new(20.0, 1.0), Cmd::ColorChange);
        p.add(Point::new(0.0, 10.0), Cmd::Jump);
        for i in 0..=20 {
            p.add(Point::new(f64::from(i), 10.0 + f64::from(i % 2) * 3.0), Cmd::Stitch);
        }
        p.add(Point::new(20.0, 13.0), Cmd::End);
        sf_formats::write(format, &p, &WriteOptions::default()).unwrap()
    }

    #[test]
    fn a_pes_reads_with_its_stitches_colours_and_size() {
        let e = read(&sample(Format::Pes), "rose.pes").unwrap();
        assert_eq!(e.format, "PES");
        assert!(e.stitches >= 40, "{}", e.stitches);
        assert_eq!(e.color_changes, 1);
        assert_eq!(e.colours.len(), 2);
        assert!(
            (e.width_mm - 20.0).abs() < 0.5 && (e.height_mm - 13.0).abs() < 0.5,
            "{} × {}",
            e.width_mm,
            e.height_mm
        );
        assert!(
            e.svg.starts_with("<svg") && e.svg.matches("<path").count() == 2,
            "{}",
            e.svg
        );
    }

    #[test]
    fn every_format_reads_whatever_its_name() {
        for format in [
            Format::Dst,
            Format::Exp,
            Format::Jef,
            Format::Pes,
            Format::Vp3,
            Format::Xxx,
            Format::U01,
        ] {
            let bytes = sample(format);
            let e = read(&bytes, "design.bin").unwrap_or_else(|why| panic!("{format:?}: {why}"));
            assert!(e.stitches >= 40, "{format:?}");
            assert!(!e.svg.is_empty(), "{format:?}");
        }
    }

    #[test]
    fn the_preview_holds_only_numbers_and_colours() {
        let e = read(&sample(Format::Vp3), "x.vp3").unwrap();
        let inside = e
            .svg
            .replace(|c: char| c.is_ascii_alphanumeric() || " #=\"-/<>._'".contains(c), "");
        assert_eq!(inside, "", "nothing from the file but numbers: {}", e.svg);
        assert!(!e.svg.contains("Red"), "thread names stay out of the SVG");
    }

    #[test]
    fn what_isnt_a_design_is_refused_with_a_reason() {
        assert!(read(b"", "a.pes").unwrap_err().contains("empty"));
        assert!(
            read(b"<svg onload=alert(1)>", "a.pes")
                .unwrap_err()
                .contains("isn't an embroidery file")
        );
        assert!(
            read(&vec![0u8; MAX_FILE_BYTES + 1], "a.dst")
                .unwrap_err()
                .contains("larger than")
        );
        let mut cut = sample(Format::Pes);
        cut.truncate(cut.len() / 3);
        let _ = read(&cut, "a.pes"); // a cut-short file: an answer either way, never a panic
    }

    #[test]
    fn random_bytes_never_panic() {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let valid = sample(Format::Pes);
        for i in 0..2000 {
            let mut bytes = if i % 2 == 0 {
                valid.clone()
            } else {
                (0..(next() % 2048)).map(|_| next() as u8).collect()
            };
            for _ in 0..(next() % 8) {
                if !bytes.is_empty() {
                    let at = (next() as usize) % bytes.len();
                    bytes[at] = next() as u8;
                }
            }
            let _ = read(&bytes, ["a.pes", "a.dst", "a.jef", "a"][i % 4]);
        }
    }
}
