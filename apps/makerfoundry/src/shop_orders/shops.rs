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

/// Keeps a shop's latest catalogue (`from` is `shop:<hub shop id>`; the id is its page's handle),
/// with its branding: its short address if it's free, and its logo to fetch if it changed.
pub async fn keep(
    pool: &PgPool,
    storage: Option<&crate::storage::Storage>,
    from: &str,
    c: &Catalog,
) -> Result<(), String> {
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
    .map_err(|e| e.to_string())?;
    let brand = super::brand::read(c.shop.brand.as_ref());
    // its short address: first come first served
    let subdomain = match &brand.handle {
        Some(h) => {
            let taken: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM mf_shops WHERE subdomain = $1 AND party <> $2)")
                    .bind(h)
                    .bind(from)
                    .fetch_one(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            if taken {
                eprintln!("shops: {from} asked for an address another shop has");
                None
            } else {
                Some(h.clone())
            }
        }
        None => None,
    };
    sqlx::query("UPDATE mf_shops SET subdomain = $2 WHERE party = $1")
        .bind(from)
        .bind(&subdomain)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    // its logo: a new one is fetched by the next round; the old one goes
    let (sha, size) = brand
        .logo
        .as_ref()
        .map_or((None, None), |l| (Some(l.sha256.clone()), i64::try_from(l.size).ok()));
    let (had_sha, had_key): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT logo_sha256, logo_key FROM mf_shops WHERE party = $1")
            .bind(from)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    let old = if had_sha == sha {
        None
    } else {
        sqlx::query(
            "UPDATE mf_shops SET logo_sha256 = $2, logo_size = $3, logo_key = NULL, logo_type = NULL WHERE party = $1",
        )
        .bind(from)
        .bind(&sha)
        .bind(size)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        Some(had_key)
    };
    if let (Some(Some(key)), Some(storage)) = (old, storage)
        && let Err(e) = storage.delete(&key).await
    {
        eprintln!("shops: deleting an old logo of {from}: {e}");
    }
    Ok(())
}

/// Shops' logos not fetched yet: each from the hub's files (the `catalog` conversation), checked
/// by size and SHA-256, drawn again as PNG or JPEG and kept. One that the hub doesn't have, or
/// that isn't a picture, is let go (the shop shows without one); one that fails for a moment is
/// tried again next round.
pub async fn fetch_logos(pool: &PgPool, hub: &super::hub::Hub, storage: &crate::storage::Storage) {
    let due: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT party, logo_sha256, logo_size FROM mf_shops
         WHERE logo_sha256 IS NOT NULL AND logo_key IS NULL AND logo_size IS NOT NULL LIMIT 5",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (party, sha, size) in due {
        let let_go = |why: String| {
            let (pool, party) = (pool.clone(), party.clone());
            async move {
                eprintln!("shops: {party}'s logo left out: {why}");
                let _ = sqlx::query("UPDATE mf_shops SET logo_sha256 = NULL, logo_size = NULL WHERE party = $1")
                    .bind(&party)
                    .execute(&pool)
                    .await;
            }
        };
        let bytes = match hub
            .download(
                super::contract::CATALOG,
                &sha,
                u64::try_from(size).unwrap_or(0),
                super::brand::MAX_LOGO_BYTES,
            )
            .await
        {
            Ok(b) => b,
            Err(why) if why.contains("no such file") || why.contains("isn't the one") || why.contains("too large") => {
                let_go(why).await;
                continue;
            }
            Err(why) => {
                eprintln!("shops: {party}'s logo: {why} (tried again)");
                continue;
            }
        };
        let drawn = tokio::task::spawn_blocking(move || {
            let out = crate::photos::reencode(&bytes)?;
            let kind = crate::photos::check(&out)?;
            Ok::<_, String>((out, kind))
        })
        .await;
        let Ok(Ok((image, kind))) = drawn else {
            let_go("not a PNG or JPEG".into()).await;
            continue;
        };
        let Ok(key) = crate::storage::new_key("shop-logo") else {
            continue;
        };
        if let Err(e) = storage.put(&key, image, kind).await {
            eprintln!("shops: {party}'s logo: {e} (tried again)");
            continue;
        }
        let kept = sqlx::query(
            "UPDATE mf_shops SET logo_key = $3, logo_type = $4 WHERE party = $1 AND logo_sha256 = $2 AND logo_key IS NULL",
        )
        .bind(&party)
        .bind(&sha)
        .bind(&key)
        .bind(kind)
        .execute(pool)
        .await;
        if !matches!(kept, Ok(r) if r.rows_affected() == 1) {
            // (the catalogue changed meanwhile: this one isn't wanted)
            let _ = storage.delete(&key).await;
        }
    }
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
    render(&state, &headers, &handle, false).await
}

/// What MakerFoundry keeps of a shop's branding besides its catalogue: its short address and
/// whether its logo is kept.
async fn branding(pool: &PgPool, party: &str) -> (Option<String>, bool) {
    sqlx::query_as::<_, (Option<String>, bool)>("SELECT subdomain, logo_key IS NOT NULL FROM mf_shops WHERE party = $1")
        .bind(party)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or((None, false))
}

/// The host MakerFoundry is served at (`makerfoundry.app`), from its public address.
fn apex(state: &AppState) -> Option<String> {
    reqwest::Url::parse(&state.public_url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

/// A shop's page, at `/shops/<handle>` or (`on_own_address`) at its short address, where its
/// stylesheet and logo are served from the same address (the pages' CSP allows only their own).
async fn render(state: &AppState, headers: &HeaderMap, handle: &str, on_own_address: bool) -> Response {
    let (Some(_), Some(pool)) = (state.stitchhub.as_ref(), state.db.as_ref()) else {
        return crate::designs::not_found();
    };
    let Some(shop) = orderable(state, pool, handle).await else {
        return crate::designs::not_found();
    };
    let signed_in = current_user(state, headers).await.is_some();
    let c = &shop.catalog;
    let brand = super::brand::read(c.shop.brand.as_ref());
    let (subdomain, has_logo) = branding(pool, &shop.party).await;
    let base = if on_own_address {
        String::new()
    } else {
        format!("/shops/{}", escape(&shop.handle))
    };
    let name = brand.display_name.clone().unwrap_or_else(|| c.shop.name.clone());
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
    let style = if super::brand::css(&brand).is_empty() {
        String::new()
    } else {
        format!(r#"<link rel="stylesheet" href="{base}/brand.css">"#)
    };
    let logo = if has_logo {
        format!(r#"<img class="shop-logo" src="{base}/logo" alt="{}">"#, escape(&name))
    } else {
        String::new()
    };
    let intro = brand.intro.as_deref().map(super::brand::intro_html).unwrap_or_default();
    let website = brand
        .website
        .as_deref()
        .map(|w| {
            format!(
                r#"<p><a href="{}" rel="noopener nofollow ugc">The shop's website</a></p>"#,
                escape(w)
            )
        })
        .unwrap_or_default();
    let address = match (&subdomain, apex(state)) {
        (Some(sub), Some(apex)) if !on_own_address => format!(
            r#"<p class="muted">Its own address: <a href="https://{sub}.{apex}/">{sub}.{apex}</a></p>"#,
            sub = escape(sub),
            apex = escape(&apex),
        ),
        _ => String::new(),
    };
    let back = match apex(state) {
        Some(apex) if on_own_address => format!(
            r#"<p><a href="https://{}/shops">Shops on MakerFoundry</a></p>"#,
            escape(&apex)
        ),
        _ => r#"<p><a href="/shops">Shops</a></p>"#.to_string(),
    };
    Html(page(
        &format!("{name} · MakerFoundry"),
        &format!(
            r#"<section class="design shop-brand">{style}{back}<div class="shop-head">{logo}<div><h1>{name_html}</h1><p class="lead">{town}</p></div></div>{intro}{website}{address}<p>{shipping}{pickup}</p><div class="products">{products}</div><p class="muted">Sold by {name_html} through MakerFoundry, which takes the payment.</p></section>"#,
            name_html = escape(&name),
            town = escape(&c.shop.town),
        ),
        signed_in,
    ))
    .into_response()
}

/// `/shops/<handle>/logo` (and `/logo` at its short address): the shop's logo, as drawn again.
pub async fn logo(State(state): State<AppState>, Path(handle): Path<String>) -> Response {
    logo_of(&state, &handle).await
}

async fn logo_of(state: &AppState, handle: &str) -> Response {
    let not_found = || axum::http::StatusCode::NOT_FOUND.into_response();
    let (Some(pool), Some(storage)) = (state.db.as_ref(), state.storage.as_ref()) else {
        return not_found();
    };
    if orderable(state, pool, handle).await.is_none() {
        return not_found();
    }
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT logo_key, logo_type FROM mf_shops WHERE handle = $1")
            .bind(handle)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let Some((Some(key), Some(kind))) = row else {
        return not_found();
    };
    let kind = if kind == "image/png" { "image/png" } else { "image/jpeg" };
    match storage.get(&key).await {
        Ok(Some(bytes)) => (
            [
                (axum::http::header::CONTENT_TYPE, kind),
                (axum::http::header::CACHE_CONTROL, "public, max-age=3600"),
            ],
            bytes,
        )
            .into_response(),
        _ => not_found(),
    }
}

/// `/shops/<handle>/brand.css` (and `/brand.css` at its short address): its accent colour.
pub async fn brand_css(State(state): State<AppState>, Path(handle): Path<String>) -> Response {
    css_of(&state, &handle).await
}

async fn css_of(state: &AppState, handle: &str) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    let Some(shop) = orderable(state, pool, handle).await else {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    };
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (axum::http::header::CACHE_CONTROL, "public, max-age=300"),
        ],
        super::brand::css(&super::brand::read(shop.catalog.shop.brand.as_ref())),
    )
        .into_response()
}

/// Shops' short addresses (`<subdomain>.makerfoundry.app`): its page at `/`, with its stylesheet
/// and logo, and the site's own stylesheet and icon; anything else goes to the same address on
/// MakerFoundry itself, where signing in and checking out stay. `www.` goes to MakerFoundry; an
/// address no shop has is "not found". Requests for MakerFoundry itself pass straight through.
pub async fn subdomains(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let host = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(|h| h.split(':').next().unwrap_or("").to_ascii_lowercase())
        .unwrap_or_default();
    let Some(apex) = apex(&state) else {
        return next.run(request).await;
    };
    let Some(label) = host.strip_suffix(&format!(".{apex}")) else {
        return next.run(request).await;
    };
    let path_and_query = request.uri().path_and_query().map_or("/", |p| p.as_str()).to_string();
    let to_main = || axum::response::Redirect::permanent(&format!("https://{apex}{path_and_query}")).into_response();
    if label == "www" {
        return to_main();
    }
    let path = request.uri().path().to_string();
    if path.starts_with("/static/") || path == "/favicon.svg" {
        return next.run(request).await;
    }
    let handle: Option<String> = match (state.db.as_ref(), super::brand::valid_handle(label)) {
        (Some(pool), true) => sqlx::query_scalar("SELECT handle FROM mf_shops WHERE subdomain = $1")
            .bind(label)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten(),
        _ => None,
    };
    let Some(handle) = handle else {
        return crate::designs::not_found();
    };
    let headers = request.headers().clone();
    match path.as_str() {
        "/" => render(&state, &headers, &handle, true).await,
        "/logo" => logo_of(&state, &handle).await,
        "/brand.css" => css_of(&state, &handle).await,
        _ => to_main(),
    }
}

// ---------------------------------------------------------------- a shop's account confirming it

/// The shops whose catalogues name `account` to be paid, for its account page: each confirmed
/// ("this is my shop") or not. A shop isn't offered until its account confirms it, so nobody can
/// route orders to someone else's payouts. Newest catalogue first, so a shop that just named the
/// account is always on the page to confirm, even when 20 older ones name it too.
pub async fn claims_html(pool: &PgPool, account: i64) -> String {
    let shops: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT handle, name, account_confirmed FROM mf_shops WHERE account_id = $1
         ORDER BY updated_at DESC, name LIMIT 20",
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
