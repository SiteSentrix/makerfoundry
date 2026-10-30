//! Likes on 3D models (MF-48): one an account a model, counted on its page; in the data download
//! and deleted with the account's data.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{User, current_user, forbidden, same_origin};
use crate::limits;

/// How many accounts like a model.
pub async fn count(pool: &PgPool, design: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM mf_likes WHERE design_id = $1")
        .bind(design)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// The like button and count for a published model's page.
pub async fn html(pool: &PgPool, viewer: Option<&User>, design: i64) -> String {
    let n = count(pool, design).await;
    let shown = match n {
        0 => String::new(),
        1 => r#" <span class="muted">1 like</span>"#.to_string(),
        n => format!(r#" <span class="muted">{n} likes</span>"#),
    };
    let liked = match viewer {
        Some(u) => sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM mf_likes WHERE account_id = $1 AND design_id = $2)",
        )
        .bind(u.account_id)
        .bind(design)
        .fetch_one(pool)
        .await
        .unwrap_or(false),
        None => false,
    };
    let (value, label) = if liked { ("no", "Liked") } else { ("yes", "Like") };
    format!(
        r#"<form method="post" action="/d/{design}/like" class="like"><input type="hidden" name="like" value="{value}"><button type="submit" class="secondary" aria-pressed="{liked}">{label}</button>{shown}</form>"#
    )
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub struct LikeForm {
    like: String,
}

/// `POST /d/<id>/like`: `like=yes` likes a published 3D model, `like=no` takes it back.
pub async fn like(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    axum::Form(f): axum::Form<LikeForm>,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.clone()) else {
        return Redirect::to(&format!("/signin?next=/d/{id}")).into_response();
    };
    let d = match crate::designs::design(&pool, id).await {
        Ok(Some(d)) if d.kind == "model3d" && d.status == "published" && super::open_to(&state, Some(&user)) => d,
        _ => return crate::designs::not_found(),
    };
    if state.limits.allow(limits::LIKE, &user.account_id.to_string()) {
        let sql = if f.like == "no" {
            "DELETE FROM mf_likes WHERE account_id = $1 AND design_id = $2"
        } else {
            "INSERT INTO mf_likes (account_id, design_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
        };
        if let Err(e) = sqlx::query(sql).bind(user.account_id).bind(id).execute(&pool).await {
            eprintln!("likes: {id}: {e}");
        }
    }
    Redirect::to(&format!("/d/{id}/{}", crate::listings::slug(&d.title))).into_response()
}
