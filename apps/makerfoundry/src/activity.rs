//! A maker's activity (MF-22): what other people did with their designs, newest first, on
//! `/activity`: comments, makes, sales and refunds, and new followers. It's drawn from those
//! tables, nothing is copied; the only thing kept is when the maker last looked
//! (`mf_activity_seen`), so what came after shows as new, and the account page counts it.
//!
//! Buyers and followers aren't named (who follows whom isn't shown; a sale says what sold).

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{current_user, private};
use crate::money::dollars;
use crate::pages::{escape, summary};

/// Events shown on `/activity`, and the most counted as new.
const SHOWN: i64 = 100;

/// Every event about account `$1`'s designs and profile, by someone else: `kind`, `at`, the
/// design (id and title), who (handle and name, for comments and makes), a line of what they
/// wrote, and an amount (cents, for sales and refunds).
const EVENTS: &str = "
    SELECT 'comment' AS kind, c.created_at AS at, d.id AS design_id, d.title, m.handle, m.name,
           left(c.body, 300) AS detail, NULL::bigint AS amount
      FROM mf_comments c JOIN mf_designs d ON d.id = c.design_id
      LEFT JOIN mf_makers m ON m.account_id = c.account_id
     WHERE d.account_id = $1 AND c.account_id <> $1
    UNION ALL
    SELECT 'make', k.created_at, d.id, d.title, m.handle, m.name, left(k.note, 300), NULL
      FROM mf_makes k JOIN mf_designs d ON d.id = k.design_id
      LEFT JOIN mf_makers m ON m.account_id = k.account_id
     WHERE d.account_id = $1 AND k.account_id <> $1
    UNION ALL
    SELECT 'sale', o.paid_at, o.design_id, o.title, NULL, NULL, '', o.seller_cents
      FROM mf_orders o WHERE o.seller_account = $1 AND o.paid_at IS NOT NULL
    UNION ALL
    SELECT 'refund', o.refunded_at, o.design_id, o.title, NULL, NULL, '', o.seller_cents
      FROM mf_orders o WHERE o.seller_account = $1 AND o.refunded_at IS NOT NULL
    UNION ALL
    SELECT 'follow', f.at, NULL, '', NULL, NULL, '', NULL
      FROM mf_follows f WHERE f.maker_account = $1";

/// When `account` last looked (the start of time if never).
const SEEN: &str = "coalesce((SELECT seen_at FROM mf_activity_seen WHERE account_id = $1), '-infinity'::timestamptz)";

/// How many events `account` hasn't seen (at most `SHOWN`).
pub async fn unseen(pool: &PgPool, account: i64) -> i64 {
    let sql = format!("SELECT count(*) FROM (SELECT 1 FROM ({EVENTS}) e WHERE e.at > {SEEN} LIMIT {SHOWN}) n");
    sqlx::query_scalar(&sql)
        .bind(account)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// "Activity", with how many are new: for the account page.
pub async fn link_html(pool: &PgPool, account: i64) -> String {
    let n = unseen(pool, account).await;
    let new = match n {
        0 => String::new(),
        n if n >= SHOWN => format!(r#" <strong class="new">{SHOWN}+ new</strong>"#),
        n => format!(r#" <strong class="new">{n} new</strong>"#),
    };
    format!(
        r#"<p><a href="/activity">Activity</a>{new}: comments, makes and sales of your designs, and new followers.</p>"#
    )
}

type Row = (
    String,
    String,
    Option<i64>,
    String,
    Option<String>,
    Option<String>,
    String,
    Option<i64>,
    bool,
    // `at` as Postgres writes it, stored back as it is (no date library needed)
    String,
);

pub async fn page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin?next=/activity").into_response();
    };
    let sql = format!(
        "SELECT e.kind, to_char(e.at, 'YYYY-MM-DD'), e.design_id, e.title, e.handle, e.name, e.detail,
                e.amount, e.at > {SEEN}, e.at::text
           FROM ({EVENTS}) e ORDER BY e.at DESC LIMIT {SHOWN}"
    );
    let rows: Vec<Row> = match sqlx::query_as(&sql).bind(user.account_id).fetch_all(pool).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("activity: {e}");
            Vec::new()
        }
    };
    // seen up to the newest shown: anything that lands while this page is made stays new
    if let Some(newest) = rows.first().map(|r| r.9.clone())
        && let Err(e) = sqlx::query(
            "INSERT INTO mf_activity_seen (account_id, seen_at) VALUES ($1, $2::timestamptz)
             ON CONFLICT (account_id) DO UPDATE SET seen_at = greatest(mf_activity_seen.seen_at, excluded.seen_at)",
        )
        .bind(user.account_id)
        .bind(newest)
        .execute(pool)
        .await
    {
        eprintln!("activity: seen: {e}");
    }
    let body = if rows.is_empty() {
        r#"<p class="lead">Nothing yet. When people comment on your designs, post what they made from them, buy them or follow you, it shows here.</p><p><a href="/designs">Your designs</a></p>"#.to_string()
    } else {
        let items: Vec<String> = rows.iter().map(item_html).collect();
        format!(r#"<ul class="activity">{}</ul>"#, items.concat())
    };
    private(
        "Activity · MakerFoundry",
        &format!(r#"<section class="design"><h1>Activity</h1>{body}</section>"#),
        true,
    )
}

fn item_html(r: &Row) -> String {
    let (kind, date, design, title, handle, name, detail, amount, new, _) = r;
    let design_link = match design {
        Some(id) => {
            let t = if title.is_empty() { "an untitled design" } else { title };
            format!(r#"<a href="/d/{id}">{}</a>"#, escape(t))
        }
        None => format!("“{}” (deleted)", escape(title)),
    };
    let who = match (handle, name) {
        (Some(h), Some(n)) => format!(r#"<a href="/makers/{}">{}</a>"#, escape(h), escape(n)),
        _ => "Someone".to_string(),
    };
    let quote = if detail.trim().is_empty() {
        String::new()
    } else {
        format!(r#"<q>{}</q>"#, escape(&summary(detail, 160)))
    };
    let money = amount.map(dollars).unwrap_or_default();
    let what = match kind.as_str() {
        "comment" => format!("{who} commented on {design_link}{quote}"),
        "make" => format!("{who} posted a make of {design_link}{quote}"),
        "sale" => format!("{design_link} sold: {money} to you"),
        "refund" => format!("A sale of {design_link} was refunded: {money} less for you"),
        "follow" => "Someone started following you".to_string(),
        _ => return String::new(),
    };
    let new = if *new {
        r#" <strong class="new">new</strong>"#
    } else {
        ""
    };
    format!(r#"<li><span class="muted">{date}</span>{new} {what}</li>"#)
}
