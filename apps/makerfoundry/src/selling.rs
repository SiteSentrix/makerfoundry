//! Selling (MF-4): a seller sets up payouts with Stripe (an Express account, paid monthly), gives
//! a design a price, and buyers pay on Stripe's checkout page. A sale is delivered (the buyer may
//! download) only once Stripe confirms the payment, by a signed webhook or by asking Stripe when
//! the buyer comes back. Every sale and refund is written to the ledger (`mf_ledger`) in cents:
//! the seller's share, MakerFoundry's 10% fee and the card fee (`money::split`).
//!
//! Sales tax (MF-24, on with `MF_STRIPE_TAX`): Stripe Tax adds it at checkout, from the buyer's
//! address; MakerFoundry is the marketplace and liable for it
//! (https://docs.stripe.com/tax/tax-for-marketplaces), so it's MakerFoundry's (`tax` rows), never
//! the seller's. A destination charge moves the whole payment, tax included, to the seller's Stripe
//! account, so the tax is taken back with a transfer reversal once the sale is paid
//! (`withhold_tax`), and a taxed sale's refund takes the price back from the seller before
//! refunding the buyer everything they paid.
//!
//! Pages: `/account/selling` (setting up), `/account/sales` (a seller's sales), `/library` (a
//! buyer's purchases), `/admin/orders` (refunds). Stripe's webhook: `/stripe/webhook`.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Row};

use crate::AppState;
use crate::accounts::{User, current_user, forbidden, private, same_origin};
use crate::designs;
use crate::limits;
use crate::listings::slug;
use crate::money::{self, dollars, split};
use crate::pages::escape;
use crate::stripe::Stripe;

/// The label on MakerFoundry's checkouts (`metadata[app]`): the Stripe account may also take
/// StitchFoundry's payments, whose events reach this webhook too.
pub const APP: &str = "makerfoundry";

/// The ledger's account for MakerFoundry itself.
pub const MAKERFOUNDRY: i64 = 0;

/// A seller's Stripe account, as Stripe last described it.
#[derive(Debug, Clone)]
pub struct Seller {
    pub stripe_account: String,
    pub details_submitted: bool,
    pub charges_enabled: bool,
    pub payouts_enabled: bool,
}

pub async fn seller(pool: &PgPool, account_id: i64) -> Option<Seller> {
    sqlx::query_as::<_, (String, bool, bool, bool)>(
        "SELECT stripe_account, details_submitted, charges_enabled, payouts_enabled FROM mf_sellers WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|(stripe_account, details_submitted, charges_enabled, payouts_enabled)| Seller {
        stripe_account,
        details_submitted,
        charges_enabled,
        payouts_enabled,
    })
}

/// Whether `account_id` can sell now: selling is switched on, they may publish, and Stripe says
/// their account can take payments.
pub async fn can_sell(state: &AppState, pool: &PgPool, account_id: i64) -> bool {
    state.stripe.is_some()
        && state.publishers.allows(account_id)
        && seller(pool, account_id).await.is_some_and(|s| s.charges_enabled)
}

/// Whether `buyer` has paid for `design` (and not been refunded).
pub async fn bought(pool: &PgPool, buyer: i64, design: i64) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM mf_orders WHERE buyer_account = $1 AND design_id = $2 AND status = 'paid'",
    )
    .bind(buyer)
    .bind(design)
    .fetch_one(pool)
    .await
    .is_ok_and(|n| n > 0)
}

/// Whether design `id` has ever been paid for (a design people bought isn't deleted, so they keep
/// their download).
pub async fn has_sales(pool: &PgPool, id: i64) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM mf_orders WHERE design_id = $1 AND status IN ('paid', 'refunded')",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_or(true, |n| n > 0)
}

/// How long a Stripe checkout page lasts.
const CHECKOUT_SECS: i64 = 2 * 3600;

/// Orders never paid for, a day on (long after their checkout page has gone), expire. Run every
/// few minutes; → how many.
pub async fn expire_orders(pool: &PgPool) -> Result<u64, sqlx::Error> {
    sqlx::query(
        "UPDATE mf_orders SET status = 'expired' WHERE status = 'pending' AND created_at < now() - interval '1 day'",
    )
    .execute(pool)
    .await
    .map(|r| r.rows_affected())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn is_stripe_id(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix).is_some_and(|rest| {
        !rest.is_empty() && rest.len() < 200 && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    })
}

fn problem_page(title: &str, message: &str) -> Response {
    private(
        &format!("{title} · MakerFoundry"),
        &format!(
            r#"<section class="form"><h1>{}</h1><p class="lead">{message}</p></section>"#,
            escape(title)
        ),
        true,
    )
}

// ---------------------------------------------------------------- setting up payouts

fn selling_html(state: &AppState, s: Option<&Seller>, allowed: bool) -> String {
    let example = split(500).map_or(String::new(), |x| {
        format!(
            "For a {price} design: MakerFoundry's fee {fee}, the card fee {card}, and you get {you}.",
            price = dollars(x.price),
            fee = dollars(x.fee),
            card = dollars(x.processing),
            you = dollars(x.seller),
        )
    });
    let terms = format!(
        r#"<p>MakerFoundry keeps 10% of each sale, and the card fee (2.9% + 30¢) comes out of your share too. {example}</p>
<p>Payments go through Stripe, which pays you once a month into your bank account. Stripe asks who you are and where to pay you; MakerFoundry never sees your bank or card details.</p>"#
    );
    let state_html = if state.stripe.is_none() {
        r#"<p class="problem">Selling isn't switched on yet. Free designs can be shared now.</p>"#.to_string()
    } else if !allowed {
        r#"<p class="problem">Selling opens when MakerFoundry launches.</p>"#.to_string()
    } else {
        match s {
            None => format!(
                r#"<form method="post" action="/account/selling/start"><button type="submit">Set up payouts with Stripe</button></form>{agree}"#,
                agree = if state.launched {
                    r#"<p class="muted">By setting up payouts you agree to the <a href="/seller-terms">Seller Terms</a>.</p>"#
                } else {
                    ""
                }
            ),
            Some(s) if !s.charges_enabled => format!(
                r#"<p class="problem">{why}</p><form method="post" action="/account/selling/start"><button type="submit">Carry on with Stripe</button></form>"#,
                why = if s.details_submitted {
                    "Stripe is checking your details, or needs something more. You can sell once it's done."
                } else {
                    "Your Stripe set-up isn't finished yet."
                }
            ),
            Some(s) => format!(
                r#"<p><strong>You can sell.</strong> Give a design a price in its details.{payouts}</p>
<p><a href="/account/sales">Your sales</a></p>
<form method="post" action="/account/selling/dashboard"><button type="submit" class="secondary">Your payouts on Stripe</button></form>"#,
                payouts = if s.payouts_enabled {
                    ""
                } else {
                    " Stripe needs something more before it can pay you: see your payouts on Stripe."
                }
            ),
        }
    };
    format!(
        r#"<section class="form"><h1>Selling</h1>{terms}{state_html}<p><a href="/account">Your account</a></p></section>"#
    )
}

pub async fn selling_page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin").into_response();
    };
    let s = seller(pool, user.account_id).await;
    private(
        "Selling · MakerFoundry",
        &selling_html(&state, s.as_ref(), state.publishers.allows(user.account_id)),
        true,
    )
}

/// The seller's Stripe account (made the first time), then Stripe's set-up pages.
pub async fn start(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin").into_response();
    };
    let Some(stripe) = state.stripe.as_ref() else {
        return Redirect::to("/account/selling").into_response();
    };
    if !state.publishers.allows(user.account_id) {
        return Redirect::to("/account/selling").into_response();
    }
    if state.launched {
        crate::legal::agree(pool, user.account_id, &[&crate::legal::SELLER_TERMS]).await;
    }
    let account = match seller(pool, user.account_id).await {
        Some(s) => s.stripe_account,
        None => match new_account(stripe, pool, user.account_id).await {
            Ok(a) => a,
            Err(e) => {
                eprintln!("selling: account for {}: {e}", user.account_id);
                return problem_page(
                    "Selling",
                    "Stripe can't be reached just now. Please try again in a few minutes.",
                );
            }
        },
    };
    let link = stripe
        .post(
            "/v1/account_links",
            &[
                ("account", account),
                ("type", "account_onboarding".into()),
                ("refresh_url", format!("{}/account/selling", stripe.public_url)),
                ("return_url", format!("{}/account/selling/return", stripe.public_url)),
            ],
            None,
        )
        .await;
    match link.as_ref().ok().and_then(|v| v.get("url")).and_then(Value::as_str) {
        Some(url) if url.starts_with("https://") => Redirect::to(url).into_response(),
        _ => {
            if let Err(e) = link {
                eprintln!("selling: link for {}: {e}", user.account_id);
            }
            problem_page(
                "Selling",
                "Stripe can't be reached just now. Please try again in a few minutes.",
            )
        }
    }
}

async fn new_account(stripe: &Stripe, pool: &PgPool, account_id: i64) -> Result<String, String> {
    let v = stripe
        .post(
            "/v1/accounts",
            &[
                ("type", "express".into()),
                ("country", "US".into()),
                ("capabilities[card_payments][requested]", "true".into()),
                ("capabilities[transfers][requested]", "true".into()),
                (
                    "business_profile[product_description]",
                    "Embroidery and laser-cut design files sold on MakerFoundry".into(),
                ),
                ("settings[payouts][schedule][interval]", "monthly".into()),
                ("settings[payouts][schedule][monthly_anchor]", "1".into()),
                ("metadata[makerfoundry_account]", account_id.to_string()),
            ],
            // one Stripe account per seller, even if the button is pressed twice at once
            Some(&format!("mf-seller-{account_id}")),
        )
        .await
        .map_err(|e| e.to_string())?;
    let id = v.get("id").and_then(Value::as_str).unwrap_or("");
    if !is_stripe_id(id, "acct_") {
        return Err("Stripe's answer has no account".into());
    }
    sqlx::query(
        "INSERT INTO mf_sellers (account_id, stripe_account) VALUES ($1, $2) ON CONFLICT (account_id) DO NOTHING",
    )
    .bind(account_id)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    seller(pool, account_id)
        .await
        .map(|s| s.stripe_account)
        .ok_or_else(|| "the seller wasn't kept".into())
}

/// Stripe's description of an account (`GET /v1/accounts/<id>` or `account.updated`) kept.
async fn save_status(conn: &mut PgConnection, account: &Value) -> Result<(), sqlx::Error> {
    let flag = |k: &str| account.get(k).and_then(Value::as_bool).unwrap_or(false);
    sqlx::query(
        "UPDATE mf_sellers SET details_submitted = $2, charges_enabled = $3, payouts_enabled = $4 WHERE stripe_account = $1",
    )
    .bind(account.get("id").and_then(Value::as_str).unwrap_or(""))
    .bind(flag("details_submitted"))
    .bind(flag("charges_enabled"))
    .bind(flag("payouts_enabled"))
    .execute(conn)
    .await
    .map(|_| ())
}

/// Back from Stripe's set-up pages: ask Stripe how it went.
pub async fn returned(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (Some(user), Some(pool), Some(stripe)) = (
        current_user(&state, &headers).await,
        state.db.as_ref(),
        state.stripe.as_ref(),
    ) else {
        return Redirect::to("/account/selling").into_response();
    };
    if let Some(s) = seller(pool, user.account_id).await
        && is_stripe_id(&s.stripe_account, "acct_")
    {
        match stripe.get(&format!("/v1/accounts/{}", s.stripe_account)).await {
            Ok(v) => {
                if let Ok(mut conn) = pool.acquire().await
                    && let Err(e) = save_status(&mut conn, &v).await
                {
                    eprintln!("selling: status: {e}");
                }
            }
            Err(e) => eprintln!("selling: status for {}: {e}", user.account_id),
        }
    }
    Redirect::to("/account/selling").into_response()
}

/// The seller's Stripe Express dashboard (payouts, bank details), by a one-time link.
pub async fn dashboard(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some(user), Some(pool), Some(stripe)) = (
        current_user(&state, &headers).await,
        state.db.as_ref(),
        state.stripe.as_ref(),
    ) else {
        return Redirect::to("/account/selling").into_response();
    };
    let Some(s) = seller(pool, user.account_id)
        .await
        .filter(|s| is_stripe_id(&s.stripe_account, "acct_"))
    else {
        return Redirect::to("/account/selling").into_response();
    };
    match stripe
        .post(&format!("/v1/accounts/{}/login_links", s.stripe_account), &[], None)
        .await
    {
        Ok(v) => match v.get("url").and_then(Value::as_str) {
            Some(url) if url.starts_with("https://") => Redirect::to(url).into_response(),
            _ => problem_page(
                "Selling",
                "Stripe can't be reached just now. Please try again in a few minutes.",
            ),
        },
        Err(e) => {
            eprintln!("selling: dashboard for {}: {e}", user.account_id);
            problem_page(
                "Selling",
                "Stripe can't be reached just now. Please try again in a few minutes.",
            )
        }
    }
}

// ---------------------------------------------------------------- buying

/// A buyer starts paying for design `id`: an order is made with its amounts fixed, then Stripe's
/// checkout page takes the payment.
pub async fn buy(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin").into_response();
    };
    let back = Redirect::to(&format!("/d/{id}")).into_response();
    let Some(stripe) = state.stripe.as_ref() else {
        return back;
    };
    let d = match designs::design(pool, id).await {
        Ok(Some(d)) if d.status == "published" && d.price_cents > 0 && d.account_id != user.account_id => d,
        _ => return back,
    };
    if bought(pool, user.account_id, id).await {
        return Redirect::to("/library").into_response();
    }
    if !state.limits.allow(limits::CHECKOUT, &user.account_id.to_string()) {
        return problem_page(
            "Buy a design",
            "You've started a lot of purchases in the last hour. Please wait a while and try again.",
        );
    }
    let (Some(parts), Some(s)) = (split(d.price_cents), seller(pool, d.account_id).await) else {
        return problem_page("Buy a design", "This design can't be bought just now.");
    };
    if !s.charges_enabled || !state.publishers.allows(d.account_id) {
        return problem_page("Buy a design", "This design can't be bought just now.");
    }
    let order: Result<i64, _> = sqlx::query_scalar(
        "INSERT INTO mf_orders (buyer_account, design_id, seller_account, price_cents, fee_cents, processing_cents, seller_cents, title)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(user.account_id)
    .bind(id)
    .bind(d.account_id)
    .bind(parts.price)
    .bind(parts.fee)
    .bind(parts.processing)
    .bind(parts.seller)
    .bind(&d.title)
    .fetch_one(pool)
    .await;
    let order = match order {
        Ok(o) => o,
        Err(e) => {
            eprintln!("selling: order for {id}: {e}");
            return problem_page(
                "Buy a design",
                "Your purchase couldn't be started just now. Please try again.",
            );
        }
    };
    let name: String = d.title.chars().take(200).collect();
    let mut form: Vec<(&str, String)> = vec![
        ("mode", "payment".into()),
        // the page lasts 2 hours; the order expires a day later (`expire_orders`)
        ("expires_at", (now_secs() + CHECKOUT_SECS).to_string()),
        ("client_reference_id", order.to_string()),
        ("success_url", format!("{}/library?order={order}", stripe.public_url)),
        ("cancel_url", format!("{}/d/{id}/{}", stripe.public_url, slug(&d.title))),
        ("line_items[0][quantity]", "1".into()),
        ("line_items[0][price_data][currency]", "usd".into()),
        ("line_items[0][price_data][unit_amount]", parts.price.to_string()),
        ("line_items[0][price_data][product_data][name]", name),
        (
            "payment_intent_data[application_fee_amount]",
            parts.application_fee().to_string(),
        ),
        (
            "payment_intent_data[transfer_data][destination]",
            s.stripe_account.clone(),
        ),
        ("payment_intent_data[metadata][order]", order.to_string()),
        ("payment_intent_data[metadata][app]", APP.into()),
        ("metadata[order]", order.to_string()),
        ("metadata[app]", APP.into()),
    ];
    if let Some(code) = &stripe.tax_code {
        // tax on top of the price, worked out by Stripe Tax from the buyer's address, with
        // MakerFoundry liable (https://docs.stripe.com/tax/checkout)
        form.extend([
            ("automatic_tax[enabled]", "true".to_string()),
            ("automatic_tax[liability][type]", "self".into()),
            ("billing_address_collection", "required".into()),
            ("line_items[0][price_data][tax_behavior]", "exclusive".into()),
            ("line_items[0][price_data][product_data][tax_code]", code.clone()),
        ]);
    }
    let session = stripe
        .post("/v1/checkout/sessions", &form, Some(&format!("mf-order-{order}")))
        .await;
    let (sid, url) = match &session {
        Ok(v) => (
            v.get("id").and_then(Value::as_str).unwrap_or(""),
            v.get("url").and_then(Value::as_str).unwrap_or(""),
        ),
        Err(_) => ("", ""),
    };
    if !is_stripe_id(sid, "cs_") || !url.starts_with("https://") {
        if let Err(e) = &session {
            eprintln!("selling: checkout for order {order}: {e}");
        }
        return problem_page(
            "Buy a design",
            "Stripe can't be reached just now. Please try again in a few minutes.",
        );
    }
    if let Err(e) = sqlx::query("UPDATE mf_orders SET stripe_session = $2 WHERE id = $1")
        .bind(order)
        .bind(sid)
        .execute(pool)
        .await
    {
        eprintln!("selling: order {order}: {e}");
        return problem_page(
            "Buy a design",
            "Your purchase couldn't be started just now. Please try again.",
        );
    }
    Redirect::to(url).into_response()
}

/// A Checkout Session Stripe says is paid → order `order` paid and its sale in the ledger, once.
/// `false` (and nothing changes) unless the session is paid and is this order's, for its price
/// (plus the tax Stripe added, at most the price itself). An expired order counts too: a delayed
/// payment (a bank debit) is confirmed days after checkout, after `expire_orders` has run
/// (https://docs.stripe.com/payments/checkout/fulfill-orders).
async fn confirm_paid(conn: &mut PgConnection, order: i64, session: &Value) -> Result<bool, sqlx::Error> {
    if session.get("payment_status").and_then(Value::as_str) != Some("paid") {
        return Ok(false);
    }
    let sid = session.get("id").and_then(Value::as_str).unwrap_or("");
    let total = session.get("amount_total").and_then(Value::as_i64).unwrap_or(-1);
    let tax = session
        .pointer("/total_details/amount_tax")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    // what the price was, before the tax (a session without tax says only its total)
    let amount = session.get("amount_subtotal").and_then(Value::as_i64).unwrap_or(total);
    if !(0..=amount).contains(&tax) || amount.checked_add(tax) != Some(total) {
        eprintln!("selling: order {order}: a paid session whose amounts don't add up ({sid})");
        return Ok(false);
    }
    let currency = session.get("currency").and_then(Value::as_str).unwrap_or("");
    let intent = session.get("payment_intent").and_then(Value::as_str);
    let row = sqlx::query(
        "UPDATE mf_orders SET status = 'paid', paid_at = now(), stripe_payment_intent = $3, tax_cents = $6
         WHERE id = $1 AND stripe_session = $2 AND status IN ('pending', 'expired') AND price_cents = $4 AND currency = $5
         RETURNING seller_account, fee_cents, processing_cents, seller_cents",
    )
    .bind(order)
    .bind(sid)
    .bind(intent)
    .bind(amount)
    .bind(currency)
    .bind(tax)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else {
        let status: Option<String> = sqlx::query_scalar("SELECT status FROM mf_orders WHERE id = $1")
            .bind(order)
            .fetch_optional(&mut *conn)
            .await?;
        if matches!(status.as_deref(), Some("pending" | "expired")) {
            eprintln!("selling: order {order}: a paid session that doesn't match it ({sid}, {amount} {currency})");
        }
        return Ok(false);
    };
    let seller: i64 = row.try_get("seller_account")?;
    for (account, kind, cents) in [
        (seller, "seller", row.try_get::<i64, _>("seller_cents")?),
        (MAKERFOUNDRY, "fee", row.try_get("fee_cents")?),
        (MAKERFOUNDRY, "processing", row.try_get("processing_cents")?),
    ] {
        ledger(conn, order, account, kind, cents, "sale").await?;
    }
    if tax > 0 {
        ledger(conn, order, MAKERFOUNDRY, "tax", tax, "sale").await?;
    }
    Ok(true)
}

/// A paid order refunded, once: the whole price taken back from the seller (Stripe reverses the
/// whole transfer; MakerFoundry still holds the application fee until `record_fee_return`).
async fn record_refund(conn: &mut PgConnection, order: i64) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "UPDATE mf_orders SET status = 'refunded', refunded_at = now() WHERE id = $1 AND status = 'paid'
         RETURNING seller_account, price_cents, tax_cents",
    )
    .bind(order)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else { return Ok(false) };
    let seller: i64 = row.try_get("seller_account")?;
    let price: i64 = row.try_get("price_cents")?;
    let tax: i64 = row.try_get("tax_cents")?;
    ledger(conn, order, seller, "seller", -price, "refund").await?;
    if tax > 0 {
        // the buyer gets the tax back too
        ledger(conn, order, MAKERFOUNDRY, "tax", -tax, "refund").await?;
    }
    Ok(true)
}

/// A refunded order's 10% given back to its seller, once: the seller is then out only the card
/// fee, which Stripe keeps (the owner's decision, 2026-10-22; Seller Terms § 7).
async fn record_fee_return(conn: &mut PgConnection, order: i64) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "UPDATE mf_orders SET fee_returned_at = now()
         WHERE id = $1 AND status = 'refunded' AND fee_returned_at IS NULL
         RETURNING seller_account, fee_cents",
    )
    .bind(order)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(row) = row else { return Ok(false) };
    let seller: i64 = row.try_get("seller_account")?;
    let fee: i64 = row.try_get("fee_cents")?;
    ledger(conn, order, seller, "seller", fee, "fee_return").await?;
    ledger(conn, order, MAKERFOUNDRY, "fee", -fee, "fee_return").await?;
    Ok(true)
}

/// The transfer that moved a sale's payment to its seller's Stripe account.
async fn transfer_of(stripe: &Stripe, intent: Option<&str>) -> Result<String, String> {
    let intent = intent
        .filter(|i| is_stripe_id(i, "pi_"))
        .ok_or_else(|| "the order has no Stripe payment.".to_string())?;
    let payment = stripe
        .get(&format!("/v1/payment_intents/{intent}?expand[]=latest_charge"))
        .await
        .map_err(|e| e.to_string())?;
    payment
        .pointer("/latest_charge/transfer")
        .and_then(Value::as_str)
        .filter(|t| is_stripe_id(t, "tr_"))
        .map(str::to_string)
        .ok_or_else(|| "Stripe shows no transfer to the seller for this sale.".to_string())
}

/// Whether `transfer` already has MakerFoundry's reversal for `order` and `reason`: a reversal
/// isn't made twice, even when recording the first one failed.
async fn reversed(stripe: &Stripe, transfer: &str, order: i64, reason: &str) -> Result<bool, String> {
    let t = stripe
        .get(&format!("/v1/transfers/{transfer}"))
        .await
        .map_err(|e| e.to_string())?;
    let order = order.to_string();
    Ok(t.pointer("/reversals/data")
        .and_then(Value::as_array)
        .is_some_and(|rs| {
            rs.iter().any(|r| {
                r.pointer("/metadata/order").and_then(Value::as_str) == Some(order.as_str())
                    && r.pointer("/metadata/reason").and_then(Value::as_str) == Some(reason)
            })
        }))
}

/// Takes `cents` back from the seller's share of `order`'s payment, once for `reason`.
async fn reverse(stripe: &Stripe, intent: Option<&str>, order: i64, cents: i64, reason: &str) -> Result<(), String> {
    let transfer = transfer_of(stripe, intent).await?;
    if reversed(stripe, &transfer, order, reason).await? {
        return Ok(());
    }
    stripe
        .post(
            &format!("/v1/transfers/{transfer}/reversals"),
            &[
                ("amount", cents.to_string()),
                ("metadata[order]", order.to_string()),
                ("metadata[reason]", reason.to_string()),
            ],
            Some(&format!("mf-reverse-{reason}-{order}")),
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// A paid, taxed sale's tax taken back from the seller's Stripe account to MakerFoundry's, once
/// (https://docs.stripe.com/tax/tax-for-marketplaces#withhold-collected-tax-amount): after the
/// payment's confirmed, and again from the sweep until it's done.
pub async fn withhold_tax(stripe: &Stripe, pool: &PgPool, order: i64) -> Result<(), String> {
    let row = sqlx::query(
        "SELECT stripe_payment_intent, tax_cents FROM mf_orders
         WHERE id = $1 AND status = 'paid' AND tax_cents > 0 AND tax_withheld_at IS NULL",
    )
    .bind(order)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some(row) = row else { return Ok(()) };
    let intent: Option<String> = row.try_get("stripe_payment_intent").unwrap_or(None);
    let tax: i64 = row.try_get("tax_cents").unwrap_or(0);
    reverse(stripe, intent.as_deref(), order, tax, "sales_tax").await?;
    sqlx::query("UPDATE mf_orders SET tax_withheld_at = now() WHERE id = $1 AND tax_withheld_at IS NULL")
        .bind(order)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The sweep: taxed sales whose tax is still with the seller (Stripe couldn't be reached when they
/// were paid), up to 50 at a time.
pub async fn withhold_taxes(stripe: &Stripe, pool: &PgPool) -> Result<(), sqlx::Error> {
    let orders: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM mf_orders WHERE status = 'paid' AND tax_cents > 0 AND tax_withheld_at IS NULL
         ORDER BY id LIMIT 50",
    )
    .fetch_all(pool)
    .await?;
    for order in orders {
        if let Err(e) = withhold_tax(stripe, pool, order).await {
            eprintln!("selling: tax of order {order}: {e}");
        }
    }
    Ok(())
}

async fn ledger(
    conn: &mut PgConnection,
    order: i64,
    account: i64,
    kind: &str,
    cents: i64,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mf_ledger (order_id, account_id, kind, cents, reason) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (order_id, kind, reason) DO NOTHING",
    )
    .bind(order)
    .bind(account)
    .bind(kind)
    .bind(cents)
    .bind(reason)
    .execute(conn)
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------- Stripe's webhook

/// Stripe's events: believed only with a good signature, each handled once (`mf_stripe_events`),
/// and only acknowledged once handled, so Stripe sends it again after a failure.
pub async fn webhook(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(stripe) = state.stripe.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let signature = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !stripe.verify(signature, &body, now_secs()) {
        return (StatusCode::BAD_REQUEST, "bad signature").into_response();
    }
    let Ok(event) = serde_json::from_slice::<Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "bad event").into_response();
    };
    let Some(pool) = state.db.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match handle_event(pool, &event).await {
        Ok(()) => {
            if let Some(order) = paid_order(&event)
                && let Err(e) = withhold_tax(stripe, pool, order).await
            {
                // (the sweep tries again)
                eprintln!("selling: tax of order {order}: {e}");
            }
            (StatusCode::OK, "ok").into_response()
        }
        Err(e) => {
            eprintln!(
                "selling: event {}: {e}",
                event.get("id").and_then(Value::as_str).unwrap_or("?")
            );
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// The order of a MakerFoundry checkout that an event says is paid.
fn paid_order(event: &Value) -> Option<i64> {
    let kind = event.get("type").and_then(Value::as_str)?;
    if kind != "checkout.session.completed" && kind != "checkout.session.async_payment_succeeded" {
        return None;
    }
    let object = event.pointer("/data/object")?;
    if object.pointer("/metadata/app").and_then(Value::as_str) != Some(APP) {
        return None;
    }
    object.pointer("/metadata/order")?.as_str()?.parse().ok()
}

async fn handle_event(pool: &PgPool, event: &Value) -> Result<(), sqlx::Error> {
    let id = event.get("id").and_then(Value::as_str).unwrap_or("");
    let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
    let object = event.pointer("/data/object").unwrap_or(&Value::Null);
    if id.is_empty() || id.len() > 255 {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    let new = sqlx::query("INSERT INTO mf_stripe_events (id, kind) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING")
        .bind(id)
        .bind(kind.chars().take(100).collect::<String>())
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if new == 0 {
        // handled before
        return tx.commit().await;
    }
    match kind {
        // (the Stripe account may be shared with StitchFoundry's credit packs: only checkouts
        // labelled as MakerFoundry's are MakerFoundry's)
        "checkout.session.completed" | "checkout.session.async_payment_succeeded"
            if object.pointer("/metadata/app").and_then(Value::as_str) == Some(APP) =>
        {
            let order = object
                .pointer("/metadata/order")
                .and_then(Value::as_str)
                .and_then(|o| o.parse::<i64>().ok());
            if let Some(order) = order {
                confirm_paid(&mut tx, order, object).await?;
            }
        }
        "charge.refunded" => {
            // only a full refund ends the sale (MakerFoundry makes only full ones)
            let full = object.get("refunded").and_then(Value::as_bool) == Some(true);
            if let (true, Some(intent)) = (full, object.get("payment_intent").and_then(Value::as_str)) {
                let order: Option<i64> =
                    sqlx::query_scalar("SELECT id FROM mf_orders WHERE stripe_payment_intent = $1")
                        .bind(intent)
                        .fetch_optional(&mut *tx)
                        .await?;
                if let Some(order) = order {
                    record_refund(&mut tx, order).await?;
                }
            }
        }
        "account.updated" => save_status(&mut tx, object).await?,
        "checkout.session.expired" => {
            sqlx::query("UPDATE mf_orders SET status = 'expired' WHERE stripe_session = $1 AND status = 'pending'")
                .bind(object.get("id").and_then(Value::as_str).unwrap_or(""))
                .execute(&mut *tx)
                .await?;
        }
        _ => {}
    }
    tx.commit().await
}

// ---------------------------------------------------------------- the buyer's library

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct LibraryQuery {
    order: Option<i64>,
}

/// A buyer's purchases. Coming back from Stripe (`?order=`), the order is checked with Stripe
/// straight away, so the download is there without waiting for the webhook.
pub async fn library(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<LibraryQuery>) -> Response {
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin").into_response();
    };
    if let (Some(order), Some(stripe)) = (q.order, state.stripe.as_ref()) {
        let pending: Option<String> = sqlx::query_scalar(
            "SELECT stripe_session FROM mf_orders WHERE id = $1 AND buyer_account = $2 AND status = 'pending'",
        )
        .bind(order)
        .bind(user.account_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        if let Some(sid) = pending.filter(|s| is_stripe_id(s, "cs_")) {
            match stripe.get(&format!("/v1/checkout/sessions/{sid}")).await {
                Ok(session) => {
                    let done = async {
                        let mut tx = pool.begin().await?;
                        confirm_paid(&mut tx, order, &session).await?;
                        tx.commit().await
                    };
                    if let Err(e) = done.await {
                        eprintln!("selling: order {order}: {e}");
                    } else if let Err(e) = withhold_tax(stripe, pool, order).await {
                        eprintln!("selling: tax of order {order}: {e}");
                    }
                }
                Err(e) => eprintln!("selling: order {order}: {e}"),
            }
        }
    }
    let rows = sqlx::query(
        "SELECT o.id, o.design_id, o.title, o.price_cents, o.tax_cents, o.status, to_char(o.created_at, 'YYYY-MM-DD') AS at
         FROM mf_orders o WHERE o.buyer_account = $1 AND (o.status IN ('paid', 'refunded') OR o.id = $2)
         ORDER BY o.created_at DESC LIMIT 500",
    )
    .bind(user.account_id)
    .bind(q.order.unwrap_or(0))
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let items: String = rows
        .iter()
        .map(|r| {
            let design: Option<i64> = r.try_get("design_id").ok().flatten();
            let title: String = r.try_get("title").unwrap_or_default();
            let status: String = r.try_get("status").unwrap_or_default();
            let at: String = r.try_get("at").unwrap_or_default();
            let price: i64 = r.try_get("price_cents").unwrap_or(0);
            let tax: i64 = r.try_get("tax_cents").unwrap_or(0);
            let price = if tax > 0 {
                format!("{} + {} tax", dollars(price), dollars(tax))
            } else {
                dollars(price)
            };
            let name = match design {
                Some(d) => format!(r#"<a href="/d/{d}/{}">{}</a>"#, slug(&title), escape(&title)),
                None => escape(&title),
            };
            let action = match (status.as_str(), design) {
                ("paid", Some(d)) => format!(r#"<a class="button" href="/d/{d}/download">Download</a>"#),
                ("paid", None) => r#"<span class="muted">The design has been deleted.</span>"#.into(),
                ("refunded", _) => r#"<span class="muted">Refunded</span>"#.into(),
                ("expired", _) => r#"<span class="muted">Not paid for: the purchase wasn't finished.</span>"#.into(),
                _ => r#"<span class="muted">Waiting for the payment to be confirmed. This page shows the download once it is: reload it in a minute.</span>"#.into(),
            };
            format!(
                r#"<li class="report"><p><strong>{name}</strong> <span class="muted">· {price} · {at}</span></p><div class="actions">{action}</div></li>"#,
                at = escape(&at),
            )
        })
        .collect();
    let body = if rows.is_empty() {
        r#"<p class="lead">You haven't bought a design yet. <a href="/browse">Browse designs</a></p>"#.to_string()
    } else {
        format!(r#"<ul class="reports">{items}</ul>"#)
    };
    private(
        "Your library · MakerFoundry",
        &format!(
            r#"<section class="design"><h1>Your library</h1><p class="muted">The designs you've bought, to download whenever you need them.</p>{body}</section>"#
        ),
        true,
    )
}

// ---------------------------------------------------------------- the seller's sales

pub async fn sales(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin").into_response();
    };
    let rows = sqlx::query(
        "SELECT o.title, o.design_id, o.price_cents, o.fee_cents, o.processing_cents, o.seller_cents, o.status,
                to_char(o.paid_at, 'YYYY-MM-DD') AS at
         FROM mf_orders o WHERE o.seller_account = $1 AND o.status IN ('paid', 'refunded')
         ORDER BY o.paid_at DESC LIMIT 500",
    )
    .bind(user.account_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    // the totals come from the ledger, which refunds have already reversed
    let earned: i64 = sqlx::query_scalar(
        "SELECT coalesce(sum(cents), 0)::bigint FROM mf_ledger WHERE account_id = $1 AND kind = 'seller'",
    )
    .bind(user.account_id)
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    let lines: String = rows
        .iter()
        .map(|r| {
            let cents = |c: &str| dollars(r.try_get::<i64, _>(c).unwrap_or(0));
            let title: String = r.try_get("title").unwrap_or_default();
            let status: String = r.try_get("status").unwrap_or_default();
            format!(
                r#"<li class="report"><p><strong>{title}</strong> <span class="muted">· {at}{refunded}</span></p><p>{price} − fee {fee} − card fee {card} = <strong>{you}</strong></p></li>"#,
                title = escape(&title),
                at = escape(&r.try_get::<Option<String>, _>("at").ok().flatten().unwrap_or_default()),
                refunded = if status == "refunded" { " · refunded: the card fee stays with Stripe" } else { "" },
                price = cents("price_cents"),
                fee = cents("fee_cents"),
                card = cents("processing_cents"),
                you = cents("seller_cents"),
            )
        })
        .collect();
    let body = if rows.is_empty() {
        r#"<p class="lead">No sales yet.</p>"#.to_string()
    } else {
        format!(
            r#"<dl class="facts"><div><dt>Sales</dt><dd>{n}</dd></div><div><dt>Your earnings</dt><dd>{earned}</dd></div></dl><ul class="reports">{lines}</ul>"#,
            n = rows.len(),
            earned = dollars(earned),
        )
    };
    private(
        "Your sales · MakerFoundry",
        &format!(
            r#"<section class="design"><h1>Your sales</h1><p class="muted">Stripe pays your earnings into your bank once a month. <a href="/account/selling">Selling</a></p>{body}</section>"#
        ),
        true,
    )
}

// ---------------------------------------------------------------- refunds (admins)

pub async fn orders(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (_, pool) = match crate::moderation::admin(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    order_list(&pool, "").await
}

async fn order_list(pool: &PgPool, note: &str) -> Response {
    let rows = sqlx::query(
        "SELECT id, title, price_cents, status, fee_returned_at IS NULL AS fee_waiting, buyer_account, seller_account,
                to_char(created_at, 'YYYY-MM-DD HH24:MI') AS at
         FROM mf_orders WHERE status IN ('paid', 'refunded') ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let items: String = rows
        .iter()
        .map(|r| {
            let id: i64 = r.try_get("id").unwrap_or(0);
            let status: String = r.try_get("status").unwrap_or_default();
            let refund = if status == "paid" {
                format!(r#"<form method="post" action="/admin/orders/{id}/refund"><button type="submit" class="secondary">Refund in full</button></form>"#)
            } else if r.try_get("fee_waiting").unwrap_or(false) {
                format!(r#"<form method="post" action="/admin/orders/{id}/fee-back"><button type="submit" class="secondary">Give the fee back</button></form>"#)
            } else {
                String::new()
            };
            format!(
                r#"<li class="report"><p><strong>{title}</strong> <span class="muted">· order {id} · {at}</span></p><p>{price} · {status} · buyer {b}, seller {s}</p><div class="actions">{refund}</div></li>"#,
                title = escape(&r.try_get::<String, _>("title").unwrap_or_default()),
                at = escape(&r.try_get::<String, _>("at").unwrap_or_default()),
                price = dollars(r.try_get("price_cents").unwrap_or(0)),
                status = escape(&status),
                b = r.try_get::<i64, _>("buyer_account").unwrap_or(0),
                s = r.try_get::<i64, _>("seller_account").unwrap_or(0),
            )
        })
        .collect();
    let note = if note.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem" role="alert">{}</p>"#, escape(note))
    };
    private(
        "Orders · MakerFoundry",
        &format!(
            r#"<section class="design"><h1>Orders</h1>{note}<p class="muted">A refund gives the buyer back the whole price, taken back from the seller, and then MakerFoundry's 10% goes back to the seller: the seller bears the card fee Stripe keeps (Seller Terms § 7). <a href="/admin">Reports</a></p><ul class="reports">{items}</ul></section>"#
        ),
        true,
    )
}

/// An admin's full refund of a paid order (Seller Terms § 7): the buyer gets the whole price back
/// and the seller's transfer is reversed (https://docs.stripe.com/connect/destination-charges,
/// "Issue refunds"); MakerFoundry keeps the application fee at first, then gives its 10% back to
/// the seller (`return_fee`), so the seller bears only the card fee Stripe keeps.
pub async fn refund(State(state): State<AppState>, headers: HeaderMap, Path(order): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (admin, pool) = match crate::moderation::admin(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(stripe) = state.stripe.as_ref() else {
        return order_list(&pool, "Selling isn't switched on.").await;
    };
    let row = sqlx::query(
        "SELECT stripe_payment_intent, price_cents, tax_cents FROM mf_orders WHERE id = $1 AND status = 'paid'",
    )
    .bind(order)
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    let paid = row.as_ref().and_then(|r| {
        let intent: Option<String> = r.try_get("stripe_payment_intent").ok().flatten();
        Some((
            intent?,
            r.try_get::<i64, _>("price_cents").ok()?,
            r.try_get::<i64, _>("tax_cents").ok()?,
        ))
    });
    let Some((intent, price, tax)) = paid else {
        return order_list(&pool, &format!("Order {order} isn't a paid order.")).await;
    };
    // a taxed sale: the tax first (if it's still with the seller), then the price, taken back from
    // the seller by MakerFoundry itself, since Stripe's reverse_transfer would reverse the whole
    // payment, tax included, from what's left of the transfer
    if tax > 0 {
        let back = async {
            withhold_tax(stripe, &pool, order).await?;
            reverse(stripe, Some(&intent), order, price, "refund").await
        };
        if let Err(e) = back.await {
            eprintln!("selling: refund {order}: {e}");
            return order_list(
                &pool,
                &format!("Order {order} couldn't be refunded: the price couldn't be taken back from the seller ({e}). Nothing was refunded; try again."),
            )
            .await;
        }
        let _ =
            sqlx::query("UPDATE mf_orders SET refund_reversed_at = now() WHERE id = $1 AND refund_reversed_at IS NULL")
                .bind(order)
                .execute(&pool)
                .await
                // (only a record: `reverse` checks Stripe itself before reversing again)
                .map_err(|e| eprintln!("selling: refund {order}: {e}"));
    }
    let refunded = stripe
        .post(
            "/v1/refunds",
            &[
                ("payment_intent", intent),
                ("reverse_transfer", (tax == 0).to_string()),
                // the 10% goes back separately, without the card fee (`return_fee`)
                ("refund_application_fee", "false".into()),
                ("metadata[order]", order.to_string()),
                ("metadata[admin]", admin.account_id.to_string()),
            ],
            Some(&format!("mf-refund-{order}")),
        )
        .await;
    match refunded {
        Ok(v) if v.get("status").and_then(Value::as_str) != Some("failed") => {
            let done = async {
                let mut tx = pool.begin().await?;
                record_refund(&mut tx, order).await?;
                tx.commit().await
            };
            if let Err(e) = done.await {
                // (Stripe's charge.refunded event records it when it comes)
                eprintln!("selling: refund {order}: {e}");
            }
            let note = match return_fee(stripe, &pool, order).await {
                Ok(()) => format!("Order {order} refunded, and MakerFoundry's fee given back to the seller."),
                Err(why) => format!("Order {order} refunded, but {why}"),
            };
            order_list(&pool, &note).await
        }
        Ok(_) => order_list(&pool, &format!("Stripe couldn't refund order {order}.")).await,
        Err(e) => {
            eprintln!("selling: refund {order}: {e}");
            order_list(&pool, &format!("Order {order} couldn't be refunded: {e}")).await
        }
    }
}

/// Trying again to give a refunded order's 10% back to its seller (after Stripe couldn't the
/// first time).
pub async fn fee_back(State(state): State<AppState>, headers: HeaderMap, Path(order): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (_, pool) = match crate::moderation::admin(&state, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    let Some(stripe) = state.stripe.as_ref() else {
        return order_list(&pool, "Selling isn't switched on.").await;
    };
    let note = match return_fee(stripe, &pool, order).await {
        Ok(()) => format!("MakerFoundry's fee for order {order} given back to the seller."),
        Err(why) => format!("Order {order}: {why}"),
    };
    order_list(&pool, &note).await
}

/// A refunded order's 10% back to its seller, as a partial refund of the sale's application fee
/// (https://docs.stripe.com/api/fee_refunds/create), once; `Err` says why not, for the admin.
async fn return_fee(stripe: &Stripe, pool: &PgPool, order: i64) -> Result<(), String> {
    let row = sqlx::query(
        "SELECT stripe_payment_intent, fee_cents FROM mf_orders
         WHERE id = $1 AND status = 'refunded' AND fee_returned_at IS NULL",
    )
    .bind(order)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        eprintln!("selling: fee back {order}: {e}");
        "the fee couldn't be given back just now.".to_string()
    })?
    .ok_or_else(|| "there's no fee waiting to go back to the seller.".to_string())?;
    let intent: Option<String> = row.try_get("stripe_payment_intent").unwrap_or(None);
    let fee: i64 = row.try_get("fee_cents").unwrap_or(0);
    let not_now =
        "Stripe couldn't give MakerFoundry's fee back to the seller just now: try again with Give the fee back.";
    if fee > 0 {
        let intent = intent
            .filter(|i| is_stripe_id(i, "pi_"))
            .ok_or_else(|| "the order has no Stripe payment.".to_string())?;
        let payment = stripe
            .get(&format!("/v1/payment_intents/{intent}?expand[]=latest_charge"))
            .await
            .map_err(|e| {
                eprintln!("selling: fee back {order}: {e}");
                not_now.to_string()
            })?;
        let application_fee = payment
            .pointer("/latest_charge/application_fee")
            .and_then(Value::as_str)
            .filter(|f| is_stripe_id(f, "fee_"))
            .ok_or_else(|| "Stripe shows no application fee for this sale.".to_string())?;
        stripe
            .post(
                &format!("/v1/application_fees/{application_fee}/refunds"),
                &[("amount", fee.to_string()), ("metadata[order]", order.to_string())],
                Some(&format!("mf-fee-back-{order}")),
            )
            .await
            .map_err(|e| {
                eprintln!("selling: fee back {order}: {e}");
                not_now.to_string()
            })?;
    }
    let done = async {
        let mut tx = pool.begin().await?;
        record_fee_return(&mut tx, order).await?;
        tx.commit().await
    };
    done.await.map_err(|e| {
        eprintln!("selling: fee back {order}: {e}");
        "Stripe gave the fee back, but it couldn't be recorded: try again with Give the fee back.".to_string()
    })
}

/// The price part of a design's public page: the button to download or to buy.
pub async fn buy_html(
    state: &AppState,
    pool: &PgPool,
    viewer: Option<&User>,
    id: i64,
    price: i64,
    licence: &str,
) -> String {
    let licence = format!(r#" <span class="muted">{licence}</span>"#);
    if price == 0 {
        return format!(
            r#"<p class="actions"><a class="button" href="/d/{id}/download">Download free</a>{licence}</p>"#
        );
    }
    let price_text = dollars(price);
    let Some(user) = viewer else {
        return format!(
            r#"<p class="actions"><a class="button" href="/signin">Sign in to buy · {price_text}</a>{licence}</p>"#
        );
    };
    let staff = state.admins.contains(&user.account_id);
    let owner = sqlx::query_scalar::<_, i64>("SELECT account_id FROM mf_designs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .is_ok_and(|a| a == user.account_id);
    if owner || staff {
        format!(
            r#"<p class="actions"><a class="button" href="/d/{id}/download">Download</a> <span class="muted">{price_text}{note}</span>{licence}</p>"#,
            note = if owner { " · your design" } else { "" }
        )
    } else if bought(pool, user.account_id, id).await {
        format!(
            r#"<p class="actions"><a class="button" href="/d/{id}/download">Download</a> <span class="muted">You bought this</span>{licence}</p>"#
        )
    } else {
        format!(
            r#"<form method="post" action="/d/{id}/buy" class="actions"><button type="submit">Buy · {price_text}</button>{licence}</form>"#
        )
    }
}

/// Prices as typed in a design's details → cents, or why not.
pub fn check_price(text: &str) -> Result<i64, String> {
    let range = format!(
        "Give a price from {} to {}, or 0 to share the design free.",
        dollars(money::MIN_PRICE),
        dollars(money::MAX_PRICE)
    );
    match money::parse_dollars(text) {
        Some(0) => Ok(0),
        Some(c) if (money::MIN_PRICE..=money::MAX_PRICE).contains(&c) => Ok(c),
        _ => Err(range),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_are_checked() {
        assert_eq!(check_price(""), Ok(0));
        assert_eq!(check_price("4.50"), Ok(450));
        assert_eq!(check_price("$200"), Ok(20_000));
        assert!(check_price("0.50").is_err());
        assert!(check_price("201").is_err());
        assert!(check_price("abc").is_err());
    }

    #[test]
    fn stripe_ids_are_checked_before_going_in_a_path() {
        assert!(is_stripe_id("acct_1AbC", "acct_"));
        assert!(is_stripe_id("cs_test_a1B2", "cs_"));
        for bad in ["acct_", "acct_../x", "cs_1", "acct_a/b", "acct_a?b"] {
            assert!(!is_stripe_id(bad, "acct_"), "{bad}");
        }
    }
}
