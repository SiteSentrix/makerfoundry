//! Importing a creator's own models in bulk (MF-52): many files at once, or ZIPs of them, each
//! kept as a pending import file as it arrives (so memory holds one file at a time), then grouped
//! into models, by folder inside a ZIP, else by name: files whose names share a stem once endings
//! like `_plate2`, `-v2`, `_part3` or ` (1)` are taken off are one model, and images join the
//! model whose stem they share. The creator sees the groups and makes them into drafts; a file
//! that fails its checks is reported and left out, the rest kept (docs/3d-hub-plan.md § 8).

/// One model found among the files: its name and its members (indexes into the names given).
#[derive(Debug, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub members: Vec<usize>,
}

/// Whether a name is an image (by its extension).
pub fn is_image(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".jpg", ".jpeg", ".png", ".webp"].iter().any(|e| lower.ends_with(e))
}

/// The folder part of a path inside a ZIP (`""` for none) and the file's own name.
fn split_path(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((folder, name)) => (folder, name),
        None => ("", path),
    }
}

/// A file's stem with the endings that mark parts, plates, versions and copies taken off:
/// `Bracket_plate2.3mf` → `Bracket`, `gear-v2 (1).stl` → `gear`.
pub fn stem(name: &str) -> String {
    let mut s = name.rsplit_once('.').map_or(name, |(s, _)| s).trim().to_string();
    loop {
        let before = s.len();
        // " (1)"
        if s.ends_with(')')
            && let Some(open) = s.rfind('(')
            && s.get(open + 1..s.len() - 1)
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        {
            s.truncate(open);
            s = s.trim_end().to_string();
        }
        // "_plate2", "-v2", " part 3", "_version10"
        let lower = s.to_ascii_lowercase();
        let digits = lower.len() - lower.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        if digits > 0 {
            let head = lower[..lower.len() - digits].trim_end_matches([' ', '_', '-']);
            for word in ["plate", "part", "version", "v"] {
                if let Some(rest) = head.strip_suffix(word)
                    && (rest.is_empty() || rest.ends_with([' ', '_', '-']))
                    && !rest.trim_end_matches([' ', '_', '-']).is_empty()
                {
                    s.truncate(rest.trim_end_matches([' ', '_', '-']).len());
                    break;
                }
            }
        }
        if s.len() == before {
            return s;
        }
    }
}

/// Groups files (paths as given, `folder/name` inside a ZIP) into models.
pub fn group(paths: &[String]) -> Vec<Group> {
    let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new(); // (key, name, members)
    let key_of = |path: &str| -> (String, String) {
        let (folder, name) = split_path(path);
        if folder.is_empty() {
            let s = stem(name);
            (format!("stem:{}", s.to_lowercase()), s)
        } else {
            let shown = folder.rsplit('/').next().unwrap_or(folder).to_string();
            (format!("folder:{}", folder.to_lowercase()), shown)
        }
    };
    // models first, so images have somewhere to go
    for (i, path) in paths.iter().enumerate().filter(|(_, p)| !is_image(p)) {
        let (key, name) = key_of(path);
        match groups.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.2.push(i),
            None => groups.push((key, name, vec![i])),
        }
    }
    for (i, path) in paths.iter().enumerate().filter(|(_, p)| is_image(p)) {
        let (key, _) = key_of(path);
        let stem_key = key.strip_prefix("stem:").unwrap_or("");
        let home = groups
            .iter()
            .position(|g| g.0 == key)
            // "bracket photo 1.jpg" beside "bracket.stl"
            .or_else(|| {
                groups.iter().position(|g| {
                    g.0.strip_prefix("stem:")
                        .is_some_and(|k| !k.is_empty() && stem_key.starts_with(k))
                })
            })
            .or_else(|| (groups.len() == 1).then_some(0));
        if let Some(g) = home.and_then(|h| groups.get_mut(h)) {
            g.2.push(i);
        }
    }
    groups
        .into_iter()
        .map(|(_, name, members)| Group { name, members })
        .collect()
}

// ---------------------------------------------------------------- the pages

use axum::extract::{Multipart, Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use sqlx::PgPool;

use super::files;
use super::upload::{self, clean_name};
use crate::AppState;
use crate::accounts::{User, current_user, forbidden, private, same_origin};
use crate::designs::Refused;
use crate::pages::escape;
use crate::storage::{Storage, new_key};

/// The most files one import may bring, counting what's inside ZIPs (the owner's decision,
/// 2026-10-30).
pub const MAX_FILES: usize = 200;
/// The most one import may bring, all told (files, and what ZIPs unpack to).
pub const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// The largest ZIP sent (it's read whole, in memory).
pub const MAX_ZIP_BYTES: usize = upload::MAX_MODEL_BYTES;
/// The import route's cap.
pub const BODY_LIMIT_BYTES: usize = 2 * 1024 * 1024 * 1024 + 1024 * 1024;
/// How long making drafts may go on in one request before the rest wait for the next press.
const DRAFTS_TIME: std::time::Duration = std::time::Duration::from_secs(200);

fn not_found() -> Response {
    crate::designs::not_found()
}

/// The signed-in maker who may import (as for uploads), and the database; or the answer instead.
async fn importer(state: &AppState, headers: &HeaderMap) -> Result<(User, PgPool), Response> {
    let viewer = current_user(state, headers).await;
    match (viewer, state.db.clone()) {
        (Some(u), Some(pool)) if super::open_to(state, Some(&u)) => Ok((u, pool)),
        (None, _) if state.three_d => Err(Redirect::to("/signin?next=/3d/import").into_response()),
        _ => Err(not_found()),
    }
}

fn import_form(problem: &str) -> String {
    let problem = if problem.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem" role="alert">{}</p>"#, escape(problem))
    };
    format!(
        r#"<section class="form">
<h1>Import your models</h1>
<p class="lead">Bring the models you've made and shared elsewhere: choose many files at once, or ZIPs of them (an export from your account on another site, or a folder of your own). Each folder in a ZIP becomes one model; loose files are grouped by name, so <code>bracket_plate1.3mf</code> and <code>bracket_plate2.3mf</code> make one model, and <code>bracket.jpg</code> joins it.</p>
{problem}
<form method="post" action="/3d/import" enctype="multipart/form-data">
<label>Files or ZIPs <small>(STL, 3MF, OBJ, STEP, images; up to {MAX_FILES} files or {gb} GB at once)</small><input type="file" name="files" multiple required></label>
<button type="submit">Import</button>
</form>
<p class="muted">Only import models you made or have the right to share. You'll see how they were grouped, and each becomes a draft for you to check before publishing.</p>
</section>"#,
        gb = MAX_BYTES / (1024 * 1024 * 1024),
    )
}

pub async fn new_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = importer(&state, &headers).await {
        return r;
    }
    private("Import your models · MakerFoundry", &import_form(""), true)
}

/// What one import has taken so far.
struct Taking<'a> {
    storage: &'a Storage,
    pool: &'a PgPool,
    import: i64,
    count: usize,
    bytes: u64,
    stored: Vec<String>,
}

impl Taking<'_> {
    /// Notes one file of the import: stored as pending if it's a model file or an image, else
    /// only noted with why it's left out.
    async fn take(&mut self, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.count += 1;
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        if self.count > MAX_FILES {
            return Err(format!(
                "An import can bring up to {MAX_FILES} files at once (counting those in ZIPs)."
            ));
        }
        if self.bytes > MAX_BYTES {
            return Err(format!(
                "An import can bring up to {} GB at once.",
                MAX_BYTES / (1024 * 1024 * 1024)
            ));
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let image = is_image(name);
        let problem = if image || files::kind_of(name).is_some() {
            None
        } else {
            Some("isn't a model file or an image MakerFoundry takes")
        };
        let key = match problem {
            None => {
                let key = new_key("import")?;
                self.storage
                    .put(&key, bytes.clone(), "application/octet-stream")
                    .await?;
                self.stored.push(key.clone());
                Some(key)
            }
            Some(_) => None,
        };
        sqlx::query(
            "INSERT INTO mf_import_files (import_id, path, file_key, file_bytes, image, problem, position)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(self.import)
        .bind(path)
        .bind(key)
        .bind(bytes.len() as i64)
        .bind(image)
        .bind(problem)
        .bind(i32::try_from(self.count).unwrap_or(i32::MAX))
        .execute(self.pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Takes every file in a ZIP sent: folders kept in their paths, system files skipped.
    async fn take_zip(&mut self, zip_name: &str, bytes: &[u8]) -> Result<(), String> {
        let archive = super::zip::Archive::open(bytes).map_err(|why| format!("{zip_name}: {why}."))?;
        for entry in archive.entries() {
            let path = entry.name.as_str();
            let base = path.rsplit('/').next().unwrap_or(path);
            if path.ends_with('/') || path.starts_with("__MACOSX/") || base.starts_with('.') || base.is_empty() {
                continue;
            }
            if entry.size > super::mesh::MAX_FILE_BYTES as u64 {
                return Err(format!("{zip_name}: {path} is larger than 100 MB."));
            }
            let data = archive.read(entry).map_err(|why| format!("{zip_name}: {why}."))?;
            self.take(path, data).await?;
        }
        Ok(())
    }
}

/// Reads one part of the form, refusing it once it passes `max` bytes.
async fn read_part(field: &mut axum::extract::multipart::Field<'_>, max: usize) -> Result<Vec<u8>, ()> {
    let mut out = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(|_| ())? {
        if out.len() + chunk.len() > max {
            return Err(());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

pub async fn create(State(state): State<AppState>, headers: HeaderMap, mut multipart: Multipart) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool) = match importer(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Some(storage) = state.storage.as_ref() else {
        return Redirect::to("/3d/import").into_response();
    };
    let again = |why: &str| private("Import your models · MakerFoundry", &import_form(why), true);
    if !state.limits.allow(crate::limits::IMPORT, &user.account_id.to_string()) {
        return again("You've imported a lot today. Please try again tomorrow.");
    }
    let import: i64 = match sqlx::query_scalar("INSERT INTO mf_imports (account_id) VALUES ($1) RETURNING id")
        .bind(user.account_id)
        .fetch_one(&pool)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            eprintln!("import: {e}");
            return again("The import couldn't be started just now. Please try again.");
        }
    };
    let mut taking = Taking {
        storage,
        pool: &pool,
        import,
        count: 0,
        bytes: 0,
        stored: Vec::new(),
    };
    let outcome: Result<(), String> = async {
        loop {
            let mut field = match multipart.next_field().await {
                Ok(Some(f)) => f,
                Ok(None) => return Ok(()),
                Err(_) => return Err("The import didn't come through whole. Please try again.".into()),
            };
            if field.name() != Some("files") {
                continue;
            }
            let name = clean_name(field.file_name().unwrap_or("file"));
            let zip = name.to_ascii_lowercase().ends_with(".zip");
            let max = if zip {
                MAX_ZIP_BYTES
            } else {
                super::mesh::MAX_FILE_BYTES
            };
            let bytes = read_part(&mut field, max).await.map_err(|()| {
                format!(
                    "{name} is larger than {} MB, or didn't come through whole.",
                    max / (1024 * 1024)
                )
            })?;
            if bytes.is_empty() {
                continue;
            }
            if zip {
                taking.take_zip(&name, &bytes).await?;
            } else {
                taking.take(&name, bytes).await?;
            }
        }
    }
    .await;
    let outcome = outcome.and_then(|()| {
        if taking.count == 0 {
            Err("Choose the files or ZIPs to import.".to_string())
        } else {
            Ok(())
        }
    });
    if let Err(why) = outcome {
        // nothing of a refused import is kept
        for key in &taking.stored {
            let _ = storage.delete(key).await;
        }
        let _ = sqlx::query("DELETE FROM mf_imports WHERE id = $1")
            .bind(import)
            .execute(&pool)
            .await;
        return again(&why);
    }
    if let Err(e) = regroup(&pool, import).await {
        eprintln!("import {import}: {e}");
    }
    Redirect::to(&format!("/3d/import/{import}")).into_response()
}

/// Groups an import's waiting files (`group`), noting each one's group; an image with no model to
/// join is noted as left out.
pub async fn regroup(pool: &PgPool, import: i64) -> Result<(), sqlx::Error> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, path FROM mf_import_files WHERE import_id = $1 AND problem IS NULL AND design_id IS NULL ORDER BY position",
    )
    .bind(import)
    .fetch_all(pool)
    .await?;
    let paths: Vec<String> = rows.iter().map(|(_, p)| p.clone()).collect();
    let mut placed = vec![false; rows.len()];
    let mut tx = pool.begin().await?;
    for g in group(&paths) {
        for &m in &g.members {
            if let (Some((id, _)), Some(p)) = (rows.get(m), placed.get_mut(m)) {
                *p = true;
                sqlx::query("UPDATE mf_import_files SET group_name = $2 WHERE id = $1")
                    .bind(id)
                    .bind(&g.name)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    for ((id, _), p) in rows.iter().zip(placed) {
        if !p {
            sqlx::query(
                "UPDATE mf_import_files SET group_name = '', problem = 'has no model of the same name to join' WHERE id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await
}

/// One file of an import as its review shows it: path, group, draft made, why it's left out.
type FileRow = (i64, String, String, Option<i64>, Option<String>);

/// The import `id` if `user` made it.
async fn own_import(pool: &PgPool, user: &User, id: i64) -> Option<()> {
    let owner: Option<i64> = sqlx::query_scalar("SELECT account_id FROM mf_imports WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    (owner == Some(user.account_id)).then_some(())
}

pub async fn review(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    let (user, pool) = match importer(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if own_import(&pool, &user, id).await.is_none() {
        return not_found();
    }
    private("Your import · MakerFoundry", &review_html(&pool, id, "").await, true)
}

async fn review_html(pool: &PgPool, id: i64, note: &str) -> String {
    let rows: Vec<FileRow> = sqlx::query_as(
        "SELECT id, path, group_name, design_id, problem FROM mf_import_files WHERE import_id = $1 ORDER BY group_name, position",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut groups: Vec<(&str, Vec<&FileRow>)> = Vec::new();
    for r in rows.iter().filter(|r| r.4.is_none()) {
        match groups.iter_mut().find(|g| g.0 == r.2) {
            Some(g) => g.1.push(r),
            None => groups.push((&r.2, vec![r])),
        }
    }
    let waiting = groups.iter().filter(|g| g.1.iter().any(|r| r.3.is_none())).count();
    let list: String = groups
        .iter()
        .map(|(name, files)| {
            let draft = files.iter().find_map(|r| r.3);
            let state = match draft {
                Some(d) => format!(r#" · <a href="/designs/{d}">its draft</a>"#),
                None => String::new(),
            };
            let items: String = files.iter().map(|r| format!("<li>{}</li>", escape(&r.1))).collect();
            format!(
                r#"<li><strong>{name}</strong>{state}<ul>{items}</ul></li>"#,
                name = escape(name)
            )
        })
        .collect();
    let left_out: String = rows
        .iter()
        .filter_map(|r| {
            r.4.as_ref()
                .map(|why| format!("<li>{}: {}</li>", escape(&r.1), escape(why)))
        })
        .collect();
    let left_out = if left_out.is_empty() {
        String::new()
    } else {
        format!(r#"<h2>Left out</h2><ul class="problems">{left_out}</ul>"#)
    };
    let action = if waiting > 0 {
        format!(
            r#"<form method="post" action="/3d/import/{id}/drafts"><button type="submit">Make {waiting} draft{s}</button></form>"#,
            s = if waiting == 1 { "" } else { "s" }
        )
    } else {
        r#"<p><a href="/designs">See your drafts</a> to check each one's details and publish it.</p>"#.to_string()
    };
    let note = if note.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem" role="alert">{}</p>"#, escape(note))
    };
    let count = groups.len();
    format!(
        r#"<section class="design"><h1>Your import</h1>{note}<p class="lead">{count} model{s} found.</p><ol class="import-groups">{list}</ol>{left_out}{action}<p><a href="/3d/import">Import more</a></p></section>"#,
        s = if count == 1 { "" } else { "s" },
    )
}

/// Makes each waiting group of an import into a draft: its files fetched and checked one by one
/// (a bad one is noted and left out, the rest kept), then added as an upload is.
pub async fn drafts(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool) = match importer(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    if own_import(&pool, &user, id).await.is_none() {
        return not_found();
    }
    let Some(storage) = state.storage.as_ref() else {
        return not_found();
    };
    let started = std::time::Instant::now();
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT group_name FROM mf_import_files
         WHERE import_id = $1 AND design_id IS NULL AND problem IS NULL AND group_name <> '' ORDER BY group_name",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .unwrap_or_default();
    let mut note = String::new();
    for name in names {
        if started.elapsed() > DRAFTS_TIME {
            note = "Some models are still waiting: press the button again to carry on.".into();
            break;
        }
        match make_draft(&state, storage, &pool, user.account_id, id, &name).await {
            Ok(()) => {}
            Err(Refused::TooMany(why)) => {
                note = why;
                break;
            }
            Err(r) => {
                let why = r.message();
                let _ = sqlx::query(
                    "UPDATE mf_import_files SET problem = $3 WHERE import_id = $1 AND group_name = $2 AND design_id IS NULL AND problem IS NULL",
                )
                .bind(id)
                .bind(&name)
                .bind(&why)
                .execute(&pool)
                .await;
            }
        }
    }
    if note.is_empty() {
        Redirect::to(&format!("/3d/import/{id}")).into_response()
    } else {
        private("Your import · MakerFoundry", &review_html(&pool, id, &note).await, true)
    }
}

async fn make_draft(
    state: &AppState,
    storage: &Storage,
    pool: &PgPool,
    account: i64,
    import: i64,
    name: &str,
) -> Result<(), Refused> {
    let rows: Vec<(i64, String, String, bool)> = sqlx::query_as(
        "SELECT id, path, file_key, image FROM mf_import_files
         WHERE import_id = $1 AND group_name = $2 AND design_id IS NULL AND problem IS NULL AND file_key IS NOT NULL
         ORDER BY position",
    )
    .bind(import)
    .bind(name)
    .fetch_all(pool)
    .await
    .map_err(|_| Refused::Failed)?;
    let mut model_files = Vec::new();
    let mut images = Vec::new();
    let mut used = Vec::new();
    let leave_out = |fid: i64, why: String| async move {
        let _ = sqlx::query("UPDATE mf_import_files SET problem = $2 WHERE id = $1")
            .bind(fid)
            .bind(why)
            .execute(pool)
            .await;
    };
    for (fid, path, key, image) in rows {
        let Ok(Some(bytes)) = storage.get(&key).await else {
            return Err(Refused::Failed);
        };
        let file_name = clean_name(&path);
        if image {
            if images.len() >= upload::MAX_IMAGES {
                leave_out(
                    fid,
                    format!("left out: a model can have up to {} images", upload::MAX_IMAGES),
                )
                .await;
                used.push((fid, key));
                continue;
            }
            let check = bytes.clone();
            match tokio::task::spawn_blocking(move || crate::photos::read(&check)).await {
                Ok(Ok(_)) => images.push(bytes),
                Ok(Err(why)) => leave_out(fid, format!("left out: {why}")).await,
                Err(_) => leave_out(fid, "left out: it couldn't be read".into()).await,
            }
        } else {
            if model_files.len() >= upload::MAX_FILES {
                leave_out(
                    fid,
                    format!("left out: a model can have up to {} files", upload::MAX_FILES),
                )
                .await;
                used.push((fid, key));
                continue;
            }
            let (check, check_name) = (bytes.clone(), file_name.clone());
            match tokio::task::spawn_blocking(move || files::check(&check_name, &check).map(|_| ())).await {
                Ok(Ok(())) => model_files.push((file_name, bytes)),
                Ok(Err(why)) => leave_out(fid, format!("left out: {why}")).await,
                Err(_) => leave_out(fid, "left out: it couldn't be read".into()).await,
            }
        }
        used.push((fid, key));
    }
    if model_files.is_empty() {
        return Err(Refused::Bad(
            "left out: none of this model's files could be read".into(),
        ));
    }
    let design = upload::add(state, pool, account, model_files, images).await?;
    let _ = sqlx::query("UPDATE mf_designs SET import_id = $2, title = $3 WHERE id = $1")
        .bind(design)
        .bind(import)
        .bind(name.chars().take(crate::listings::MAX_TITLE).collect::<String>())
        .execute(pool)
        .await;
    for (fid, key) in used {
        let _ = sqlx::query("UPDATE mf_import_files SET design_id = $2, file_key = NULL WHERE id = $1")
            .bind(fid)
            .bind(design)
            .execute(pool)
            .await;
        let _ = storage.delete(&key).await;
    }
    Ok(())
}

/// Pending import files and unpublished drafts from imports, 30 days on (the 10-minute sweep).
pub async fn forget_old(pool: &PgPool, storage: Option<&Storage>) -> Result<(), sqlx::Error> {
    let keys: Vec<String> = sqlx::query_scalar(
        "WITH old AS (
             SELECT f.id, f.file_key FROM mf_import_files f JOIN mf_imports i ON i.id = f.import_id
             WHERE i.created_at < now() - interval '30 days' AND f.file_key IS NOT NULL FOR UPDATE OF f)
         UPDATE mf_import_files f SET file_key = NULL, problem = coalesce(f.problem, 'not made into a draft within 30 days')
         FROM old WHERE f.id = old.id RETURNING old.file_key",
    )
    .fetch_all(pool)
    .await?;
    let old: Vec<i64> = sqlx::query_scalar(
        "SELECT d.id FROM mf_designs d JOIN mf_imports i ON i.id = d.import_id
         WHERE d.status = 'draft' AND d.published_at IS NULL AND i.created_at < now() - interval '30 days'
           AND NOT EXISTS (SELECT 1 FROM mf_orders o WHERE o.design_id = d.id)",
    )
    .fetch_all(pool)
    .await?;
    let mut keys = keys;
    if !old.is_empty() {
        keys.extend(upload::keys(pool, &old).await);
        let photos: Vec<(String, String)> =
            sqlx::query_as("SELECT photo_key, thumb_key FROM mf_design_photos WHERE design_id = ANY($1)")
                .bind(&old)
                .fetch_all(pool)
                .await?;
        keys.extend(photos.into_iter().flat_map(|(a, b)| [a, b]));
        sqlx::query("DELETE FROM mf_designs WHERE id = ANY($1)")
            .bind(&old)
            .execute(pool)
            .await?;
    }
    if let Some(storage) = storage {
        for key in keys {
            if let Err(e) = storage.delete(&key).await {
                eprintln!("imports: {e}");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn grouped(list: &[&str]) -> Vec<(String, Vec<String>)> {
        let paths = names(list);
        group(&paths)
            .into_iter()
            .map(|g| (g.name, g.members.iter().map(|&i| paths[i].clone()).collect()))
            .collect()
    }

    #[test]
    fn endings_come_off_stems() {
        for (name, want) in [
            ("Bracket_plate2.3mf", "Bracket"),
            ("gear-v2.stl", "gear"),
            ("gear-v2 (1).stl", "gear"),
            ("Robot arm_part3.stl", "Robot arm"),
            ("Robot arm part 12.stl", "Robot arm"),
            ("vase_version10.obj", "vase"),
            ("cube (1).stl", "cube"),
            ("cube.stl", "cube"),
            ("v2.stl", "v2"),
            ("plate2.stl", "plate2"),
            ("level2.stl", "level2"),
            ("Benchy.STL", "Benchy"),
        ] {
            assert_eq!(stem(name), want, "{name}");
        }
    }

    #[test]
    fn files_group_by_name_and_images_join_their_stem() {
        let g = grouped(&[
            "Bracket_plate1.3mf",
            "Bracket_plate2.3mf",
            "Bracket.stl",
            "gear-v2.stl",
            "gear (1).stl",
            "Bracket.jpg",
            "gear photo.png",
            "stray.jpg",
        ]);
        assert_eq!(
            g,
            [
                (
                    "Bracket".to_string(),
                    names(&["Bracket_plate1.3mf", "Bracket_plate2.3mf", "Bracket.stl", "Bracket.jpg"])
                ),
                (
                    "gear".to_string(),
                    names(&["gear-v2.stl", "gear (1).stl", "gear photo.png"])
                ),
            ],
            "a stray image with no model of its name is left out"
        );
    }

    #[test]
    fn files_in_folders_group_by_folder() {
        let g = grouped(&[
            "my models/Planter/planter_base.stl",
            "my models/Planter/planter_tray.stl",
            "my models/Planter/photo.jpg",
            "my models/Hook/hook.3mf",
            "my models/Hook/README.txt",
            "loose.stl",
        ]);
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].0, "Planter");
        assert_eq!(g[0].1.len(), 3);
        assert_eq!(
            g[1],
            (
                "Hook".to_string(),
                names(&["my models/Hook/hook.3mf", "my models/Hook/README.txt"])
            )
        );
        assert_eq!(g[2].0, "loose");
    }

    #[test]
    fn one_model_takes_every_image() {
        let g = grouped(&["dragon.stl", "IMG_0001.jpg", "IMG_0002.jpg"]);
        assert_eq!(
            g,
            [(
                "dragon".to_string(),
                names(&["dragon.stl", "IMG_0001.jpg", "IMG_0002.jpg"])
            )]
        );
        assert!(group(&[]).is_empty());
        assert!(grouped(&["only.jpg"]).is_empty());
    }
}
