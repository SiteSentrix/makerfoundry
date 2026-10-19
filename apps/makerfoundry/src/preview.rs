//! "See your stitches" (MF-2): a signed-in maker uploads an embroidery file and sees it drawn
//! with its stitch count, size and colours. Nothing is kept: the file is read, checked and
//! answered. Listings (MF-3) will use the same reading when a design is uploaded for good.

use axum::extract::{Multipart, State};
use axum::http::HeaderMap;
use axum::response::Response;

use crate::AppState;
use crate::accounts::{current_user, forbidden, private, same_origin};
use crate::embroidery::{self, Embroidery};
use crate::pages::escape;

/// The extensions the file picker offers (any file is still read by its content).
const ACCEPT: &str = ".dst,.pes,.pec,.jef,.exp,.vp3,.xxx,.u01,.pcs,.tap,.dsb,.dsz";

const TITLE: &str = "See your stitches · MakerFoundry";

fn form(problem: &str) -> String {
    let problem = if problem.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem" role="alert">{}</p>"#, escape(problem))
    };
    format!(
        r#"<section class="form">
<h1>See your stitches</h1>
<p class="lead">Upload a machine-embroidery file (DST, PES, JEF, EXP, VP3, XXX and more) to see it drawn stitch by stitch, with its size and colours. The file isn't kept.</p>
{problem}
<form method="post" action="/preview" enctype="multipart/form-data">
<label>Embroidery file <small>(up to {mb} MB)</small><input type="file" name="file" accept="{ACCEPT}" required></label>
<button type="submit">Show the stitches</button>
</form>
</section>"#,
        mb = embroidery::MAX_FILE_BYTES / (1024 * 1024),
    )
}

fn signed_out() -> Response {
    private(
        TITLE,
        r#"<section class="form"><h1>See your stitches</h1><p class="lead">Sign in to upload an embroidery file and see it drawn stitch by stitch.</p><p><a href="/signin">Sign in</a> or <a href="/signup">make an account</a>.</p></section>"#,
        false,
    )
}

pub async fn page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if current_user(&state, &headers).await.is_none() {
        return signed_out();
    }
    private(TITLE, &form(""), true)
}

/// `n` with commas between thousands.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn result(name: &str, e: &Embroidery) -> String {
    let colours: String = e
        .colours
        .iter()
        .map(|c| {
            let name = if c.name.is_empty() { c.hex.clone() } else { format!("{} ({})", c.name, c.hex) };
            format!(
                r#"<li><svg class="swatch" viewBox="0 0 10 10" aria-hidden="true"><circle cx="5" cy="5" r="4.5" fill="{hex}"/></svg>{name} <span class="muted">{n} stitches</span></li>"#,
                hex = escape(&c.hex),
                name = escape(&name),
                n = thousands(c.stitches),
            )
        })
        .collect();
    format!(
        r#"<section class="design">
<h1>{name}</h1>
<figure class="preview">{svg}</figure>
<dl class="facts">
<div><dt>Format</dt><dd>{format}</dd></div>
<div><dt>Size</dt><dd>{w:.1} × {h:.1} mm <span class="muted">({wi:.2} × {hi:.2} in)</span></dd></div>
<div><dt>Stitches</dt><dd>{stitches}</dd></div>
<div><dt>Colour changes</dt><dd>{changes}</dd></div>
<div><dt>Trims</dt><dd>{trims}</dd></div>
</dl>
<h2>Colours</h2>
<ul class="colours">{colours}</ul>
<p><a href="/preview">Try another file</a></p>
</section>"#,
        name = escape(name),
        svg = e.svg,
        format = escape(&e.format),
        w = e.width_mm,
        h = e.height_mm,
        wi = e.width_mm / 25.4,
        hi = e.height_mm / 25.4,
        stitches = thousands(e.stitches),
        changes = e.color_changes,
        trims = e.trims,
    )
}

pub async fn upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    if current_user(&state, &headers).await.is_none() {
        return signed_out();
    }
    let problem = |why: &str| private(TITLE, &form(why), true);
    let mut file: Option<(String, Vec<u8>)> = None;
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) if field.name() == Some("file") => {
                let name = field.file_name().unwrap_or("design").to_string();
                match field.bytes().await {
                    Ok(bytes) => file = Some((name, bytes.to_vec())),
                    Err(_) => {
                        return problem(&format!(
                            "The file couldn't be read: is it larger than {} MB?",
                            embroidery::MAX_FILE_BYTES / (1024 * 1024)
                        ));
                    }
                }
            }
            Ok(Some(_)) => {}
            Ok(None) => break,
            // (also where an upload over the size cap is cut off)
            Err(_) => {
                return problem(&format!(
                    "The upload didn't come through whole. Files can be up to {} MB.",
                    embroidery::MAX_FILE_BYTES / (1024 * 1024)
                ));
            }
        }
    }
    let Some((name, bytes)) = file else {
        return problem("Choose a file to upload.");
    };
    match embroidery::read(&bytes, &name) {
        Ok(e) => private(TITLE, &result(&name, &e), true),
        Err(why) => problem(&format!("{}: {why}.", name.chars().take(80).collect::<String>())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_separated() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(12_345), "12,345");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
