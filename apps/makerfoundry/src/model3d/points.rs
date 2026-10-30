//! Points (issue #34), off unless `MF_POINTS=yes`, and with no rewards: what points could be
//! exchanged for, and the budget, are the owner's to decide. Points invite gaming (re-uploads,
//! low-effort makes), so none are awarded for an action itself. They're counted afresh from what
//! other people did with a member's work, and anything taken down, unpublished or deleted stops
//! counting at once:
//!
//! - **10** for each published 3D model of theirs that someone else has made (posted a make of)
//!   or that 3 other members have liked; a model with neither earns nothing, so uploading copies
//!   earns nothing.
//! - **3** for each other member who has posted a make of one of their models (each maker once a
//!   model).
//! - **2** for each published 3D model of someone else's they've posted a make of (each model
//!   once), up to 20 models a month.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{current_user, private};

/// Makes of others' models counted in one month.
pub const MAX_MAKES_A_MONTH: i64 = 20;

/// A member's points, by where they came from.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Models of theirs others have made or liked.
    pub models: i64,
    /// Others' makes of their models (each maker once a model).
    pub made_by_others: i64,
    /// Others' models they've made (each once, up to 20 a month).
    pub makes: i64,
}

impl Tally {
    pub fn total(&self) -> i64 {
        self.models * 10 + self.made_by_others * 3 + self.makes * 2
    }
}

/// Counts `account`'s points now.
pub async fn tally(pool: &PgPool, account: i64) -> Tally {
    let row: Result<(i64, i64, i64), _> = sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM mf_designs d
             WHERE d.account_id = $1 AND d.kind = 'model3d' AND d.status = 'published'
               AND (EXISTS (SELECT 1 FROM mf_makes k WHERE k.design_id = d.id AND k.account_id <> $1)
                    OR (SELECT count(*) FROM mf_likes l WHERE l.design_id = d.id AND l.account_id <> $1) >= 3)),
           (SELECT count(*) FROM (SELECT DISTINCT k.design_id, k.account_id FROM mf_makes k
              JOIN mf_designs d ON d.id = k.design_id
             WHERE d.account_id = $1 AND d.kind = 'model3d' AND d.status = 'published' AND k.account_id <> $1) x),
           (SELECT coalesce(sum(least(n, $2)), 0)::bigint FROM (
              SELECT date_trunc('month', first) AS month, count(*) AS n FROM (
                SELECT k.design_id, min(k.created_at) AS first FROM mf_makes k JOIN mf_designs d ON d.id = k.design_id
                 WHERE k.account_id = $1 AND d.account_id <> $1 AND d.kind = 'model3d' AND d.status = 'published'
                 GROUP BY k.design_id) firsts
              GROUP BY 1) months)",
    )
    .bind(account)
    .bind(MAX_MAKES_A_MONTH)
    .fetch_one(pool)
    .await;
    match row {
        Ok((models, made_by_others, makes)) => Tally {
            models,
            made_by_others,
            makes,
        },
        Err(e) => {
            eprintln!("points: {e}");
            Tally::default()
        }
    }
}

/// A maker's points for their profile, when points are on.
pub async fn profile_html(state: &AppState, pool: &PgPool, account: i64) -> String {
    if !state.points {
        return String::new();
    }
    let total = tally(pool, account).await.total();
    if total == 0 {
        String::new()
    } else {
        format!(r#"<p class="muted">{total} points</p>"#)
    }
}

/// `GET /account/points`: a member's points and how they're counted.
pub async fn page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !state.points {
        return crate::designs::not_found();
    }
    let user = current_user(&state, &headers).await;
    let (Some(user), Some(pool)) = (user, state.db.as_ref()) else {
        return Redirect::to("/signin?next=/account/points").into_response();
    };
    let t = tally(pool, user.account_id).await;
    private(
        "Your points · MakerFoundry",
        &format!(
            r#"<section class="form"><h1>Your points</h1><p class="lead"><strong>{total}</strong> points</p>
<ul>
<li>Your models made or liked by others: {models} ({mp} points)</li>
<li>Makes of your models by others: {mbo} ({mbop} points)</li>
<li>Your makes of others' models: {makes} ({mkp} points)</li>
</ul>
<h2>How points are counted</h2>
<p>Points come from what other people do with your work, never from uploading itself: 10 for each published model someone else has made or 3 other members have liked, 3 for each member who's posted a make of one of your models, and 2 for each of someone else's models you've made (up to {max} a month). Anything taken down, unpublished or deleted stops counting.</p>
<p><a href="/account">Your account</a></p></section>"#,
            total = t.total(),
            models = t.models,
            mp = t.models * 10,
            mbo = t.made_by_others,
            mbop = t.made_by_others * 3,
            makes = t.makes,
            mkp = t.makes * 2,
            max = MAX_MAKES_A_MONTH,
        ),
        true,
    )
}
