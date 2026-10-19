//! MakerFoundry's web service: server-rendered pages for the marketplace.
//!
//! MF-0: a home page, `/health` for the host's health check, a stylesheet and a 404 page, all
//! behind the security headers every page gets. MF-1: accounts, which are StitchFoundry accounts
//! (`cloud`), with MakerFoundry's own sessions in Postgres (`db`) and the pages in `accounts`.

pub mod accounts;
pub mod cloud;
pub mod db;
pub mod pages;

use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use sqlx::PgPool;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;

use crate::pages::{MARK_SVG, page};

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

/// What every request may use: the account service, and the database when there is one (without
/// `DATABASE_URL` the site runs, but nobody can sign in).
#[derive(Clone)]
pub struct AppState {
    pub cloud: cloud::Cloud,
    pub db: Option<PgPool>,
}

/// The whole site: every route, with the security headers and limits applied to all of them.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/health", get(health))
        .route("/static/site.css", get(stylesheet))
        .route("/favicon.svg", get(favicon))
        .route("/signin", get(accounts::signin_page).post(accounts::signin))
        .route("/signup", get(accounts::signup_page).post(accounts::signup))
        .route("/signout", post(accounts::signout))
        .route("/forgot", get(accounts::forgot_page).post(accounts::forgot))
        .route("/account", get(accounts::account_page))
        .fallback(not_found)
        .with_state(state)
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

async fn home(State(state): State<AppState>, headers: HeaderMap) -> Html<String> {
    let signed_in = accounts::current_user(&state, &headers).await.is_some();
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
        signed_in,
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
            false,
        )),
    )
}
