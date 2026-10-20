//! The apps' API (MF-7): StitchFoundry and LightFoundry send a design to MakerFoundry with
//! **Publish to MakerFoundry**. The app is already signed in to the person's Foundry Account, so it
//! sends that sign-in (the account service's session token) as a bearer token; MakerFoundry checks
//! it with the account service, uses it for this one request, and never keeps it.
//!
//! The design arrives as a draft, read and checked like an upload from the site; the app then
//! opens its page (`url`) in the browser, where the maker adds the details and publishes it. The
//! contract the apps follow is `docs/api.md`.
//!
//! Browsers may call it from the web apps' own sites (`MF_API_ORIGINS`), without cookies.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use base64::Engine;
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::designs::{self, MAX_PHOTOS, Refused};
use crate::{db, limits};

/// The most a request to the API may be: an upload's limit, as base64, and room for the rest.
pub const API_LIMIT_BYTES: usize = designs::MAX_UPLOAD_BYTES / 3 * 4 + 64 * 1024;

/// The web apps that may call the API from a browser, unless `MF_API_ORIGINS` says otherwise:
/// StitchFoundry's site, and its web app's (GitHub Pages).
pub const DEFAULT_ORIGINS: &str = "https://stitchfoundry.app,https://stitchwright.github.io";

/// StitchFoundry's web app, which opens a design from `?open=<address>` (Open in StitchFoundry).
pub const STITCHFOUNDRY_WEB: &str = "https://stitchwright.github.io/StitchFoundry/";
/// LaserFoundry's web app, which opens laser designs the same way once it reads `?open=`.
pub const LASERFOUNDRY_WEB: &str = "https://stitchwright.github.io/laserfoundry/";
/// Whether LaserFoundry's web app opens a design from an address yet: until it does, laser designs
/// show no button (their files are served all the same, for when it does).
pub const LASERFOUNDRY_OPENS_LINKS: bool = false;

/// The web app that opens design `d`, by its kind: (address, name). Embroidery opens in
/// StitchFoundry; an SVG or DXF laser design in LaserFoundry (what it imports), once it opens links.
pub fn web_app(d: &designs::DesignRow) -> Option<(&'static str, &'static str)> {
    match d.kind.as_str() {
        "embroidery" => Some((STITCHFOUNDRY_WEB, "StitchFoundry")),
        "laser" if LASERFOUNDRY_OPENS_LINKS && opens_in_laserfoundry(d) => Some((LASERFOUNDRY_WEB, "LaserFoundry")),
        _ => None,
    }
}

/// Whether design `d`'s file is one a web app can open: any embroidery file; an SVG or DXF laser file.
pub fn openable(d: &designs::DesignRow) -> bool {
    d.kind == "embroidery" || (d.kind == "laser" && opens_in_laserfoundry(d))
}

fn opens_in_laserfoundry(d: &designs::DesignRow) -> bool {
    matches!(
        d.facts.get("format").and_then(serde_json::Value::as_str),
        Some("SVG" | "DXF")
    )
}

/// The link that opens design `id` in StitchFoundry's web app: its file's address on this site
/// (`public_url`), ending in `name` so the app knows how to read it.
pub fn open_in_stitchfoundry(public_url: &str, id: i64, name: &str) -> String {
    open_link(STITCHFOUNDRY_WEB, public_url, id, name, None)
}

/// The link that opens design `id`'s file (named `name`) in web app `app`.
pub fn open_in(app: &str, public_url: &str, id: i64, name: &str) -> String {
    open_link(app, public_url, id, name, None)
}

/// The same, for a paid design: the file address carries a buyer's link (`t`, see `open_paid`).
fn open_link(app: &str, public_url: &str, id: i64, name: &str, token: Option<&str>) -> String {
    let file = format!(
        "{}/api/v1/designs/{id}/file/{}{}",
        public_url.trim_end_matches('/'),
        crate::listings::url_word(name),
        token.map(|t| format!("?t={t}")).unwrap_or_default()
    );
    format!("{app}?open={}", crate::listings::url_word(&file))
}

/// How long a buyer's link to a paid design's file lasts.
const OPEN_LINK_MINUTES: i32 = 10;

fn token_hash(token: &str) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `GET /d/{id}/open`: Open in StitchFoundry for a paid design. The web app fetches without
/// anyone's sign-in, so this checks here that the person may download it (a buyer, its maker or an
/// admin), makes a link good for this design for ten minutes (kept only as its hash), and sends
/// them to the web app with it.
pub async fn open_paid(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    let back = Redirect::to(&format!("/d/{id}")).into_response();
    let Some(viewer) = crate::accounts::current_user(&state, &headers).await else {
        return Redirect::to(&format!("/signin?next=/d/{id}")).into_response();
    };
    let Some(pool) = state.db.as_ref() else { return back };
    let Ok(Some(d)) = designs::design(pool, id).await else {
        return back;
    };
    let (_, may) = crate::listings::access(&state, pool, Some(&viewer), &d, id).await;
    let Some((app, _)) = web_app(&d).filter(|_| may) else {
        return back;
    };
    let mut bytes = [0u8; 24];
    if getrandom::fill(&mut bytes).is_err() {
        return back;
    }
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let made = sqlx::query(
        "INSERT INTO mf_open_links (token_hash, design_id, expires_at) VALUES ($1, $2, now() + make_interval(mins => $3))",
    )
    .bind(token_hash(&token))
    .bind(id)
    .bind(OPEN_LINK_MINUTES)
    .execute(pool)
    .await;
    if let Err(e) = made {
        eprintln!("api: open link {id}: {e}");
        return back;
    }
    let ext = d.file_name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let name = format!("{}.{ext}", crate::listings::slug(&d.title));
    Redirect::to(&open_link(app, &state.public_url, id, &name, Some(&token))).into_response()
}

/// Whether `token` is a buyer's link to design `id` that hasn't run out.
async fn link_good(pool: &sqlx::PgPool, token: &str, id: i64) -> bool {
    if token.len() != 48 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM mf_open_links WHERE token_hash = $1 AND design_id = $2 AND expires_at > now()",
    )
    .bind(token_hash(token))
    .bind(id)
    .fetch_one(pool)
    .await
    .is_ok_and(|n| n > 0)
}

/// Forgets buyers' links that have run out. Run every few minutes.
pub async fn forget_old_links(pool: &sqlx::PgPool) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM mf_open_links WHERE expires_at < now()")
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct FileQuery {
    /// A buyer's link to a paid design's file (`open_paid`).
    t: String,
}

/// An error as the apps get it: `{"error": {"message"}}` with the status.
fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": { "message": message } }))).into_response()
}

/// The CORS headers for a request from `origin`, if it's one of the web apps.
fn cors(state: &AppState, headers: &HeaderMap) -> Vec<(HeaderName, HeaderValue)> {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) else {
        return Vec::new();
    };
    if !state.api_origins.iter().any(|o| o == origin) {
        return Vec::new();
    }
    let Ok(origin) = HeaderValue::from_str(origin) else {
        return Vec::new();
    };
    vec![
        (header::ACCESS_CONTROL_ALLOW_ORIGIN, origin),
        (header::VARY, HeaderValue::from_static("Origin")),
    ]
}

fn with_cors(state: &AppState, headers: &HeaderMap, mut resp: Response) -> Response {
    for (k, v) in cors(state, headers) {
        resp.headers_mut().insert(k, v);
    }
    resp
}

/// `GET /api/v1/designs/{id}/file/{name}`: a free, published embroidery design's file, for
/// StitchFoundry's web app (Open in StitchFoundry). The file itself is sent (not a link to the
/// storage, which another site's page couldn't follow), with CORS for the web apps' sites; cookies
/// aren't read, so it is only what anyone may download. `name` only ends the address, for the app.
pub async fn design_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, _name)): Path<(i64, String)>,
    axum::extract::Query(q): axum::extract::Query<FileQuery>,
) -> Response {
    let mut resp = design_file_answer(&state, &headers, id, &q.t).await;
    resp.headers_mut().insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("content-disposition"),
    );
    with_cors(&state, &headers, resp)
}

async fn design_file_answer(state: &AppState, headers: &HeaderMap, id: i64, token: &str) -> Response {
    let (Some(pool), Some(storage)) = (state.db.as_ref(), state.storage.as_ref()) else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "MakerFoundry can't send designs just now.",
        );
    };
    let missing = || error(StatusCode::NOT_FOUND, "That design isn't on MakerFoundry.");
    let Ok(Some(d)) = designs::design(pool, id).await else {
        return missing();
    };
    // what anyone may see and download: published, not taken down, and free; or a paid design
    // with a buyer's link (made once their right to it was checked), unless it was taken down
    let (seen, may) = crate::listings::access(state, pool, None, &d, id).await;
    let bought = !may && d.status != "removed" && !token.is_empty() && link_good(pool, token, id).await;
    if (!seen && !bought) || !openable(&d) {
        return missing();
    }
    if !may && !bought {
        return error(
            StatusCode::FORBIDDEN,
            "This design is for sale: buy it on MakerFoundry, then download it there.",
        );
    }
    if !state.limits.allow(limits::OPEN, &limits::client(headers)) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many designs opened lately. Please wait a while.",
        );
    }
    let bytes = match storage.get(&d.file_key).await {
        Ok(Some(b)) => b,
        Ok(None) => return missing(),
        Err(e) => {
            eprintln!("api: file {id}: {e}");
            return error(
                StatusCode::BAD_GATEWAY,
                "The design can't be fetched just now. Please try again.",
            );
        }
    };
    let _ = sqlx::query("UPDATE mf_designs SET downloads = downloads + 1 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await; // (a count; the file goes out anyway)
    let ext = d.file_name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let name: String = format!("{}.{ext}", crate::listings::slug(&d.title))
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c))
        .collect();
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{name}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (header::CONTENT_DISPOSITION, disposition),
            (header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=300")),
        ],
        bytes,
    )
        .into_response()
}

/// A browser asking first whether a web app may send to the API.
pub async fn preflight(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let allowed = cors(&state, &headers);
    if allowed.is_empty() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    let h = resp.headers_mut();
    for (k, v) in allowed {
        h.insert(k, v);
    }
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("POST"));
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type"),
    );
    h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    resp
}

/// What an app sends: the design file and, if it has them, photos of the result, each base64.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct NewDesign {
    file_name: String,
    file: String,
    photos: Vec<String>,
    /// The app and its version, for the log (`StitchFoundry 0.9.1`).
    app: String,
}

/// `POST /api/v1/designs`: a new draft → `201 {id, url, kind, format}`.
pub async fn new_design(State(state): State<AppState>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let resp = answer(&state, &headers, &body).await;
    with_cors(&state, &headers, resp)
}

async fn answer(state: &AppState, headers: &HeaderMap, body: &[u8]) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "MakerFoundry can't take designs just now.",
        );
    };
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    if token.is_empty() || token.len() > 200 {
        return error(StatusCode::UNAUTHORIZED, "Sign in to your Foundry Account first.");
    }
    // a wrong token costs a try, as a wrong password does
    let client = limits::client(headers);
    let account = match state.cloud.account(token).await {
        Ok(a) => a,
        Err(e) if e.is_signed_out() => {
            if !state.limits.allow(limits::SIGNIN, &client) {
                return error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Too many tries lately. Please wait a while.",
                );
            }
            return error(
                StatusCode::UNAUTHORIZED,
                "Your Foundry Account sign-in has ended: sign in again.",
            );
        }
        Err(_) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Foundry Accounts can't be reached right now. Please try again in a few minutes.",
            );
        }
    };
    if let Err(e) = db::remember_account(pool, &account).await {
        eprintln!("api: {e}");
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "MakerFoundry can't take designs just now.",
        );
    }
    let Ok(d) = serde_json::from_slice::<NewDesign>(body) else {
        return error(
            StatusCode::BAD_REQUEST,
            "The request isn't a design MakerFoundry can read.",
        );
    };
    let file_name = clean_name(&d.file_name);
    if file_name.is_empty() {
        return error(StatusCode::BAD_REQUEST, "Give the design file's name.");
    }
    let b64 = base64::engine::general_purpose::STANDARD;
    let Ok(bytes) = b64.decode(d.file.as_bytes()) else {
        return error(StatusCode::BAD_REQUEST, "The design file isn't base64.");
    };
    if bytes.is_empty() {
        return error(StatusCode::BAD_REQUEST, "The design file is empty.");
    }
    if d.photos.len() > MAX_PHOTOS {
        return error(
            StatusCode::BAD_REQUEST,
            &format!("A design can have up to {MAX_PHOTOS} photos."),
        );
    }
    let mut photos = Vec::new();
    for (i, p) in d.photos.iter().enumerate() {
        match b64.decode(p.as_bytes()) {
            Ok(bytes) => photos.push(bytes),
            Err(_) => return error(StatusCode::BAD_REQUEST, &format!("Photo {} isn't base64.", i + 1)),
        }
    }
    match designs::add(state, pool, account.id, &file_name, bytes, photos).await {
        Ok(id) => {
            let app: String = d.app.chars().filter(|c| !c.is_control()).take(60).collect();
            eprintln!("api: design {id} from {}", if app.is_empty() { "an app" } else { &app });
            let read: Option<(String, String)> = sqlx::query_as("SELECT kind, format FROM mf_designs WHERE id = $1")
                .bind(id)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
            let (kind, format) = read.unwrap_or_default();
            (
                StatusCode::CREATED,
                Json(json!({
                    "id": id,
                    "url": format!("{}/designs/{id}", state.public_url),
                    "kind": kind,
                    "format": format,
                })),
            )
                .into_response()
        }
        Err(Refused::Unavailable) => error(StatusCode::SERVICE_UNAVAILABLE, &Refused::Unavailable.message()),
        Err(r @ Refused::TooMany(_)) => error(StatusCode::TOO_MANY_REQUESTS, &r.message()),
        Err(r @ Refused::Bad(_)) => error(StatusCode::UNPROCESSABLE_ENTITY, &r.message()),
        Err(r @ Refused::Failed) => error(StatusCode::SERVICE_UNAVAILABLE, &r.message()),
    }
}

/// A file name as sent, reduced to its last part, without control characters, at most 200
/// characters.
fn clean_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    last.chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect::<String>()
        .trim()
        .to_string()
}

/// `GET /api/v1`: what the API is, for an app checking it's talking to MakerFoundry.
pub async fn about(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let resp = Json(json!({
        "name": "MakerFoundry",
        "version": crate::VERSION,
        "designs": "/api/v1/designs",
        "max_upload_bytes": designs::MAX_UPLOAD_BYTES,
        "max_photos": MAX_PHOTOS,
    }))
    .into_response();
    with_cors(&state, &headers, resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_keep_only_their_last_part() {
        assert_eq!(clean_name("rose.pes"), "rose.pes");
        assert_eq!(clean_name("../../etc/passwd"), "passwd");
        assert_eq!(clean_name(r"C:\Users\ann\rose.dst"), "rose.dst");
        assert_eq!(clean_name("a\u{0}b\nc.svg"), "abc.svg");
        assert_eq!(clean_name("dir/"), "");
        assert_eq!(clean_name(&"x".repeat(300)).len(), 200);
    }

    #[test]
    fn the_limit_takes_a_whole_upload_in_base64() {
        let b64 = base64::engine::general_purpose::STANDARD;
        let encoded = b64.encode(vec![0u8; designs::MAX_UPLOAD_BYTES - 3]);
        assert!(encoded.len() < API_LIMIT_BYTES);
    }
}
