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

/// How files that are certainly not embroidery begin (#86): programs (Windows, Linux, macOS, Java,
/// WebAssembly), scripts, archives, installers and documents. Formats without a header, such as
/// EXP, would otherwise read any of them as stitches.
const NOT_EMBROIDERY: [&[u8]; 18] = [
    // a Windows program or library
    b"MZ",
    // a Linux program
    b"\x7fELF",
    // a macOS program, or Java classes
    b"\xca\xfe\xba\xbe",
    // a macOS program (64-bit)
    b"\xcf\xfa\xed\xfe",
    // a macOS program (32-bit)
    b"\xce\xfa\xed\xfe",
    // a macOS program (big-endian)
    b"\xfe\xed\xfa",
    // WebAssembly
    b"\x00asm",
    // a script
    b"#!",
    // a ZIP (and Office files, JARs, APKs)
    b"PK",
    // a RAR archive
    b"Rar!",
    // a 7-Zip archive
    b"7z\xbc\xaf",
    // gzip
    b"\x1f\x8b",
    // bzip2
    b"BZh",
    // xz
    b"\xfd7zXZ",
    // a Windows cabinet
    b"MSCF",
    // an old Office document or an MSI installer
    b"\xd0\xcf\x11\xe0",
    // a PDF
    b"%PDF",
    // rich text
    b"{\\rtf",
];

/// How text files that are certainly not embroidery begin, after any byte-order mark and spaces:
/// web pages, XML (SVG among them), scripts, PHP (compared in lower case).
const NOT_EMBROIDERY_TEXT: [&[u8]; 6] = [b"<!", b"<?", b"<html", b"<svg", b"<script", b"#!"];

/// Whether `bytes` begin as a file that is certainly not embroidery (#86).
pub fn certainly_not_embroidery(bytes: &[u8]) -> bool {
    if NOT_EMBROIDERY.iter().any(|magic| bytes.starts_with(magic)) {
        return true;
    }
    let text = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let start = text.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(text.len());
    let head: Vec<u8> = text
        .get(start..)
        .unwrap_or_default()
        .iter()
        .take(8)
        .map(u8::to_ascii_lowercase)
        .collect();
    NOT_EMBROIDERY_TEXT.iter().any(|magic| head.starts_with(magic))
}

/// For a format without a header (EXP, TAP), whether what was read looks like a design (#86)
/// rather than some other file read as stitches. Real designs sew mostly short stitches (fills and
/// satins of a few millimetres) and few stitches that don't move; bytes that aren't a design read
/// as stitches of every length up to about 12 mm each way, or (programs, padding) as runs of zeros.
fn plausible_headerless(bytes: &[u8], record: usize, pattern: &Pattern) -> Result<(), String> {
    // whole records: EXP's are two bytes (and commands four), TAP's three
    if !bytes.len().is_multiple_of(record) {
        return Err(NOT_A_DESIGN.into());
    }
    let mut lengths: Vec<f64> = Vec::new();
    let mut last: Option<(f64, f64)> = None;
    for s in &pattern.stitches {
        match s.cmd {
            Cmd::Stitch => {
                if let Some((x, y)) = last {
                    lengths.push((s.x - x).hypot(s.y - y));
                }
                last = Some((s.x, s.y));
            }
            Cmd::Jump | Cmd::Trim | Cmd::ColorChange | Cmd::Stop | Cmd::End => last = None,
        }
    }
    if lengths.is_empty() {
        return Err(NOT_A_DESIGN.into());
    }
    let still = lengths.iter().filter(|l| **l < 0.05).count();
    if still * 2 > lengths.len() {
        return Err(NOT_A_DESIGN.into());
    }
    lengths.sort_by(f64::total_cmp);
    let median = lengths.get(lengths.len() / 2).copied().unwrap_or(f64::INFINITY);
    if !median.is_finite() || median > MAX_MEDIAN_STITCH_MM {
        return Err(NOT_A_DESIGN.into());
    }
    Ok(())
}

/// The longest a headerless design's middle stitch may be (mm): real fills and satins are a few
/// millimetres; random bytes read as stitches of about 10 mm.
const MAX_MEDIAN_STITCH_MM: f64 = 8.5;

const NOT_A_DESIGN: &str = "this doesn't look like an embroidery design: its stitches don't read as one";

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
    let unreadable = "this isn't an embroidery file MakerFoundry can read (DST, PES, PEC, JEF, EXP, VP3, XXX, U01, PCS, TAP, DSB or DSZ)";
    // a program, archive or web page is refused before any reader sees it (#86)
    if certainly_not_embroidery(bytes) {
        return Err(unreadable.into());
    }
    let hint = file_name.rsplit_once('.').map(|(_, ext)| ext);
    let (format, pattern) = sf_formats::read_any(bytes, hint).map_err(|_| unreadable.to_string())?;
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
    match format {
        sf_formats::Format::Exp => plausible_headerless(bytes, 2, &pattern)?,
        sf_formats::Format::Tap => plausible_headerless(bytes, 3, &pattern)?,
        _ => {}
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

    /// A Windows program: `MZ`, then filler (as the staging check of #86 sends).
    fn program() -> Vec<u8> {
        let mut bytes = b"MZ".to_vec();
        bytes.extend(std::iter::repeat_n(0x90u8, 4094));
        bytes
    }

    #[test]
    fn programs_archives_and_pages_are_never_designs_whatever_their_name() {
        let with = |start: &[u8], fill: u8| {
            let mut bytes = start.to_vec();
            bytes.extend(std::iter::repeat_n(fill, 1000));
            bytes
        };
        let files = [
            program(),
            with(b"\x7fELF", 7),
            with(b"PK\x03\x04", 3),
            with(b"%PDF-1.7\n", b'1'),
            with(b"#!/bin/sh\n", b'e'),
            with(b"  <!DOCTYPE html><html>", b'x'),
            with(b"\xef\xbb\xbf<SVG>", b'x'),
        ];
        for bytes in files {
            for name in ["setup.exp", "setup.tap", "setup.dst", "setup.pes", "setup.exe"] {
                let r = read(&bytes, name);
                assert!(
                    r.as_ref().is_err_and(|e| e.contains("isn't an embroidery file")),
                    "{name} {:?}: {:?}",
                    bytes.get(..4),
                    r.map(|e| e.format)
                );
            }
        }
    }

    #[test]
    fn a_real_exp_reads_and_other_bytes_named_exp_do_not() {
        let e = read(&sample(Format::Exp), "rose.exp").unwrap();
        assert_eq!(e.format, "EXP");
        assert!(e.stitches >= 40, "{}", e.stitches);
        // one byte too many: not whole records
        let mut odd = sample(Format::Exp);
        odd.push(1);
        assert!(!matches!(read(&odd, "rose.exp"), Ok(e) if e.format == "EXP"));
        // a run of zeros (padding in a program) reads as stitches that never move
        assert!(read(&[0u8; 4096], "zeros.exp").is_err());
        // random bytes read as stitches of every length
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let random: Vec<u8> = (0..4000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        let r = read(&random, "noise.exp");
        assert!(r.is_err(), "{:?}", r.map(|e| (e.format, e.stitches)));
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
