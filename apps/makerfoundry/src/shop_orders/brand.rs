//! A shop's branding (2026-11-02, the owner's decision): the optional `shop.brand` of its
//! catalogue: a display name, intro text, a logo (through the hub's files), one accent colour, a
//! preferred short address (`<handle>.makerfoundry.app`) and a website link. It never refuses a
//! catalogue: a field that's missing, too long or not what it should be is simply left out, and the
//! shop's page falls back to its plain look.
//!
//! The accent colour is used only where it can be read: as the colour of links and buttons on the
//! shop's page when it has at least 4.5:1 contrast with the page (WCAG 2.2, "contrast minimum",
//! https://www.w3.org/TR/WCAG22/#contrast-minimum), worked out separately for the light and dark
//! looks; otherwise MakerFoundry's own colour stays.

use serde_json::Value;

use super::contract::Image;

/// The longest display name, intro and website address taken.
const NAME_CHARS: usize = 80;
const INTRO_CHARS: usize = 1000;
const WEBSITE_CHARS: usize = 200;
/// The largest logo taken (as the contract's pictures).
pub const MAX_LOGO_BYTES: usize = 2 * 1024 * 1024;

/// Short addresses no shop may take: MakerFoundry's own names, mail and web conventions, and the
/// family's products.
pub const RESERVED: &[&str] = &[
    "about",
    "account",
    "accounts",
    "admin",
    "api",
    "app",
    "apps",
    "assets",
    "auth",
    "autoconfig",
    "autodiscover",
    "beta",
    "billing",
    "blog",
    "browse",
    "buildkiosk",
    "cdn",
    "checkout",
    "cloud",
    "contact",
    "dev",
    "dmca",
    "docs",
    "email",
    "files",
    "foundry",
    "ftp",
    "help",
    "hub",
    "imap",
    "img",
    "images",
    "laserfoundry",
    "legal",
    "login",
    "mail",
    "makerfoundry",
    "media",
    "mx",
    "news",
    "ns1",
    "ns2",
    "oauth",
    "patternfoundry",
    "pay",
    "payments",
    "pop",
    "privacy",
    "screenfoundry",
    "secure",
    "shop",
    "shops",
    "signin",
    "signup",
    "sitesentrix",
    "smtp",
    "staging",
    "static",
    "status",
    "stitchbench",
    "stitchfoundry",
    "stitchhub",
    "store",
    "stripe",
    "support",
    "terms",
    "test",
    "webmail",
    "www",
];

/// A shop's branding, as far as it reads.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Brand {
    pub display_name: Option<String>,
    pub intro: Option<String>,
    pub logo: Option<Image>,
    pub accent: Option<[u8; 3]>,
    pub handle: Option<String>,
    pub website: Option<String>,
}

/// Plain text of at most `max` characters, with no control characters but new lines (when
/// `lines`), trimmed; `None` when empty or too long.
fn plain(v: Option<&Value>, max: usize, lines: bool) -> Option<String> {
    let s = v?.as_str()?.trim();
    if s.is_empty() || s.chars().count() > max {
        return None;
    }
    if s.chars().any(|c| c.is_control() && !(lines && c == '\n')) {
        return None;
    }
    Some(s.to_string())
}

/// Whether `h` can be a shop's short address: 3 to 40 of a-z, 0-9 and `-`, not starting or ending
/// with `-`, and not one of the reserved names.
pub fn valid_handle(h: &str) -> bool {
    (3..=40).contains(&h.len())
        && h.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !h.starts_with('-')
        && !h.ends_with('-')
        && !RESERVED.contains(&h)
}

fn colour(v: Option<&Value>) -> Option<[u8; 3]> {
    let hex = v?.as_str()?.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn website(v: Option<&Value>) -> Option<String> {
    let s = plain(v, WEBSITE_CHARS, false)?;
    let url = reqwest::Url::parse(&s).ok()?;
    let host = url.host_str()?;
    (url.scheme() == "https" && url.username().is_empty() && url.password().is_none() && host.contains('.'))
        .then(|| url.to_string())
}

fn logo(v: Option<&Value>) -> Option<Image> {
    let image: Image = serde_json::from_value(v?.clone()).ok()?;
    let sha_ok = image.sha256.len() == 64 && image.sha256.bytes().all(|b| b.is_ascii_hexdigit());
    (sha_ok
        && image.size > 0
        && image.size <= MAX_LOGO_BYTES as u64
        && matches!(image.kind.as_str(), "image/png" | "image/jpeg"))
    .then_some(image)
}

/// The branding in a catalogue's `shop.brand`, leaving out whatever doesn't read.
pub fn read(v: Option<&Value>) -> Brand {
    let Some(b) = v.filter(|b| b.is_object()) else {
        return Brand::default();
    };
    Brand {
        display_name: plain(b.get("display_name"), NAME_CHARS, false),
        intro: plain(b.get("intro"), INTRO_CHARS, true),
        logo: logo(b.get("logo")),
        accent: colour(b.get("accent")),
        handle: b
            .get("handle")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|h| valid_handle(h))
            .map(str::to_string),
        website: website(b.get("website")),
    }
}

/// The relative luminance of a colour (WCAG 2.2's definition).
fn luminance([r, g, b]: [u8; 3]) -> f64 {
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// The contrast ratio of two colours, 1 to 21.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The site's page colours (`static/site.css`) and the two text colours a button may take.
const LIGHT_PAGE: [u8; 3] = [0xfb, 0xfa, 0xf7];
const DARK_PAGE: [u8; 3] = [0x16, 0x18, 0x1c];
const WHITE: [u8; 3] = [0xff, 0xff, 0xff];
const INK: [u8; 3] = [0x1f, 0x23, 0x28];

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The accent and its button text for a page of `page`, if both read at 4.5:1.
fn accent_on(accent: [u8; 3], page: [u8; 3]) -> Option<(String, String)> {
    if contrast(accent, page) < 4.5 {
        return None;
    }
    let text = if contrast(accent, WHITE) >= contrast(accent, INK) {
        WHITE
    } else {
        INK
    };
    (contrast(accent, text) >= 4.5).then(|| (hex(accent), hex(text)))
}

/// The shop page's stylesheet: its accent for the light and dark looks, each only where it reads.
pub fn css(brand: &Brand) -> String {
    let Some(accent) = brand.accent else {
        return String::new();
    };
    // (each look set only on its own, so where the accent doesn't read MakerFoundry's own stays)
    let mut out = String::new();
    if let Some((a, t)) = accent_on(accent, LIGHT_PAGE) {
        out.push_str(&format!(
            "@media not all and (prefers-color-scheme: dark) {{ .shop-brand {{ --accent: {a}; --on-accent: {t}; }} }}\n"
        ));
    }
    if let Some((a, t)) = accent_on(accent, DARK_PAGE) {
        out.push_str(&format!(
            "@media (prefers-color-scheme: dark) {{ .shop-brand {{ --accent: {a}; --on-accent: {t}; }} }}\n"
        ));
    }
    out
}

/// The intro as paragraphs (split at blank lines), escaped.
pub fn intro_html(intro: &str) -> String {
    intro
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("<p>{}</p>", crate::pages::escape(p).replace('\n', "<br>")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn branding_reads_what_it_can_and_leaves_out_the_rest() {
        let b = read(Some(&json!({
            "display_name": "Main Street Threads",
            "intro": "Family shop since 1998.\n\nWe stitch it here.",
            "logo": {"size": 1200, "sha256": "a".repeat(64), "type": "image/png"},
            "accent": "#1F5FAD",
            "handle": "mainst-threads",
            "website": "https://mainst.example/"
        })));
        assert_eq!(b.display_name.as_deref(), Some("Main Street Threads"));
        assert_eq!(b.accent, Some([0x1f, 0x5f, 0xad]));
        assert_eq!(b.handle.as_deref(), Some("mainst-threads"));
        assert_eq!(b.website.as_deref(), Some("https://mainst.example/"));
        assert!(b.logo.is_some() && b.intro.is_some());
        // none of these read, and none refuses anything
        let bad = read(Some(&json!({
            "display_name": "x".repeat(81),
            "intro": "bell\u{7}",
            "logo": {"size": 3_000_000, "sha256": "a".repeat(64), "type": "image/png"},
            "accent": "blue",
            "handle": "www",
            "website": "http://plain.example"
        })));
        assert_eq!(bad, Brand::default());
        assert_eq!(read(Some(&json!("not an object"))), Brand::default());
        assert_eq!(read(None), Brand::default());
        for (website, ok) in [
            ("https://user:pw@x.example", false),
            ("javascript:alert(1)", false),
            ("https://localhost", false),
            ("https://x.example/path?q=1", true),
        ] {
            assert_eq!(
                read(Some(&json!({"website": website}))).website.is_some(),
                ok,
                "{website}"
            );
        }
        for (handle, ok) in [
            ("ab", false),
            ("-ab", false),
            ("ab-", false),
            ("Main", false),
            ("a_b", false),
            ("abc", true),
            ("shops", false),
        ] {
            assert_eq!(valid_handle(handle), ok, "{handle}");
        }
    }

    #[test]
    fn the_accent_is_used_only_where_it_reads() {
        // WCAG's own examples: black on white is 21:1
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 0.01);
        // a dark blue reads on the light page, not the dark one
        let out = super::css(&Brand {
            accent: Some([0x1f, 0x5f, 0xad]),
            ..Brand::default()
        });
        assert!(
            out.starts_with("@media not all and (prefers-color-scheme: dark) { .shop-brand { --accent: #1f5fad; --on-accent: #ffffff; } }"),
            "{out}"
        );
        assert!(!out.contains("@media (prefers-color-scheme: dark)"), "{out}");
        // a pale yellow reads only on the dark page, with dark button text
        let out = super::css(&Brand {
            accent: Some([0xf5, 0xd0, 0x42]),
            ..Brand::default()
        });
        assert!(!out.contains("not all and"), "{out}");
        assert!(out.contains("--accent: #f5d042; --on-accent: #1f2328;"), "{out}");
        // no accent, nothing
        assert!(super::css(&Brand::default()).is_empty());
    }

    #[test]
    fn the_intro_is_escaped_paragraphs() {
        assert_eq!(
            intro_html("<b>Hi</b>\nthere\n\nSecond"),
            "<p>&lt;b&gt;Hi&lt;/b&gt;<br>there</p><p>Second</p>"
        );
    }
}
