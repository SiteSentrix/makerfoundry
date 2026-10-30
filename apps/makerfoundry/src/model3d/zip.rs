//! A small ZIP reader and writer (MF-38) for 3MF files and bulk imports, from the format's public
//! specification (PKWARE's APPNOTE.TXT,
//! <https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT>). Our own, so every limit
//! against zip bombs is ours and checked before anything is inflated: entries are read in memory,
//! one at a time when asked for, and never written to disk by their own names.
//!
//! Only what 3MF writers use: stored (0) and deflated (8) entries, no encryption, no ZIP64, no
//! spanning. The central directory is what's trusted for sizes; each entry's inflated length and
//! CRC-32 must match it.

use std::collections::HashSet;
use std::io::{Read, Write};

/// The most entries an archive may have.
pub const MAX_ENTRIES: usize = 4096;
/// The most one entry may inflate to.
pub const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
/// The most all entries together may inflate to.
pub const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
/// How many times larger than the archive its entries may be, together.
pub const MAX_RATIO: u64 = 200;
/// The longest entry name, in bytes.
const MAX_NAME: usize = 512;

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;

/// One entry, as the central directory describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub size: u64,
    compressed: u64,
    method: u16,
    crc: u32,
    /// Where its data starts in the archive.
    data: usize,
}

/// An archive checked against every limit, its entries readable one by one.
#[derive(Debug)]
pub struct Archive<'a> {
    bytes: &'a [u8],
    entries: Vec<Entry>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

fn damaged() -> String {
    "the archive is damaged or incomplete".into()
}

/// Whether an entry's name is a plain relative path: no `..`, no leading `/`, no backslashes, no
/// drive letters, no control characters.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.chars().any(char::is_control)
        && !name.split('/').any(|part| part == ".." || part == ".")
        && !name.split('/').next().is_some_and(|first| first.contains(':'))
}

impl<'a> Archive<'a> {
    /// Reads the central directory and checks it: counts, names, sizes, the inflation ratio, and
    /// that no two entries share data.
    pub fn open(bytes: &'a [u8]) -> Result<Self, String> {
        // the end record is in the last 22 + 65,535 (its comment) bytes
        let from = bytes.len().saturating_sub(22 + 0xffff);
        let end = (from..bytes.len().saturating_sub(21))
            .rev()
            .find(|&i| u32_at(bytes, i) == Some(END))
            .ok_or_else(|| "the file isn't a ZIP archive (or is incomplete)".to_string())?;
        let count = usize::from(u16_at(bytes, end + 10).ok_or_else(damaged)?);
        let dir_size = u32_at(bytes, end + 12).ok_or_else(damaged)?;
        let dir_at = u32_at(bytes, end + 16).ok_or_else(damaged)?;
        if dir_size == u32::MAX || dir_at == u32::MAX || count == 0xffff {
            return Err("ZIP64 archives aren't taken".into());
        }
        if count > MAX_ENTRIES {
            return Err(format!("the archive has more than {MAX_ENTRIES} files"));
        }
        let mut at = dir_at as usize;
        let mut entries = Vec::with_capacity(count);
        let mut names = HashSet::with_capacity(count);
        let mut total: u64 = 0;
        for _ in 0..count {
            if u32_at(bytes, at) != Some(CENTRAL) {
                return Err(damaged());
            }
            let field = |o: usize| u16_at(bytes, at + o).ok_or_else(damaged);
            let flags = field(8)?;
            let method = field(10)?;
            let crc = u32_at(bytes, at + 16).ok_or_else(damaged)?;
            let compressed = u64::from(u32_at(bytes, at + 20).ok_or_else(damaged)?);
            let size = u64::from(u32_at(bytes, at + 24).ok_or_else(damaged)?);
            let name_len = usize::from(field(28)?);
            let extra_len = usize::from(field(30)?);
            let comment_len = usize::from(field(32)?);
            let local = u32_at(bytes, at + 42).ok_or_else(damaged)? as usize;
            let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(damaged)?;
            let name = std::str::from_utf8(name)
                .map_err(|_| "a file in the archive has a name that isn't text".to_string())?
                .to_string();
            at += 46 + name_len + extra_len + comment_len;
            if flags & 1 != 0 {
                return Err("the archive is encrypted".into());
            }
            if method != 0 && method != 8 {
                return Err("the archive uses a compression MakerFoundry doesn't read".into());
            }
            if !safe_name(&name) {
                return Err("a file in the archive has a name with a path outside it".into());
            }
            if !names.insert(name.to_lowercase()) {
                return Err("the archive has two files with the same name".into());
            }
            if size > MAX_ENTRY_BYTES {
                return Err(format!(
                    "a file in the archive is larger than {} MB",
                    MAX_ENTRY_BYTES >> 20
                ));
            }
            total = total.saturating_add(size);
            if total > MAX_TOTAL_BYTES {
                return Err(format!("the archive unpacks to more than {} MB", MAX_TOTAL_BYTES >> 20));
            }
            // the entry's own header gives where its data starts
            if u32_at(bytes, local) != Some(LOCAL) {
                return Err(damaged());
            }
            let local_name = usize::from(u16_at(bytes, local + 26).ok_or_else(damaged)?);
            let local_extra = usize::from(u16_at(bytes, local + 28).ok_or_else(damaged)?);
            let data = local + 30 + local_name + local_extra;
            if (data as u64).saturating_add(compressed) > bytes.len() as u64 {
                return Err(damaged());
            }
            entries.push(Entry {
                name,
                size,
                compressed,
                method,
                crc,
                data,
            });
        }
        if total > (bytes.len() as u64).saturating_mul(MAX_RATIO) {
            return Err("the archive unpacks to far more than its size (a zip bomb?)".into());
        }
        // no two entries may share bytes (one way to make a bomb from a small file)
        let mut spans: Vec<(usize, u64)> = entries.iter().map(|e| (e.data, e.compressed)).collect();
        spans.sort_unstable();
        if spans.windows(2).any(|w| match w {
            [(a, len), (b, _)] => (*a as u64).saturating_add(*len) > *b as u64,
            _ => false,
        }) {
            return Err("the archive's files overlap (a zip bomb?)".into());
        }
        Ok(Archive { bytes, entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The entry with this name (ignoring case, as 3MF part names do).
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Inflates an entry: never more than the size the directory gives, which it must match, with
    /// a matching CRC-32.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>, String> {
        let end = entry.data.saturating_add(entry.compressed as usize);
        let raw = self.bytes.get(entry.data..end).ok_or_else(damaged)?;
        let out = match entry.method {
            0 => raw.to_vec(),
            _ => {
                let mut out = Vec::with_capacity(entry.size.min(64 * 1024 * 1024) as usize);
                flate2::read::DeflateDecoder::new(raw)
                    .take(entry.size.saturating_add(1))
                    .read_to_end(&mut out)
                    .map_err(|_| damaged())?;
                out
            }
        };
        if out.len() as u64 != entry.size || crc32fast::hash(&out) != entry.crc {
            return Err(format!("{} in the archive is damaged", entry.name));
        }
        Ok(out)
    }
}

/// Writes an archive of `files` (name, contents), deflated.
pub fn write(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    let too_big = || "the archive would be too large".to_string();
    for (name, data) in files {
        let mut deflate = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        deflate.write_all(data).map_err(|_| too_big())?;
        let packed = deflate.finish().map_err(|_| too_big())?;
        let (method, packed): (u16, &[u8]) = if packed.len() < data.len() {
            (8, &packed)
        } else {
            (0, data)
        };
        let crc = crc32fast::hash(data);
        let offset = u32::try_from(out.len()).map_err(|_| too_big())?;
        let packed_len = u32::try_from(packed.len()).map_err(|_| too_big())?;
        let size = u32::try_from(data.len()).map_err(|_| too_big())?;
        let name_len = u16::try_from(name.len()).map_err(|_| too_big())?;
        // version 2.0, UTF-8 names (bit 11), no time (1980-01-01)
        let common = |b: &mut Vec<u8>| {
            b.extend(20u16.to_le_bytes());
            b.extend(0x0800u16.to_le_bytes());
            b.extend(method.to_le_bytes());
            b.extend(0u16.to_le_bytes());
            b.extend(0x0021u16.to_le_bytes());
            b.extend(crc.to_le_bytes());
            b.extend(packed_len.to_le_bytes());
            b.extend(size.to_le_bytes());
            b.extend(name_len.to_le_bytes());
            b.extend(0u16.to_le_bytes());
        };
        out.extend(LOCAL.to_le_bytes());
        common(&mut out);
        out.extend(name.as_bytes());
        out.extend(packed);
        directory.extend(CENTRAL.to_le_bytes());
        directory.extend(20u16.to_le_bytes());
        common(&mut directory);
        directory.extend([0u8; 6]); // comment length, disk, internal attributes
        directory.extend(0u32.to_le_bytes()); // external attributes
        directory.extend(offset.to_le_bytes());
        directory.extend(name.as_bytes());
    }
    let count = u16::try_from(files.len()).map_err(|_| too_big())?;
    let dir_at = u32::try_from(out.len()).map_err(|_| too_big())?;
    let dir_size = u32::try_from(directory.len()).map_err(|_| too_big())?;
    out.extend(directory);
    out.extend(END.to_le_bytes());
    out.extend([0u8; 4]); // disk numbers
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(dir_size.to_le_bytes());
    out.extend(dir_at.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[(&str, &[u8])]) -> Vec<(String, Vec<u8>)> {
        list.iter().map(|(n, d)| (n.to_string(), d.to_vec())).collect()
    }

    #[test]
    fn what_is_written_reads_back() {
        // model-like text: numbers that repeat in places, so it packs a few times smaller
        let big: Vec<u8> = (0..20_000u32)
            .flat_map(|i| format!("<vertex x=\"{}\" y=\"{}\" z=\"0\"/>", i % 97, (i * 7) % 89).into_bytes())
            .collect();
        let zip = write(&files(&[
            ("3D/3dmodel.model", &big),
            ("tiny.txt", b"x"),
            ("empty", b""),
        ]))
        .unwrap();
        assert!(zip.len() < big.len() / 3, "deflated");
        let a = Archive::open(&zip).unwrap();
        assert_eq!(a.entries().len(), 3);
        assert_eq!(a.read(a.find("3d/3DMODEL.model").unwrap()).unwrap(), big);
        assert_eq!(a.read(a.find("tiny.txt").unwrap()).unwrap(), b"x");
        assert_eq!(a.read(a.find("empty").unwrap()).unwrap(), b"");
    }

    #[test]
    fn a_path_traversal_name_is_refused() {
        for bad in [
            "../evil",
            "a/../../b",
            "/etc/passwd",
            "a\\b",
            "C:/x",
            "a/./b",
            "",
            "a\u{0}b",
        ] {
            assert!(!safe_name(bad), "{bad:?}");
            let zip = write(&files(&[(bad, b"x")])).unwrap();
            assert!(Archive::open(&zip).is_err(), "{bad:?}");
        }
        assert!(safe_name("Metadata/plate_1.png"));
        assert!(safe_name("3D/Objects/object 1.model"));
    }

    #[test]
    fn too_many_entries_are_refused() {
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("f{i}")).collect();
        let at_limit: Vec<_> = names[..MAX_ENTRIES].iter().map(|n| (n.clone(), Vec::new())).collect();
        assert!(Archive::open(&write(&at_limit).unwrap()).is_ok());
        let over: Vec<_> = names.iter().map(|n| (n.clone(), Vec::new())).collect();
        assert!(Archive::open(&write(&over).unwrap()).unwrap_err().contains("more than"));
    }

    /// Sets an entry's sizes in the central directory (the first entry's).
    fn claim(zip: &mut [u8], size: u32) {
        let dir = (0..zip.len()).find(|&i| u32_at(zip, i) == Some(CENTRAL)).unwrap();
        zip[dir + 24..dir + 28].copy_from_slice(&size.to_le_bytes());
    }

    #[test]
    fn a_zip_bomb_is_refused() {
        // a megabyte of zeros packs to about a kilobyte: over 200 times
        let zeros = vec![0u8; 1024 * 1024];
        let zip = write(&files(&[("3D/3dmodel.model", &zeros)])).unwrap();
        assert!(Archive::open(&zip).unwrap_err().contains("zip bomb"));
        // one entry claiming more than 512 MB, or many adding up to more than 1 GB
        let mut zip = write(&files(&[("a", b"x")])).unwrap();
        claim(&mut zip, 513 * 1024 * 1024);
        assert!(Archive::open(&zip).unwrap_err().contains("larger than"));
        let many: Vec<_> = (0..3).map(|i| (format!("f{i}"), b"x".to_vec())).collect();
        let mut zip = write(&many).unwrap();
        let mut at = 0;
        while let Some(dir) = (at..zip.len()).find(|&i| u32_at(&zip, i) == Some(CENTRAL)) {
            zip[dir + 24..dir + 28].copy_from_slice(&(400u32 * 1024 * 1024).to_le_bytes());
            at = dir + 4;
        }
        assert!(Archive::open(&zip).unwrap_err().contains("unpacks to more than"));
    }

    #[test]
    fn an_entry_bigger_than_it_claims_is_refused_without_inflating_it_all() {
        let data = vec![7u8; 50_000];
        let mut zip = write(&files(&[("a", &data)])).unwrap();
        claim(&mut zip, 100);
        let a = Archive::open(&zip).unwrap();
        assert!(a.read(&a.entries()[0]).is_err());
    }

    #[test]
    fn a_damaged_entry_is_refused() {
        let mut zip = write(&files(&[("a.txt", b"hello hello hello hello hello hello")])).unwrap();
        // flip a byte of the packed data
        zip[30 + 5 + 2] ^= 0xff;
        let a = Archive::open(&zip).unwrap();
        assert!(a.read(&a.entries()[0]).is_err());
    }

    #[test]
    fn a_truncated_zip_is_refused() {
        let zip = write(&files(&[("a", b"hello"), ("b", b"world")])).unwrap();
        for cut in [0, 10, zip.len() / 2, zip.len() - 1] {
            assert!(Archive::open(&zip[..cut]).is_err(), "cut at {cut}");
        }
        assert!(Archive::open(b"not a zip at all").is_err());
    }

    #[test]
    fn duplicate_and_overlapping_entries_are_refused() {
        let zip = write(&files(&[("a", b"1"), ("A", b"2")])).unwrap();
        assert!(Archive::open(&zip).unwrap_err().contains("same name"));
        // two directory records pointing at the same data
        let mut zip = write(&files(&[("a", b"hello"), ("b", b"hello")])).unwrap();
        let first = (0..zip.len()).find(|&i| u32_at(&zip, i) == Some(CENTRAL)).unwrap();
        let second = (first + 4..zip.len())
            .find(|&i| u32_at(&zip, i) == Some(CENTRAL))
            .unwrap();
        zip[second + 42..second + 46].copy_from_slice(&0u32.to_le_bytes());
        assert!(Archive::open(&zip).is_err());
    }

    #[test]
    fn encrypted_and_unknown_compression_are_refused() {
        let mut zip = write(&files(&[("a", b"x")])).unwrap();
        let dir = (0..zip.len()).find(|&i| u32_at(&zip, i) == Some(CENTRAL)).unwrap();
        zip[dir + 8] |= 1;
        assert!(Archive::open(&zip).unwrap_err().contains("encrypted"));
        let mut zip = write(&files(&[("a", b"x")])).unwrap();
        zip[dir + 10] = 14; // LZMA
        assert!(Archive::open(&zip).is_err());
    }
}
