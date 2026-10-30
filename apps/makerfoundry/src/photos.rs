//! Photos of a design's stitched or cut result (MF-2). An uploaded photo is untrusted: it is
//! decoded (JPEG, PNG or WebP only, under size and memory limits), turned upright from its
//! orientation tag, and saved again as a JPEG of our own, so nothing of the upload's bytes is
//! kept or served: not its metadata (phones write where a photo was taken), not anything hidden
//! after the picture, and not a file that only pretends to be a picture.

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

/// The largest photo file accepted.
pub const MAX_FILE_BYTES: usize = 15 * 1024 * 1024;
/// The largest picture decoded (pixels a side), and the memory a decoder may use.
const MAX_SIDE: u32 = 12_000;
const MAX_ALLOC: u64 = 400 * 1024 * 1024;
/// The long side of the photo kept, and of its thumbnail.
pub const PHOTO_SIDE: u32 = 2000;
pub const THUMB_SIDE: u32 = 480;
const QUALITY: u8 = 85;

/// A photo, made clean: the picture at most `PHOTO_SIDE` a side and a thumbnail, both JPEG.
#[derive(Debug)]
pub struct Photo {
    pub jpeg: Vec<u8>,
    pub thumb: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Reads an uploaded photo and makes it clean.
pub fn read(bytes: &[u8]) -> Result<Photo, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!(
            "the photo is larger than {} MB",
            MAX_FILE_BYTES / (1024 * 1024)
        ));
    }
    let (picture, _) = decode(bytes)?;
    let picture = DynamicImage::ImageRgb8(picture.into_rgb8());
    let large = shrink(&picture, PHOTO_SIDE);
    let thumb = shrink(&picture, THUMB_SIDE);
    Ok(Photo {
        width: large.width(),
        height: large.height(),
        jpeg: jpeg(&large)?,
        thumb: jpeg(&thumb)?,
    })
}

/// Decodes a JPEG, PNG or WebP picture within the limits, turned upright → it and its format.
fn decode(bytes: &[u8]) -> Result<(DynamicImage, ImageFormat), String> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| not_a_photo())?;
    let format = match reader.format() {
        Some(f @ (ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP)) => f,
        _ => return Err(not_a_photo()),
    };
    let mut reader = reader;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| not_a_photo())?;
    let orientation = decoder.orientation().ok();
    let mut picture = DynamicImage::from_decoder(decoder)
        .map_err(|_| "the photo couldn't be read whole (or is too large)".to_string())?;
    if let Some(o) = orientation {
        picture.apply_orientation(o);
    }
    if picture.width() == 0 || picture.height() == 0 {
        return Err(not_a_photo());
    }
    Ok((picture, format))
}

/// A picture inside a model file (a 3MF's thumbnail, MF-38), decoded within the limits and
/// written again in the same format (PNG or JPEG), at most `PHOTO_SIDE` a side: nothing of the
/// original bytes is kept.
pub fn reencode(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!(
            "the picture is larger than {} MB",
            MAX_FILE_BYTES / (1024 * 1024)
        ));
    }
    let (picture, format) = decode(bytes)?;
    let picture = shrink(&picture, PHOTO_SIDE);
    match format {
        ImageFormat::Png => {
            let mut out = Cursor::new(Vec::new());
            DynamicImage::ImageRgba8(picture.into_rgba8())
                .write_to(&mut out, ImageFormat::Png)
                .map_err(|e| format!("the picture couldn't be saved: {e}"))?;
            Ok(out.into_inner())
        }
        ImageFormat::Jpeg => jpeg(&DynamicImage::ImageRgb8(picture.into_rgb8())),
        _ => Err("only PNG and JPEG pictures are kept in a model file".into()),
    }
}

fn not_a_photo() -> String {
    "this isn't a JPEG, PNG or WebP photo".into()
}

/// `picture` with its long side at most `side`.
fn shrink(picture: &DynamicImage, side: u32) -> DynamicImage {
    if picture.width() <= side && picture.height() <= side {
        picture.clone()
    } else {
        picture.resize(side, side, image::imageops::FilterType::Triangle)
    }
}

fn jpeg(picture: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, QUALITY)
        .encode_image(picture)
        .map_err(|e| format!("the photo couldn't be saved: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn encoded(picture: &DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        picture.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    fn gradient(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        }))
    }

    #[test]
    fn photos_are_shrunk_and_saved_again_as_jpeg() {
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
            let p = read(&encoded(&gradient(3000, 1500), format)).unwrap();
            assert_eq!((p.width, p.height), (2000, 1000), "{format:?}");
            assert!(p.jpeg.starts_with(&[0xff, 0xd8]) && p.thumb.starts_with(&[0xff, 0xd8]));
            let thumb = image::load_from_memory(&p.thumb).unwrap();
            assert_eq!((thumb.width(), thumb.height()), (480, 240));
        }
        let small = read(&encoded(&gradient(300, 200), ImageFormat::Png)).unwrap();
        assert_eq!((small.width, small.height), (300, 200), "never enlarged");
    }

    #[test]
    fn what_rides_along_in_the_upload_is_left_behind() {
        let mut bytes = encoded(&gradient(64, 64), ImageFormat::Jpeg);
        // an EXIF segment with a GPS-ish marker right after the start, and a payload after the end
        let exif = b"\xff\xe1\x00\x18Exif\x00\x00GPSLatitude=41.8";
        bytes.splice(2..2, exif.iter().copied());
        bytes.extend_from_slice(b"<script>alert(1)</script>");
        let p = read(&bytes).unwrap();
        for needle in [&b"GPSLatitude"[..], b"Exif", b"<script>"] {
            assert!(
                !p.jpeg.windows(needle.len()).any(|w| w == needle),
                "{:?} kept",
                String::from_utf8_lossy(needle)
            );
        }
    }

    #[test]
    fn what_isnt_a_photo_is_refused() {
        assert!(read(b"").is_err());
        assert!(read(b"<svg xmlns='http://www.w3.org/2000/svg'/>").is_err());
        assert!(read(b"GIF89a\x01\x00\x01\x00").unwrap_err().contains("isn't a JPEG"));
        assert!(
            read(&vec![0u8; MAX_FILE_BYTES + 1])
                .unwrap_err()
                .contains("larger than")
        );
        // a PNG header claiming 100,000 pixels a side: refused before anything that size is made
        let mut huge = encoded(&gradient(4, 4), ImageFormat::Png);
        huge[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        huge[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(read(&huge).is_err());
    }

    #[test]
    fn damaged_photos_never_panic() {
        let base = encoded(&gradient(40, 30), ImageFormat::Jpeg);
        let png = encoded(&gradient(40, 30), ImageFormat::Png);
        let mut x: u64 = 0xD1B5_4A32_D192_ED03;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for i in 0..600 {
            let mut bytes = if i % 2 == 0 { base.clone() } else { png.clone() };
            for _ in 0..(next() % 6) {
                let at = (next() as usize) % bytes.len();
                bytes[at] = next() as u8;
            }
            bytes.truncate((next() as usize) % (bytes.len() + 1));
            let _ = read(&bytes);
        }
    }
}
