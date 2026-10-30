//! Comments (MF-14): questions and notes on a published design, shown oldest first under it.
//! Commenting needs a maker profile (the name shown) and, until the launch, an account that may
//! publish. Comments are plain text: nothing in them becomes a link. The author, the design's
//! maker and the admins can take a comment down, and anyone can report one (`moderation`).

use axum::Form;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{User, current_user, forbidden, private, same_origin};
use crate::listings::{paragraphs, slug};
use crate::pages::escape;
use crate::{limits, makers};

/// The longest comment, in characters.
pub const MAX_BODY: usize = 2000;
/// Comments shown under a design (the oldest).
const SHOWN: i64 = 200;

pub async fn count(pool: &PgPool, design: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM mf_comments WHERE design_id = $1")
        .bind(design)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// A comment as shown: id, author, text, date, the author's maker address and name.
type Row = (i64, i64, String, String, Option<String>, Option<String>);

/// The Comments part of a published design's page (`owner` is the design's maker).
pub async fn section(state: &AppState, pool: &PgPool, viewer: Option<&User>, id: i64, owner: i64) -> String {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT c.id, c.account_id, c.body, to_char(c.created_at, 'YYYY-MM-DD'), m.handle, m.name
         FROM mf_comments c LEFT JOIN mf_makers m ON m.account_id = c.account_id
         WHERE c.design_id = $1 ORDER BY c.created_at LIMIT $2",
    )
    .bind(id)
    .bind(SHOWN)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let staff = viewer.is_some_and(|u| u.account_id == owner || state.admins.contains(&u.account_id));
    let items: String = rows
        .iter()
        .map(|(cid, author, body, at, handle, name)| {
            let by = match (handle, name) {
                (Some(h), Some(n)) => format!(r#"<a href="/makers/{}">{}</a>"#, escape(h), escape(n)),
                _ => "a maker".to_string(),
            };
            let maker_badge = if *author == owner { r#" <span class="badge">the maker</span>"# } else { "" };
            let mine = viewer.is_some_and(|u| u.account_id == *author);
            let act = if staff || mine {
                format!(
                    r#"<form method="post" action="/comments/{cid}/delete"><button type="submit" class="secondary">Take down</button></form>"#
                )
            } else {
                format!(r#"<a href="/comments/{cid}/report" class="muted">Report</a>"#)
            };
            format!(
                r#"<li class="comment" id="c{cid}"><p class="muted">{by}{maker_badge} · {at}</p>{body}<div class="comment-actions">{act}</div></li>"#,
                at = escape(at),
                body = paragraphs(body),
            )
        })
        .collect();
    let list = if rows.is_empty() {
        r#"<p class="muted">No comments yet.</p>"#.to_string()
    } else {
        format!(r#"<ol class="comments">{items}</ol>"#)
    };
    let add = match viewer {
        None => format!(r#"<p class="muted"><a href="/signin?next=/d/{id}">Sign in to comment</a></p>"#),
        Some(u) if !state.publishers.allows(u.account_id) => String::new(),
        Some(_) => format!(
            r#"<form method="post" action="/d/{id}/comments" class="add-file">
<label>Add a comment <small>(a question for the maker, or a tip for others; plain text)</small><textarea name="body" rows="3" maxlength="{MAX_BODY}" required></textarea></label>
<button type="submit" class="secondary">Post the comment</button>
</form>"#
        ),
    };
    format!(r#"<h2>Comments</h2>{list}{add}"#)
}

fn problem(id: i64, why: &str) -> Response {
    private(
        "Comment · MakerFoundry",
        &format!(
            r#"<section class="form"><h1>Comment</h1><p class="problem" role="alert">{}</p><p><a href="/d/{id}">Back to the design</a></p></section>"#,
            escape(why)
        ),
        true,
    )
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CommentForm {
    body: String,
}

/// A comment's text as typed, tidied, or why it won't do.
fn clean_body(body: &str) -> Result<String, String> {
    let body = body.trim().replace("\r\n", "\n");
    if body.is_empty() {
        return Err("Write something first.".into());
    }
    if body.chars().count() > MAX_BODY || body.chars().any(|c| c.is_control() && c != '\n') {
        return Err(format!("A comment can be up to {MAX_BODY} characters."));
    }
    Ok(body)
}

pub async fn post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(f): Form<CommentForm>,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to(&format!("/signin?next=/d/{id}")).into_response();
    };
    let title: Option<String> =
        sqlx::query_scalar("SELECT title FROM mf_designs WHERE id = $1 AND status = 'published'")
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let Some(title) = title else {
        return Redirect::to("/browse").into_response();
    };
    if !state.publishers.allows(user.account_id) {
        return problem(id, "Comments open when MakerFoundry launches.");
    }
    if makers::profile(pool, user.account_id).await.is_none() {
        return problem(
            id,
            "Set up your maker profile first (Account › Your maker profile): it's the name shown with your comment.",
        );
    }
    let body = match clean_body(&f.body) {
        Ok(b) => b,
        Err(why) => return problem(id, &why),
    };
    if !state.limits.allow(limits::COMMENT, &user.account_id.to_string()) {
        return problem(id, "You've commented a lot in the last hour. Please wait a while.");
    }
    let saved: Result<i64, _> =
        sqlx::query_scalar("INSERT INTO mf_comments (design_id, account_id, body) VALUES ($1, $2, $3) RETURNING id")
            .bind(id)
            .bind(user.account_id)
            .bind(&body)
            .fetch_one(pool)
            .await;
    match saved {
        Ok(cid) => {
            if let Some(owner) = crate::notices::owner(pool, id).await {
                crate::notices::record(&state, pool, owner, user.account_id, "comment", Some(id)).await;
            }
            Redirect::to(&format!("/d/{id}/{}#c{cid}", slug(&title))).into_response()
        }
        Err(e) => {
            eprintln!("comments: {id}: {e}");
            problem(id, "The comment couldn't be posted just now. Please try again.")
        }
    }
}

/// Takes comment `cid` down; → its design, if it was there.
pub async fn remove(pool: &PgPool, cid: i64) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("DELETE FROM mf_comments WHERE id = $1 RETURNING design_id")
        .bind(cid)
        .fetch_optional(pool)
        .await
}

/// The author, the design's maker or an admin takes a comment down.
pub async fn delete(State(state): State<AppState>, headers: HeaderMap, Path(cid): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin").into_response();
    };
    let row: Option<(i64, i64, i64)> = sqlx::query_as(
        "SELECT c.design_id, c.account_id, d.account_id FROM mf_comments c JOIN mf_designs d ON d.id = c.design_id WHERE c.id = $1",
    )
    .bind(cid)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some((design, author, owner)) = row else {
        return Redirect::to("/browse").into_response();
    };
    if (user.account_id == author || user.account_id == owner || state.admins.contains(&user.account_id))
        && let Err(e) = remove(pool, cid).await
    {
        eprintln!("comments: delete {cid}: {e}");
    }
    Redirect::to(&format!("/d/{design}")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_are_checked() {
        assert_eq!(
            clean_body("  Which stabilizer?\r\nThanks "),
            Ok("Which stabilizer?\nThanks".to_string())
        );
        assert!(clean_body("   ").is_err());
        assert!(clean_body(&"x".repeat(MAX_BODY + 1)).is_err());
        assert!(clean_body("bell\u{7}").is_err());
    }
}
