//! The shared page layout and HTML helpers.

/// The site's mark, drawn in code: an embroidery hoop crossed by a run of stitches. Also served as
/// `/favicon.svg`.
pub const MARK_SVG: &str = r#"<svg class="mark" viewBox="0 0 32 32" width="32" height="32" aria-hidden="true">
<circle cx="16" cy="16" r="13" fill="none" stroke="currentColor" stroke-width="3"/>
<circle cx="16" cy="16" r="8.5" fill="none" stroke="currentColor" stroke-width="1.5"/>
<path d="M6 22 L26 10" stroke="currentColor" stroke-width="2" stroke-dasharray="3 2.5" stroke-linecap="round"/>
</svg>"#;

/// `text` made safe to put in HTML, in an element or a quoted attribute.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Wraps a page's body in the shared layout. `title` and `body` are HTML: anything a person typed
/// goes through [`escape`] before it gets here. `signed_in` says whether the header offers the
/// account or a way to sign in.
pub fn page(title: &str, body: &str, signed_in: bool) -> String {
    page_with(title, body, signed_in, None)
}

/// What a link to a public page shows where it's shared (a chat, a social site): the standard
/// Open Graph tags. Plain text; escaped here.
pub struct Preview<'a> {
    pub title: &'a str,
    pub description: &'a str,
    /// The page's address, in full.
    pub url: &'a str,
    /// A picture's address, in full.
    pub image: Option<&'a str>,
}

/// `text` on one line, at most `max` characters.
pub fn summary(text: &str, max: usize) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        one
    } else {
        format!(
            "{}…",
            one.chars().take(max.saturating_sub(1)).collect::<String>().trim_end()
        )
    }
}

/// [`page`], with a shared link's preview.
pub fn page_with(title: &str, body: &str, signed_in: bool, preview: Option<&Preview>) -> String {
    let head = match preview {
        None => r#"<meta name="description" content="Machine-embroidery designs and laser-cut files, shared and sold by makers.">"#.to_string(),
        Some(p) => {
            let image = match p.image {
                Some(i) => format!(
                    r#"<meta property="og:image" content="{}"><meta name="twitter:card" content="summary_large_image">"#,
                    escape(i)
                ),
                None => r#"<meta name="twitter:card" content="summary">"#.to_string(),
            };
            format!(
                r#"<meta name="description" content="{d}">
<link rel="canonical" href="{u}">
<meta property="og:site_name" content="MakerFoundry"><meta property="og:type" content="website">
<meta property="og:title" content="{t}"><meta property="og:description" content="{d}"><meta property="og:url" content="{u}">{image}"#,
                t = escape(p.title),
                d = escape(p.description),
                u = escape(p.url),
            )
        }
    };
    let nav = if signed_in {
        r#"<a href="/browse">Browse</a><a href="/account">Account</a>"#
    } else {
        r#"<a href="/browse">Browse</a><a href="/signin">Sign in</a>"#
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
{head}
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="/static/site.css">
</head>
<body>
<header class="top"><a class="brand" href="/">{MARK_SVG}<span>MakerFoundry</span></a><nav>{nav}</nav></header>
<main>
{body}
</main>
<footer class="bottom"><p>MakerFoundry is made by <a href="https://sitesentrix.com">SiteSentrix</a>,
the makers of <a href="https://stitchfoundry.app">StitchFoundry</a>.</p>
<p class="links"><a href="/terms">Terms</a> <a href="/privacy">Privacy</a> <a href="/seller-terms">Seller Terms</a> <a href="/dmca">Copyright</a></p></footer>
</body>
</html>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_makes_text_safe_in_elements_and_attributes() {
        assert_eq!(
            escape(r#"<a href="x">Tom & Jerry's</a>"#),
            "&lt;a href=&quot;x&quot;&gt;Tom &amp; Jerry&#39;s&lt;/a&gt;"
        );
        assert_eq!(escape("plain ✓"), "plain ✓");
    }
}
