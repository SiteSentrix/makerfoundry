//! Converted downloads (MF-9): an embroidery design in the formats its maker didn't upload, made
//! from the maker's file by StitchFoundry's writers (`sf_formats::write`) when someone downloads it.
//! Its maker can turn this off for a design. Laser files aren't converted.
//!
//! A converted file is the same stitches in another format; formats without thread colours (DST,
//! EXP, U01, TAP, DSB) keep only the colour changes, as files made by any other program do.

use sf_formats::{Format, WriteOptions};

use crate::embroidery::MAX_FILE_BYTES;

/// The formats a design is offered in, converted: every format StitchFoundry writes, less the ones
/// the design has files in (`had`: upper-case extensions, `PES`).
pub fn offered(had: &[&str]) -> Vec<Format> {
    Format::ALL
        .into_iter()
        .filter(|f| f.writable())
        .filter(|f| !had.iter().any(|h| h.eq_ignore_ascii_case(f.extension())))
        .collect()
}

/// A format asked for in an address (`dst`), if it's one that can be written.
pub fn format(ext: &str) -> Option<Format> {
    Format::from_extension(ext).filter(|f| f.writable() && ext.len() <= 4)
}

/// `bytes` (the maker's file, named `file_name`) as a `to` file, labelled `label`.
pub fn convert(bytes: &[u8], file_name: &str, to: Format, label: &str) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("the file is too large to convert".into());
    }
    let hint = file_name.rsplit_once('.').map(|(_, ext)| ext);
    let (_, pattern) = sf_formats::read_any(bytes, hint).map_err(|e| e.to_string())?;
    // labels are short in some formats (DST's is 16 characters): letters and digits are safest
    let label: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ')
        .take(16)
        .collect();
    sf_formats::write(to, &pattern, &WriteOptions::named(label.trim())).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::{Cmd, Pattern, Stitch};

    fn square() -> Vec<u8> {
        let mut p = Pattern::default();
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)] {
            p.stitches.push(Stitch::new(x, y, Cmd::Stitch));
        }
        sf_formats::write(Format::Pes, &p, &WriteOptions::default()).unwrap()
    }

    #[test]
    fn a_pes_is_offered_and_converted_in_every_other_format() {
        let pes = square();
        let offered = offered(&["PES", "dst"]);
        assert!(!offered.contains(&Format::Pes) && !offered.contains(&Format::Dst));
        assert!(offered.contains(&Format::Jef) && !offered.contains(&Format::Dsz));
        for to in Format::ALL {
            let out = convert(&pes, "square.pes", to, "Rose & Fern!").unwrap();
            // reads back as that format, with its stitches
            let back = sf_formats::read(to, &out).unwrap();
            assert!(
                back.stitches.iter().filter(|s| s.cmd == Cmd::Stitch).count() >= 4,
                "{to:?}"
            );
        }
    }

    #[test]
    fn only_writable_formats_are_asked_for() {
        assert_eq!(format("dst"), Some(Format::Dst));
        assert_eq!(format("DST"), Some(Format::Dst));
        assert_eq!(format("dsz"), None);
        assert_eq!(format("svg"), None);
        assert_eq!(format("x.dst"), None);
        assert!(convert(b"not a design at all", "x.pes", Format::Dst, "x").is_err());
    }
}
