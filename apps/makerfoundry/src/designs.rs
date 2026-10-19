//! A maker's designs (MF-2): upload a design file and photos of the stitched or cut result; the
//! file is read and checked (embroidery or laser), the photos cleaned (`photos`), all of it kept in
//! storage (`storage`) and noted in the database. Designs are drafts, seen only by their maker,
//! until MF-3 adds their details and publishes them.

use axum::extract::{Multipart, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use sqlx::{PgPool, Row};

use crate::AppState;
use crate::accounts::{User, current_user, forbidden, private, same_origin};
use crate::embroidery::{self, Embroidery};
use crate::laser::{self, Laser};
use crate::pages::{escape, page};
use crate::photos;
use crate::preview::{laser_result, result};
use crate::storage::{Storage, new_key};

/// Photos a design may have.
pub const MAX_PHOTOS: usize = 6;
/// The most an upload (a design and its photos) may be, all told.
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Html(page(
            "Not found · MakerFoundry",
            r#"<section class="hero"><h1>Design not found</h1><p class="lead"><a href="/designs">Your designs</a></p></section>"#,
            true,
        )),
    )
        .into_response()
}

fn problem_page(title: &str, message: &str) -> Response {
    private(
        &format!("{title} · MakerFoundry"),
        &format!(
            r#"<section class="form"><h1>{}</h1><p class="lead">{}</p></section>"#,
            escape(title),
            message
        ),
        true,
    )
}

/// The signed-in maker, or the way to sign in.
async fn maker(state: &AppState, headers: &HeaderMap) -> Result<(User, PgPool), Response> {
    let Some(user) = current_user(state, headers).await else {
        return Err(Redirect::to("/signin").into_response());
    };
    let Some(pool) = state.db.clone() else {
        return Err(Redirect::to("/signin").into_response());
    };
    Ok((user, pool))
}

// ---------------------------------------------------------------- the upload

fn upload_form(problem: &str) -> String {
    let problem = if problem.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem" role="alert">{}</p>"#, escape(problem))
    };
    format!(
        r#"<section class="form">
<h1>Add a design</h1>
<p class="lead">Upload your design file and photos of what it makes. It stays a draft, seen only by you, until you publish it.</p>
{problem}
<form method="post" action="/designs/new" enctype="multipart/form-data">
<label>Design file <small>(embroidery: DST, PES, JEF, VP3 and more; laser: SVG, DXF, PDF)</small><input type="file" name="file" required></label>
<label>Photos of the stitched or cut result <small>(JPEG, PNG or WebP, up to {max} photos)</small><input type="file" name="photos" accept="image/jpeg,image/png,image/webp" multiple></label>
<button type="submit">Add the design</button>
</form>
<p class="muted">Photos are saved again without their hidden details (such as where they were taken).</p>
</section>"#,
        max = MAX_PHOTOS,
    )
}

pub async fn new_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = maker(&state, &headers).await {
        return r;
    }
    if state.storage.is_none() {
        return problem_page(
            "Add a design",
            r#"Keeping designs isn't switched on yet. You can still <a href="/preview">see how a file reads</a>."#,
        );
    }
    private("Add a design · MakerFoundry", &upload_form(""), true)
}

/// A design file, read: what kind it is and what was found.
enum Read {
    Embroidery(Embroidery),
    Laser(Laser),
}

impl Read {
    fn kind(&self) -> &'static str {
        match self {
            Read::Embroidery(_) => "embroidery",
            Read::Laser(_) => "laser",
        }
    }
}

fn read_design(bytes: &[u8], name: &str) -> Result<Read, String> {
    match laser::kind(bytes, name) {
        Some(kind) => laser::read(bytes, kind).map(Read::Laser),
        None => embroidery::read(bytes, name).map(Read::Embroidery),
    }
}

pub async fn upload(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool) = match maker(&state, &headers).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    let Some(storage) = state.storage.clone() else {
        return Redirect::to("/designs/new").into_response();
    };
    let again = |why: &str| private("Add a design · MakerFoundry", &upload_form(why), true);
    // the form's parts: one design file, some photos
    let mut file: Option<(String, Vec<u8>)> = None;
    let mut uploaded_photos: Vec<Vec<u8>> = Vec::new();
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or_default().to_string();
                let file_name = field
                    .file_name()
                    .unwrap_or("design")
                    .chars()
                    .take(200)
                    .collect::<String>();
                let Ok(bytes) = field.bytes().await else {
                    return again(&format!(
                        "The upload didn't come through whole. It can be up to {} MB in all.",
                        MAX_UPLOAD_BYTES / (1024 * 1024)
                    ));
                };
                match name.as_str() {
                    "file" if !bytes.is_empty() => file = Some((file_name, bytes.to_vec())),
                    // (a form sent without any photo chosen has one empty part)
                    "photos" if !bytes.is_empty() => {
                        if uploaded_photos.len() >= MAX_PHOTOS {
                            return again(&format!("A design can have up to {MAX_PHOTOS} photos."));
                        }
                        uploaded_photos.push(bytes.to_vec());
                    }
                    _ => {}
                }
            }
            Ok(None) => break,
            Err(_) => {
                return again(&format!(
                    "The upload didn't come through whole. It can be up to {} MB in all.",
                    MAX_UPLOAD_BYTES / (1024 * 1024)
                ));
            }
        }
    }
    let Some((file_name, bytes)) = file else {
        return again("Choose a design file to upload.");
    };
    // reading and photo work is CPU-heavy: off the threads that answer requests
    let read_name = file_name.clone();
    let read_bytes = bytes.clone();
    let read = match tokio::task::spawn_blocking(move || read_design(&read_bytes, &read_name)).await {
        Ok(Ok(r)) => r,
        Ok(Err(why)) => return again(&format!("{}: {why}.", file_name.chars().take(80).collect::<String>())),
        Err(_) => return again("The file couldn't be read. Please try again."),
    };
    let mut clean = Vec::new();
    for (i, photo) in uploaded_photos.into_iter().enumerate() {
        match tokio::task::spawn_blocking(move || photos::read(&photo)).await {
            Ok(Ok(p)) => clean.push(p),
            Ok(Err(why)) => return again(&format!("Photo {}: {why}.", i + 1)),
            Err(_) => return again(&format!("Photo {} couldn't be read.", i + 1)),
        }
    }
    match keep(&storage, &pool, &user, &file_name, bytes, &read, clean).await {
        Ok(id) => Redirect::to(&format!("/designs/{id}")).into_response(),
        Err(why) => {
            eprintln!("designs: {why}");
            again("The design couldn't be kept just now. Please try again in a few minutes.")
        }
    }
}

/// Stores the files, then notes the design; if noting fails, the stored files go again.
async fn keep(
    storage: &Storage,
    pool: &PgPool,
    user: &User,
    file_name: &str,
    bytes: Vec<u8>,
    read: &Read,
    clean: Vec<photos::Photo>,
) -> Result<i64, String> {
    let mut stored: Vec<String> = Vec::new();
    let result = async {
        let file_key = new_key("design")?;
        let file_bytes = bytes.len() as i64;
        storage.put(&file_key, bytes, "application/octet-stream").await?;
        stored.push(file_key.clone());
        let mut photo_keys = Vec::new();
        for p in clean {
            let (photo_key, thumb_key) = (new_key("photo")?, new_key("thumb")?);
            storage.put(&photo_key, p.jpeg, "image/jpeg").await?;
            stored.push(photo_key.clone());
            storage.put(&thumb_key, p.thumb, "image/jpeg").await?;
            stored.push(thumb_key.clone());
            photo_keys.push((photo_key, thumb_key, p.width, p.height));
        }
        let (format, facts, svg) = match read {
            Read::Embroidery(e) => (e.format.clone(), serde_json::to_value(e), e.svg.clone()),
            Read::Laser(l) => (l.format.clone(), serde_json::to_value(l), l.svg.clone()),
        };
        let facts = facts.map_err(|e| e.to_string())?;
        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO mf_designs (account_id, kind, file_name, file_key, file_bytes, format, facts, preview_svg)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
        )
        .bind(user.account_id)
        .bind(read.kind())
        .bind(file_name)
        .bind(&file_key)
        .bind(file_bytes)
        .bind(format)
        .bind(facts)
        .bind(svg)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        for (position, (photo_key, thumb_key, w, h)) in photo_keys.iter().enumerate() {
            sqlx::query("INSERT INTO mf_design_photos (design_id, photo_key, thumb_key, width, height, position) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(id)
                .bind(photo_key)
                .bind(thumb_key)
                .bind(i32::try_from(*w).unwrap_or(i32::MAX))
                .bind(i32::try_from(*h).unwrap_or(i32::MAX))
                .bind(i32::try_from(position).unwrap_or(i32::MAX))
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        }
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(id)
    }
    .await;
    if result.is_err() {
        for key in &stored {
            // (best effort: an orphaned file costs a little storage, nothing else)
            let _ = storage.delete(key).await;
        }
    }
    result
}

// ---------------------------------------------------------------- reading them back

struct DesignRow {
    account_id: i64,
    kind: String,
    file_name: String,
    file_key: String,
    facts: serde_json::Value,
    preview_svg: String,
}

async fn design(pool: &PgPool, id: i64) -> Result<Option<DesignRow>, sqlx::Error> {
    let row =
        sqlx::query("SELECT account_id, kind, file_name, file_key, facts, preview_svg FROM mf_designs WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    row.map(|r| {
        Ok(DesignRow {
            account_id: r.try_get("account_id")?,
            kind: r.try_get("kind")?,
            file_name: r.try_get("file_name")?,
            file_key: r.try_get("file_key")?,
            facts: r.try_get("facts")?,
            preview_svg: r.try_get("preview_svg")?,
        })
    })
    .transpose()
}

/// The design `id` if `user` made it (someone else's draft is "not found", not "forbidden").
async fn own_design(pool: &PgPool, user: &User, id: i64) -> Result<DesignRow, Response> {
    match design(pool, id).await {
        Ok(Some(d)) if d.account_id == user.account_id => Ok(d),
        Ok(_) => Err(not_found()),
        Err(e) => {
            eprintln!("designs: {e}");
            Err(not_found())
        }
    }
}

/// (id, width, height) of a design's photos, in order.
async fn photo_list(pool: &PgPool, design_id: i64) -> Vec<(i64, i32, i32)> {
    sqlx::query_as("SELECT id, width, height FROM mf_design_photos WHERE design_id = $1 ORDER BY position")
        .bind(design_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (user, pool) = match maker(&state, &headers).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    let rows: Vec<(i64, String, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT d.id, d.file_name, d.kind, d.format,
                (SELECT p.id FROM mf_design_photos p WHERE p.design_id = d.id ORDER BY p.position LIMIT 1)
         FROM mf_designs d WHERE d.account_id = $1 ORDER BY d.created_at DESC LIMIT 200",
    )
    .bind(user.account_id)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();
    let cards: String = rows
        .iter()
        .map(|(id, name, kind, format, photo)| {
            let picture = match photo {
                Some(p) => format!(r#"<img src="/designs/{id}/photos/{p}/thumb" alt="" loading="lazy">"#),
                None => r#"<span class="muted">No photo yet</span>"#.to_string(),
            };
            format!(
                r#"<li><a href="/designs/{id}"><span class="thumb">{picture}</span><strong>{name}</strong><span class="muted">{kind}, {format} · draft</span></a></li>"#,
                name = escape(name),
                kind = if kind == "laser" { "Laser" } else { "Embroidery" },
                format = escape(format),
            )
        })
        .collect();
    let body = if rows.is_empty() {
        r#"<p class="lead">You haven't added a design yet.</p>"#.to_string()
    } else {
        format!(r#"<ul class="cards">{cards}</ul>"#)
    };
    private(
        "Your designs · MakerFoundry",
        &format!(
            r#"<section class="design"><h1>Your designs</h1><p><a href="/designs/new">Add a design</a></p>{body}</section>"#
        ),
        true,
    )
}

pub async fn show(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    let (user, pool) = match maker(&state, &headers).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    let d = match own_design(&pool, &user, id).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let photo_html: String = photo_list(&pool, id)
        .await
        .iter()
        .enumerate()
        .map(|(n, (pid, w, h))| {
            format!(
                r#"<a href="/designs/{id}/photos/{pid}/large"><img src="/designs/{id}/photos/{pid}/thumb" alt="Photo {n}" width="{tw}" height="{th}" loading="lazy"></a>"#,
                n = n + 1,
                tw = photos::THUMB_SIDE.min(u32::try_from(*w).unwrap_or(0)),
                th = (f64::from(*h) * f64::from(photos::THUMB_SIDE.min(u32::try_from(*w).unwrap_or(1))) / f64::from((*w).max(1))).round(),
            )
        })
        .collect();
    let photos_section = if photo_html.is_empty() {
        String::new()
    } else {
        format!(r#"<h2>Photos</h2><div class="photos">{photo_html}</div>"#)
    };
    let footer = format!(
        r#"{photos_section}
<p class="actions"><a class="button" href="/designs/{id}/download">Download the file</a></p>
<form method="post" action="/designs/{id}/delete"><button type="submit" class="secondary">Delete this draft</button></form>
<p><a href="/designs">Your designs</a></p>"#
    );
    let section = match d.kind.as_str() {
        "laser" => serde_json::from_value::<Laser>(d.facts).map(|mut l| {
            l.svg = d.preview_svg;
            laser_result(&d.file_name, &l, &footer)
        }),
        _ => serde_json::from_value::<Embroidery>(d.facts).map(|mut e| {
            e.svg = d.preview_svg;
            result(&d.file_name, &e, &footer)
        }),
    };
    match section {
        Ok(html) => private(&format!("{} · MakerFoundry", escape(&d.file_name)), &html, true),
        Err(e) => {
            eprintln!("designs: facts of {id}: {e}");
            not_found()
        }
    }
}

pub async fn download(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    let (user, pool) = match maker(&state, &headers).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    let (d, Some(storage)) = (
        match own_design(&pool, &user, id).await {
            Ok(d) => d,
            Err(r) => return r,
        },
        state.storage.as_ref(),
    ) else {
        return not_found();
    };
    Redirect::to(storage.download_url(&d.file_key, &d.file_name).as_str()).into_response()
}

pub async fn photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, photo_id, size)): Path<(i64, i64, String)>,
) -> Response {
    let (user, pool) = match maker(&state, &headers).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    if let Err(r) = own_design(&pool, &user, id).await {
        return r;
    }
    let column = match size.as_str() {
        "thumb" => "thumb_key",
        "large" => "photo_key",
        _ => return not_found(),
    };
    let key: Option<String> = sqlx::query_scalar(&format!(
        "SELECT {column} FROM mf_design_photos WHERE id = $1 AND design_id = $2"
    ))
    .bind(photo_id)
    .bind(id)
    .fetch_optional(&pool)
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
            eprintln!("designs: photo {photo_id}: {e}");
            (StatusCode::BAD_GATEWAY, "The photo can't be fetched just now.").into_response()
        }
    }
}

pub async fn delete(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool) = match maker(&state, &headers).await {
        Ok(m) => m,
        Err(r) => return r,
    };
    let d = match own_design(&pool, &user, id).await {
        Ok(d) => d,
        Err(r) => return r,
    };
    let keys: Vec<(String, String)> =
        sqlx::query_as("SELECT photo_key, thumb_key FROM mf_design_photos WHERE design_id = $1")
            .bind(id)
            .fetch_all(&pool)
            .await
            .unwrap_or_default();
    if let Err(e) = sqlx::query("DELETE FROM mf_designs WHERE id = $1 AND account_id = $2")
        .bind(id)
        .bind(user.account_id)
        .execute(&pool)
        .await
    {
        eprintln!("designs: delete {id}: {e}");
        return problem_page(
            "Delete this draft",
            "The draft couldn't be deleted just now. Please try again.",
        );
    }
    if let Some(storage) = &state.storage {
        for key in std::iter::once(d.file_key).chain(keys.into_iter().flat_map(|(a, b)| [a, b])) {
            // (the design is gone either way; a file left behind costs a little storage)
            if let Err(e) = storage.delete(&key).await {
                eprintln!("designs: delete {id}: {e}");
            }
        }
    }
    Redirect::to("/designs").into_response()
}
