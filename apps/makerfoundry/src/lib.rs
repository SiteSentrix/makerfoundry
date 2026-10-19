//! MakerFoundry's web service: server-rendered pages for the marketplace.
//!
//! MF-0 holds the skeleton: a home page, `/health` for the host's health check, a stylesheet and
//! a 404 page, all behind the security headers every later page gets too.

use std::time::Duration;

use axum::Router;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;

/// The version shown on `/health`, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Bodies bigger than this are refused before a handler sees them. Uploads (MF-2) get their own,
/// larger cap on their own route.
const BODY_LIMIT_BYTES: usize = 64 * 1024;

/// A request that takes longer than this is answered with 408.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Scripts, frames, plug-ins and other origins are off; pages use only our own stylesheet.
const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; style-src 'self'; img-src 'self' data:; \
     base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

const STYLESHEET: &str = include_str!("../static/site.css");

/// The whole site: every route, with the security headers and limits applied to all of them.
pub fn app() -> Router {
    Router::new()
        .route("/", get(home))
        .route("/health", get(health))
        .route("/static/site.css", get(stylesheet))
        .route("/favicon.svg", get(favicon))
        .fallback(not_found)
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT_BYTES))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(header_layer(header::CONTENT_SECURITY_POLICY, CONTENT_SECURITY_POLICY))
        .layer(header_layer(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(header_layer(header::REFERRER_POLICY, "strict-origin-when-cross-origin"))
        .layer(header_layer(header::X_FRAME_OPTIONS, "DENY"))
}

fn header_layer(name: HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
}

/// The host's health check: answers as soon as the server is up. Never touches the database, so
/// a database outage shows as errors on pages rather than a restart loop.
async fn health() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        format!("ok {VERSION}\n"),
    )
        .into_response()
}

async fn stylesheet() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        STYLESHEET,
    )
        .into_response()
}

/// The mark as a stand-alone icon, in the accent colour (a favicon can't inherit the page's).
async fn favicon() -> Response {
    let icon = MARK_SVG
        .replace(r#"class="mark" "#, r#"xmlns="http://www.w3.org/2000/svg" "#)
        .replace("currentColor", "#b4462a");
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        icon,
    )
        .into_response()
}

async fn home() -> Html<String> {
    Html(page(
        "MakerFoundry",
        r#"<section class="hero">
  <h1>Designs for your machines</h1>
  <p class="lead">Machine-embroidery designs and laser-cut files, shared free or sold by the
  makers who made them.</p>
  <p class="soon">Coming soon. MakerFoundry is being built now.</p>
</section>
<section class="kinds">
  <article>
    <h2>Embroidery</h2>
    <p>DST, PES, JEF, EXP, VP3 and more, with a preview of every stitch, the stitch count,
    the size and the colours.</p>
  </article>
  <article>
    <h2>Laser cutting</h2>
    <p>SVG, DXF and PDF files for cutting and engraving, drawn from their paths with their
    size.</p>
  </article>
  <article>
    <h2>For makers</h2>
    <p>Share a design for free, or sell it and get paid for each sale.</p>
  </article>
</section>"#,
    ))
}

async fn not_found() -> (StatusCode, Html<String>) {
    (
        StatusCode::NOT_FOUND,
        Html(page(
            "Not found · MakerFoundry",
            r#"<section class="hero">
  <h1>Page not found</h1>
  <p class="lead">There's nothing at this address. <a href="/">Go to the home page</a>.</p>
</section>"#,
        )),
    )
}

/// The site's mark, drawn in code: an embroidery hoop crossed by a run of stitches. Also served as
/// `/favicon.svg`.
const MARK_SVG: &str = r#"<svg class="mark" viewBox="0 0 32 32" width="32" height="32" aria-hidden="true">
<circle cx="16" cy="16" r="13" fill="none" stroke="currentColor" stroke-width="3"/>
<circle cx="16" cy="16" r="8.5" fill="none" stroke="currentColor" stroke-width="1.5"/>
<path d="M6 22 L26 10" stroke="currentColor" stroke-width="2" stroke-dasharray="3 2.5" stroke-linecap="round"/>
</svg>"#;

/// Wraps a page's body in the shared layout. `title` and `body` are our own text, never a user's:
/// pages that show user input escape it before it gets here.
fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<meta name="description" content="Machine-embroidery designs and laser-cut files, shared and sold by makers.">
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="/static/site.css">
</head>
<body>
<header class="top"><a class="brand" href="/">{MARK_SVG}<span>MakerFoundry</span></a></header>
<main>
{body}
</main>
<footer class="bottom"><p>MakerFoundry is made by <a href="https://sitesentrix.com">SiteSentrix</a>,
the makers of <a href="https://stitchfoundry.app">StitchFoundry</a>.</p></footer>
</body>
</html>
"#
    )
}
