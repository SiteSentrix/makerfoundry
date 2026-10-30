//! The opt-in showcase (MF-55): a creator may let MakerFoundry feature their project on its social
//! pages by ticking an unticked box when they upload a 3D model, post a make of one, or publish an
//! import. The consent is kept with its time; `/account/showcase` lists the creator's consents
//! with Withdraw on each; deleting the account's data withdraws them all (docs/3d-hub-plan.md § 9).
//! The box's wording and what it permits are for the owner to approve in the Terms (MF-58).

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{current_user, forbidden, private, same_origin};
use crate::pages::escape;

/// The unticked box, as every form that offers it shows it.
pub const CHECKBOX: &str = r#"<label class="choice"><input type="checkbox" name="showcase" value="yes"> <span>Feature my project on MakerFoundry's social pages (Facebook, Instagram, YouTube), credited to my maker name. Optional; <a href="/account/showcase">withdraw it any time</a>.</span></label>"#;

/// Notes a consent, for a model or a make.
pub async fn give(pool: &PgPool, account: i64, design: Option<i64>, make: Option<i64>) {
    if let Err(e) = sqlx::query("INSERT INTO mf_showcase_consents (account_id, design_id, make_id) VALUES ($1, $2, $3)")
        .bind(account)
        .bind(design)
        .bind(make)
        .execute(pool)
        .await
    {
        eprintln!("showcase: consent: {e}");
    }
}

/// One consent as the creator's list shows it: id, what it's for, given, withdrawn.
type ConsentRow = (i64, Option<i64>, Option<String>, Option<i64>, String, Option<String>);

/// `GET /account/showcase`: the signed-in creator's consents.
pub async fn page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin?next=/account/showcase").into_response();
    };
    let rows: Vec<ConsentRow> = sqlx::query_as(
        "SELECT c.id, c.design_id, d.title, c.make_id, to_char(c.given_at, 'YYYY-MM-DD HH24:MI'), to_char(c.withdrawn_at, 'YYYY-MM-DD HH24:MI')
         FROM mf_showcase_consents c LEFT JOIN mf_designs d ON d.id = coalesce(c.design_id, (SELECT design_id FROM mf_makes WHERE id = c.make_id))
         WHERE c.account_id = $1 ORDER BY c.given_at DESC LIMIT 500",
    )
    .bind(user.account_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let items: String = rows
        .iter()
        .map(|(id, _, title, make, given, withdrawn)| {
            let what = match (make, title) {
                (Some(_), Some(t)) => format!("Your make of {}", escape(t)),
                (Some(_), None) => "Your make".to_string(),
                (None, Some(t)) if !t.is_empty() => escape(t),
                _ => "A model of yours".to_string(),
            };
            let state = match withdrawn {
                Some(at) => format!(r#"<span class="muted">Withdrawn {at}</span>"#),
                None => format!(
                    r#"<form method="post" action="/account/showcase/{id}/withdraw" class="inline"><button type="submit" class="secondary small">Withdraw</button></form>"#
                ),
            };
            format!(r#"<li>{what} <span class="muted">· given {given}</span> {state}</li>"#)
        })
        .collect();
    let list = if items.is_empty() {
        r#"<p class="muted">You haven't asked us to feature anything.</p>"#.to_string()
    } else {
        format!(r#"<ul class="consents">{items}</ul>"#)
    };
    private(
        "Featuring your projects · MakerFoundry",
        &format!(
            r#"<section class="design"><h1>Featuring your projects</h1>
<p class="lead">What you've said MakerFoundry may feature on its social pages, credited to your maker name. Withdrawing takes it off our list at once, and we take down anything already posted from it.</p>
{list}<p><a href="/account">Your account</a></p></section>"#
        ),
        true,
    )
}

/// `POST /account/showcase/<id>/withdraw`: only its creator's, once; its times are kept.
pub async fn withdraw(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin?next=/account/showcase").into_response();
    };
    let owner: Option<i64> = sqlx::query_scalar("SELECT account_id FROM mf_showcase_consents WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    if owner != Some(user.account_id) {
        return crate::designs::not_found();
    }
    let _ = sqlx::query("UPDATE mf_showcase_consents SET withdrawn_at = now() WHERE id = $1 AND withdrawn_at IS NULL")
        .bind(id)
        .execute(pool)
        .await;
    Redirect::to("/account/showcase").into_response()
}
