//! Picks of the week (2026-11-01): one design a week for each craft (embroidery, laser cutting,
//! and 3D printing while MakerFoundry 3D is open), on the home page, on the design's own page and
//! at `/picks` with the weeks before. An admin picks one with the button on a design's page; when
//! no admin has by the time the home page first shows that week, the published design of that
//! craft most liked and made in the last seven days (then most downloaded, then newest) is kept as
//! its pick, so the week's pick doesn't change under people's feet. Weeks start on Monday (UTC).
//!
//! Also here: the home page's gallery of what people made from designs (their make photos).

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Redirect, Response};
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{current_user, forbidden, same_origin};
use crate::listings::{Card, card_html, cards_of, slug};
use crate::pages::{escape, page};

/// The crafts with a pick, and how a pick of each is labelled.
const KINDS: &[(&str, &str)] = &[
    ("embroidery", "Embroidery pick"),
    ("laser", "Laser pick"),
    ("model3d", "3D printing pick"),
];

fn label(kind: &str) -> &'static str {
    KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or("Pick of the week", |(_, l)| l)
}

/// This week's pick for `kind`: the one kept for the week (an admin's, or one kept automatically),
/// or else, kept now, the week's most liked and made design of that craft.
async fn pick_for(pool: &PgPool, kind: &str) -> Option<i64> {
    let kept: Option<i64> = sqlx::query_scalar(
        "SELECT p.design_id FROM mf_picks p JOIN mf_designs d ON d.id = p.design_id
         WHERE p.week = date_trunc('week', now() AT TIME ZONE 'UTC')::date AND p.kind = $1 AND d.status = 'published'",
    )
    .bind(kind)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    if kept.is_some() {
        return kept;
    }
    let best: i64 = sqlx::query_scalar(
        "SELECT d.id FROM mf_designs d
         WHERE d.status = 'published' AND d.kind = $1
         ORDER BY (SELECT count(*) FROM mf_likes l WHERE l.design_id = d.id AND l.created_at > now() - interval '7 days')
                + 2 * (SELECT count(*) FROM mf_makes k WHERE k.design_id = d.id AND k.created_at > now() - interval '7 days') DESC,
                  d.downloads DESC, d.published_at DESC
         LIMIT 1",
    )
    .bind(kind)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    // (kept for the week, unless an admin picked one meanwhile, or the week's pick was taken down)
    let _ = sqlx::query(
        "INSERT INTO mf_picks (week, kind, design_id) VALUES (date_trunc('week', now() AT TIME ZONE 'UTC')::date, $1, $2)
         ON CONFLICT (week, kind) DO UPDATE SET design_id = $2, picked_by = NULL, picked_at = now()
         WHERE NOT EXISTS (SELECT 1 FROM mf_designs d WHERE d.id = mf_picks.design_id AND d.status = 'published')",
    )
    .bind(kind)
    .bind(best)
    .execute(pool)
    .await;
    Some(best)
}

/// The home page's picks of the week (empty while there are no designs).
pub async fn home_html(state: &AppState) -> String {
    let Some(pool) = state.db.as_ref() else {
        return String::new();
    };
    let mut picked: Vec<(i64, &str)> = Vec::new();
    for (kind, name) in KINDS {
        if *kind == "model3d" && !state.three_d {
            continue;
        }
        if let Some(id) = pick_for(pool, kind).await {
            picked.push((id, name));
        }
    }
    let ids: Vec<i64> = picked.iter().map(|(id, _)| *id).collect();
    let cards = cards_of(pool, &ids, state.three_d).await;
    if cards.is_empty() {
        return String::new();
    }
    let items: String = cards.iter().map(|c| card_html(c, Some(label(&c.kind)))).collect();
    format!(
        r#"<section class="design picks"><h2>Picks of the week</h2><ul class="cards">{items}</ul><p><a href="/picks">Past picks</a></p></section>"#
    )
}

/// What people made from designs lately, as a gallery of their photos (empty while there are none).
pub async fn makes_html(state: &AppState) -> String {
    let Some(pool) = state.db.as_ref() else {
        return String::new();
    };
    let makes: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT k.id, d.id, d.title FROM mf_makes k JOIN mf_designs d ON d.id = k.design_id
         WHERE d.status = 'published' AND (d.kind <> 'model3d' OR $1)
         ORDER BY k.created_at DESC LIMIT 8",
    )
    .bind(state.three_d)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    if makes.is_empty() {
        return String::new();
    }
    let items: String = makes
        .iter()
        .map(|(mid, id, title)| {
            format!(
                r#"<li><a href="/d/{id}/{slug}"><img src="/makes/{mid}/thumb" alt="Made from {title}" loading="lazy"><span>{title}</span></a></li>"#,
                slug = slug(title),
                title = escape(title),
            )
        })
        .collect();
    format!(r#"<section class="design"><h2>Made by the community</h2><ul class="gallery">{items}</ul></section>"#)
}

/// Design `id`'s weeks as a pick, for its page: "Embroidery pick of the week of November 2, 2026".
pub async fn badge_html(pool: &PgPool, id: i64) -> String {
    let weeks: Vec<(String, String)> = sqlx::query_as(
        "SELECT kind, to_char(week, 'FMMonth FMDD, YYYY') FROM mf_picks WHERE design_id = $1 ORDER BY week DESC LIMIT 3",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    weeks
        .iter()
        .map(|(kind, week)| {
            format!(
                r#"<p class="pick-badge"><a href="/picks">{} of the week of {}</a></p>"#,
                escape(label(kind)),
                escape(week)
            )
        })
        .collect()
}

/// The admins' button on a published design's page: make it its craft's pick of this week.
pub fn admin_button(id: i64, kind: &str) -> String {
    format!(
        r#"<form method="post" action="/d/{id}/pick" class="inline"><button type="submit" class="secondary">Make this the {} of the week</button></form>"#,
        escape(&label(kind).to_lowercase())
    )
}

/// `POST /d/<id>/pick` (admins only): design `id` becomes its craft's pick of this week.
pub async fn pick(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (admin, pool) = match crate::moderation::admin(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM mf_designs WHERE id = $1 AND status = 'published'")
        .bind(id)
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten();
    let Some(kind) = kind.filter(|k| KINDS.iter().any(|(c, _)| c == k)) else {
        return crate::designs::not_found();
    };
    let done = sqlx::query(
        "INSERT INTO mf_picks (week, kind, design_id, picked_by) VALUES (date_trunc('week', now() AT TIME ZONE 'UTC')::date, $1, $2, $3)
         ON CONFLICT (week, kind) DO UPDATE SET design_id = $2, picked_by = $3, picked_at = now()",
    )
    .bind(&kind)
    .bind(id)
    .bind(admin.account_id)
    .execute(&pool)
    .await;
    if let Err(e) = done {
        eprintln!("picks: {id}: {e}");
        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Try again in a minute.").into_response();
    }
    Redirect::to(&format!("/d/{id}")).into_response()
}

/// `/picks`: the picks of the weeks so far, newest first (the last 26 weeks).
pub async fn past(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let signed_in = current_user(&state, &headers).await.is_some();
    let Some(pool) = state.db.as_ref() else {
        return crate::designs::not_found();
    };
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT to_char(p.week, 'FMMonth FMDD, YYYY'), p.design_id FROM mf_picks p
         JOIN mf_designs d ON d.id = p.design_id
         WHERE d.status = 'published' AND (d.kind <> 'model3d' OR $1)
           AND p.week > (now() AT TIME ZONE 'UTC')::date - 26 * 7
         ORDER BY p.week DESC, array_position(ARRAY['embroidery', 'laser', 'model3d'], p.kind)",
    )
    .bind(state.three_d)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let ids: Vec<i64> = rows.iter().map(|(_, id)| *id).collect();
    let cards: Vec<Card> = cards_of(pool, &ids, state.three_d).await;
    let mut weeks = String::new();
    let mut current: Option<&str> = None;
    let mut items = String::new();
    for (week, id) in &rows {
        if current != Some(week.as_str()) {
            if let Some(w) = current {
                weeks.push_str(&format!(
                    r#"<h2>The week of {}</h2><ul class="cards">{items}</ul>"#,
                    escape(w)
                ));
                items.clear();
            }
            current = Some(week.as_str());
        }
        if let Some(c) = cards.iter().find(|c| c.id == *id) {
            items.push_str(&card_html(c, Some(label(&c.kind))));
        }
    }
    if let Some(w) = current {
        weeks.push_str(&format!(
            r#"<h2>The week of {}</h2><ul class="cards">{items}</ul>"#,
            escape(w)
        ));
    }
    if weeks.is_empty() {
        weeks = r#"<p class="muted">No picks yet: the first ones come with the first designs.</p>"#.to_string();
    }
    Html(page(
        "Picks of the week · MakerFoundry",
        &format!(
            r#"<section class="design"><h1>Picks of the week</h1><p class="lead">Each week, one design for each craft, chosen by MakerFoundry's team or by what makers liked and made most that week.</p>{weeks}<p><a href="/browse">Browse all designs</a></p></section>"#
        ),
        signed_in,
    ))
    .into_response()
}
