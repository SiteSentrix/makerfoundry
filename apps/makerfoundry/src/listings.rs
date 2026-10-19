//! Listings (MF-3): a draft gets its details (title, description, tags, licence) and is
//! published; published designs have a public page (`/d/<id>`), a free download, and are found
//! by browsing and searching (`/browse`), on their maker's page and on the home page.
//!
//! Until the public launch (MF-6, with the takedown process of MF-5 in place), publishing is open
//! only to the accounts in `MF_PUBLISHERS` (account ids, or `*` for everyone).

use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use sqlx::{PgPool, Row};

use crate::AppState;
use crate::accounts::{User, current_user, forbidden, private, same_origin};
use crate::designs::{self, DesignRow};
use crate::licences::{self, LICENCES};
use crate::makers;
use crate::money::dollars;
use crate::pages::{escape, page};
use crate::selling;

pub const MAX_TITLE: usize = 100;
pub const MAX_DESCRIPTION: usize = 5000;
pub const MAX_TAGS: usize = 10;
pub const MAX_TAG: usize = 30;
/// Designs on a page of browsing.
const PAGE_SIZE: i64 = 24;

/// Who may publish.
#[derive(Clone, Debug, PartialEq)]
pub enum Publishers {
    Everyone,
    Only(Vec<i64>),
}

impl Publishers {
    /// From `MF_PUBLISHERS`: `*`, or account ids apart by commas; nobody without it.
    pub fn from_setting(value: Option<&str>) -> Publishers {
        match value.map(str::trim) {
            Some("*") => Publishers::Everyone,
            Some(list) => Publishers::Only(list.split(',').filter_map(|s| s.trim().parse().ok()).collect()),
            None => Publishers::Only(Vec::new()),
        }
    }

    pub fn allows(&self, account_id: i64) -> bool {
        match self {
            Publishers::Everyone => true,
            Publishers::Only(ids) => ids.contains(&account_id),
        }
    }
}

/// Tags as typed (apart by commas) made tidy: lower case, letters, digits, spaces and hyphens,
/// each at most `MAX_TAG`, no repeats, at most `MAX_TAGS`.
pub fn tidy_tags(text: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for raw in text.split(',') {
        let tag: String = raw
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
            .collect();
        let tag = tag
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(MAX_TAG)
            .collect::<String>();
        if !tag.is_empty() && !tags.contains(&tag) && tags.len() < MAX_TAGS {
            tags.push(tag);
        }
    }
    tags
}

/// Plain text as HTML paragraphs (a blank line apart), escaped, line breaks kept.
pub fn paragraphs(text: &str) -> String {
    text.split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("<p>{}</p>", escape(p).replace('\n', "<br>")))
        .collect()
}

/// A design's address words: its title in lower case with hyphens.
pub fn slug(title: &str) -> String {
    let s: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-')
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(60)
        .collect()
}

// ---------------------------------------------------------------- finding designs

/// What to find: published designs, narrowed by any of these.
#[derive(Default, Debug)]
pub struct Filter {
    pub kind: Option<String>,
    pub tag: Option<String>,
    pub words: Option<String>,
    pub account: Option<i64>,
    /// `Some(true)`: free designs only; `Some(false)`: paid only.
    pub free: Option<bool>,
    pub popular: bool,
    pub page: i64,
}

/// A published design as a card shows it.
pub struct Card {
    pub id: i64,
    pub title: String,
    pub kind: String,
    pub format: String,
    pub photo: Option<i64>,
    pub maker: String,
    pub downloads: i64,
    pub price: i64,
}

pub async fn find(pool: &PgPool, f: &Filter, limit: i64) -> Vec<Card> {
    let rows = sqlx::query(
        "SELECT d.id, d.title, d.kind, d.format, d.downloads, d.price_cents, m.name AS maker,
                (SELECT p.id FROM mf_design_photos p WHERE p.design_id = d.id ORDER BY p.position LIMIT 1) AS photo
         FROM mf_designs d JOIN mf_makers m ON m.account_id = d.account_id
         WHERE d.status = 'published'
           AND ($1::text IS NULL OR d.kind = $1)
           AND ($2::text IS NULL OR $2 = ANY (d.tags))
           AND ($3::text IS NULL OR d.search @@ websearch_to_tsquery('english', $3))
           AND ($4::bigint IS NULL OR d.account_id = $4)
           AND ($8::bool IS NULL OR (d.price_cents = 0) = $8)
         ORDER BY CASE WHEN $5 THEN d.downloads END DESC NULLS LAST,
                  CASE WHEN $3::text IS NOT NULL AND NOT $5 THEN ts_rank(d.search, websearch_to_tsquery('english', $3)) END DESC NULLS LAST,
                  d.published_at DESC
         LIMIT $6 OFFSET $7",
    )
    .bind(&f.kind)
    .bind(&f.tag)
    .bind(&f.words)
    .bind(f.account)
    .bind(f.popular)
    .bind(limit)
    .bind(f.page.max(0).saturating_mul(limit))
    .bind(f.free)
    .fetch_all(pool)
    .await;
    match rows {
        Ok(rows) => rows
            .iter()
            .filter_map(|r| {
                Some(Card {
                    id: r.try_get("id").ok()?,
                    title: r.try_get("title").ok()?,
                    kind: r.try_get("kind").ok()?,
                    format: r.try_get("format").ok()?,
                    photo: r.try_get("photo").ok()?,
                    maker: r.try_get("maker").ok()?,
                    downloads: r.try_get("downloads").ok()?,
                    price: r.try_get("price_cents").ok()?,
                })
            })
            .collect(),
        Err(e) => {
            eprintln!("listings: {e}");
            Vec::new()
        }
    }
}

fn kind_name(kind: &str) -> &'static str {
    if kind == "laser" { "Laser" } else { "Embroidery" }
}

pub fn cards_html(cards: &[Card]) -> String {
    if cards.is_empty() {
        return String::new();
    }
    let items: String = cards
        .iter()
        .map(|c| {
            let picture = match c.photo {
                Some(p) => format!(r#"<img src="/designs/{}/photos/{p}/thumb" alt="" loading="lazy">"#, c.id),
                None => r#"<span class="muted">No photo</span>"#.to_string(),
            };
            format!(
                r#"<li><a href="/d/{id}/{slug}"><span class="thumb">{picture}</span><strong>{title}</strong><span class="muted">by {maker} · {kind}, {format}{downloads}</span><span class="price">{price}</span></a></li>"#,
                id = c.id,
                slug = slug(&c.title),
                title = escape(&c.title),
                maker = escape(&c.maker),
                kind = kind_name(&c.kind),
                format = escape(&c.format),
                downloads = match c.downloads {
                    0 => String::new(),
                    1 => " · 1 download".to_string(),
                    n => format!(" · {n} downloads"),
                },
                price = if c.price == 0 { "Free".to_string() } else { dollars(c.price) },
            )
        })
        .collect();
    format!(r#"<ul class="cards">{items}</ul>"#)
}

/// A maker's published designs, as cards.
pub async fn cards(pool: &PgPool, account: i64, limit: i64) -> String {
    cards_html(
        &find(
            pool,
            &Filter {
                account: Some(account),
                ..Filter::default()
            },
            limit,
        )
        .await,
    )
}

/// The home page's newest designs (empty while there are none).
pub async fn newest(state: &AppState) -> String {
    let Some(pool) = state.db.as_ref() else {
        return String::new();
    };
    let found = find(pool, &Filter::default(), 8).await;
    if found.is_empty() {
        return String::new();
    }
    format!(
        r#"<section class="design"><h2>Newest designs</h2>{}<p><a href="/browse">Browse all designs</a></p></section>"#,
        cards_html(&found)
    )
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct BrowseQuery {
    q: String,
    kind: String,
    tag: String,
    sort: String,
    price: String,
    page: i64,
}

pub async fn browse(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<BrowseQuery>) -> Response {
    let signed_in = current_user(&state, &headers).await.is_some();
    let words = q.q.trim().chars().take(200).collect::<String>();
    let kind = matches!(q.kind.as_str(), "embroidery" | "laser").then(|| q.kind.clone());
    let tag = tidy_tags(&q.tag).into_iter().next();
    let popular = q.sort == "popular";
    let page_no = q.page.clamp(0, 1000);
    let free = match q.price.as_str() {
        "free" => Some(true),
        "paid" => Some(false),
        _ => None,
    };
    let price = match free {
        Some(true) => "free",
        Some(false) => "paid",
        None => "",
    };
    let filter = Filter {
        kind: kind.clone(),
        tag: tag.clone(),
        words: (!words.is_empty()).then(|| words.clone()),
        account: None,
        free,
        popular,
        page: page_no,
    };
    let found = match state.db.as_ref() {
        Some(pool) => find(pool, &filter, PAGE_SIZE).await,
        None => Vec::new(),
    };
    let selected = |on: bool| if on { " selected" } else { "" };
    let tag_note = match &tag {
        Some(t) => format!(
            r#"<p>Tagged <strong>{t}</strong> · <a href="/browse?q={q}&amp;kind={k}&amp;price={price}&amp;sort={s}">show all tags</a></p>"#,
            t = escape(t),
            q = escape(&url_word(&words)),
            k = escape(kind.as_deref().unwrap_or("")),
            s = if popular { "popular" } else { "newest" },
        ),
        None => String::new(),
    };
    let results = if found.is_empty() {
        r#"<p class="lead">No designs found.</p>"#.to_string()
    } else {
        cards_html(&found)
    };
    let more = if found.len() as i64 == PAGE_SIZE {
        format!(
            r#"<p><a href="/browse?q={q}&amp;kind={k}&amp;price={price}&amp;tag={t}&amp;sort={s}&amp;page={n}">More designs</a></p>"#,
            q = escape(&url_word(&words)),
            k = escape(kind.as_deref().unwrap_or("")),
            t = escape(&url_word(tag.as_deref().unwrap_or(""))),
            s = if popular { "popular" } else { "newest" },
            n = page_no + 1,
        )
    } else {
        String::new()
    };
    Html(page(
        "Browse designs · MakerFoundry",
        &format!(
            r#"<section class="design">
<h1>Browse designs</h1>
<form class="search" method="get" action="/browse">
<input type="search" name="q" value="{words}" placeholder="Search designs" aria-label="Search designs" maxlength="200">
<select name="kind" aria-label="Kind"><option value="">Embroidery and laser</option><option value="embroidery"{e}>Embroidery</option><option value="laser"{l}>Laser</option></select>
<select name="price" aria-label="Price"><option value="">Free and paid</option><option value="free"{fr}>Free</option><option value="paid"{pd}>Paid</option></select>
<select name="sort" aria-label="Order"><option value="newest">Newest</option><option value="popular"{p}>Most downloaded</option></select>
<input type="hidden" name="tag" value="{tag}">
<button type="submit">Search</button>
</form>
{tag_note}{results}{more}
</section>"#,
            words = escape(&words),
            e = selected(kind.as_deref() == Some("embroidery")),
            l = selected(kind.as_deref() == Some("laser")),
            p = selected(popular),
            fr = selected(free == Some(true)),
            pd = selected(free == Some(false)),
            tag = escape(tag.as_deref().unwrap_or("")),
        ),
        signed_in,
    ))
    .into_response()
}

/// `text` for a URL's query.
fn url_word(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

// ---------------------------------------------------------------- a design's public page

fn not_found(signed_in: bool) -> Response {
    (StatusCode::NOT_FOUND, Html(page("Not found · MakerFoundry", r#"<section class="hero"><h1>Design not found</h1><p class="lead"><a href="/browse">Browse designs</a></p></section>"#, signed_in))).into_response()
}

/// Whether `viewer` may see design `d`: anyone once it's published; otherwise (a draft, or taken
/// down) only its maker and the admins.
pub fn visible(d: &DesignRow, viewer: Option<&User>, admins: &[i64]) -> bool {
    d.status == "published" || viewer.is_some_and(|u| u.account_id == d.account_id || admins.contains(&u.account_id))
}

pub async fn public_page(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    show(state, headers, id).await
}

pub async fn public_page_slug(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, _slug)): Path<(i64, String)>,
) -> Response {
    show(state, headers, id).await
}

async fn show(state: AppState, headers: HeaderMap, id: i64) -> Response {
    let viewer = current_user(&state, &headers).await;
    let signed_in = viewer.is_some();
    let Some(pool) = state.db.as_ref() else {
        return not_found(signed_in);
    };
    let d = match designs::design(pool, id).await {
        Ok(Some(d)) if visible(&d, viewer.as_ref(), &state.admins) => d,
        _ => return not_found(signed_in),
    };
    let maker = makers::profile(pool, d.account_id).await;
    let mine = viewer.as_ref().is_some_and(|u| u.account_id == d.account_id);
    let draft_note = match d.status.as_str() {
        "published" => String::new(),
        "removed" => format!(
            r#"<p class="problem">Taken down: only its maker and the admins can see this page. {}</p>"#,
            escape(d.removed_reason.as_deref().unwrap_or(""))
        ),
        _ => format!(
            r#"<p class="problem">A draft: only you can see this page. <a href="/designs/{id}">Back to the draft</a></p>"#
        ),
    };
    let byline = match &maker {
        Some(m) => format!(
            r#"by <a href="/makers/{h}">{n}</a>"#,
            h = escape(&m.handle),
            n = escape(&m.name)
        ),
        None => String::new(),
    };
    let downloads = match d.downloads {
        0 => String::new(),
        1 => " · 1 download".into(),
        n => format!(" · {n} downloads"),
    };
    let licence = d.licence.as_deref().and_then(licences::find);
    let licence_html = match licence {
        Some(l) => format!(
            r#"<h2>Licence: {name}</h2><p>{plain}{link}</p>"#,
            name = escape(l.name),
            plain = escape(&l.plain()),
            link = l
                .url
                .map(|u| format!(r#" <a href="{u}" rel="noopener">The licence</a>"#))
                .unwrap_or_default(),
        ),
        None => String::new(),
    };
    let tags: String = d
        .tags
        .iter()
        .map(|t| {
            format!(
                r#"<li><a href="/browse?tag={u}">{t}</a></li>"#,
                u = escape(&url_word(t)),
                t = escape(t)
            )
        })
        .collect();
    let tags = if tags.is_empty() {
        String::new()
    } else {
        format!(r#"<ul class="tags">{tags}</ul>"#)
    };
    let buy = selling::buy_html(
        &state,
        pool,
        viewer.as_ref(),
        id,
        d.price_cents,
        &licence.map(|l| escape(l.name)).unwrap_or_default(),
    )
    .await;
    let intro = format!(r#"{draft_note}<p class="byline">{byline}{downloads}</p>{buy}"#);
    let photos = designs::photos_html(pool, id).await;
    let edit = if mine {
        format!(r#"<p><a href="/designs/{id}">Manage this design</a></p>"#)
    } else if d.status == "published" {
        format!(r#"<p class="muted"><a href="/d/{id}/report">Report this design</a></p>"#)
    } else {
        String::new()
    };
    let footer = format!(
        r#"{desc}{photos}{tags}{licence_html}{edit}"#,
        desc = paragraphs(&d.description)
    );
    let title = if d.title.is_empty() {
        d.file_name.clone()
    } else {
        d.title.clone()
    };
    match designs::facts_section(&d, &title, &intro, &footer) {
        Some(html) => Html(page(&format!("{} · MakerFoundry", escape(&title)), &html, signed_in)).into_response(),
        None => not_found(signed_in),
    }
}

/// A design's file, by a short-lived link: a free published design's for anyone, a paid one's for
/// the people who bought it (even once it's unpublished, unless it was taken down), and any for its
/// maker and the admins. Downloads of published designs are counted.
pub async fn download(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    let viewer = current_user(&state, &headers).await;
    let (Some(pool), Some(storage)) = (state.db.as_ref(), state.storage.as_ref()) else {
        return not_found(viewer.is_some());
    };
    let Ok(Some(d)) = designs::design(pool, id).await else {
        return not_found(viewer.is_some());
    };
    let staff = viewer
        .as_ref()
        .is_some_and(|u| u.account_id == d.account_id || state.admins.contains(&u.account_id));
    let buyer = match &viewer {
        Some(u) if d.status != "removed" && !staff => selling::bought(pool, u.account_id, id).await,
        _ => false,
    };
    let seen = visible(&d, viewer.as_ref(), &state.admins) || buyer;
    if !seen {
        return not_found(viewer.is_some());
    }
    if d.price_cents > 0 && !staff && !buyer {
        // not bought: the page with the button to buy it
        return Redirect::to(&format!("/d/{id}/{}", slug(&d.title))).into_response();
    }
    if d.status == "published" {
        let _ = sqlx::query("UPDATE mf_designs SET downloads = downloads + 1 WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await;
    }
    let name = if d.title.is_empty() {
        d.file_name.clone()
    } else {
        // the title, with the file's own extension
        let ext = d.file_name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        format!("{}.{ext}", slug(&d.title))
    };
    Redirect::to(storage.download_url(&d.file_key, &name).as_str()).into_response()
}

// ---------------------------------------------------------------- details and publishing

/// What a design's details form shows.
struct Details<'a> {
    title: &'a str,
    description: &'a str,
    tags: &'a str,
    licence: &'a str,
    price: &'a str,
    can_sell: bool,
}

fn details_form(id: i64, f: &Details, problem: &str) -> String {
    let Details {
        title,
        description,
        tags,
        licence,
        price,
        can_sell,
    } = *f;
    let problem = if problem.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem" role="alert">{}</p>"#, escape(problem))
    };
    let choices: String = LICENCES
        .iter()
        .map(|l| {
            format!(
                r#"<label class="choice"><input type="radio" name="licence" value="{id}"{checked}> <span><strong>{name}</strong><br><span class="muted">{plain}</span></span></label>"#,
                id = l.id,
                checked = if l.id == licence { " checked" } else { "" },
                name = escape(l.name),
                plain = escape(&l.plain()),
            )
        })
        .collect();
    format!(
        r#"<section class="form">
<h1>Design details</h1>
{problem}
<form method="post" action="/designs/{id}/edit">
<label>Title<input name="title" value="{title}" required maxlength="{MAX_TITLE}"></label>
<label>Description <small>(what it is, sizes, fabrics or materials, tips)</small><textarea name="description" rows="8" maxlength="{MAX_DESCRIPTION}">{description}</textarea></label>
<label>Tags <small>(apart by commas, up to {MAX_TAGS}: flowers, christmas, coaster…)</small><input name="tags" value="{tags}" maxlength="400"></label>
<fieldset><legend>Licence: what people may do with what they make</legend>{choices}</fieldset>
<label>Price <small>({price_note})</small><input name="price" value="{price}" inputmode="decimal" maxlength="10"></label>
<button type="submit">Save</button>
</form>
<p><a href="/designs/{id}">Back to the design</a></p>
</section>"#,
        title = escape(title),
        description = escape(description),
        tags = escape(tags),
        price = escape(price),
        price_note = if can_sell {
            "US dollars, from $1 to $200; 0 shares it free. You get the price less 10% and the card fee."
        } else {
            r#"0 shares it free. To sell it, first <a href="/account/selling">set up selling</a>."#
        },
    )
}

async fn own(state: &AppState, headers: &HeaderMap, id: i64) -> Result<(User, PgPool, DesignRow), Response> {
    let (Some(user), Some(pool)) = (current_user(state, headers).await, state.db.clone()) else {
        return Err(Redirect::to("/signin").into_response());
    };
    match designs::design(&pool, id).await {
        Ok(Some(d)) if d.account_id == user.account_id => Ok((user, pool, d)),
        _ => Err(not_found(true)),
    }
}

pub async fn edit_page(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    let (user, pool, d) = match own(&state, &headers, id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    let title = if d.title.is_empty() {
        d.file_name
            .rsplit_once('.')
            .map_or(d.file_name.as_str(), |(n, _)| n)
            .to_string()
    } else {
        d.title.clone()
    };
    let price = if d.price_cents == 0 {
        "0".to_string()
    } else {
        dollars(d.price_cents).trim_start_matches('$').to_string()
    };
    let can_sell = selling::can_sell(&state, &pool, user.account_id).await;
    private(
        "Design details · MakerFoundry",
        &details_form(
            id,
            &Details {
                title: &title,
                description: &d.description,
                tags: &d.tags.join(", "),
                licence: d.licence.as_deref().unwrap_or(""),
                price: &price,
                can_sell,
            },
            "",
        ),
        true,
    )
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct DetailsForm {
    title: String,
    description: String,
    tags: String,
    licence: String,
    price: String,
}

pub async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(f): Form<DetailsForm>,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool, _) = match own(&state, &headers, id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    let title = f.title.split_whitespace().collect::<Vec<_>>().join(" ");
    let description = f.description.trim().replace("\r\n", "\n");
    let tags = tidy_tags(&f.tags);
    let can_sell = selling::can_sell(&state, &pool, user.account_id).await;
    let again = |why: &str| {
        private(
            "Design details · MakerFoundry",
            &details_form(
                id,
                &Details {
                    title: &title,
                    description: &description,
                    tags: &f.tags,
                    licence: &f.licence,
                    price: &f.price,
                    can_sell,
                },
                why,
            ),
            true,
        )
    };
    if title.is_empty() || title.chars().count() > MAX_TITLE {
        return again(&format!("Give a title of up to {MAX_TITLE} characters."));
    }
    if description.chars().count() > MAX_DESCRIPTION {
        return again(&format!("The description can be up to {MAX_DESCRIPTION} characters."));
    }
    if licences::find(&f.licence).is_none() {
        return again("Choose a licence.");
    }
    let price = match selling::check_price(&f.price) {
        Ok(p) => p,
        Err(why) => return again(&why),
    };
    if price > 0 && !can_sell {
        return again("To sell a design, first set up selling (Account › Selling). Until then, give it a price of 0.");
    }
    let saved = sqlx::query(
        "UPDATE mf_designs SET title = $2, description = $3, tags = $4, licence = $5, price_cents = $7,
             search = setweight(to_tsvector('english', $2), 'A')
                   || setweight(to_tsvector('english', array_to_string($4::text[], ' ')), 'B')
                   || setweight(to_tsvector('english', $3), 'C')
         WHERE id = $1 AND account_id = $6",
    )
    .bind(id)
    .bind(&title)
    .bind(&description)
    .bind(&tags)
    .bind(&f.licence)
    .bind(user.account_id)
    .bind(price)
    .execute(&pool)
    .await;
    match saved {
        Ok(_) => Redirect::to(&format!("/designs/{id}")).into_response(),
        Err(e) => {
            eprintln!("listings: save {id}: {e}");
            again("The details couldn't be saved just now. Please try again.")
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PublishForm {
    confirm: String,
}

pub async fn publish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(f): Form<PublishForm>,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool, d) = match own(&state, &headers, id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    let problem = if d.status == "removed" {
        Some(
            "This design was taken down after a report, so it can't be published again. If you believe that was a mistake, send a counter-notice.",
        )
    } else if !state.publishers.allows(user.account_id) {
        Some("Publishing opens when MakerFoundry launches. Your design stays saved as a draft.")
    } else if makers::profile(&pool, user.account_id).await.is_none() {
        Some("Set up your maker profile first: it's the name people see on your designs.")
    } else if d.title.is_empty() || d.licence.is_none() {
        Some("Give the design a title and a licence first (Edit details).")
    } else if d.price_cents > 0 && !selling::can_sell(&state, &pool, user.account_id).await {
        Some("To sell this design, finish setting up selling first (Account › Selling), or make it free.")
    } else if f.confirm != "yes" {
        Some(if state.launched {
            "Confirm that the design is yours to share, and that you agree to the Seller Terms."
        } else {
            "Confirm that the design is yours to share."
        })
    } else {
        None
    };
    if let Some(p) = problem {
        return designs::manage_page(&state, &pool, &user, id, p).await;
    }
    let done = sqlx::query("UPDATE mf_designs SET status = 'published', published_at = coalesce(published_at, now()) WHERE id = $1 AND account_id = $2 AND status = 'draft'")
        .bind(id)
        .bind(user.account_id)
        .execute(&pool)
        .await;
    match done {
        Ok(_) => {
            if state.launched {
                crate::legal::agree(&pool, user.account_id, &[&crate::legal::SELLER_TERMS]).await;
            }
            Redirect::to(&format!("/d/{id}/{}", slug(&d.title))).into_response()
        }
        Err(e) => {
            eprintln!("listings: publish {id}: {e}");
            designs::manage_page(
                &state,
                &pool,
                &user,
                id,
                "The design couldn't be published just now. Please try again.",
            )
            .await
        }
    }
}

pub async fn unpublish(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (user, pool, _) = match own(&state, &headers, id).await {
        Ok(o) => o,
        Err(r) => return r,
    };
    if let Err(e) =
        sqlx::query("UPDATE mf_designs SET status = 'draft' WHERE id = $1 AND account_id = $2 AND status = 'published'")
            .bind(id)
            .bind(user.account_id)
            .execute(&pool)
            .await
    {
        eprintln!("listings: unpublish {id}: {e}");
    }
    Redirect::to(&format!("/designs/{id}")).into_response()
}

/// The part of a design's own (maker's) page about its listing: its details, and publishing.
pub fn manage_html(d: &DesignRow, id: i64, has_profile: bool, launched: bool) -> String {
    let licence = d
        .licence
        .as_deref()
        .and_then(licences::find)
        .map(|l| escape(l.name))
        .unwrap_or_else(|| "none yet".into());
    let details = format!(
        r#"<h2>Listing</h2><dl class="facts"><div><dt>Title</dt><dd>{title}</dd></div><div><dt>Licence</dt><dd>{licence}</dd></div><div><dt>Price</dt><dd>{price}</dd></div><div><dt>Tags</dt><dd>{tags}</dd></div></dl><p><a href="/designs/{id}/edit">Edit details</a></p>"#,
        title = if d.title.is_empty() {
            "none yet".into()
        } else {
            escape(&d.title)
        },
        price = if d.price_cents == 0 {
            "Free".to_string()
        } else {
            dollars(d.price_cents)
        },
        tags = if d.tags.is_empty() {
            "none".into()
        } else {
            escape(&d.tags.join(", "))
        },
    );
    let profile = if has_profile {
        String::new()
    } else {
        r#"<p>Before publishing, <a href="/account/maker">set up your maker profile</a>.</p>"#.to_string()
    };
    let publishing = if d.status == "removed" {
        format!(
            r#"<p class="problem">{why} Only you and the admins can see it. If you believe it was taken down by mistake, <a href="/designs/{id}/counter">send a counter-notice</a>.</p>"#,
            why = escape(d.removed_reason.as_deref().unwrap_or("Taken down after a report."))
        )
    } else if d.status == "published" {
        format!(
            r#"<p class="byline">Published: <a href="/d/{id}/{slug}">see its public page</a>.</p><form method="post" action="/designs/{id}/unpublish"><button type="submit" class="secondary">Unpublish (back to a draft)</button></form>"#,
            slug = slug(&d.title)
        )
    } else {
        let seller_terms = if launched {
            r#" I agree to the <a href="/seller-terms">Seller Terms</a>."#
        } else {
            ""
        };
        format!(
            r#"{profile}<form method="post" action="/designs/{id}/publish" class="publish">
<label class="choice"><input type="checkbox" name="confirm" value="yes"> <span>I made this design, or have the right to share it, and it doesn't copy anyone else's logos, characters or designs.{seller_terms}</span></label>
<button type="submit">Publish</button>
</form>"#
        )
    };
    format!("{details}{publishing}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_tidied() {
        assert_eq!(
            tidy_tags(" Flowers, CHRISTMAS ,flowers,, coaster <b>,  two   words "),
            ["flowers", "christmas", "coaster b", "two words"]
        );
        assert_eq!(tidy_tags(&"a,".repeat(20)).len(), 1);
        let many: String = (0..20).map(|i| format!("t{i},")).collect();
        assert_eq!(tidy_tags(&many).len(), MAX_TAGS);
        assert_eq!(tidy_tags(&"x".repeat(50))[0].len(), MAX_TAG);
    }

    #[test]
    fn slugs_and_paragraphs() {
        assert_eq!(slug("Rose Bouquet — 4×4 hoop!"), "rose-bouquet-4-4-hoop");
        assert_eq!(slug("<script>"), "script");
        assert_eq!(
            paragraphs("One\nline two\n\n<b>Two</b>"),
            "<p>One<br>line two</p><p>&lt;b&gt;Two&lt;/b&gt;</p>"
        );
    }

    #[test]
    fn publishers_come_from_the_setting() {
        assert!(!Publishers::from_setting(None).allows(1));
        assert!(Publishers::from_setting(Some("*")).allows(42));
        let some = Publishers::from_setting(Some("3, 7,x"));
        assert!(some.allows(7) && !some.allows(4));
    }

    #[test]
    fn query_words_are_encoded() {
        assert_eq!(url_word("cat & dog"), "cat%20%26%20dog");
    }
}
