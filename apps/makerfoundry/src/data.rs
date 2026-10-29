//! Your data (MF-15): what the Privacy Policy promises, done by the person themselves at
//! `/account/data`: a copy of everything MakerFoundry keeps about their account (JSON), and
//! deleting it.
//!
//! Deleting takes the maker profile, designs and their files and photos, makes, comments,
//! collections, follows and sign-ins. It keeps what the law asks to be kept: purchases and sales
//! and their ledger rows, agreements to the legal pages, and reports and copyright notices. A
//! design people have bought is unpublished but kept, so its buyers keep their downloads (Seller
//! Terms § 8). The Foundry Account itself (email, password, credits) is StitchFoundry's, closed
//! there.

use axum::Form;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{COOKIE, User, current_user, forbidden, private, same_origin};
use crate::pages::escape;

fn page_html(problem: &str) -> String {
    let problem = if problem.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem" role="alert">{}</p>"#, escape(problem))
    };
    format!(
        r#"<section class="form prose">
<h1>Your data</h1>
<h2>A copy</h2>
<p>Everything MakerFoundry keeps about your account, in one file (JSON): your maker profile, your designs' details, your collections, the makers you follow, your comments and makes, your purchases and sales, and the legal pages you agreed to. Your design files and photos you can download from your designs' pages.</p>
<p><a class="button" href="/account/data.json">Download a copy</a></p>
<h2>Delete your MakerFoundry data</h2>
<p>This deletes your maker profile, your designs with their files and photos, your makes, comments, collections and follows, and signs you out everywhere. It can't be undone.</p>
<p>Kept, because the law asks us to: your purchases and sales and their amounts, which legal pages you agreed to, and reports and copyright notices. A design people have bought is unpublished but kept, so they keep their downloads.</p>
<p>Your Foundry Account itself (email, password, credits) is kept by StitchFoundry, and closed there.</p>
{problem}
<form method="post" action="/account/data/delete" class="add-file">
<label><span>To delete, type <strong>delete</strong></span><input name="confirm" autocomplete="off" required></label>
<button type="submit">Delete my MakerFoundry data</button>
</form>
<p><a href="/account">Your account</a></p>
</section>"#
    )
}

pub async fn page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if current_user(&state, &headers).await.is_none() {
        return Redirect::to("/signin?next=/account/data").into_response();
    }
    private("Your data · MakerFoundry", &page_html(""), true)
}

/// Rows of `sql` (taking the account id as `$1`) as JSON objects.
async fn rows(pool: &PgPool, sql: &str, account: i64) -> Result<Value, sqlx::Error> {
    let wrapped = format!("SELECT coalesce(json_agg(t), '[]'::json) FROM ({sql}) t");
    sqlx::query_scalar::<_, Value>(&wrapped)
        .bind(account)
        .fetch_one(pool)
        .await
}

/// Everything kept about `user`'s account.
pub async fn export(pool: &PgPool, user: &User) -> Result<Value, sqlx::Error> {
    let a = user.account_id;
    Ok(json!({
        "account": { "id": a, "email": user.email },
        "maker_profile": rows(pool, "SELECT handle, name, bio FROM mf_makers WHERE account_id = $1", a).await?,
        "designs": rows(pool,
            "SELECT d.id, d.kind, d.format, d.file_name, d.title, d.description, d.tags, d.licence, d.price_cents,
                    d.status, d.material, d.thickness_mm, d.convert, d.downloads, d.created_at, d.published_at,
                    (SELECT count(*) FROM mf_design_photos p WHERE p.design_id = d.id) AS photos,
                    (SELECT json_agg(f.format) FROM mf_design_files f WHERE f.design_id = d.id) AS other_formats
             FROM mf_designs d WHERE d.account_id = $1 ORDER BY d.id", a).await?,
        "collections": rows(pool,
            "SELECT c.name, c.public, c.is_saved AS saved_list, c.created_at,
                    (SELECT json_agg(i.design_id) FROM mf_collection_items i WHERE i.collection_id = c.id) AS designs
             FROM mf_collections c WHERE c.account_id = $1 ORDER BY c.id", a).await?,
        "following": rows(pool,
            "SELECT m.handle, f.at FROM mf_follows f JOIN mf_makers m ON m.account_id = f.maker_account
             WHERE f.follower_account = $1", a).await?,
        "comments": rows(pool, "SELECT design_id, body, created_at FROM mf_comments WHERE account_id = $1 ORDER BY id", a).await?,
        "makes": rows(pool, "SELECT design_id, note, created_at FROM mf_makes WHERE account_id = $1 ORDER BY id", a).await?,
        "purchases": rows(pool,
            "SELECT id, design_id, title, price_cents, status, created_at, paid_at, refunded_at
             FROM mf_orders WHERE buyer_account = $1 ORDER BY id", a).await?,
        "sales": rows(pool,
            "SELECT id, design_id, title, price_cents, fee_cents, processing_cents, seller_cents, status, paid_at, refunded_at
             FROM mf_orders WHERE seller_account = $1 AND status IN ('paid', 'refunded') ORDER BY id", a).await?,
        "selling": rows(pool, "SELECT stripe_account, charges_enabled, payouts_enabled, created_at FROM mf_sellers WHERE account_id = $1", a).await?,
        "agreements": rows(pool, "SELECT document, version, at FROM mf_agreements WHERE account_id = $1 ORDER BY at", a).await?,
        "activity_seen": rows(pool, "SELECT seen_at FROM mf_activity_seen WHERE account_id = $1", a).await?,
        "proofs": rows(pool,
            "SELECT p.id, p.title, p.shop_name, p.customer, p.message, p.status, p.created_at, p.active_at,
                    (SELECT json_agg(json_build_object('number', v.number, 'file_name', v.file_name, 'format', v.format, 'note', v.note, 'sent', v.created_at) ORDER BY v.number)
                       FROM mf_proof_versions v WHERE v.proof_id = p.id) AS versions,
                    (SELECT json_agg(json_build_object('version', r.version, 'by_shop', r.by_shop, 'name', r.name, 'body', r.body, 'decision', r.decision, 'at', r.at) ORDER BY r.at)
                       FROM mf_proof_replies r WHERE r.proof_id = p.id) AS replies
             FROM mf_proofs p WHERE p.account_id = $1 ORDER BY p.id", a).await?,
        "reports_sent": rows(pool, "SELECT kind, reason, details, created_at, status FROM mf_reports WHERE reporter_account = $1 ORDER BY id", a).await?,
    }))
}

pub async fn download(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin?next=/account/data").into_response();
    };
    match export(pool, &user).await {
        Ok(v) => (
            [
                (header::CONTENT_TYPE, "application/json; charset=utf-8"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"makerfoundry-data.json\"",
                ),
                (header::CACHE_CONTROL, "private, no-store"),
            ],
            serde_json::to_string_pretty(&v).unwrap_or_default(),
        )
            .into_response(),
        Err(e) => {
            eprintln!("data: export {}: {e}", user.account_id);
            private(
                "Your data · MakerFoundry",
                &page_html("The copy couldn't be made just now. Please try again."),
                true,
            )
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct DeleteForm {
    confirm: String,
}

pub async fn delete(State(state): State<AppState>, headers: HeaderMap, Form(f): Form<DeleteForm>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin?next=/account/data").into_response();
    };
    if !f.confirm.trim().eq_ignore_ascii_case("delete") {
        return private("Your data · MakerFoundry", &page_html("Type delete to confirm."), true);
    }
    let keys = match erase(pool, user.account_id).await {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("data: delete {}: {e}", user.account_id);
            return private(
                "Your data · MakerFoundry",
                &page_html("Your data couldn't be deleted just now. Nothing was changed. Please try again."),
                true,
            );
        }
    };
    if let Some(storage) = &state.storage {
        for key in keys {
            // (the records are gone either way; a file left behind costs a little storage)
            if let Err(e) = storage.delete(&key).await {
                eprintln!("data: delete a file of {}: {e}", user.account_id);
            }
        }
    }
    let mut resp = private(
        "Deleted · MakerFoundry",
        r#"<section class="hero"><h1>Your MakerFoundry data is deleted</h1><p class="lead">You're signed out. Your Foundry Account itself is kept by StitchFoundry; you can still sign in with it, or close it there.</p></section>"#,
        false,
    );
    if let Ok(v) =
        header::HeaderValue::from_str(&format!("{COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0"))
    {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    resp
}

/// Deletes `account`'s data in one transaction → the storage keys of the files it had.
pub async fn erase(pool: &PgPool, account: i64) -> Result<Vec<String>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    // designs nobody has bought go, with everything on them; bought ones are unpublished and kept
    let gone: Vec<i64> = sqlx::query_scalar(
        "SELECT d.id FROM mf_designs d WHERE d.account_id = $1
           AND NOT EXISTS (SELECT 1 FROM mf_orders o WHERE o.design_id = d.id AND o.status IN ('paid', 'refunded'))",
    )
    .bind(account)
    .fetch_all(&mut *tx)
    .await?;
    let mut keys: Vec<String> = sqlx::query_scalar(
        "SELECT file_key FROM mf_designs WHERE id = ANY($1)
         UNION ALL SELECT file_key FROM mf_design_files WHERE design_id = ANY($1)
         UNION ALL SELECT photo_key FROM mf_design_photos WHERE design_id = ANY($1)
         UNION ALL SELECT thumb_key FROM mf_design_photos WHERE design_id = ANY($1)
         UNION ALL SELECT photo_key FROM mf_makes WHERE design_id = ANY($1) OR account_id = $2
         UNION ALL SELECT thumb_key FROM mf_makes WHERE design_id = ANY($1) OR account_id = $2",
    )
    .bind(&gone)
    .bind(account)
    .fetch_all(&mut *tx)
    .await?;
    // proofs go whole (MF-25)
    let proofs: Vec<i64> = sqlx::query_scalar("SELECT id FROM mf_proofs WHERE account_id = $1")
        .bind(account)
        .fetch_all(&mut *tx)
        .await?;
    keys.extend(crate::proofs::erase(&mut tx, &proofs).await?);
    keys.sort();
    keys.dedup();
    for sql in [
        "UPDATE mf_orders SET design_id = NULL WHERE design_id = ANY($1) AND status IN ('pending', 'expired')",
        "DELETE FROM mf_designs WHERE id = ANY($1)",
    ] {
        sqlx::query(sql).bind(&gone).execute(&mut *tx).await?;
    }
    for sql in [
        "UPDATE mf_designs SET status = 'draft' WHERE account_id = $1 AND status = 'published'",
        "DELETE FROM mf_makes WHERE account_id = $1",
        "DELETE FROM mf_comments WHERE account_id = $1",
        "DELETE FROM mf_collections WHERE account_id = $1",
        "DELETE FROM mf_follows WHERE follower_account = $1 OR maker_account = $1",
        "DELETE FROM mf_makers WHERE account_id = $1",
        "DELETE FROM mf_sessions WHERE account_id = $1",
        "DELETE FROM mf_activity_seen WHERE account_id = $1",
        "UPDATE mf_accounts SET email = '' WHERE account_id = $1",
    ] {
        sqlx::query(sql).bind(account).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(keys)
}
