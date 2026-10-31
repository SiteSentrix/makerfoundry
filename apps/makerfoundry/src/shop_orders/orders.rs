//! Ordering from a shop and paying (MF-63): the customer picks a colour, quantities by size, a
//! decoration and its placement, and uploads their art; MakerFoundry works out the price from the
//! shop's published catalogue only, keeps the art, and shows the order before Stripe Checkout. The
//! customer pays MakerFoundry; MakerFoundry keeps 10% and the card fee comes out of the shop's
//! share (the owner's decisions of 2026-10-31), as for designs (`money::split`). Once Stripe's
//! verified webhook (or the return from Checkout) says it's paid, the sale goes in the ledger and
//! `order.placed` goes in the outbox, in the same transaction: never before the payment.
//!
//! The art links in `order.placed` are MakerFoundry's own (`/shop-orders/<ref>/art/<sha256>`, with
//! the order's secret token), so they still work when the order waits days for a shop that's
//! offline. Sales tax on physical goods isn't collected yet: the owner's decision is open.

use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool, Row};

use super::contract::{self, Address, Customer, Decoration, FileRef, Line, Message, OrderPlaced, Totals};
use super::shops::{self, Shop, dollars, method_name};
use crate::AppState;
use crate::accounts::{User, current_user, forbidden, private, same_origin};
use crate::pages::escape;

/// The largest art file (as the hub's).
pub const MAX_ART_BYTES: usize = 10 * 1024 * 1024;
/// The order form's largest body: the art and the fields.
pub const BODY_LIMIT_BYTES: usize = MAX_ART_BYTES + 512 * 1024;
/// How long the shop has to take an order from the hub before it's refunded.
const ORDER_DAYS: i64 = 7;

/// An order's amounts: what the customer pays, and how it's shared. Exact cents; MakerFoundry's
/// 10% and the card fee (2.9% + 30¢) each rounded to the nearest cent, the shop getting the rest.
pub fn totals(goods: i64, setup: i64, shipping: i64) -> Option<Totals> {
    let paid = goods.checked_add(setup)?.checked_add(shipping)?;
    if !(1..=contract::MAX_CENTS).contains(&paid) {
        return None;
    }
    let fee = (paid * 10 + 50) / 100;
    let card_fee = (paid * 29 + 500) / 1000 + 30;
    let shop = paid - fee - card_fee;
    (shop > 0).then_some(Totals {
        goods_cents: goods,
        setup_cents: setup,
        shipping_cents: shipping,
        tax_cents: 0,
        paid_cents: paid,
        fee_cents: fee,
        card_fee_cents: card_fee,
        shop_gets_cents: shop,
    })
}

/// Whether two secrets are the same, compared in constant time for equal lengths.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_hex(n: usize) -> Result<String, String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| format!("no randomness: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

// ---------------------------------------------------------------- the form

fn form_html(shop: &Shop, sku: &str, problem: &str) -> Option<String> {
    let p = shop.catalog.products.iter().find(|p| p.sku == sku)?;
    let colours: String = p
        .colours
        .iter()
        .enumerate()
        .map(|(i, c)| format!(r#"<option value="{i}">{}</option>"#, escape(c)))
        .collect();
    let sizes: String = p
        .sizes
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                r#"<label class="size">{} <input type="number" name="q{i}" min="0" max="{}" value="0" inputmode="numeric"></label>"#,
                escape(s),
                contract::MAX_PIECES
            )
        })
        .collect();
    let decorations: String = p
        .methods
        .iter()
        .enumerate()
        .flat_map(|(mi, m)| {
            m.placements.iter().enumerate().map(move |(pi, pl)| {
                format!(
                    r#"<option value="m{mi}p{pi}">{} on the {}</option>"#,
                    method_name(&m.method),
                    escape(pl)
                )
            })
        })
        .collect();
    let c = &shop.catalog;
    let delivery = match (c.shop.ships, c.shop.pickup) {
        (true, true) => format!(
            r#"<label>Delivery <small>({})</small> <select name="delivery"><option value="ship">Ship it to me</option><option value="pickup">I'll pick it up at the shop</option></select></label>"#,
            escape(&shops::shipping_words(&c.shipping))
        ),
        (true, false) => format!(
            r#"<input type="hidden" name="delivery" value="ship"><p>{}</p>"#,
            escape(&shops::shipping_words(&c.shipping))
        ),
        _ => r#"<input type="hidden" name="delivery" value="pickup"><p>You pick it up at the shop.</p>"#.to_string(),
    };
    let address = if c.shop.ships {
        r#"<fieldset><legend>Shipping address <small>(only if it's shipped)</small></legend>
<label>Street <input name="line1" maxlength="100" autocomplete="address-line1"></label>
<label>Apartment, suite <small>(optional)</small> <input name="line2" maxlength="100" autocomplete="address-line2"></label>
<label>City <input name="city" maxlength="100" autocomplete="address-level2"></label>
<label>State <input name="region" maxlength="100" autocomplete="address-level1"></label>
<label>ZIP code <input name="postcode" maxlength="20" autocomplete="postal-code"></label>
<input type="hidden" name="country" value="US"></fieldset>"#
    } else {
        ""
    };
    let problem = if problem.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem">{}</p>"#, escape(problem))
    };
    Some(format!(
        r#"<section class="form"><p><a href="/shops/{handle}">{shop_name}</a></p><h1>Order: {name}</h1>{problem}
<form method="post" action="/shops/{handle}/order/{sku}" enctype="multipart/form-data">
<label>Colour <select name="colour">{colours}</select></label>
<fieldset><legend>How many of each size</legend><div class="sizes">{sizes}</div></fieldset>
<label>Decoration <select name="decoration">{decorations}</select></label>
<label>Your art <small>(an embroidery file for embroidery; a PNG or JPEG for printing; 10 MB at most)</small><input type="file" name="art" required></label>
<label>Your name <small>(for the shop)</small> <input name="name" maxlength="100" required autocomplete="name"></label>
{delivery}{address}
<label>A note for the shop <small>(optional)</small> <textarea name="note" maxlength="1000" rows="3"></textarea></label>
<label class="choice"><input type="checkbox" name="rights" value="yes"> <span>I own this art or have the right to have it printed or stitched.</span></label>
<button type="submit">See the price</button>
</form></section>"#,
        handle = escape(&shop.handle),
        shop_name = escape(&c.shop.name),
        name = escape(&p.name),
        sku = escape(sku),
    ))
}

async fn signed_in_shop(state: &AppState, headers: &HeaderMap, handle: &str) -> Result<(User, PgPool, Shop), Response> {
    let (Some(_), Some(pool)) = (state.stitchhub.as_ref(), state.db.clone()) else {
        return Err(crate::designs::not_found());
    };
    let Some(user) = current_user(state, headers).await else {
        return Err(Redirect::to(&format!("/signin?next=/shops/{handle}")).into_response());
    };
    let Some(shop) = shops::orderable(state, &pool, handle).await else {
        return Err(crate::designs::not_found());
    };
    Ok((user, pool, shop))
}

/// `GET /shops/<handle>/order/<sku>`.
pub async fn form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((handle, sku)): Path<(String, String)>,
) -> Response {
    let (_, _, shop) = match signed_in_shop(&state, &headers, &handle).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    match form_html(&shop, &sku, "") {
        Some(html) => private(
            &format!("Order from {} · MakerFoundry", shop.catalog.shop.name),
            &html,
            true,
        ),
        None => crate::designs::not_found(),
    }
}

/// What the order form sent.
#[derive(Default)]
struct Sent {
    fields: std::collections::BTreeMap<String, String>,
    art: Option<(String, Vec<u8>)>,
}

async fn read_form(mut multipart: Multipart) -> Result<Sent, String> {
    let mut sent = Sent::default();
    let mut fields = 0;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| "The form didn't come through whole.".to_string())?
    {
        fields += 1;
        if fields > 60 {
            return Err("The form has too many fields.".into());
        }
        let name = field.name().unwrap_or_default().to_string();
        if name == "art" {
            let file = crate::model3d::upload::clean_name(field.file_name().unwrap_or("art"));
            let bytes = field
                .bytes()
                .await
                .map_err(|_| "The art didn't come through whole.".to_string())?;
            if bytes.len() > MAX_ART_BYTES {
                return Err("The art is larger than 10 MB.".into());
            }
            if !bytes.is_empty() {
                sent.art = Some((file, bytes.to_vec()));
            }
        } else {
            let text = field
                .text()
                .await
                .map_err(|_| "The form didn't come through whole.".to_string())?;
            sent.fields.insert(name, text.chars().take(1000).collect());
        }
    }
    Ok(sent)
}

/// An order as worked out from the form, before it's kept.
struct Draft {
    line: Line,
    art: (String, Vec<u8>, &'static str),
    totals: Totals,
    customer: Customer,
    note: String,
}

fn work_out(shop: &Shop, sku: &str, sent: Sent) -> Result<Draft, String> {
    let c = &shop.catalog;
    let p = c
        .products
        .iter()
        .find(|p| p.sku == sku)
        .ok_or("That product isn't offered any more.")?;
    let f = |k: &str| sent.fields.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
    let colour = f("colour")
        .parse::<usize>()
        .ok()
        .and_then(|i| p.colours.get(i))
        .ok_or("Choose a colour.")?
        .clone();
    let mut sizes = std::collections::BTreeMap::new();
    let mut pieces: u32 = 0;
    for (i, s) in p.sizes.iter().enumerate() {
        let n: u32 = f(&format!("q{i}")).parse().unwrap_or(0);
        if n > 0 {
            sizes.insert(s.clone(), n);
            pieces = pieces.saturating_add(n);
        }
    }
    if pieces == 0 {
        return Err("Say how many of each size.".into());
    }
    if pieces > contract::MAX_PIECES {
        return Err(format!("An order has at most {} pieces.", contract::MAX_PIECES));
    }
    let d = f("decoration");
    let (mi, pi) = d
        .strip_prefix('m')
        .and_then(|r| r.split_once('p'))
        .and_then(|(m, p)| Some((m.parse::<usize>().ok()?, p.parse::<usize>().ok()?)))
        .ok_or("Choose a decoration.")?;
    let method = p.methods.get(mi).ok_or("Choose a decoration.")?;
    let placement = method.placements.get(pi).ok_or("Choose a decoration.")?.clone();
    if f("rights") != "yes" {
        return Err("Confirm the art is yours to have printed or stitched.".into());
    }
    let (file, bytes) = sent.art.ok_or("Choose your art.")?;
    let kind = if method.method == "embroidery" {
        crate::embroidery::read(&bytes, &file).map_err(|e| format!("The art: {e}."))?;
        "application/octet-stream"
    } else {
        crate::photos::check(&bytes).map_err(|e| format!("The art: {e}."))?
    };
    let name = f("name");
    if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err("Give your name for the shop.".into());
    }
    let ship = f("delivery") == "ship" && c.shop.ships;
    if !ship && !c.shop.pickup {
        return Err("This shop only ships.".into());
    }
    let ship_to = if ship {
        let a = Address {
            line1: f("line1"),
            line2: f("line2"),
            city: f("city"),
            region: f("region"),
            postcode: f("postcode"),
            country: if f("country").is_empty() {
                "US".into()
            } else {
                f("country").to_uppercase()
            },
        };
        if a.line1.is_empty() || a.city.is_empty() || a.region.is_empty() || a.postcode.is_empty() {
            return Err("Give the shipping address.".into());
        }
        Some(a)
    } else {
        None
    };
    let goods = i64::from(pieces)
        .checked_mul(shops::piece_cents(p, method, pieces))
        .ok_or("The order is too large.")?;
    let shipping = if ship {
        goods
            .checked_add(method.setup_cents)
            .and_then(|subtotal| c.shipping.cents(pieces, subtotal))
            .ok_or("The order is too large.")?
    } else {
        0
    };
    let totals = totals(goods, method.setup_cents, shipping).ok_or("The order is too large or too small to take.")?;
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let line = Line {
        sku: p.sku.clone(),
        colour,
        sizes,
        decorations: vec![Decoration {
            method: method.method.clone(),
            placement,
            art: FileRef {
                file: file.clone(),
                size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                sha256: sha,
            },
        }],
    };
    Ok(Draft {
        line,
        art: (file, bytes, kind),
        totals,
        customer: Customer { name, ship_to },
        note: f("note"),
    })
}

/// `POST /shops/<handle>/order/<sku>`: the order worked out and kept, not yet paid.
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((handle, sku)): Path<(String, String)>,
    multipart: Multipart,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool, shop) = match signed_in_shop(&state, &headers, &handle).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let again = |why: &str| match form_html(&shop, &sku, why) {
        Some(html) => private("Order · MakerFoundry", &html, true),
        None => crate::designs::not_found(),
    };
    if user.account_id == shop.account {
        return again("You can't order from your own shop.");
    }
    if !state
        .limits
        .allow(crate::limits::CHECKOUT, &user.account_id.to_string())
    {
        return again("You've started a lot of orders in the last hour. Please wait a while.");
    }
    let Some(storage) = state.storage.as_ref() else {
        return again("Orders can't be taken just now.");
    };
    let sent = match read_form(multipart).await {
        Ok(s) => s,
        Err(why) => return again(&why),
    };
    let (s, k) = (shop.clone(), sku.clone());
    let d = match tokio::task::spawn_blocking(move || work_out(&s, &k, sent)).await {
        Ok(Ok(d)) => d,
        Ok(Err(why)) => return again(&why),
        Err(_) => return again("The order couldn't be worked out. Please try again."),
    };
    let made = async {
        let reference = format!("mf-{}", random_hex(8)?);
        let token = random_hex(16)?;
        let key = crate::storage::new_key("shop-art")?;
        storage.put(&key, d.art.1.clone(), d.art.2).await?;
        let sha = d.line.decorations.first().map(|x| x.art.sha256.clone()).unwrap_or_default();
        let art = serde_json::json!([{ "sha256": sha, "key": key, "file": d.art.0, "type": d.art.2 }]);
        let lines = serde_json::to_value(vec![&d.line]).map_err(|e| e.to_string())?;
        let ship_to = serde_json::to_value(&d.customer.ship_to).map_err(|e| e.to_string())?;
        let t = &d.totals;
        sqlx::query(
            "INSERT INTO mf_shop_orders (ref, token, buyer_account, shop_party, shop_account, lines, art, customer_name, ship_to, note,
                 goods_cents, setup_cents, shipping_cents, tax_cents, paid_cents, fee_cents, card_fee_cents, shop_cents)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)",
        )
        .bind(&reference)
        .bind(&token)
        .bind(user.account_id)
        .bind(&shop.party)
        .bind(shop.account)
        .bind(&lines)
        .bind(&art)
        .bind(&d.customer.name)
        .bind(&ship_to)
        .bind(&d.note)
        .bind(t.goods_cents)
        .bind(t.setup_cents)
        .bind(t.shipping_cents)
        .bind(t.tax_cents)
        .bind(t.paid_cents)
        .bind(t.fee_cents)
        .bind(t.card_fee_cents)
        .bind(t.shop_gets_cents)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;
        Ok::<String, String>(reference)
    }
    .await;
    match made {
        Ok(reference) => Redirect::to(&format!("/shop-orders/{reference}")).into_response(),
        Err(e) => {
            eprintln!("shop orders: {e}");
            again("The order couldn't be kept just now. Please try again.")
        }
    }
}

// ---------------------------------------------------------------- paying

/// An order, as its page and its payment need it.
pub struct Order {
    pub id: i64,
    pub reference: String,
    pub token: String,
    pub buyer: i64,
    pub shop_party: String,
    pub shop_account: i64,
    pub status: String,
    pub totals: Totals,
    pub lines: Vec<Line>,
    pub customer: Customer,
    pub note: String,
    pub art: Value,
    pub stripe_session: Option<String>,
}

const ORDER_COLUMNS: &str = "id, ref, token, buyer_account, shop_party, shop_account, status, lines, art, customer_name, ship_to,
    note, goods_cents, setup_cents, shipping_cents, tax_cents, paid_cents, fee_cents, card_fee_cents, shop_cents, stripe_session";

fn order_of(r: &sqlx::postgres::PgRow) -> Option<Order> {
    Some(Order {
        id: r.try_get("id").ok()?,
        reference: r.try_get("ref").ok()?,
        token: r.try_get("token").ok()?,
        buyer: r.try_get("buyer_account").ok()?,
        shop_party: r.try_get("shop_party").ok()?,
        shop_account: r.try_get("shop_account").ok()?,
        status: r.try_get("status").ok()?,
        lines: serde_json::from_value(r.try_get("lines").ok()?).ok()?,
        art: r.try_get("art").ok()?,
        customer: Customer {
            name: r.try_get("customer_name").ok()?,
            ship_to: serde_json::from_value(r.try_get("ship_to").ok()?).ok()?,
        },
        note: r.try_get("note").ok()?,
        totals: Totals {
            goods_cents: r.try_get("goods_cents").ok()?,
            setup_cents: r.try_get("setup_cents").ok()?,
            shipping_cents: r.try_get("shipping_cents").ok()?,
            tax_cents: r.try_get("tax_cents").ok()?,
            paid_cents: r.try_get("paid_cents").ok()?,
            fee_cents: r.try_get("fee_cents").ok()?,
            card_fee_cents: r.try_get("card_fee_cents").ok()?,
            shop_gets_cents: r.try_get("shop_cents").ok()?,
        },
        stripe_session: r.try_get("stripe_session").ok()?,
    })
}

pub async fn order(conn: &mut PgConnection, reference: &str) -> Option<Order> {
    let row = sqlx::query(&format!("SELECT {ORDER_COLUMNS} FROM mf_shop_orders WHERE ref = $1"))
        .bind(reference)
        .fetch_optional(conn)
        .await
        .ok()
        .flatten()?;
    order_of(&row)
}

/// `POST /shop-orders/<ref>/pay`: Stripe Checkout for the order's total, MakerFoundry's 10% and
/// the card fee as the application fee, the rest to the shop's Stripe account.
pub async fn pay(State(state): State<AppState>, headers: HeaderMap, Path(reference): Path<String>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some(user), Some(pool), Some(stripe)) = (
        current_user(&state, &headers).await,
        state.db.as_ref(),
        state.stripe.as_ref(),
    ) else {
        return crate::designs::not_found();
    };
    let Ok(mut conn) = pool.acquire().await else {
        return (StatusCode::SERVICE_UNAVAILABLE, "Try again in a minute.").into_response();
    };
    let Some(o) = order(&mut conn, &reference)
        .await
        .filter(|o| o.buyer == user.account_id)
    else {
        return crate::designs::not_found();
    };
    let back = Redirect::to(&format!("/shop-orders/{reference}")).into_response();
    // (an order is paid within a day, so no checkout outlives the two days it's kept unpaid)
    let fresh: bool =
        sqlx::query_scalar("SELECT created_at > now() - interval '20 hours' FROM mf_shop_orders WHERE id = $1")
            .bind(o.id)
            .fetch_one(pool)
            .await
            .unwrap_or(false);
    if o.status != "pending" {
        return back;
    }
    if !fresh {
        return private(
            "Order · MakerFoundry",
            r#"<section class="form"><h1>This order is over a day old</h1><p>Prices may have changed: please start the order again from the shop's page.</p></section>"#,
            true,
        );
    }
    let Some(seller) = crate::selling::seller(pool, o.shop_account)
        .await
        .filter(|s| s.charges_enabled)
    else {
        return private(
            "Order · MakerFoundry",
            r#"<section class="form"><h1>This shop can't take payments just now</h1><p>Please try again later.</p></section>"#,
            true,
        );
    };
    let shop_name: String = sqlx::query_scalar("SELECT name FROM mf_shops WHERE party = $1")
        .bind(&o.shop_party)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "a shop".into());
    // (a session already made and still open is used again, so a second click can't pay twice)
    if let Some(sid) = o.stripe_session.as_deref()
        && let Ok(s) = stripe.get(&format!("/v1/checkout/sessions/{sid}")).await
        && s.get("status").and_then(Value::as_str) == Some("open")
        && let Some(url) = s
            .get("url")
            .and_then(Value::as_str)
            .filter(|u| u.starts_with("https://"))
    {
        return Redirect::to(url).into_response();
    }
    let t = &o.totals;
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
        .unwrap_or(0)
        + 2 * 3600;
    let form: Vec<(&str, String)> = vec![
        ("mode", "payment".into()),
        ("expires_at", expires.to_string()),
        ("client_reference_id", o.reference.clone()),
        (
            "success_url",
            format!(
                "{}/shop-orders/{}?session={{CHECKOUT_SESSION_ID}}",
                stripe.public_url, o.reference
            ),
        ),
        (
            "cancel_url",
            format!("{}/shop-orders/{}", stripe.public_url, o.reference),
        ),
        ("line_items[0][quantity]", "1".into()),
        ("line_items[0][price_data][currency]", "usd".into()),
        ("line_items[0][price_data][unit_amount]", t.paid_cents.to_string()),
        (
            "line_items[0][price_data][product_data][name]",
            format!(
                "Order {} from {}",
                o.reference,
                shop_name.chars().take(100).collect::<String>()
            ),
        ),
        (
            "payment_intent_data[application_fee_amount]",
            (t.fee_cents + t.card_fee_cents).to_string(),
        ),
        (
            "payment_intent_data[transfer_data][destination]",
            seller.stripe_account.clone(),
        ),
        ("payment_intent_data[metadata][shop_order]", o.reference.clone()),
        ("payment_intent_data[metadata][app]", crate::selling::APP.into()),
        ("metadata[shop_order]", o.reference.clone()),
        ("metadata[app]", crate::selling::APP.into()),
    ];
    let session = stripe
        .post(
            "/v1/checkout/sessions",
            &form,
            Some(&format!(
                "mf-shop-order-{}-{}",
                o.reference,
                o.stripe_session.as_deref().unwrap_or("first")
            )),
        )
        .await;
    let (sid, url) = match &session {
        Ok(v) => (
            v.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
            v.get("url").and_then(Value::as_str).unwrap_or("").to_string(),
        ),
        Err(e) => {
            eprintln!("shop orders: checkout for {}: {e}", o.reference);
            (String::new(), String::new())
        }
    };
    if !sid.starts_with("cs_") || !url.starts_with("https://") {
        return private(
            "Order · MakerFoundry",
            r#"<section class="form"><h1>Stripe can't be reached just now</h1><p>Please try again in a few minutes.</p></section>"#,
            true,
        );
    }
    let _ = sqlx::query("UPDATE mf_shop_orders SET stripe_session = $2 WHERE id = $1 AND status = 'pending'")
        .bind(o.id)
        .bind(&sid)
        .execute(pool)
        .await;
    Redirect::to(&url).into_response()
}

/// A Checkout Session Stripe says is paid → the order paid, its sale in the ledger and
/// `order.placed` in the outbox, all once. `false` (and nothing changes) unless the session is
/// paid and is this order's, for its total.
pub async fn confirm_paid(
    conn: &mut PgConnection,
    reference: &str,
    session: &Value,
    public_url: &str,
) -> Result<bool, String> {
    let ours = session.pointer("/metadata/shop_order").and_then(Value::as_str) == Some(reference)
        && session.pointer("/metadata/app").and_then(Value::as_str) == Some(crate::selling::APP);
    if !ours || session.get("payment_status").and_then(Value::as_str) != Some("paid") {
        return Ok(false);
    }
    let sid = session.get("id").and_then(Value::as_str).unwrap_or("");
    let total = session.get("amount_total").and_then(Value::as_i64).unwrap_or(-1);
    let currency = session.get("currency").and_then(Value::as_str).unwrap_or("");
    let intent = session.get("payment_intent").and_then(Value::as_str);
    let paid: Option<i64> = sqlx::query_scalar(
        "UPDATE mf_shop_orders SET status = 'paid', paid_at = now(), stripe_session = $2, stripe_payment_intent = $3, updated_at = now()
         WHERE ref = $1 AND status = 'pending' AND paid_cents = $4 AND $5 = 'usd'
         RETURNING id",
    )
    .bind(reference)
    .bind(sid)
    .bind(intent)
    .bind(total)
    .bind(currency)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| e.to_string())?;
    let Some(id) = paid else {
        return Ok(false);
    };
    let o = order(&mut *conn, reference).await.ok_or("the order went")?;
    for (account, kind, cents) in [
        (o.shop_account, "shop", o.totals.shop_gets_cents),
        (crate::selling::MAKERFOUNDRY, "fee", o.totals.fee_cents),
        (crate::selling::MAKERFOUNDRY, "processing", o.totals.card_fee_cents),
    ] {
        sqlx::query(
            "INSERT INTO mf_shop_ledger (order_id, account_id, kind, cents, reason) VALUES ($1, $2, $3, $4, 'sale')
             ON CONFLICT (order_id, kind, reason) DO NOTHING",
        )
        .bind(id)
        .bind(account)
        .bind(kind)
        .bind(cents)
        .execute(&mut *conn)
        .await
        .map_err(|e| e.to_string())?;
    }
    let art_urls = o
        .lines
        .iter()
        .flat_map(|l| &l.decorations)
        .map(|d| {
            (
                d.art.sha256.clone(),
                format!(
                    "{public_url}/shop-orders/{}/art/{}?t={}",
                    o.reference, d.art.sha256, o.token
                ),
            )
        })
        .collect();
    let placed = Message::OrderPlaced(Box::new(OrderPlaced {
        lines: o.lines.clone(),
        art_urls,
        preview: None,
        customer: o.customer.clone(),
        totals: o.totals.clone(),
        note: o.note.clone(),
    }));
    super::hub::enqueue(&mut *conn, &placed, Some(&o.reference), &o.shop_party, ORDER_DAYS).await?;
    sqlx::query("UPDATE mf_shop_orders SET status = 'sent' WHERE id = $1")
        .bind(id)
        .execute(&mut *conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

// ---------------------------------------------------------------- the order's page and its art

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub struct Back {
    session: String,
}

/// What a status means to the customer.
pub fn status_name(s: &str) -> &'static str {
    match s {
        "pending" => "Not paid yet",
        "paid" | "sent" => "Paid. Sent to the shop; it gets your order when its program is next online",
        "received" => "Received by the shop",
        "proof_sent" => "A proof is waiting for you",
        "approved" => "Proof approved",
        "in_production" => "Being made",
        "ready" => "Ready",
        "shipped" => "Shipped",
        "collected" => "Picked up",
        "cancelled" => "Cancelled",
        "refunded" => "Refunded",
        "expired" => "The shop didn't take the order in time, so it's refunded",
        "abandoned" => "Not paid, and given up after two days",
        _ => "Unknown",
    }
}

fn lines_html(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|l| {
            let sizes: Vec<String> = l.sizes.iter().map(|(s, n)| format!("{} × {n}", escape(s))).collect();
            let decos: Vec<String> = l
                .decorations
                .iter()
                .map(|d| {
                    format!(
                        "{} on the {} ({})",
                        method_name(&d.method),
                        escape(&d.placement),
                        escape(&d.art.file)
                    )
                })
                .collect();
            format!(
                "<li>{sku}, {colour}: {sizes}<br>{decos}</li>",
                sku = escape(&l.sku),
                colour = escape(&l.colour),
                sizes = sizes.join(", "),
                decos = decos.join("; ")
            )
        })
        .collect()
}

fn totals_html(t: &Totals) -> String {
    let mut rows = vec![("Items", t.goods_cents), ("Set-up", t.setup_cents)];
    if t.shipping_cents > 0 {
        rows.push(("Shipping", t.shipping_cents));
    }
    let body: String = rows
        .iter()
        .map(|(k, v)| format!("<tr><td>{k}</td><td>{}</td></tr>", dollars(*v)))
        .collect();
    format!(
        r#"<table class="totals">{body}<tr><th>Total</th><th>{}</th></tr></table>"#,
        dollars(t.paid_cents)
    )
}

/// `GET /shop-orders/<ref>`: the customer's order. Coming back from Checkout (`?session=`), the
/// payment is checked with Stripe straight away.
pub async fn page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(reference): Path<String>,
    Query(back): Query<Back>,
) -> Response {
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to(&format!("/signin?next=/shop-orders/{reference}")).into_response();
    };
    let Ok(mut conn) = pool.acquire().await else {
        return (StatusCode::SERVICE_UNAVAILABLE, "Try again in a minute.").into_response();
    };
    let Some(mut o) = order(&mut conn, &reference)
        .await
        .filter(|o| o.buyer == user.account_id)
    else {
        return crate::designs::not_found();
    };
    if o.status == "pending"
        && back.session.starts_with("cs_")
        && back.session.len() <= 255
        && back.session.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && let Some(stripe) = state.stripe.as_ref()
        && let Ok(session) = stripe.get(&format!("/v1/checkout/sessions/{}", back.session)).await
        && let Ok(mut tx) = pool.begin().await
    {
        match confirm_paid(&mut tx, &o.reference, &session, &state.public_url).await {
            Ok(true) => {
                if tx.commit().await.is_ok()
                    && let Some(fresh) = order(&mut conn, &reference).await
                {
                    o = fresh;
                }
            }
            Ok(false) => {}
            Err(e) => eprintln!("shop orders: confirming {reference}: {e}"),
        }
    }
    let shop: Option<(String, String)> = sqlx::query_as("SELECT name, handle FROM mf_shops WHERE party = $1")
        .bind(&o.shop_party)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let (shop_name, handle) = shop.unwrap_or_else(|| ("the shop".into(), String::new()));
    let action = if o.status == "pending" {
        format!(
            r#"<form method="post" action="/shop-orders/{r}/pay"><button type="submit">Pay {total}</button></form>
<p class="muted">You pay MakerFoundry, which pays the shop. Nothing is sent to the shop until you've paid.</p>"#,
            r = escape(&o.reference),
            total = dollars(o.totals.paid_cents)
        )
    } else {
        String::new()
    };
    let after = super::after::page_html(pool, &o).await;
    let delivery = match &o.customer.ship_to {
        Some(a) => format!(
            "Shipping to {}, {}, {} {}",
            escape(&a.line1),
            escape(&a.city),
            escape(&a.region),
            escape(&a.postcode)
        ),
        None => "Pick up at the shop".to_string(),
    };
    private(
        &format!("Order {} · MakerFoundry", o.reference),
        &format!(
            r#"<section class="form"><h1>Your order from {shop}</h1>
<p class="lead">{status}</p>
<p class="muted">Order {r}{shop_link}</p>
<ul class="order-lines">{lines}</ul>
<p>{delivery}</p>
{after}
{totals}
{action}</section>"#,
            shop = escape(&shop_name),
            status = status_name(&o.status),
            r = escape(&o.reference),
            shop_link = if handle.is_empty() {
                String::new()
            } else {
                format!(r#" · <a href="/shops/{}">the shop</a>"#, escape(&handle))
            },
            lines = lines_html(&o.lines),
            totals = totals_html(&o.totals),
        ),
        true,
    )
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub struct ArtQuery {
    t: String,
}

/// `GET /shop-orders/<ref>/art/<sha256>?t=<token>`: an order's art for its shop, by the order's
/// secret token, while the order is open; always a download.
pub async fn art(
    State(state): State<AppState>,
    Path((reference, sha)): Path<(String, String)>,
    Query(q): Query<ArtQuery>,
) -> Response {
    let (Some(pool), Some(storage)) = (state.db.as_ref(), state.storage.as_ref()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(mut conn) = pool.acquire().await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let open = ["sent", "received", "proof_sent", "approved", "in_production"];
    let Some(o) = order(&mut conn, &reference)
        .await
        .filter(|o| !q.t.is_empty() && same_secret(&o.token, &q.t) && open.contains(&o.status.as_str()))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let found = o.art.as_array().and_then(|a| {
        a.iter()
            .find(|x| x.get("sha256").and_then(Value::as_str) == Some(sha.as_str()))
            .and_then(|x| {
                Some((
                    x.get("key")?.as_str()?.to_string(),
                    x.get("file")?.as_str()?.to_string(),
                ))
            })
    });
    let Some((key, file)) = found else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match storage.get(&key).await {
        Ok(Some(bytes)) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", file.replace(['"', '\\'], "_")),
                ),
                (header::CACHE_CONTROL, "private, no-store".to_string()),
            ],
            bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_share_the_payment_exactly() {
        let t = totals(10_200, 2_500, 899).unwrap();
        assert_eq!(t.paid_cents, 13_599);
        assert_eq!(t.fee_cents, 1_360);
        assert_eq!(t.card_fee_cents, 424);
        assert_eq!(t.shop_gets_cents, 13_599 - 1_360 - 424);
        contract::check_totals(&t).unwrap();
        for paid in [1i64, 31, 99, 100, 101, 12_345, 1_000_000, contract::MAX_CENTS] {
            if let Some(t) = totals(paid, 0, 0) {
                contract::check_totals(&t).unwrap();
                assert!(t.shop_gets_cents > 0);
            }
        }
        assert!(totals(0, 0, 0).is_none());
        assert!(totals(contract::MAX_CENTS, 1, 0).is_none());
        assert!(totals(20, 0, 0).is_none(), "too small to leave the shop anything");
    }
}
