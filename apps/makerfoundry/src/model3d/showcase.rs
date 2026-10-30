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
    // posts made through a platform's API that it lets us delete go now (issue #30); the rest stay
    // flagged for the admins
    if let Some(social) = &state.social {
        let posts: Vec<(i64, String, String)> = sqlx::query_as(
            "SELECT id, platform, remote_id FROM mf_showcase_posts WHERE consent_id = $1 AND taken_down_at IS NULL AND remote_id IS NOT NULL",
        )
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        for (pid, platform, remote) in posts {
            match social.delete(&platform, &remote).await {
                Ok(true) => {
                    let _ = sqlx::query("UPDATE mf_showcase_posts SET taken_down_at = now() WHERE id = $1")
                        .bind(pid)
                        .execute(pool)
                        .await;
                }
                Ok(false) => {}
                Err(e) => eprintln!("showcase: delete post {pid}: {e}"),
            }
        }
    }
    Redirect::to("/account/showcase").into_response()
}

// ---------------------------------------------------------------- the admins' queue (MF-56)

/// Where posts are made.
const PLATFORMS: &[(&str, &str)] = &[
    ("facebook", "Facebook"),
    ("instagram", "Instagram"),
    ("youtube", "YouTube"),
];

/// One item waiting to be featured: consent, make, model, its title and licence, the creator's
/// display name, a photo, a model picture, when consent was given.
type QueueRow = (
    i64,
    Option<i64>,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    String,
);

/// One post made: id, consent, platform, address, when.
type PostRow = (i64, i64, String, String, String);

/// The caption for a post: credits the creator's display name and links the model (our template;
/// the owner approves it with the Terms).
pub fn caption(title: &str, name: &str, licence: Option<&str>, url: &str, make: bool) -> String {
    let licence = licence
        .and_then(crate::licences::find)
        .map(|l| format!(" ({})", l.name))
        .unwrap_or_default();
    if make {
        format!("{name}'s print of {title} on MakerFoundry{licence}: {url}")
    } else {
        format!("{title} by {name} on MakerFoundry{licence}: {url}")
    }
}

async fn admin(state: &AppState, headers: &HeaderMap) -> Option<(crate::accounts::User, PgPool)> {
    let user = current_user(state, headers).await?;
    let pool = state.db.clone()?;
    state.admins.contains(&user.account_id).then_some((user, pool))
}

async fn queue_html(state: &AppState, pool: &PgPool, problem: &str) -> String {
    let items: Vec<QueueRow> = sqlx::query_as(
        "SELECT c.id, c.make_id, d.id, d.title, d.licence, m.name,
                (SELECT p.id FROM mf_design_photos p WHERE p.design_id = d.id ORDER BY p.position LIMIT 1),
                (SELECT f.id FROM mf_model_files f WHERE f.design_id = d.id AND f.thumb_key IS NOT NULL ORDER BY f.position LIMIT 1),
                to_char(c.given_at, 'YYYY-MM-DD')
         FROM mf_showcase_consents c
         JOIN mf_designs d ON d.id = coalesce(c.design_id, (SELECT k.design_id FROM mf_makes k WHERE k.id = c.make_id))
         LEFT JOIN mf_makers m ON m.account_id = c.account_id
         WHERE c.withdrawn_at IS NULL AND d.status = 'published' AND (c.design_id IS NOT NULL OR c.make_id IS NOT NULL)
         ORDER BY c.given_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let posts: Vec<PostRow> = sqlx::query_as(
        "SELECT id, consent_id, platform, url, to_char(posted_at, 'YYYY-MM-DD') FROM mf_showcase_posts ORDER BY posted_at",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let platform_name = |p: &str| PLATFORMS.iter().find(|(id, _)| *id == p).map_or("", |(_, n)| n);
    let options: String = PLATFORMS
        .iter()
        .map(|(id, n)| format!(r#"<option value="{id}">{n}</option>"#))
        .collect();
    let list: String = items
        .iter()
        .map(|(cid, make, did, title, licence, name, photo, file, given)| {
            let name = name.as_deref().unwrap_or("a maker");
            let url = format!("{}/d/{did}/{}", state.public_url, crate::listings::slug(title));
            let picture = match (make, photo, file) {
                (Some(m), _, _) => format!("/makes/{m}/thumb"),
                (None, Some(p), _) => format!("/designs/{did}/photos/{p}/thumb"),
                (None, None, Some(f)) => format!("/3d/{did}/files/{f}/hero"),
                _ => String::new(),
            };
            let picture = if picture.is_empty() {
                String::new()
            } else {
                format!(r#"<img src="{picture}" alt="" width="120" height="120" loading="lazy">"#)
            };
            let made: String = posts
                .iter()
                .filter(|p| p.1 == *cid)
                .map(|p| format!(r#"<li>{} {}: <a href="{u}" rel="noopener">{u}</a></li>"#, platform_name(&p.2), p.4, u = escape(&p.3)))
                .collect();
            let made = if made.is_empty() { String::new() } else { format!(r#"<ul class="posts">{made}</ul>"#) };
            format!(
                r#"<li class="showcase-item">{picture}<div><strong><a href="/d/{did}/{slug}">{title}</a></strong> <span class="muted">· {who} · consent given {given}</span>
<label>Caption <textarea readonly rows="3">{caption}</textarea></label>{made}
{post_now}<form method="post" action="/admin/showcase/{cid}/posted" class="inline-form"><select name="platform" aria-label="Where it was posted">{options}</select><input name="url" type="url" placeholder="The post's address" aria-label="The post's address" required maxlength="300"><button type="submit" class="secondary small">Posted</button></form></div></li>"#,
                slug = crate::listings::slug(title),
                title = escape(title),
                who = if make.is_some() { format!("a make by {}", escape(name)) } else { format!("by {}", escape(name)) },
                caption = escape(&caption(title, name, licence.as_deref(), &url, make.is_some())),
                post_now = state
                    .social
                    .as_ref()
                    .map(|s| {
                        s.platforms()
                            .iter()
                            .map(|p| {
                                format!(
                                    r#"<form method="post" action="/admin/showcase/{cid}/post/{p}" class="inline"><button type="submit" class="small">Post to {}</button></form> "#,
                                    platform_name(p)
                                )
                            })
                            .collect::<String>()
                    })
                    .unwrap_or_default(),
            )
        })
        .collect();
    // posts to take down: their consent withdrawn, or their model taken down
    let flagged: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT p.id, p.platform, p.url FROM mf_showcase_posts p JOIN mf_showcase_consents c ON c.id = p.consent_id
         LEFT JOIN mf_designs d ON d.id = coalesce(c.design_id, (SELECT k.design_id FROM mf_makes k WHERE k.id = c.make_id))
         WHERE p.taken_down_at IS NULL AND (c.withdrawn_at IS NOT NULL OR d.id IS NULL OR d.status <> 'published')
         ORDER BY p.posted_at",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let flagged = if flagged.is_empty() {
        String::new()
    } else {
        let rows: String = flagged
            .iter()
            .map(|(pid, platform, url)| {
                format!(
                    r#"<li>{} <a href="{u}" rel="noopener">{u}</a> <form method="post" action="/admin/showcase/posts/{pid}/taken-down" class="inline"><button type="submit" class="secondary small">Taken down</button></form></li>"#,
                    platform_name(platform),
                    u = escape(url)
                )
            })
            .collect();
        format!(
            r#"<div class="problem" role="alert"><h2>Take these down</h2><p>Their creator withdrew their consent, or the model was taken down.</p><ul>{rows}</ul></div>"#
        )
    };
    let problem = if problem.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="problem" role="alert">{}</p>"#, escape(problem))
    };
    let list = if list.is_empty() {
        r#"<p class="muted">Nothing waiting.</p>"#.to_string()
    } else {
        format!(r#"<ul class="showcase">{list}</ul>"#)
    };
    format!(
        r#"<section class="design"><h1>Showcase</h1>{problem}{flagged}<p class="lead">Published projects their creators asked us to feature, newest first. Post by hand, crediting the display name shown, then note where.</p>{list}<p><a href="/admin">The reports queue</a></p></section>"#
    )
}

/// `GET /admin/showcase`.
pub async fn queue(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some((_, pool)) = admin(&state, &headers).await else {
        return crate::designs::not_found();
    };
    private("Showcase · MakerFoundry", &queue_html(&state, &pool, "").await, true)
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub struct PostedForm {
    platform: String,
    url: String,
}

/// `POST /admin/showcase/<consent>/posted`: notes a post made from an active consent.
pub async fn posted(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(cid): Path<i64>,
    axum::Form(f): axum::Form<PostedForm>,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let Some((user, pool)) = admin(&state, &headers).await else {
        return crate::designs::not_found();
    };
    let url = f.url.trim();
    let problem = if !PLATFORMS.iter().any(|(id, _)| *id == f.platform) {
        Some("Choose where it was posted.")
    } else if !url.starts_with("https://") || url.len() > 300 || url::Url::parse(url).is_err() {
        Some("Give the post's address (https://…).")
    } else {
        None
    };
    if let Some(p) = problem {
        return private("Showcase · MakerFoundry", &queue_html(&state, &pool, p).await, true);
    }
    let _ = sqlx::query(
        "INSERT INTO mf_showcase_posts (consent_id, platform, url, posted_by)
         SELECT id, $2, $3, $4 FROM mf_showcase_consents WHERE id = $1 AND withdrawn_at IS NULL",
    )
    .bind(cid)
    .bind(&f.platform)
    .bind(url)
    .bind(user.account_id)
    .execute(&pool)
    .await;
    Redirect::to("/admin/showcase").into_response()
}

/// `POST /admin/showcase/posts/<post>/taken-down`.
pub async fn taken_down(State(state): State<AppState>, headers: HeaderMap, Path(pid): Path<i64>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let Some((_, pool)) = admin(&state, &headers).await else {
        return crate::designs::not_found();
    };
    let _ = sqlx::query("UPDATE mf_showcase_posts SET taken_down_at = now() WHERE id = $1 AND taken_down_at IS NULL")
        .bind(pid)
        .execute(&pool)
        .await;
    Redirect::to("/admin/showcase").into_response()
}

/// One item to post: make, model, its title and licence, the creator's display name, a photo, a
/// model picture.
type Item = (
    Option<i64>,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
);

/// `POST /admin/showcase/<consent>/post/<platform>`: posts an active consent's item through the
/// platform's API (issue #30) and notes it.
pub async fn post_now(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((cid, platform)): Path<(i64, String)>,
) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    let (Some((user, pool)), Some(social)) = (admin(&state, &headers).await, state.social.clone()) else {
        return crate::designs::not_found();
    };
    if !social.platforms().contains(&platform.as_str()) {
        return crate::designs::not_found();
    }
    // only an active consent for a published item: the same rule as the queue
    let item: Option<Item> = sqlx::query_as(
        "SELECT c.make_id, d.id, d.title, d.licence, m.name,
                (SELECT p.id FROM mf_design_photos p WHERE p.design_id = d.id ORDER BY p.position LIMIT 1),
                (SELECT f.id FROM mf_model_files f WHERE f.design_id = d.id AND f.thumb_key IS NOT NULL ORDER BY f.position LIMIT 1)
         FROM mf_showcase_consents c
         JOIN mf_designs d ON d.id = coalesce(c.design_id, (SELECT k.design_id FROM mf_makes k WHERE k.id = c.make_id))
         LEFT JOIN mf_makers m ON m.account_id = c.account_id
         WHERE c.id = $1 AND c.withdrawn_at IS NULL AND d.status = 'published'",
    )
    .bind(cid)
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    let Some((make, did, title, licence, name, photo, file)) = item else {
        return private(
            "Showcase · MakerFoundry",
            &queue_html(
                &state,
                &pool,
                "That item can't be posted: its consent was withdrawn, or it isn't published.",
            )
            .await,
            true,
        );
    };
    let base = &state.public_url;
    let image = match (make, photo, file) {
        (Some(m), _, _) => format!("{base}/makes/{m}/large"),
        (None, Some(p), _) => format!("{base}/designs/{did}/photos/{p}/large"),
        (None, None, Some(f)) => format!("{base}/3d/{did}/files/{f}/hero"),
        _ => {
            return private(
                "Showcase · MakerFoundry",
                &queue_html(&state, &pool, "That item has no picture to post.").await,
                true,
            );
        }
    };
    let name = name.unwrap_or_else(|| "a maker".into());
    let url = format!("{base}/d/{did}/{}", crate::listings::slug(&title));
    let text = caption(&title, &name, licence.as_deref(), &url, make.is_some());
    match social.post(&platform, &image, &text).await {
        Ok(posted) => {
            let _ = sqlx::query(
                "INSERT INTO mf_showcase_posts (consent_id, platform, url, posted_by, remote_id) VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(cid)
            .bind(&platform)
            .bind(&posted.url)
            .bind(user.account_id)
            .bind(&posted.remote_id)
            .execute(&pool)
            .await;
            Redirect::to("/admin/showcase").into_response()
        }
        Err(e) => {
            eprintln!("showcase: post {cid} to {platform}: {e}");
            private(
                "Showcase · MakerFoundry",
                &queue_html(
                    &state,
                    &pool,
                    "The post didn't go through. Please try again, or post it by hand.",
                )
                .await,
                true,
            )
        }
    }
}
