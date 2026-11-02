//! Shops and their catalogues (MF-82): what a shop publishes (`catalog`) is kept as it came,
//! checked (`contract::parse`), one row per shop. A shop is offered to customers only while its
//! Foundry Account can be paid (`selling::can_sell`: payouts set up on MakerFoundry). Pages:
//! `/shops` and `/shops/<handle>`, both missing until StitchHub is switched on.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Response};
use sqlx::PgPool;

use super::contract::{Catalog, Method, Product};
use crate::AppState;
use crate::accounts::current_user;
use crate::pages::{escape, page};

/// The money format used on these pages.
pub fn dollars(cents: i64) -> String {
    format!("${}.{:02}", cents / 100, (cents % 100).abs())
}

/// A method as people say it.
pub fn method_name(m: &str) -> &'static str {
    match m {
        "embroidery" => "Embroidery",
        "screen_print" => "Screen print",
        "dtg" => "Direct to garment (DTG)",
        "heat_transfer" => "Heat transfer",
        _ => "Vinyl",
    }
}

/// A shop's shipping prices, as people say them.
pub fn shipping_words(s: &super::contract::Shipping) -> String {
    let mut words = format!("Shipping {} an order", dollars(s.flat_cents));
    if s.per_extra_piece_cents > 0 {
        words.push_str(&format!(
            ", plus {} for each piece after the first",
            dollars(s.per_extra_piece_cents)
        ));
    }
    if s.free_over_cents > 0 {
        words.push_str(&format!("; free on orders of {} or more", dollars(s.free_over_cents)));
    }
    words.push('.');
    words
}

/// Keeps a shop's latest catalogue (`from` is `shop:<hub shop id>`; the id is its page's handle).
pub async fn keep(pool: &PgPool, from: &str, c: &Catalog) -> Result<(), String> {
    let handle = from.strip_prefix("shop:").ok_or("not from a shop")?;
    let value = serde_json::to_value(c).map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO mf_shops (party, handle, account_id, name, town, ships, pickup, catalog, shipping_cents)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (party) DO UPDATE SET account_id = $3, name = $4, town = $5, ships = $6, pickup = $7,
             catalog = $8, shipping_cents = $9, updated_at = now(),
             -- (naming another account to be paid needs that account's holder to confirm it again)
             account_confirmed = mf_shops.account_confirmed AND mf_shops.account_id = $3",
    )
    .bind(from)
    .bind(handle)
    .bind(c.shop.account)
    .bind(&c.shop.name)
    .bind(&c.shop.town)
    .bind(c.shop.ships)
    .bind(c.shop.pickup)
    .bind(&value)
    .bind(c.shipping.flat_cents)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// A shop as MakerFoundry keeps it.
#[derive(Clone)]
pub struct Shop {
    pub party: String,
    pub handle: String,
    pub account: i64,
    pub catalog: Catalog,
}

type ShopRow = (String, String, i64, serde_json::Value);

fn shop_of(row: ShopRow) -> Option<Shop> {
    let (party, handle, account, catalog) = row;
    Some(Shop {
        party,
        handle,
        account,
        catalog: serde_json::from_value(catalog).ok()?,
    })
}

/// Marks the shops the hub lists as ready (`GET /v2/shops`), and only them, as taking orders.
pub async fn refresh(pool: &PgPool, ready: &[String]) -> Result<(), String> {
    sqlx::query("UPDATE mf_shops SET hub_ready = (party = ANY($1)) WHERE hub_ready <> (party = ANY($1))")
        .bind(ready)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Shop `handle`, if it can take orders now: the hub lists it as ready, the Foundry Account its
/// catalogue names has confirmed the shop is theirs, and that account can be paid.
pub async fn orderable(state: &AppState, pool: &PgPool, handle: &str) -> Option<Shop> {
    let row: ShopRow = sqlx::query_as(
        "SELECT party, handle, account_id, catalog FROM mf_shops WHERE handle = $1 AND hub_ready AND account_confirmed",
    )
    .bind(handle)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    let shop = shop_of(row)?;
    crate::selling::can_sell(state, pool, shop.account)
        .await
        .then_some(shop)
}

/// A product's price for one piece with one decoration, at `pieces` pieces: the garment plus
/// the decoration at the price break that applies.
pub fn piece_cents(product: &Product, method: &Method, pieces: u32) -> i64 {
    let decoration = method
        .price_breaks
        .iter()
        .rfind(|b| b.min <= pieces.max(1))
        .map_or(0, |b| b.cents);
    product.cents + decoration
}

fn breaks_html(m: &Method) -> String {
    m.price_breaks
        .iter()
        .map(|b| format!("{}+: {}", b.min, dollars(b.cents)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn product_html(handle: &str, p: &Product) -> String {
    let methods: String = p
        .methods
        .iter()
        .map(|m| {
            let setup = if m.setup_cents > 0 {
                format!(", set-up {} an order", dollars(m.setup_cents))
            } else {
                String::new()
            };
            format!(
                r#"<li><strong>{name}</strong> <span class="muted">({places})</span><br>{breaks} a piece{setup}</li>"#,
                name = method_name(&m.method),
                places = escape(&m.placements.join(", ")),
                breaks = escape(&breaks_html(m)),
            )
        })
        .collect();
    format!(
        r#"<article class="product"><h2>{name}</h2>
<p class="muted">{kind} · {price} a piece before decoration</p>
<p>Colours: {colours}<br>Sizes: {sizes}</p>
<ul class="methods">{methods}</ul>
<p><a class="button" href="/shops/{handle}/order/{sku}">Order</a></p></article>"#,
        name = escape(&p.name),
        kind = escape(&p.kind),
        price = dollars(p.cents),
        colours = escape(&p.colours.join(", ")),
        sizes = escape(&p.sizes.join(", ")),
        handle = escape(handle),
        sku = escape(&p.sku),
    )
}

/// `GET /shops`: the shops that take orders now.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let (Some(_), Some(pool)) = (state.stitchhub.as_ref(), state.db.as_ref()) else {
        return crate::designs::not_found();
    };
    let signed_in = current_user(&state, &headers).await.is_some();
    let rows: Vec<ShopRow> = sqlx::query_as(
        "SELECT party, handle, account_id, catalog FROM mf_shops WHERE hub_ready AND account_confirmed ORDER BY name LIMIT 500",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let mut items = String::new();
    for shop in rows.into_iter().filter_map(shop_of) {
        if !crate::selling::can_sell(&state, pool, shop.account).await {
            continue;
        }
        let c = &shop.catalog;
        let how = match (c.shop.ships, c.shop.pickup) {
            (true, true) => "ships or pick up",
            (true, false) => "ships",
            _ => "pick up",
        };
        items.push_str(&format!(
            r#"<li><a href="/shops/{handle}"><strong>{name}</strong></a> <span class="muted">{town} · {n} products · {how}</span></li>"#,
            handle = escape(&shop.handle),
            name = escape(&c.shop.name),
            town = escape(&c.shop.town),
            n = c.products.len(),
        ));
    }
    let body = if items.is_empty() {
        "<p>No shops take orders yet.</p>".to_string()
    } else {
        format!(r#"<ul class="shops">{items}</ul>"#)
    };
    Html(page(
        "Order from a shop · MakerFoundry",
        &format!(
            r#"<section class="design"><h1>Order from a shop</h1><p class="lead">Shirts, hoodies, hats and totes, embroidered or printed with your design by a local shop.</p>{body}</section>"#
        ),
        signed_in,
    ))
    .into_response()
}

/// `GET /shops/<handle>`: a shop's products and prices.
pub async fn show(State(state): State<AppState>, headers: HeaderMap, Path(handle): Path<String>) -> Response {
    let (Some(_), Some(pool)) = (state.stitchhub.as_ref(), state.db.as_ref()) else {
        return crate::designs::not_found();
    };
    let Some(shop) = orderable(&state, pool, &handle).await else {
        return crate::designs::not_found();
    };
    let signed_in = current_user(&state, &headers).await.is_some();
    let c = &shop.catalog;
    let products: String = c.products.iter().map(|p| product_html(&shop.handle, p)).collect();
    let shipping = if c.shop.ships {
        shipping_words(&c.shipping)
    } else {
        String::new()
    };
    let pickup = if c.shop.pickup {
        " Pick up at the shop for free."
    } else {
        ""
    };
    Html(page(
        &format!("{} · MakerFoundry", c.shop.name),
        &format!(
            r#"<section class="design"><p><a href="/shops">Shops</a></p><h1>{name}</h1><p class="lead">{town}</p><p>{shipping}{pickup}</p><div class="products">{products}</div></section>"#,
            name = escape(&c.shop.name),
            town = escape(&c.shop.town),
        ),
        signed_in,
    ))
    .into_response()
}

// ---------------------------------------------------------------- a shop's account confirming it

/// The shops whose catalogues name `account` to be paid, for its account page: each confirmed
/// ("this is my shop") or not. A shop isn't offered until its account confirms it, so nobody can
/// route orders to someone else's payouts.
pub async fn claims_html(pool: &PgPool, account: i64) -> String {
    let shops: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT handle, name, account_confirmed FROM mf_shops WHERE account_id = $1 ORDER BY name LIMIT 20",
    )
    .bind(account)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    if shops.is_empty() {
        return String::new();
    }
    let rows: String = shops
        .iter()
        .map(|(handle, name, confirmed)| {
            let (state, action, label) = if *confirmed {
                ("confirmed as yours; customers can order from it", "disown", "This isn't my shop")
            } else {
                ("names your account to be paid, but isn't confirmed yet, so customers can't order from it", "confirm", "This is my shop")
            };
            format!(
                r#"<li><strong>{name}</strong> ({state}) <form method="post" action="/account/shops/{handle}/{action}" class="inline"><button type="submit" class="secondary">{label}</button></form></li>"#,
                name = escape(name),
                handle = escape(handle),
            )
        })
        .collect();
    format!(
        r#"<h2>Your shops on StitchHub</h2><p>Orders from these shops are paid to your account's payouts.</p><ul>{rows}</ul>"#
    )
}

/// `POST /account/shops/<handle>/<confirm|disown>`: the shop's account confirms it's theirs, or
/// says it isn't.
pub async fn confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((handle, action)): Path<(String, String)>,
) -> Response {
    if !crate::accounts::same_origin(&headers) {
        return crate::accounts::forbidden();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return axum::response::Redirect::to("/signin?next=/account").into_response();
    };
    let confirmed = match action.as_str() {
        "confirm" => true,
        "disown" => false,
        _ => return crate::designs::not_found(),
    };
    let done = sqlx::query("UPDATE mf_shops SET account_confirmed = $3 WHERE handle = $1 AND account_id = $2")
        .bind(&handle)
        .bind(user.account_id)
        .bind(confirmed)
        .execute(pool)
        .await;
    match done {
        Ok(r) if r.rows_affected() == 1 => axum::response::Redirect::to("/account").into_response(),
        Ok(_) => crate::designs::not_found(),
        Err(e) => {
            eprintln!("shops: confirm {handle}: {e}");
            (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Try again in a minute.").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shop_orders::contract::PriceBreak;

    #[test]
    fn a_piece_costs_the_garment_and_the_decoration_at_its_break() {
        let m = Method {
            method: "embroidery".into(),
            placements: vec!["left chest".into()],
            price_breaks: vec![PriceBreak { min: 1, cents: 1200 }, PriceBreak { min: 12, cents: 900 }],
            setup_cents: 2500,
        };
        let p = Product {
            sku: "t".into(),
            name: "Tee".into(),
            kind: "tee".into(),
            cents: 800,
            colours: vec!["Navy".into()],
            sizes: vec!["M".into()],
            methods: vec![m.clone()],
        };
        assert_eq!(piece_cents(&p, &m, 1), 2000);
        assert_eq!(piece_cents(&p, &m, 11), 2000);
        assert_eq!(piece_cents(&p, &m, 12), 1700);
        assert_eq!(piece_cents(&p, &m, 500), 1700);
        assert_eq!(dollars(1700), "$17.00");
        assert_eq!(dollars(5), "$0.05");
    }
}
