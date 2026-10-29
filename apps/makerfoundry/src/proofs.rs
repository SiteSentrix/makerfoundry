//! Approval links (MF-25): a shop sends a customer a proof of a design before stitching or cutting
//! it. Any Foundry Account can be a shop (the owner's decision, 2026-10-29). A proof is a design
//! file, read and checked exactly as an upload (`designs::read_design`), drawn from what was read
//! (never the upload itself), with mock-up photos cleaned as a design's are (`photos`).
//!
//! The customer has no account: they open a private link, see the latest version, comment, and
//! approve it or ask for changes, each under the name they type. The shop sends new versions; the
//! link always shows the latest, with the earlier ones and the whole conversation below.
//!
//! The link isn't kept anywhere: it is the proof's id and an HMAC of it and its link generation
//! with the server's key (`MF_PROOF_KEY`), so the database alone can't open a proof, the shop can
//! see its link whenever it likes, and a new link (the generation + 1) ends the old one.
//!
//! Switched on by `MF_PROOFS=yes` (with `MF_PROOF_KEY` set); until then only admins can use it.
//! A customer's name and words are the shop's (the owner's decision, 2026-10-29). A proof is
//! deleted a year after its last activity (`forget_old`, run every few minutes).

use std::collections::HashMap;

use axum::Form;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use sqlx::{PgConnection, PgPool, Row};

use crate::AppState;
use crate::accounts::{User, current_user, forbidden, same_origin};
use crate::designs::{self, Read, drawing_section, read_design};
use crate::listings::paragraphs;
use crate::pages::{escape, page};
use crate::storage::{Storage, new_key};
use crate::{limits, photos};

/// The most proofs one shop may keep at once.
pub const MAX_PROOFS: i64 = 500;
/// The most versions one proof may have.
pub const MAX_VERSIONS: i32 = 30;
/// The longest title, and the longest name (a shop's, a customer's), in characters.
const MAX_TITLE: usize = 120;
const MAX_NAME: usize = 100;
/// The longest message, note or reply, in characters.
pub const MAX_TEXT: usize = 2000;
/// Days a proof is kept after its last activity (the owner's decision, 2026-10-29).
pub const KEEP_DAYS: i32 = 365;
/// Replies shown under a proof (the oldest).
const SHOWN_REPLIES: i64 = 500;

/// The approval-link setting: the key links are made with, and whether everyone may use them
/// (`MF_PROOFS=yes`) or only the admins.
#[derive(Clone)]
pub struct Proofs {
    key: Vec<u8>,
    pub open: bool,
}

impl std::fmt::Debug for Proofs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // (never the key)
        f.debug_struct("Proofs").field("open", &self.open).finish()
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|p| u8::from_str_radix(p, 16).ok())
        })
        .collect()
}

impl Proofs {
    /// `key`: at least 32 random bytes. `open`: whether everyone may send proofs.
    pub fn new(key: &[u8], open: bool) -> Result<Proofs, String> {
        if key.len() < 32 {
            return Err("MF_PROOF_KEY must be at least 64 hex digits (32 random bytes)".into());
        }
        Ok(Proofs {
            key: key.to_vec(),
            open,
        })
    }

    /// The setting the environment gives (`env` gives a setting's value): none without
    /// `MF_PROOF_KEY`.
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Result<Option<Proofs>, String> {
        let Some(key) = env("MF_PROOF_KEY")
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
        else {
            return Ok(None);
        };
        let key = unhex(&key).ok_or("MF_PROOF_KEY must be hex digits (openssl rand -hex 32)")?;
        let open = env("MF_PROOFS").is_some_and(|v| v.trim() == "yes");
        Proofs::new(&key, open).map(Some)
    }

    fn mac(&self, id: i64, generation: i32) -> Option<Hmac<Sha256>> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).ok()?;
        mac.update(format!("makerfoundry proof {id} link {generation}").as_bytes());
        Some(mac)
    }

    /// The secret part of proof `id`'s link at `generation`.
    fn token(&self, id: i64, generation: i32) -> Option<String> {
        self.mac(id, generation).map(|m| hex(&m.finalize().into_bytes()))
    }

    /// Whether `token` is proof `id`'s link at `generation` (compared in constant time).
    fn token_good(&self, id: i64, generation: i32, token: &str) -> bool {
        match (token.len() == 64, unhex(token), self.mac(id, generation)) {
            (true, Some(bytes), Some(mac)) => mac.verify_slice(&bytes).is_ok(),
            _ => false,
        }
    }
}

/// Whether `user` may send proofs: everyone once they're switched on, admins before.
pub fn available(state: &AppState, user: &User) -> bool {
    state
        .proofs
        .as_ref()
        .is_some_and(|p| p.open || state.admins.contains(&user.account_id))
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(page(
            "Not found · MakerFoundry",
            r#"<section class="hero"><h1>Proof not found</h1><p class="lead">There's no proof at this address. If a shop sent you this link, ask them for a new one.</p></section>"#,
            false,
        )),
    )
        .into_response()
}

/// A page only for whoever has it open: not kept in caches, not found by search engines, and
/// the link not passed on to other sites.
fn private_page(title: &str, body: &str, signed_in: bool) -> Response {
    let mut resp = Html(page(title, body, signed_in)).into_response();
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert("x-robots-tag", HeaderValue::from_static("noindex, nofollow"));
    resp
}

fn problem_html(problem: &str) -> String {
    if problem.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem" role="alert">{}</p>"#, escape(problem))
    }
}

fn problem_page(title: &str, message: &str) -> Response {
    private_page(
        &format!("{} · MakerFoundry", escape(title)),
        &format!(
            r#"<section class="form"><h1>{}</h1><p class="lead">{message}</p><p><a href="/proofs">Your proofs</a></p></section>"#,
            escape(title)
        ),
        true,
    )
}

/// The signed-in shop that may send proofs, its database and the setting; or where to go.
async fn shop(state: &AppState, headers: &HeaderMap, next: &str) -> Result<(User, PgPool, Proofs), Response> {
    let Some(user) = current_user(state, headers).await else {
        return Err(Redirect::to(&format!("/signin?next={next}")).into_response());
    };
    let Some(pool) = state.db.clone() else {
        return Err(Redirect::to("/signin").into_response());
    };
    match state.proofs.clone() {
        Some(p) if p.open || state.admins.contains(&user.account_id) => Ok((user, pool, p)),
        _ => Err(not_found()),
    }
}

// ---------------------------------------------------------------- what's kept

struct ProofRow {
    account_id: i64,
    title: String,
    shop_name: String,
    customer: String,
    message: String,
    link_generation: i32,
    link_on: bool,
    status: String,
}

async fn proof(pool: &PgPool, id: i64) -> Result<Option<ProofRow>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT account_id, title, shop_name, customer, message, link_generation, link_on, status
         FROM mf_proofs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.map(|r| {
        Ok(ProofRow {
            account_id: r.try_get("account_id")?,
            title: r.try_get("title")?,
            shop_name: r.try_get("shop_name")?,
            customer: r.try_get("customer")?,
            message: r.try_get("message")?,
            link_generation: r.try_get("link_generation")?,
            link_on: r.try_get("link_on")?,
            status: r.try_get("status")?,
        })
    })
    .transpose()
}

/// Proof `id` if `user` sent it (someone else's is "not found").
async fn own_proof(pool: &PgPool, user: &User, id: i64) -> Result<ProofRow, Response> {
    match proof(pool, id).await {
        Ok(Some(p)) if p.account_id == user.account_id => Ok(p),
        Ok(_) => Err(not_found()),
        Err(e) => {
            eprintln!("proofs: {e}");
            Err(not_found())
        }
    }
}

struct Version {
    id: i64,
    number: i32,
    kind: String,
    format: String,
    facts: serde_json::Value,
    preview_svg: String,
    note: String,
    at: String,
}

/// A proof's versions, newest first.
async fn versions(pool: &PgPool, proof_id: i64) -> Vec<Version> {
    let rows = sqlx::query(
        "SELECT id, number, kind, format, facts, preview_svg, note, to_char(created_at, 'YYYY-MM-DD HH24:MI') AS at
         FROM mf_proof_versions WHERE proof_id = $1 ORDER BY number DESC",
    )
    .bind(proof_id)
    .fetch_all(pool)
    .await;
    match rows {
        Ok(rows) => rows
            .iter()
            .filter_map(|r| {
                Some(Version {
                    id: r.try_get("id").ok()?,
                    number: r.try_get("number").ok()?,
                    kind: r.try_get("kind").ok()?,
                    format: r.try_get("format").ok()?,
                    facts: r.try_get("facts").ok()?,
                    preview_svg: r.try_get("preview_svg").ok()?,
                    note: r.try_get("note").ok()?,
                    at: r.try_get("at").ok()?,
                })
            })
            .collect(),
        Err(e) => {
            eprintln!("proofs: versions of {proof_id}: {e}");
            Vec::new()
        }
    }
}

/// The storage keys of proofs `ids`' files and photos.
async fn file_keys(conn: &mut PgConnection, ids: &[i64]) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT v.file_key FROM mf_proof_versions v WHERE v.proof_id = ANY($1)
         UNION ALL SELECT p.photo_key FROM mf_proof_photos p JOIN mf_proof_versions v ON v.id = p.version_id WHERE v.proof_id = ANY($1)
         UNION ALL SELECT p.thumb_key FROM mf_proof_photos p JOIN mf_proof_versions v ON v.id = p.version_id WHERE v.proof_id = ANY($1)",
    )
    .bind(ids)
    .fetch_all(conn)
    .await
}

/// Deletes proofs `ids` (with their versions, photos and replies) → their files' storage keys.
pub async fn erase(conn: &mut PgConnection, ids: &[i64]) -> Result<Vec<String>, sqlx::Error> {
    let keys = file_keys(conn, ids).await?;
    sqlx::query("DELETE FROM mf_proofs WHERE id = ANY($1)")
        .bind(ids)
        .execute(conn)
        .await?;
    Ok(keys)
}

async fn delete_files(storage: Option<&Storage>, keys: &[String]) {
    if let Some(storage) = storage {
        for key in keys {
            // (the proof is gone either way; a file left behind costs a little storage)
            if let Err(e) = storage.delete(key).await {
                eprintln!("proofs: delete a file: {e}");
            }
        }
    }
}

/// Deletes the proofs with no activity for `KEEP_DAYS` days, files and all → how many. Run every
/// few minutes.
pub async fn forget_old(pool: &PgPool, storage: Option<&Storage>) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM mf_proofs WHERE active_at < now() - make_interval(days => $1) LIMIT 500 FOR UPDATE",
    )
    .bind(KEEP_DAYS)
    .fetch_all(&mut *tx)
    .await?;
    if ids.is_empty() {
        return Ok(0);
    }
    let keys = erase(&mut tx, &ids).await?;
    tx.commit().await?;
    delete_files(storage, &keys).await;
    Ok(u64::try_from(ids.len()).unwrap_or(0))
}

// ---------------------------------------------------------------- uploads

/// A proof form as sent: its text fields, the design file and the photos.
#[derive(Default)]
struct Upload {
    text: HashMap<String, String>,
    file: Option<(String, Vec<u8>)>,
    photos: Vec<Vec<u8>>,
}

impl Upload {
    fn text(&self, name: &str) -> &str {
        self.text.get(name).map_or("", |t| t.trim())
    }
}

const NOT_WHOLE: &str = "The upload didn't come through whole. It can be up to 64 MB in all.";

async fn read_upload(mut multipart: Multipart) -> Result<Upload, String> {
    let mut up = Upload::default();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return Err(NOT_WHOLE.into()),
        };
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "file" | "photos" => {
                let file_name: String = field.file_name().unwrap_or("design").chars().take(200).collect();
                let bytes = field.bytes().await.map_err(|_| NOT_WHOLE.to_string())?;
                if bytes.is_empty() {
                    // (a form sent without a file chosen has one empty part)
                    continue;
                }
                if name == "file" {
                    up.file = Some((file_name, bytes.to_vec()));
                } else if up.photos.len() >= designs::MAX_PHOTOS {
                    return Err(format!("A proof can have up to {} photos.", designs::MAX_PHOTOS));
                } else {
                    up.photos.push(bytes.to_vec());
                }
            }
            "title" | "shop_name" | "customer" | "message" | "note" => {
                let text = field.text().await.map_err(|_| NOT_WHOLE.to_string())?;
                if text.chars().count() > MAX_TEXT {
                    return Err(format!("Each box holds up to {MAX_TEXT} characters."));
                }
                up.text.insert(name, text);
            }
            _ => {}
        }
    }
    Ok(up)
}

/// One line of text as typed, or why it won't do (`what` names the box).
fn line(text: &str, what: &str, max: usize, required: bool) -> Result<String, String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if required && text.is_empty() {
        return Err(format!("Give {what}."));
    }
    if text.chars().count() > max || text.chars().any(char::is_control) {
        return Err(format!("Keep {what} to {max} characters of plain text."));
    }
    Ok(text)
}

/// Text of several lines as typed, tidied, or why it won't do.
fn passage(text: &str) -> Result<String, String> {
    let text = text.trim().replace("\r\n", "\n");
    if text.chars().count() > MAX_TEXT || text.chars().any(|c| c.is_control() && c != '\n') {
        return Err(format!("Each box holds up to {MAX_TEXT} characters of plain text."));
    }
    Ok(text)
}

/// A design file and photos, read and cleaned (off the threads that answer requests).
async fn read_files(
    file_name: &str,
    bytes: Vec<u8>,
    uploaded: Vec<Vec<u8>>,
) -> Result<(Read, Vec<photos::Photo>), String> {
    let read_name = file_name.to_string();
    let read = match tokio::task::spawn_blocking(move || read_design(&bytes, &read_name)).await {
        Ok(Ok(r)) => r,
        Ok(Err(why)) => return Err(format!("{}: {why}.", file_name.chars().take(80).collect::<String>())),
        Err(_) => return Err("The file couldn't be read. Please try again.".into()),
    };
    let mut clean = Vec::new();
    for (i, photo) in uploaded.into_iter().enumerate() {
        match tokio::task::spawn_blocking(move || photos::read(&photo)).await {
            Ok(Ok(p)) => clean.push(p),
            Ok(Err(why)) => return Err(format!("Photo {}: {why}.", i + 1)),
            Err(_) => return Err(format!("Photo {} couldn't be read.", i + 1)),
        }
    }
    Ok((read, clean))
}

/// Files put in storage for a version, to note in the database (or delete if that fails).
struct Stored {
    file_key: String,
    file_bytes: i64,
    photos: Vec<(String, String, u32, u32)>,
    keys: Vec<String>,
}

async fn put_files(
    storage: &Storage,
    bytes: Vec<u8>,
    clean: Vec<photos::Photo>,
) -> Result<Stored, (String, Vec<String>)> {
    let mut keys = Vec::new();
    let file_bytes = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    let file_key = new_key("proof").map_err(|e| (e, keys.clone()))?;
    storage
        .put(&file_key, bytes, "application/octet-stream")
        .await
        .map_err(|e| (e, keys.clone()))?;
    keys.push(file_key.clone());
    let mut stored_photos = Vec::new();
    for p in clean {
        let photo_key = new_key("proof-photo").map_err(|e| (e, keys.clone()))?;
        storage
            .put(&photo_key, p.jpeg, "image/jpeg")
            .await
            .map_err(|e| (e, keys.clone()))?;
        keys.push(photo_key.clone());
        let thumb_key = new_key("proof-thumb").map_err(|e| (e, keys.clone()))?;
        storage
            .put(&thumb_key, p.thumb, "image/jpeg")
            .await
            .map_err(|e| (e, keys.clone()))?;
        keys.push(thumb_key.clone());
        stored_photos.push((photo_key, thumb_key, p.width, p.height));
    }
    Ok(Stored {
        file_key,
        file_bytes,
        photos: stored_photos,
        keys,
    })
}

async fn insert_version(
    conn: &mut PgConnection,
    proof_id: i64,
    number: i32,
    file_name: &str,
    read: &Read,
    stored: &Stored,
    note: &str,
) -> Result<(), String> {
    let (facts, svg, w, h) = match read {
        Read::Embroidery(e) => (serde_json::to_value(e), e.svg.clone(), e.width_mm, e.height_mm),
        Read::Laser(l) => (serde_json::to_value(l), l.svg.clone(), l.width_mm, l.height_mm),
    };
    let facts = facts.map_err(|e| e.to_string())?;
    let size = |v: f64| v.is_finite().then_some(v);
    let version: i64 = sqlx::query_scalar(
        "INSERT INTO mf_proof_versions (proof_id, number, kind, file_name, file_key, file_bytes, format, facts, preview_svg, width_mm, height_mm, note)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) RETURNING id",
    )
    .bind(proof_id)
    .bind(number)
    .bind(read.kind())
    .bind(file_name)
    .bind(&stored.file_key)
    .bind(stored.file_bytes)
    .bind(read.format())
    .bind(facts)
    .bind(svg)
    .bind(size(w))
    .bind(size(h))
    .bind(note)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;
    for (position, (photo_key, thumb_key, w, h)) in stored.photos.iter().enumerate() {
        sqlx::query(
            "INSERT INTO mf_proof_photos (version_id, photo_key, thumb_key, width, height, position) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(version)
        .bind(photo_key)
        .bind(thumb_key)
        .bind(i32::try_from(*w).unwrap_or(i32::MAX))
        .bind(i32::try_from(*h).unwrap_or(i32::MAX))
        .bind(i32::try_from(position).unwrap_or(i32::MAX))
        .execute(&mut *conn)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

const NOT_KEPT: &str = "The proof couldn't be kept just now. Please try again in a few minutes.";

// ---------------------------------------------------------------- the shop's pages

fn status_words(status: &str) -> &'static str {
    match status {
        "viewed" => "Viewed",
        "approved" => "Approved",
        "changes" => "Changes asked for",
        _ => "Sent",
    }
}

/// A proof's state as a badge.
fn status_badge(status: &str) -> String {
    format!(
        r#"<span class="badge status-{}">{}</span>"#,
        escape(status),
        status_words(status)
    )
}

pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (user, pool, _) = match shop(&state, &headers, "/proofs").await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let rows: Vec<(i64, String, String, String, i32, String)> = sqlx::query_as(
        "SELECT p.id, p.title, p.customer, p.status,
                (SELECT coalesce(max(v.number), 0) FROM mf_proof_versions v WHERE v.proof_id = p.id),
                to_char(p.active_at, 'YYYY-MM-DD')
         FROM mf_proofs p WHERE p.account_id = $1 ORDER BY p.active_at DESC LIMIT 500",
    )
    .bind(user.account_id)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();
    let items: String = rows
        .iter()
        .map(|(id, title, customer, status, version, at)| {
            let customer = if customer.is_empty() {
                String::new()
            } else {
                format!(" for {}", escape(customer))
            };
            format!(
                r#"<li class="report"><p><a href="/proofs/{id}"><strong>{title}</strong></a>{customer}</p><p>{badge} <span class="muted">version {version} · {at}</span></p></li>"#,
                title = escape(title),
                badge = status_badge(status),
                at = escape(at),
            )
        })
        .collect();
    let body = if rows.is_empty() {
        r#"<p class="lead">You haven't sent a proof yet.</p>"#.to_string()
    } else {
        format!(r#"<ul class="reports">{items}</ul>"#)
    };
    private_page(
        "Your proofs · MakerFoundry",
        &format!(
            r#"<section class="design"><h1>Your proofs</h1><p class="muted">Send a customer a design to approve before you stitch or cut it. A proof is deleted a year after its last activity.</p><p><a class="button" href="/proofs/new">Send a proof</a></p>{body}<p><a href="/account">Your account</a></p></section>"#
        ),
        true,
    )
}

fn new_form(problem: &str, up: &Upload, default_shop: &str) -> String {
    let value = |name: &str, fallback: &str| {
        let v = up.text(name);
        escape(if v.is_empty() { fallback } else { v })
    };
    format!(
        r#"<section class="form">
<h1>Send a proof</h1>
<p class="lead">Your customer gets a private link to see the design, comment, and approve it or ask for changes. They don't need an account.</p>
{problem}
<form method="post" action="/proofs/new" enctype="multipart/form-data">
<label>What it is <small>(the customer sees this)</small><input name="title" maxlength="{MAX_TITLE}" required value="{title}"></label>
<label>Your shop's name <small>(the customer sees this)</small><input name="shop_name" maxlength="{MAX_NAME}" required value="{shop}"></label>
<label>Who it's for <small>(only you see this)</small><input name="customer" maxlength="{MAX_NAME}" value="{customer}"></label>
<label>A message for the customer <small>(optional)</small><textarea name="message" rows="3" maxlength="{MAX_TEXT}">{message}</textarea></label>
<label>Design file <small>(embroidery: DST, PES, JEF, VP3 and more; laser: SVG, DXF, PDF)</small><input type="file" name="file" required></label>
<label>Mock-up photos <small>(optional: JPEG, PNG or WebP, up to {photos})</small><input type="file" name="photos" accept="image/jpeg,image/png,image/webp" multiple></label>
<button type="submit">Make the proof</button>
</form>
<p class="muted">Photos are saved again without their hidden details (such as where they were taken).</p>
<p><a href="/proofs">Your proofs</a></p>
</section>"#,
        problem = problem_html(problem),
        title = value("title", ""),
        shop = value("shop_name", default_shop),
        customer = value("customer", ""),
        message = escape(up.text("message")),
        photos = designs::MAX_PHOTOS,
    )
}

async fn default_shop_name(pool: &PgPool, account: i64) -> String {
    crate::makers::profile(pool, account)
        .await
        .map(|m| m.name)
        .unwrap_or_default()
}

pub async fn new_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (user, pool, _) = match shop(&state, &headers, "/proofs/new").await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if state.storage.is_none() {
        return problem_page("Send a proof", "Keeping files isn't switched on yet.");
    }
    let shop_name = default_shop_name(&pool, user.account_id).await;
    private_page(
        "Send a proof · MakerFoundry",
        &new_form("", &Upload::default(), &shop_name),
        true,
    )
}

pub async fn create(State(state): State<AppState>, headers: HeaderMap, multipart: Multipart) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool, _) = match shop(&state, &headers, "/proofs/new").await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let Some(storage) = state.storage.as_ref() else {
        return problem_page("Send a proof", "Keeping files isn't switched on yet.");
    };
    let shop_name = default_shop_name(&pool, user.account_id).await;
    let up = match read_upload(multipart).await {
        Ok(up) => up,
        Err(why) => {
            return private_page(
                "Send a proof · MakerFoundry",
                &new_form(&why, &Upload::default(), &shop_name),
                true,
            );
        }
    };
    let Upload {
        text,
        file,
        photos: uploaded,
    } = up;
    let form = Upload {
        text,
        ..Upload::default()
    };
    let again = |why: &str| private_page("Send a proof · MakerFoundry", &new_form(why, &form, &shop_name), true);
    let fields = (|| {
        Ok::<_, String>((
            line(form.text("title"), "what it is", MAX_TITLE, true)?,
            line(form.text("shop_name"), "your shop's name", MAX_NAME, true)?,
            line(form.text("customer"), "who it's for", MAX_NAME, false)?,
            passage(form.text("message"))?,
        ))
    })();
    let (title, shop_name_given, customer, message) = match fields {
        Ok(f) => f,
        Err(why) => return again(&why),
    };
    let Some((file_name, bytes)) = file else {
        return again("Choose a design file.");
    };
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM mf_proofs WHERE account_id = $1")
        .bind(user.account_id)
        .fetch_one(&pool)
        .await
        .unwrap_or(0);
    if kept >= MAX_PROOFS {
        return again(&format!(
            "A shop can keep up to {MAX_PROOFS} proofs. Delete some you're done with to send more."
        ));
    }
    if !state.limits.allow(limits::PROOF_UPLOAD, &user.account_id.to_string()) {
        return again("You've sent a lot of proofs today. Please try again tomorrow.");
    }
    let (read, clean) = match read_files(&file_name, bytes.clone(), uploaded).await {
        Ok(r) => r,
        Err(why) => return again(&why),
    };
    let stored = match put_files(storage, bytes, clean).await {
        Ok(s) => s,
        Err((e, keys)) => {
            eprintln!("proofs: store: {e}");
            delete_files(Some(storage), &keys).await;
            return again(NOT_KEPT);
        }
    };
    let made = async {
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO mf_proofs (account_id, title, shop_name, customer, message) VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(user.account_id)
        .bind(&title)
        .bind(&shop_name_given)
        .bind(&customer)
        .bind(&message)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        insert_version(&mut tx, id, 1, &file_name, &read, &stored, "").await?;
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok::<_, String>(id)
    }
    .await;
    match made {
        Ok(id) => Redirect::to(&format!("/proofs/{id}")).into_response(),
        Err(e) => {
            eprintln!("proofs: new: {e}");
            delete_files(Some(storage), &stored.keys).await;
            again(NOT_KEPT)
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct VersionQuery {
    v: Option<i32>,
}

/// A version's photos as thumbnails opening the large ones, under `base` (the photo addresses'
/// start: the shop's or the customer's).
async fn photos_html(pool: &PgPool, version: i64, base: &str) -> String {
    let list: Vec<(i64, i32, i32)> =
        sqlx::query_as("SELECT id, width, height FROM mf_proof_photos WHERE version_id = $1 ORDER BY position")
            .bind(version)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let html: String = list
        .iter()
        .enumerate()
        .map(|(n, (pid, w, h))| {
            let tw = photos::THUMB_SIDE.min(u32::try_from(*w).unwrap_or(1)).max(1);
            let th = (f64::from(*h) * f64::from(tw) / f64::from((*w).max(1))).round();
            format!(
                r#"<a href="{base}/photos/{pid}/large"><img src="{base}/photos/{pid}/thumb" alt="Mock-up {n}" width="{tw}" height="{th}" loading="lazy"></a>"#,
                n = n + 1,
            )
        })
        .collect();
    if html.is_empty() {
        String::new()
    } else {
        format!(r#"<h2>Mock-ups</h2><div class="photos">{html}</div>"#)
    }
}

/// The conversation under a proof, oldest first (`shop_name` names the shop's replies).
async fn replies_html(pool: &PgPool, proof_id: i64, shop_name: &str) -> String {
    let rows: Vec<(i32, bool, String, String, String, String)> = sqlx::query_as(
        "SELECT version, by_shop, name, body, decision, to_char(at, 'YYYY-MM-DD HH24:MI')
         FROM mf_proof_replies WHERE proof_id = $1 ORDER BY at, id LIMIT $2",
    )
    .bind(proof_id)
    .bind(SHOWN_REPLIES)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    if rows.is_empty() {
        return r#"<h2>Conversation</h2><p class="muted">No comments yet.</p>"#.to_string();
    }
    let items: String = rows
        .iter()
        .map(|(version, by_shop, name, body, decision, at)| {
            let who = if *by_shop { escape(shop_name) } else { escape(name) };
            let what = match decision.as_str() {
                "approved" => format!(r#" <span class="badge status-approved">approved version {version}</span>"#),
                "changes" => format!(r#" <span class="badge status-changes">asked for changes to version {version}</span>"#),
                _ => format!(r#" <span class="muted">on version {version}</span>"#),
            };
            let shop_badge = if *by_shop { r#" <span class="badge">the shop</span>"# } else { "" };
            format!(
                r#"<li class="comment"><p class="muted"><strong>{who}</strong>{shop_badge}{what} · {at} UTC</p>{body}</li>"#,
                at = escape(at),
                body = paragraphs(body),
            )
        })
        .collect();
    format!(r#"<h2>Conversation</h2><ol class="comments">{items}</ol>"#)
}

/// Links to a proof's versions (`base` + `?v=`), the one shown marked.
fn versions_nav(all: &[Version], shown: i32, base: &str) -> String {
    if all.len() < 2 {
        return String::new();
    }
    let links: Vec<String> = all
        .iter()
        .map(|v| {
            let label = if Some(v.number) == all.first().map(|f| f.number) {
                format!("Version {} (latest)", v.number)
            } else {
                format!("Version {}", v.number)
            };
            if v.number == shown {
                format!("<strong>{label}</strong>")
            } else {
                format!(r#"<a href="{base}?v={n}">{label}</a>"#, n = v.number)
            }
        })
        .collect();
    format!(r#"<p class="versions">{}</p>"#, links.join(" · "))
}

/// The address a customer opens proof `id` at, if its link is on.
fn customer_link(state: &AppState, keys: &Proofs, id: i64, p: &ProofRow) -> Option<String> {
    if !p.link_on {
        return None;
    }
    keys.token(id, p.link_generation)
        .map(|t| format!("{}/p/{id}/{t}", state.public_url))
}

/// The shop's own page for a proof: its link, the version asked for (the latest by default), the
/// conversation, a reply, a new version, and deleting it; `problem` and `note` above.
async fn manage_page(
    state: &AppState,
    pool: &PgPool,
    keys: &Proofs,
    user: &User,
    id: i64,
    shown: Option<i32>,
    problem: &str,
) -> Response {
    let p = match own_proof(pool, user, id).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let all = versions(pool, id).await;
    let latest = all.first().map_or(0, |v| v.number);
    let Some(v) = all.iter().find(|v| v.number == shown.unwrap_or(latest)) else {
        return not_found();
    };
    let link = match customer_link(state, keys, id, &p) {
        Some(url) => format!(
            r#"<label>The customer's link <small>(copy it into an email or a message; anyone with it can see this proof)</small><input readonly value="{url}"></label>
<form method="post" action="/proofs/{id}/link"><input type="hidden" name="action" value="new"><button type="submit" class="secondary">Make a new link</button> <small class="muted">The old link stops working.</small></form>
<form method="post" action="/proofs/{id}/link"><input type="hidden" name="action" value="off"><button type="submit" class="secondary">Turn the link off</button></form>"#,
            url = escape(&url),
        ),
        None => format!(
            r#"<p class="muted">The customer's link is off.</p>
<form method="post" action="/proofs/{id}/link"><input type="hidden" name="action" value="on"><button type="submit" class="secondary">Turn it back on</button></form>"#
        ),
    };
    let customer = if p.customer.is_empty() {
        String::new()
    } else {
        format!(r#" <span class="muted">for {}</span>"#, escape(&p.customer))
    };
    let base = format!("/proofs/{id}");
    let note = if v.note.is_empty() {
        String::new()
    } else {
        format!(r#"<blockquote class="quoted">{}</blockquote>"#, paragraphs(&v.note))
    };
    let intro = format!(
        r#"{problem}<p>{badge}{customer} <span class="muted">· version {n} of {latest}, sent {at} UTC</span></p>
{nav}{note}"#,
        problem = problem_html(problem),
        badge = status_badge(&p.status),
        n = v.number,
        at = escape(&v.at),
        nav = versions_nav(&all, v.number, &base),
    );
    let photos = photos_html(pool, v.id, &base).await;
    let conversation = replies_html(pool, id, &p.shop_name).await;
    let message = if p.message.is_empty() {
        String::new()
    } else {
        format!(r#"<h2>Your message</h2>{}"#, paragraphs(&p.message))
    };
    let more = if latest >= MAX_VERSIONS {
        format!(r#"<p class="muted">A proof can have up to {MAX_VERSIONS} versions. Send a new proof for more.</p>"#)
    } else {
        format!(
            r#"<h2>Send a new version</h2>
<form method="post" action="/proofs/{id}/versions" enctype="multipart/form-data" class="add-file">
<label>Design file<input type="file" name="file" required></label>
<label>Mock-up photos <small>(optional, up to {photos})</small><input type="file" name="photos" accept="image/jpeg,image/png,image/webp" multiple></label>
<label>What changed <small>(the customer sees this)</small><textarea name="note" rows="2" maxlength="{MAX_TEXT}"></textarea></label>
<button type="submit" class="secondary">Send version {next}</button>
</form>"#,
            photos = designs::MAX_PHOTOS,
            next = latest + 1,
        )
    };
    let footer = format!(
        r#"{photos}
<h2>Sharing</h2>
<div class="share">{link}</div>
{message}
{conversation}
<form method="post" action="/proofs/{id}/reply" class="add-file">
<label>Reply to the customer <small>(plain text)</small><textarea name="body" rows="3" maxlength="{MAX_TEXT}" required></textarea></label>
<button type="submit" class="secondary">Send the reply</button>
</form>
{more}
<h2>Delete</h2>
<form method="post" action="/proofs/{id}/delete"><button type="submit" class="secondary">Delete this proof</button> <small class="muted">The link stops working and the files go.</small></form>
<p><a href="/proofs">Your proofs</a></p>"#
    );
    match drawing_section(&v.kind, &v.facts, &v.preview_svg, &p.title, &intro, &footer) {
        Some(html) => private_page(&format!("{} · MakerFoundry", escape(&p.title)), &html, true),
        None => not_found(),
    }
}

pub async fn show(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(q): Query<VersionQuery>,
) -> Response {
    let (user, pool, keys) = match shop(&state, &headers, &format!("/proofs/{id}")).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    manage_page(&state, &pool, &keys, &user, id, q.v, "").await
}

pub async fn add_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool, keys) = match shop(&state, &headers, &format!("/proofs/{id}")).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(r) = own_proof(&pool, &user, id).await {
        return r;
    }
    let Some(storage) = state.storage.as_ref() else {
        return manage_page(
            &state,
            &pool,
            &keys,
            &user,
            id,
            None,
            "Keeping files isn't switched on yet.",
        )
        .await;
    };
    let up = match read_upload(multipart).await {
        Ok(up) => up,
        Err(why) => return manage_page(&state, &pool, &keys, &user, id, None, &why).await,
    };
    let note = match passage(up.text("note")) {
        Ok(n) => n,
        Err(why) => return manage_page(&state, &pool, &keys, &user, id, None, &why).await,
    };
    let Some((file_name, bytes)) = up.file else {
        return manage_page(&state, &pool, &keys, &user, id, None, "Choose a design file.").await;
    };
    if !state.limits.allow(limits::PROOF_UPLOAD, &user.account_id.to_string()) {
        return manage_page(
            &state,
            &pool,
            &keys,
            &user,
            id,
            None,
            "You've sent a lot of proofs today. Please try again tomorrow.",
        )
        .await;
    }
    let (read, clean) = match read_files(&file_name, bytes.clone(), up.photos).await {
        Ok(r) => r,
        Err(why) => return manage_page(&state, &pool, &keys, &user, id, None, &why).await,
    };
    let stored = match put_files(storage, bytes, clean).await {
        Ok(s) => s,
        Err((e, stored_keys)) => {
            eprintln!("proofs: store: {e}");
            delete_files(Some(storage), &stored_keys).await;
            return manage_page(&state, &pool, &keys, &user, id, None, NOT_KEPT).await;
        }
    };
    let made = async {
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        // (the proof's row locked, so two versions sent at once get different numbers)
        let latest: Option<i32> = sqlx::query_scalar(
            "SELECT (SELECT coalesce(max(v.number), 0) FROM mf_proof_versions v WHERE v.proof_id = p.id)
             FROM mf_proofs p WHERE p.id = $1 AND p.account_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(user.account_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        let Some(latest) = latest else {
            return Err("gone".to_string());
        };
        if latest >= MAX_VERSIONS {
            return Ok(false);
        }
        insert_version(&mut tx, id, latest + 1, &file_name, &read, &stored, &note).await?;
        sqlx::query("UPDATE mf_proofs SET status = 'sent', active_at = now() WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok::<_, String>(true)
    }
    .await;
    match made {
        Ok(true) => Redirect::to(&format!("/proofs/{id}")).into_response(),
        Ok(false) => {
            delete_files(Some(storage), &stored.keys).await;
            manage_page(
                &state,
                &pool,
                &keys,
                &user,
                id,
                None,
                &format!("A proof can have up to {MAX_VERSIONS} versions. Send a new proof for more."),
            )
            .await
        }
        Err(e) => {
            eprintln!("proofs: new version of {id}: {e}");
            delete_files(Some(storage), &stored.keys).await;
            manage_page(&state, &pool, &keys, &user, id, None, NOT_KEPT).await
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct LinkForm {
    action: String,
}

/// The customer's link: a new one (the old stops working), off, or back on.
pub async fn link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(f): Form<LinkForm>,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool, keys) = match shop(&state, &headers, &format!("/proofs/{id}")).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let sql = match f.action.as_str() {
        "new" => {
            "UPDATE mf_proofs SET link_generation = link_generation + 1, link_on = true, active_at = now() WHERE id = $1 AND account_id = $2"
        }
        "off" => "UPDATE mf_proofs SET link_on = false, active_at = now() WHERE id = $1 AND account_id = $2",
        "on" => "UPDATE mf_proofs SET link_on = true, active_at = now() WHERE id = $1 AND account_id = $2",
        _ => return manage_page(&state, &pool, &keys, &user, id, None, "").await,
    };
    if let Err(e) = sqlx::query(sql).bind(id).bind(user.account_id).execute(&pool).await {
        eprintln!("proofs: link of {id}: {e}");
        return manage_page(
            &state,
            &pool,
            &keys,
            &user,
            id,
            None,
            "The link couldn't be changed just now.",
        )
        .await;
    }
    Redirect::to(&format!("/proofs/{id}")).into_response()
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ShopReply {
    body: String,
}

/// The shop's reply in the conversation.
pub async fn shop_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(f): Form<ShopReply>,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool, keys) = match shop(&state, &headers, &format!("/proofs/{id}")).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(r) = own_proof(&pool, &user, id).await {
        return r;
    }
    let body = match passage(&f.body) {
        Ok(b) if !b.is_empty() => b,
        Ok(_) => return manage_page(&state, &pool, &keys, &user, id, None, "Write something first.").await,
        Err(why) => return manage_page(&state, &pool, &keys, &user, id, None, &why).await,
    };
    if !state.limits.allow(limits::PROOF_REPLY, &user.account_id.to_string()) {
        return manage_page(
            &state,
            &pool,
            &keys,
            &user,
            id,
            None,
            "That's a lot of replies: please wait a while.",
        )
        .await;
    }
    if let Err(e) = add_reply(&pool, id, true, "", &body, "comment").await {
        eprintln!("proofs: reply on {id}: {e}");
        return manage_page(
            &state,
            &pool,
            &keys,
            &user,
            id,
            None,
            "The reply couldn't be sent just now.",
        )
        .await;
    }
    Redirect::to(&format!("/proofs/{id}")).into_response()
}

/// A reply on the latest version of proof `id` (a decision also changes the proof's state).
async fn add_reply(
    pool: &PgPool,
    id: i64,
    by_shop: bool,
    name: &str,
    body: &str,
    decision: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let latest: i32 = sqlx::query_scalar("SELECT coalesce(max(number), 0) FROM mf_proof_versions WHERE proof_id = $1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO mf_proof_replies (proof_id, version, by_shop, name, body, decision) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(latest)
    .bind(by_shop)
    .bind(name)
    .bind(body)
    .bind(decision)
    .execute(&mut *tx)
    .await?;
    let status = match decision {
        "approved" => "approved",
        "changes" => "changes",
        _ => "",
    };
    sqlx::query(
        "UPDATE mf_proofs SET active_at = now(), status = CASE WHEN $2 = '' THEN status ELSE $2 END WHERE id = $1",
    )
    .bind(id)
    .bind(status)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

pub async fn delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool, keys) = match shop(&state, &headers, &format!("/proofs/{id}")).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(r) = own_proof(&pool, &user, id).await {
        return r;
    }
    let gone = async {
        let mut tx = pool.begin().await?;
        let files = erase(&mut tx, &[id]).await?;
        tx.commit().await?;
        Ok::<_, sqlx::Error>(files)
    }
    .await;
    match gone {
        Ok(files) => {
            delete_files(state.storage.as_ref(), &files).await;
            Redirect::to("/proofs").into_response()
        }
        Err(e) => {
            eprintln!("proofs: delete {id}: {e}");
            manage_page(
                &state,
                &pool,
                &keys,
                &user,
                id,
                None,
                "The proof couldn't be deleted just now.",
            )
            .await
        }
    }
}

/// A photo of proof `id` (`size` thumb or large), if it is one of its versions'.
async fn photo_of(state: &AppState, pool: &PgPool, id: i64, photo_id: i64, size: &str) -> Response {
    let column = match size {
        "thumb" => "thumb_key",
        "large" => "photo_key",
        _ => return not_found(),
    };
    let key: Option<String> = sqlx::query_scalar(&format!(
        "SELECT p.{column} FROM mf_proof_photos p JOIN mf_proof_versions v ON v.id = p.version_id
         WHERE p.id = $1 AND v.proof_id = $2"
    ))
    .bind(photo_id)
    .bind(id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let (Some(key), Some(storage)) = (key, state.storage.as_ref()) else {
        return not_found();
    };
    match storage.get(&key).await {
        Ok(Some(bytes)) => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "private, max-age=3600"),
            ],
            bytes,
        )
            .into_response(),
        Ok(None) => not_found(),
        Err(e) => {
            eprintln!("proofs: photo {photo_id}: {e}");
            (StatusCode::BAD_GATEWAY, "The photo can't be fetched just now.").into_response()
        }
    }
}

pub async fn shop_photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, photo_id, size)): Path<(i64, i64, String)>,
) -> Response {
    let (user, pool, _) = match shop(&state, &headers, &format!("/proofs/{id}")).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if let Err(r) = own_proof(&pool, &user, id).await {
        return r;
    }
    photo_of(&state, &pool, id, photo_id, &size).await
}

// ---------------------------------------------------------------- the customer's link

/// The proof a customer's link opens, if the link is good and proofs are on for them: everyone
/// once switched on, admins before.
async fn by_link(
    state: &AppState,
    headers: &HeaderMap,
    id: i64,
    token: &str,
) -> Option<(PgPool, ProofRow, Option<User>)> {
    let keys = state.proofs.as_ref()?;
    let pool = state.db.clone()?;
    let viewer = current_user(state, headers).await;
    if !keys.open && !viewer.as_ref().is_some_and(|u| state.admins.contains(&u.account_id)) {
        return None;
    }
    let p = proof(&pool, id).await.ok().flatten()?;
    (p.link_on && keys.token_good(id, p.link_generation, token)).then_some((pool, p, viewer))
}

pub async fn customer_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, token)): Path<(i64, String)>,
    Query(q): Query<VersionQuery>,
) -> Response {
    let Some((pool, p, viewer)) = by_link(&state, &headers, id, &token).await else {
        return not_found();
    };
    // the shop (or an admin) looking at it isn't the customer seeing it
    let staff = viewer
        .as_ref()
        .is_some_and(|u| u.account_id == p.account_id || state.admins.contains(&u.account_id));
    if !staff
        && let Err(e) = sqlx::query(
            "UPDATE mf_proofs SET active_at = now(), status = CASE WHEN status = 'sent' THEN 'viewed' ELSE status END WHERE id = $1",
        )
        .bind(id)
        .execute(&pool)
        .await
    {
        eprintln!("proofs: viewed {id}: {e}");
    }
    customer_view(&pool, &p, id, &token, q.v, "", viewer.is_some()).await
}

async fn customer_view(
    pool: &PgPool,
    p: &ProofRow,
    id: i64,
    token: &str,
    shown: Option<i32>,
    problem: &str,
    signed_in: bool,
) -> Response {
    let all = versions(pool, id).await;
    let latest = all.first().map_or(0, |v| v.number);
    let Some(v) = all.iter().find(|v| v.number == shown.unwrap_or(latest)) else {
        return not_found();
    };
    let base = format!("/p/{id}/{token}");
    let message = if p.message.is_empty() {
        String::new()
    } else {
        paragraphs(&p.message)
    };
    let note = if v.note.is_empty() {
        String::new()
    } else {
        format!(
            r#"<p class="muted">What changed in version {}:</p><blockquote class="quoted">{}</blockquote>"#,
            v.number,
            paragraphs(&v.note)
        )
    };
    let state_line = match p.status.as_str() {
        "approved" => format!(
            r#"<p>{} <span class="muted">The shop has your approval.</span></p>"#,
            status_badge("approved")
        ),
        "changes" => format!(
            r#"<p>{} <span class="muted">The shop will send a new version.</span></p>"#,
            status_badge("changes")
        ),
        _ => String::new(),
    };
    let older = if v.number == latest {
        String::new()
    } else {
        format!(
            r#"<p class="problem">This is version {n}. <a href="{base}">See the latest, version {latest}</a>.</p>"#,
            n = v.number
        )
    };
    let intro = format!(
        r#"<p class="lead">A proof from <strong>{shop}</strong> for you to check{v}.</p>
{problem}{older}{state_line}{message}{nav}{note}"#,
        shop = escape(&p.shop_name),
        v = if latest > 1 {
            format!(", version {}", v.number)
        } else {
            String::new()
        },
        problem = problem_html(problem),
        nav = versions_nav(&all, v.number, &base),
    );
    let photos = photos_html(pool, v.id, &base).await;
    let conversation = replies_html(pool, id, &p.shop_name).await;
    let decide = if v.number == latest {
        format!(
            r#"<h2>Your answer</h2>
<form method="post" action="{base}" class="add-file reply">
<input type="hidden" name="version" value="{latest}">
<label>Your name<input name="name" maxlength="{MAX_NAME}" required autocomplete="name"></label>
<label>Comments <small>(what to change, or anything else; plain text)</small><textarea name="body" rows="3" maxlength="{MAX_TEXT}"></textarea></label>
<p class="actions">{approve}<button type="submit" name="decision" value="changes" class="secondary">Ask for changes</button>
<button type="submit" name="decision" value="comment" class="secondary">Just comment</button></p>
</form>
<p class="muted">Your name and comments go to {shop}, who sent you this link. Approving tells them to go ahead with version {latest} as shown.</p>"#,
            shop = escape(&p.shop_name),
            // (approved already: nothing more to approve, but they may still change their mind)
            approve = if p.status == "approved" {
                String::new()
            } else {
                format!(
                    r#"<button type="submit" name="decision" value="approved">Approve version {latest}</button>
"#
                )
            },
        )
    } else {
        String::new()
    };
    let format_line = format!(
        r#"<p class="muted">The drawing is made from the {format} file itself: stitches or lines as the machine will run them.</p>"#,
        format = escape(&v.format),
    );
    let footer = format!("{format_line}{photos}{conversation}{decide}");
    match drawing_section(&v.kind, &v.facts, &v.preview_svg, &p.title, &intro, &footer) {
        Some(html) => private_page(
            &format!("{} · a proof from {}", escape(&p.title), escape(&p.shop_name)),
            &html,
            signed_in,
        ),
        None => not_found(),
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CustomerReply {
    name: String,
    body: String,
    decision: String,
    version: i32,
}

pub async fn customer_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, token)): Path<(i64, String)>,
    Form(f): Form<CustomerReply>,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let Some((pool, p, viewer)) = by_link(&state, &headers, id, &token).await else {
        return not_found();
    };
    let signed_in = viewer.is_some();
    let problem = check_answer(&state, &pool, &headers, id, &f).await;
    let why = match problem {
        Ok((name, body, decision)) => match add_reply(&pool, id, false, &name, &body, decision).await {
            Ok(()) => return Redirect::to(&format!("/p/{id}/{token}")).into_response(),
            Err(e) => {
                eprintln!("proofs: answer on {id}: {e}");
                "Your answer couldn't be sent just now. Please try again."
            }
        },
        Err(why) => why,
    };
    customer_view(&pool, &p, id, &token, None, why, signed_in).await
}

/// A customer's answer checked → their name, comments and decision; or why it won't do.
async fn check_answer(
    state: &AppState,
    pool: &PgPool,
    headers: &HeaderMap,
    id: i64,
    f: &CustomerReply,
) -> Result<(String, String, &'static str), &'static str> {
    let decision = match f.decision.as_str() {
        "approved" => "approved",
        "changes" => "changes",
        "comment" => "comment",
        _ => return Err("Choose to approve, ask for changes or comment."),
    };
    let name = line(&f.name, "your name", MAX_NAME, true)
        .map_err(|_| "Give your name (up to 100 characters), so the shop knows who answered.")?;
    let body = passage(&f.body).map_err(|_| "Comments can be up to 2,000 characters of plain text.")?;
    if decision != "approved" && body.is_empty() {
        return Err("Write what you'd like changed, or your comment.");
    }
    let latest: i32 = sqlx::query_scalar("SELECT coalesce(max(number), 0) FROM mf_proof_versions WHERE proof_id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    if f.version != latest {
        return Err("The shop has sent a newer version since: have a look at it first.");
    }
    if !state.limits.allow(limits::PROOF_REPLY, &limits::client(headers)) {
        return Err("That's a lot of answers from here lately. Please wait a while and try again.");
    }
    Ok((name, body, decision))
}

pub async fn customer_photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, token, photo_id, size)): Path<(i64, String, i64, String)>,
) -> Response {
    let Some((pool, _, _)) = by_link(&state, &headers, id, &token).await else {
        return not_found();
    };
    photo_of(&state, &pool, id, photo_id, &size).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_opens_only_its_own_proof_at_its_own_generation() {
        let keys = Proofs::new(&[7u8; 32], true).unwrap();
        let t = keys.token(42, 1).unwrap();
        assert_eq!(t.len(), 64);
        assert!(keys.token_good(42, 1, &t));
        assert!(!keys.token_good(43, 1, &t));
        assert!(!keys.token_good(42, 2, &t));
        assert!(!keys.token_good(42, 1, &t.to_uppercase().replace('A', "B")));
        assert!(!keys.token_good(42, 1, &t[..62]));
        assert!(!keys.token_good(42, 1, "zz"));
        // another key makes other links
        let other = Proofs::new(&[8u8; 32], true).unwrap();
        assert!(!other.token_good(42, 1, &t));
    }

    #[test]
    fn the_key_must_be_long_hex() {
        let env = |key: &'static str, open: &'static str| {
            move |k: &str| match k {
                "MF_PROOF_KEY" => Some(key.to_string()),
                "MF_PROOFS" => Some(open.to_string()),
                _ => None,
            }
        };
        assert!(Proofs::from_env(env("", "yes")).unwrap().is_none());
        assert!(Proofs::from_env(env("abcd", "yes")).is_err());
        assert!(Proofs::from_env(env("zz".repeat(32).leak(), "yes")).is_err());
        let p = Proofs::from_env(env("ab".repeat(32).leak(), "yes")).unwrap().unwrap();
        assert!(p.open);
        let p = Proofs::from_env(env("ab".repeat(32).leak(), "no")).unwrap().unwrap();
        assert!(!p.open);
        assert!(!format!("{p:?}").contains("ab"));
    }

    #[test]
    fn typed_text_is_checked() {
        assert_eq!(line("  Rose   Café ", "x", 100, true), Ok("Rose Café".to_string()));
        assert!(line("  ", "your name", 100, true).is_err());
        assert_eq!(line("", "who", 100, false), Ok(String::new()));
        assert!(line(&"a".repeat(101), "x", 100, false).is_err());
        assert!(line("Ana\u{7}", "your name", 100, true).is_err());
        assert_eq!(passage(" a\r\nb "), Ok("a\nb".to_string()));
        assert!(passage("bell\u{7}").is_err());
        assert!(passage(&"a".repeat(MAX_TEXT + 1)).is_err());
    }
}
