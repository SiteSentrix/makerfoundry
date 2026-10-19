//! MakerFoundry's web service: server-rendered pages for the marketplace.
//!
//! MF-0: a home page, `/health` for the host's health check, a stylesheet and a 404 page, all
//! behind the security headers every page gets. MF-1: accounts, which are Foundry Accounts kept by StitchFoundry's account service
//! (`cloud`), with MakerFoundry's own sessions in Postgres (`db`) and the pages in `accounts`.

pub mod accounts;
pub mod api;
pub mod cloud;
pub mod db;
pub mod designs;
pub mod embroidery;
pub mod laser;
pub mod legal;
pub mod licences;
pub mod limits;
pub mod listings;
pub mod makers;
pub mod moderation;
pub mod money;
pub mod pages;
pub mod photos;
pub mod preview;
pub mod selling;
pub mod storage;
pub mod stripe;

use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
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

/// The upload routes' cap: the largest file accepted, and room for the form around it.
const UPLOAD_LIMIT_BYTES: usize = laser::MAX_FILE_BYTES + 64 * 1024;

/// A request that takes longer than this is answered with 408.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Scripts, frames, plug-ins and other origins are off; pages use only our own stylesheet. Forms
/// post only here, and may be sent on to Stripe's checkout and set-up pages (browsers check
/// `form-action` against a form's redirects too).
const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; style-src 'self'; img-src 'self' data:; \
     base-uri 'none'; form-action 'self' https://checkout.stripe.com https://connect.stripe.com; \
     frame-ancestors 'none'";

/// Stripe's events can be larger than a form.
const WEBHOOK_LIMIT_BYTES: usize = 1024 * 1024;

const STYLESHEET: &str = include_str!("../static/site.css");

/// What every request may use: the account service, and the database when there is one (without
/// `DATABASE_URL` the site runs, but nobody can sign in).
#[derive(Clone)]
pub struct AppState {
    pub cloud: cloud::Cloud,
    pub db: Option<PgPool>,
    /// Where files are kept (R2); without it designs can be previewed but not kept.
    pub storage: Option<storage::Storage>,
    /// Who may publish designs (`MF_PUBLISHERS`; nobody until the launch).
    pub publishers: listings::Publishers,
    /// The accounts that see reports and take designs down (`MF_ADMINS`).
    pub admins: Vec<i64>,
    /// How often an address or an account may do things.
    pub limits: std::sync::Arc<limits::Limits>,
    /// Stripe, for selling (MF-4); without it every design is free.
    pub stripe: Option<stripe::Stripe>,
    /// Whether MakerFoundry has opened to everyone (MF-6: `MF_LAUNCHED`, once the legal pages are
    /// approved): the legal pages are public, and signing up, publishing and selling ask people to
    /// agree to them.
    pub launched: bool,
    /// The site's address (`MF_PUBLIC_URL`), for the sitemap and the links the API answers.
    pub public_url: String,
    /// The web apps' sites that may call the API from a browser (`MF_API_ORIGINS`).
    pub api_origins: Vec<String>,
}

/// The whole site: every route, with the security headers and limits applied to all of them.
pub fn app(state: AppState) -> Router {
    let pages = Router::new()
        .route("/", get(home))
        .route("/health", get(health))
        .route("/static/site.css", get(stylesheet))
        .route("/favicon.svg", get(favicon))
        .route("/robots.txt", get(robots))
        .route("/sitemap.xml", get(sitemap))
        .route("/terms", get(legal::terms))
        .route("/privacy", get(legal::privacy))
        .route("/seller-terms", get(legal::seller_terms))
        .route("/signin", get(accounts::signin_page).post(accounts::signin))
        .route("/signup", get(accounts::signup_page).post(accounts::signup))
        .route("/signout", post(accounts::signout))
        .route("/forgot", get(accounts::forgot_page).post(accounts::forgot))
        .route("/account", get(accounts::account_page))
        .route("/designs", get(designs::list))
        .route("/designs/{id}", get(designs::show))
        .route("/designs/{id}/download", get(designs::download))
        .route("/designs/{id}/photos/{photo}/{size}", get(designs::photo))
        .route("/designs/{id}/delete", post(designs::delete))
        .route("/designs/{id}/edit", get(listings::edit_page).post(listings::save))
        .route("/designs/{id}/publish", post(listings::publish))
        .route("/designs/{id}/unpublish", post(listings::unpublish))
        .route("/d/{id}", get(listings::public_page))
        .route("/d/{id}/download", get(listings::download))
        .route("/d/{id}/{slug}", get(listings::public_page_slug))
        .route("/browse", get(listings::browse))
        .route("/makers/{handle}", get(makers::show))
        .route("/account/maker", get(makers::edit_page).post(makers::save))
        .route("/d/{id}/report", get(moderation::report_page).post(moderation::report))
        .route(
            "/designs/{id}/counter",
            get(moderation::counter_page).post(moderation::counter),
        )
        .route("/dmca", get(moderation::policy))
        .route("/dmca/notice", get(moderation::notice_page).post(moderation::notice))
        .route("/admin", get(moderation::queue))
        .route("/admin/reports/{rid}/takedown", post(moderation::takedown))
        .route("/admin/reports/{rid}/dismiss", post(moderation::dismiss))
        .route("/admin/reports/{rid}/restore", post(moderation::restore))
        .route("/account/selling", get(selling::selling_page))
        .route("/account/selling/start", post(selling::start))
        .route("/account/selling/return", get(selling::returned))
        .route("/account/selling/dashboard", post(selling::dashboard))
        .route("/account/sales", get(selling::sales))
        .route("/library", get(selling::library))
        .route("/d/{id}/buy", post(selling::buy))
        .route("/admin/orders", get(selling::orders))
        .route("/admin/orders/{order}/refund", post(selling::refund))
        .fallback(not_found)
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT_BYTES));
    let uploads = Router::new()
        .route("/preview", get(preview::page).post(preview::upload))
        .layer(DefaultBodyLimit::max(UPLOAD_LIMIT_BYTES))
        .layer(RequestBodyLimitLayer::new(UPLOAD_LIMIT_BYTES));
    let new_designs = Router::new()
        .route("/designs/new", get(designs::new_page).post(designs::upload))
        .layer(DefaultBodyLimit::max(designs::MAX_UPLOAD_BYTES))
        .layer(RequestBodyLimitLayer::new(designs::MAX_UPLOAD_BYTES));
    let webhooks = Router::new()
        .route("/stripe/webhook", post(selling::webhook))
        .layer(DefaultBodyLimit::max(WEBHOOK_LIMIT_BYTES))
        .layer(RequestBodyLimitLayer::new(WEBHOOK_LIMIT_BYTES));
    let api = Router::new()
        .route("/api/v1", get(api::about))
        .route("/api/v1/designs", post(api::new_design).options(api::preflight))
        .layer(DefaultBodyLimit::max(api::API_LIMIT_BYTES))
        .layer(RequestBodyLimitLayer::new(api::API_LIMIT_BYTES));
    pages
        .merge(uploads)
        .merge(api)
        .merge(new_designs)
        .merge(webhooks)
        .with_state(state)
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
    let newest = listings::newest(&state).await;
    let call = if state.launched {
        let share = if signed_in { "/designs/new" } else { "/signup" };
        format!(
            r#"<p class="actions"><a class="button" href="/browse">Browse designs</a> <a class="button secondary" href="{share}">Share your designs</a></p>"#
        )
    } else {
        r#"<p class="soon">Coming soon. MakerFoundry is being built now.</p>"#.to_string()
    };
    Html(page(
        "MakerFoundry",
        &format!(
            r#"<section class="hero">
  <h1>Designs for your machines</h1>
  <p class="lead">Machine-embroidery designs and laser-cut files, shared free or sold by the
  makers who made them.</p>
  {call}
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
    <p>Share a design for free, or sell it: you keep 90% of the price less the card fee, paid
    monthly, and your designs stay yours.</p>
  </article>
</section>
{newest}"#
        ),
        signed_in,
    ))
}

/// Search engines: everything public once launched, nothing before.
async fn robots(State(state): State<AppState>) -> Response {
    let text = if state.launched {
        format!(
            "User-agent: *\nDisallow: /account\nDisallow: /admin\nDisallow: /designs\nDisallow: /library\nDisallow: /preview\nDisallow: /signin\nDisallow: /signup\nDisallow: /forgot\nDisallow: /d/*/download\nDisallow: /d/*/report\nSitemap: {}/sitemap.xml\n",
            state.public_url
        )
    } else {
        "User-agent: *\nDisallow: /\n".to_string()
    };
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        text,
    )
        .into_response()
}

/// The public pages for search engines: the fixed pages, published designs and makers.
async fn sitemap(State(state): State<AppState>) -> Response {
    if !state.launched {
        return not_found().await.into_response();
    }
    let base = &state.public_url;
    let mut urls: Vec<String> = ["/", "/browse", "/terms", "/privacy", "/seller-terms", "/dmca"]
        .iter()
        .map(|p| format!("{base}{p}"))
        .collect();
    if let Some(pool) = &state.db {
        let designs: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, title FROM mf_designs WHERE status = 'published' ORDER BY published_at DESC LIMIT 40000",
        )
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        urls.extend(
            designs
                .iter()
                .map(|(id, t)| format!("{base}/d/{id}/{}", listings::slug(t))),
        );
        let makers: Vec<String> = sqlx::query_scalar(
            "SELECT handle FROM mf_makers m WHERE EXISTS (SELECT 1 FROM mf_designs d WHERE d.account_id = m.account_id AND d.status = 'published') LIMIT 9000",
        )
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        urls.extend(makers.iter().map(|h| format!("{base}/makers/{h}")));
    }
    let body: String = urls
        .iter()
        .map(|u| format!("<url><loc>{}</loc></url>", pages::escape(u)))
        .collect();
    (
        [
            (header::CONTENT_TYPE, "application/xml; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        format!(r#"<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">{body}</urlset>"#),
    )
        .into_response()
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
