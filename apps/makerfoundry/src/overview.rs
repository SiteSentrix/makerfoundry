//! The admins' overview (MF-16): the site's numbers on one page at `/admin/overview`: accounts,
//! designs, downloads, sales and MakerFoundry's fees (from the ledger), and what's waiting in the
//! report queue. Only admins (`MF_ADMINS`) see it.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::private;
use crate::moderation::admin;
use crate::money::dollars;
use crate::pages::escape;

/// One number, or 0 if it can't be counted just now (logged).
async fn n(pool: &PgPool, sql: &str) -> i64 {
    match sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("overview: {e}");
            0
        }
    }
}

fn group(title: &str, facts: &[(&str, String)]) -> String {
    let items: String = facts
        .iter()
        .map(|(label, value)| format!("<div><dt>{}</dt><dd>{}</dd></div>", escape(label), escape(value)))
        .collect();
    format!(r#"<h2>{}</h2><dl class="facts">{items}</dl>"#, escape(title))
}

pub async fn overview(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (_, pool) = match admin(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let p = &pool;
    let count = |v: i64| v.to_string();
    let people = group(
        "People",
        &[
            (
                "Accounts",
                count(n(p, "SELECT count(*) FROM mf_accounts WHERE email <> ''").await),
            ),
            (
                "New this week",
                count(
                    n(
                        p,
                        "SELECT count(*) FROM mf_accounts WHERE created_at > now() - interval '7 days'",
                    )
                    .await,
                ),
            ),
            ("Maker profiles", count(n(p, "SELECT count(*) FROM mf_makers").await)),
            (
                "Sellers ready",
                count(n(p, "SELECT count(*) FROM mf_sellers WHERE charges_enabled").await),
            ),
            ("Follows", count(n(p, "SELECT count(*) FROM mf_follows").await)),
        ],
    );
    let designs = group(
        "Designs",
        &[
            (
                "Published",
                count(n(p, "SELECT count(*) FROM mf_designs WHERE status = 'published'").await),
            ),
            (
                "Embroidery",
                count(
                    n(
                        p,
                        "SELECT count(*) FROM mf_designs WHERE status = 'published' AND kind = 'embroidery'",
                    )
                    .await,
                ),
            ),
            (
                "Laser",
                count(
                    n(
                        p,
                        "SELECT count(*) FROM mf_designs WHERE status = 'published' AND kind = 'laser'",
                    )
                    .await,
                ),
            ),
            (
                "Paid",
                count(
                    n(
                        p,
                        "SELECT count(*) FROM mf_designs WHERE status = 'published' AND price_cents > 0",
                    )
                    .await,
                ),
            ),
            (
                "Drafts",
                count(n(p, "SELECT count(*) FROM mf_designs WHERE status = 'draft'").await),
            ),
            (
                "Taken down",
                count(n(p, "SELECT count(*) FROM mf_designs WHERE status = 'removed'").await),
            ),
            (
                "New this week",
                count(
                    n(
                        p,
                        "SELECT count(*) FROM mf_designs WHERE published_at > now() - interval '7 days'",
                    )
                    .await,
                ),
            ),
            (
                "Downloads",
                count(n(p, "SELECT coalesce(sum(downloads), 0)::bigint FROM mf_designs").await),
            ),
            ("Makes", count(n(p, "SELECT count(*) FROM mf_makes").await)),
            ("Comments", count(n(p, "SELECT count(*) FROM mf_comments").await)),
        ],
    );
    // money, from the ledger (refunds already reversed there)
    let sales = group(
        "Sales",
        &[
            (
                "Sales",
                count(n(p, "SELECT count(*) FROM mf_orders WHERE status = 'paid'").await),
            ),
            (
                "Refunded",
                count(n(p, "SELECT count(*) FROM mf_orders WHERE status = 'refunded'").await),
            ),
            (
                "Paid by buyers",
                dollars(n(p, "SELECT coalesce(sum(cents), 0)::bigint FROM mf_ledger").await),
            ),
            (
                "To sellers",
                dollars(n(p, "SELECT coalesce(sum(cents), 0)::bigint FROM mf_ledger WHERE kind = 'seller'").await),
            ),
            (
                "MakerFoundry's fees",
                dollars(n(p, "SELECT coalesce(sum(cents), 0)::bigint FROM mf_ledger WHERE kind = 'fee'").await),
            ),
            (
                "Card fees",
                dollars(n(p, "SELECT coalesce(sum(cents), 0)::bigint FROM mf_ledger WHERE kind = 'processing'").await),
            ),
            (
                "Sales tax collected",
                dollars(n(p, "SELECT coalesce(sum(cents), 0)::bigint FROM mf_ledger WHERE kind = 'tax'").await),
            ),
            (
                "Fees this month",
                dollars(
                    n(
                        p,
                        "SELECT coalesce(sum(cents), 0)::bigint FROM mf_ledger WHERE kind = 'fee' AND at >= date_trunc('month', now())",
                    )
                    .await,
                ),
            ),
        ],
    );
    let reports = group(
        "Waiting in the queue",
        &[
            ("Copyright notices", count(n(p, "SELECT count(*) FROM mf_reports WHERE status = 'open' AND kind = 'dmca'").await)),
            ("Counter-notices", count(n(p, "SELECT count(*) FROM mf_reports WHERE status = 'open' AND kind = 'counter'").await)),
            ("Reports", count(n(p, "SELECT count(*) FROM mf_reports WHERE status = 'open' AND kind IN ('report', 'make', 'comment')").await)),
        ],
    );
    private(
        "Overview · MakerFoundry",
        &format!(
            r#"<section class="design"><h1>Overview</h1><p class="muted"><a href="/admin">Reports</a> · <a href="/admin/orders">Orders</a></p>{people}{designs}{sales}{reports}</section>"#
        ),
        true,
    )
    .into_response()
}
