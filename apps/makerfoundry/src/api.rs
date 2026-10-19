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
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::designs::{self, MAX_PHOTOS, Refused};
use crate::{db, limits};

/// The most a request to the API may be: an upload's limit, as base64, and room for the rest.
pub const API_LIMIT_BYTES: usize = designs::MAX_UPLOAD_BYTES / 3 * 4 + 64 * 1024;

/// The web apps that may call the API from a browser, unless `MF_API_ORIGINS` says otherwise.
pub const DEFAULT_ORIGINS: &str = "https://stitchfoundry.app";

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
