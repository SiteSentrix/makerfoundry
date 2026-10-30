//! Importing from a linked Thingiverse account (issue #28), through Thingiverse's official API
//! (<https://www.thingiverse.com/developers/getting-started>): the creator signs in to Thingiverse
//! (OAuth), `/users/me` says who they are, and only the things they created (`/users/{name}/things`,
//! each thing's creator checked again) are brought in, with their files, licence, date and link,
//! into an ordinary import (`import`) for the creator to review. Off until the owner registers an
//! app and sets `MF_THINGIVERSE_CLIENT_ID` and `MF_THINGIVERSE_CLIENT_SECRET`.
//!
//! The access token lives only in this request's memory: it's never stored or logged. The fields
//! read from the API's answers are read defensively (anything missing or odd is left out) and are
//! to be confirmed against the live API when the app is registered.

use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use sqlx::PgPool;

use super::import::{Taking, regroup};
use crate::AppState;
use crate::accounts::{User, private};

/// The most things brought in at once (the import's own limits apply to their files too).
const MAX_THINGS: usize = 200;
/// The largest answer read from the API (not a file).
const MAX_ANSWER: usize = 4 * 1024 * 1024;
/// How long a sign-in may take between leaving for Thingiverse and coming back.
const STATE_MINUTES: i32 = 10;

/// The app's settings and Thingiverse's addresses (stand-ins in tests).
#[derive(Clone)]
pub struct Thingiverse {
    client_id: String,
    client_secret: String,
    www: String,
    api: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for Thingiverse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // (never the secret)
        f.write_str("Thingiverse")
    }
}

impl Thingiverse {
    /// From the settings; `None` without both of the app's settings.
    pub fn new(client_id: &str, client_secret: &str, www: Option<&str>, api: Option<&str>) -> Option<Self> {
        let (id, secret) = (client_id.trim(), client_secret.trim());
        if id.is_empty() || secret.is_empty() {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .ok()?;
        Some(Thingiverse {
            client_id: id.to_string(),
            client_secret: secret.to_string(),
            www: www
                .unwrap_or("https://www.thingiverse.com")
                .trim_end_matches('/')
                .to_string(),
            api: api
                .unwrap_or("https://api.thingiverse.com")
                .trim_end_matches('/')
                .to_string(),
            http,
        })
    }

    /// The access token for a sign-in's code (kept by the caller in memory only).
    async fn token(&self, code: &str) -> Result<String, String> {
        let answer = self
            .http
            .post(format!("{}/login/oauth/access_token", self.www))
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("code", code),
            ])
            .send()
            .await
            .map_err(|e| format!("token: {}", e.without_url()))?;
        let text = read(answer).await?;
        let text = String::from_utf8_lossy(&text);
        // `access_token=…&token_type=Bearer` (or the same as JSON)
        let token = url::form_urlencoded::parse(text.as_bytes())
            .find(|(k, _)| k == "access_token")
            .map(|(_, v)| v.into_owned())
            .or_else(|| {
                serde_json::from_str::<serde_json::Value>(&text)
                    .ok()?
                    .get("access_token")?
                    .as_str()
                    .map(str::to_string)
            })
            .filter(|t| !t.is_empty() && t.len() <= 200)
            .ok_or_else(|| "Thingiverse didn't give a sign-in".to_string())?;
        Ok(token)
    }

    async fn get(&self, token: &str, path: &str) -> Result<serde_json::Value, String> {
        let answer = self
            .http
            .get(format!("{}{path}", self.api))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("{path}: {}", e.without_url()))?;
        let bytes = read(answer).await?;
        serde_json::from_slice(&bytes).map_err(|_| format!("{path}: not JSON"))
    }

    /// A file's bytes, at most `max`.
    async fn download(&self, token: &str, url: &str, max: usize) -> Result<Vec<u8>, String> {
        // only from Thingiverse's own addresses (or the stand-in's, in tests)
        let ours =
            url.starts_with(&self.api) || url.starts_with(&self.www) || url.starts_with("https://cdn.thingiverse.com/");
        if !ours {
            return Err("a file address that isn't Thingiverse's".into());
        }
        let mut answer = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("download: {}", e.without_url()))?;
        if !answer.status().is_success() {
            return Err(format!("download: {}", answer.status()));
        }
        let mut out = Vec::new();
        while let Some(chunk) = answer
            .chunk()
            .await
            .map_err(|e| format!("download: {}", e.without_url()))?
        {
            if out.len() + chunk.len() > max {
                return Err("a file larger than 100 MB".into());
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }
}

/// An answer's body, refused past `MAX_ANSWER`.
async fn read(mut answer: reqwest::Response) -> Result<Vec<u8>, String> {
    if !answer.status().is_success() {
        return Err(format!("Thingiverse answered {}", answer.status()));
    }
    let mut out = Vec::new();
    while let Some(chunk) = answer.chunk().await.map_err(|e| e.without_url().to_string())? {
        if out.len() + chunk.len() > MAX_ANSWER {
            return Err("an answer too large".into());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// A Thingiverse licence's name → MakerFoundry's id for the same licence, when it's unambiguous.
pub fn licence(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    let n = n.trim();
    let cc = |parts: &[&str]| parts.iter().all(|p| n.contains(p));
    Some(match () {
        _ if n.contains("public domain dedication") || n == "cc0" => "cc0",
        _ if n.contains("creative commons") && cc(&["non-commercial", "no derivatives"]) => "cc-by-nc-nd",
        _ if n.contains("creative commons") && cc(&["non-commercial", "share alike"]) => "cc-by-nc-sa",
        _ if n.contains("creative commons") && cc(&["non-commercial"]) => "cc-by-nc",
        _ if n.contains("creative commons") && cc(&["no derivatives"]) => "cc-by-nd",
        _ if n.contains("creative commons") && cc(&["share alike"]) => "cc-by-sa",
        _ if n.contains("creative commons") && cc(&["attribution"]) => "cc-by",
        _ if n.contains("bsd") => "bsd-3-clause",
        _ if n.contains("all rights reserved") => "all-rights-reserved",
        _ => return None,
    })
}

/// `GET /3d/import/thingiverse`: off to Thingiverse to sign in, with a one-time state.
pub async fn start(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(tv) = state.thingiverse.clone() else {
        return crate::designs::not_found();
    };
    let (user, pool) = match super::import::importer(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let Ok(nonce) = crate::storage::new_key("oauth") else {
        return crate::designs::not_found();
    };
    let nonce = nonce.rsplit('/').next().unwrap_or(&nonce).to_string();
    if sqlx::query("INSERT INTO mf_oauth_states (state, account_id) VALUES ($1, $2)")
        .bind(&nonce)
        .bind(user.account_id)
        .execute(&pool)
        .await
        .is_err()
    {
        return crate::designs::not_found();
    }
    let back = format!("{}/3d/import/thingiverse/back", state.public_url);
    let mut to = match url::Url::parse(&format!("{}/login/oauth/authorize", tv.www)) {
        Ok(u) => u,
        Err(_) => return crate::designs::not_found(),
    };
    to.query_pairs_mut()
        .append_pair("client_id", &tv.client_id)
        .append_pair("redirect_uri", &back)
        .append_pair("response_type", "code")
        .append_pair("state", &nonce);
    Redirect::to(to.as_str()).into_response()
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Back {
    code: String,
    state: String,
}

fn text(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

/// `GET /3d/import/thingiverse/back`: Thingiverse sends the creator back with a code; their own
/// things are brought into a new import.
pub async fn back(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<Back>) -> Response {
    let Some(tv) = state.thingiverse.clone() else {
        return crate::designs::not_found();
    };
    let (user, pool) = match super::import::importer(&state, &headers).await {
        Ok(u) => u,
        Err(r) => return r,
    };
    let fail = |why: &str| {
        private(
            "Import from Thingiverse · MakerFoundry",
            &format!(
                r#"<section class="form"><h1>Import from Thingiverse</h1><p class="problem" role="alert">{}</p><p><a href="/3d/import">Back to importing</a></p></section>"#,
                crate::pages::escape(why)
            ),
            true,
        )
    };
    // the state must be one this account started, lately, and is used once
    let ok: Option<i64> = sqlx::query_scalar(&format!(
        "DELETE FROM mf_oauth_states WHERE state = $1 AND account_id = $2 AND created_at > now() - interval '{STATE_MINUTES} minutes' RETURNING account_id"
    ))
    .bind(&q.state)
    .bind(user.account_id)
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    if ok.is_none() || q.code.is_empty() || q.code.len() > 200 {
        return fail("That sign-in didn't come from here, or took too long. Please start again.");
    }
    let Some(storage) = state.storage.as_ref() else {
        return fail("Keeping models isn't switched on yet.");
    };
    match bring_in(&tv, storage, &pool, &user, &q.code).await {
        Ok(import) => Redirect::to(&format!("/3d/import/{import}")).into_response(),
        Err(why) => {
            eprintln!("thingiverse import for {}: {why}", user.account_id);
            fail("The import from Thingiverse didn't work just now. Please try again later.")
        }
    }
}

async fn bring_in(
    tv: &Thingiverse,
    storage: &crate::storage::Storage,
    pool: &PgPool,
    user: &User,
    code: &str,
) -> Result<i64, String> {
    // the token stays in this function's memory
    let token = tv.token(code).await?;
    let me = tv.get(&token, "/users/me").await?;
    let name = text(&me, "name")
        .filter(|n| {
            !n.is_empty() && n.len() <= 100 && n.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
        })
        .ok_or("no user name")?;
    let things = tv.get(&token, &format!("/users/{name}/things")).await?;
    let ids: Vec<i64> = things
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|t| t.get("id")?.as_i64())
                .take(MAX_THINGS)
                .collect()
        })
        .unwrap_or_default();
    let import: i64 = sqlx::query_scalar("INSERT INTO mf_imports (account_id) VALUES ($1) RETURNING id")
        .bind(user.account_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut taking = Taking::new(storage, pool, import);
    let today = super::origin::today();
    let result: Result<(), String> = async {
        for id in ids {
            let thing = tv.get(&token, &format!("/things/{id}")).await?;
            // only things this account created
            let creator = thing.get("creator").and_then(|c| text(c, "name"));
            if creator.as_deref() != Some(name.as_str()) {
                continue;
            }
            let title: String = text(&thing, "name")
                .unwrap_or_else(|| format!("Thing {id}"))
                .replace('/', "-")
                .chars()
                .filter(|c| !c.is_control())
                .take(100)
                .collect();
            let date = text(&thing, "added")
                .map(|d| d.chars().take(10).collect::<String>())
                .unwrap_or_default();
            let link = text(&thing, "public_url").unwrap_or_default();
            let origin = super::origin::check("thingiverse", &link, &date, &today)
                .or_else(|_| super::origin::check("thingiverse", &link, "", &today))
                .or_else(|_| super::origin::check("thingiverse", "", "", &today))
                .ok()
                .flatten();
            taking.origin = origin.map(|o| (o.url, o.date, text(&thing, "license").as_deref().and_then(licence)));
            let files = tv.get(&token, &format!("/things/{id}/files")).await?;
            for f in files.as_array().map(Vec::as_slice).unwrap_or_default() {
                let (Some(file_name), Some(url)) = (text(f, "name"), text(f, "download_url")) else {
                    continue;
                };
                let bytes = tv.download(&token, &url, super::mesh::MAX_FILE_BYTES).await?;
                let file_name = super::upload::clean_name(&file_name);
                taking.take(&format!("{title}/{file_name}"), bytes).await?;
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        taking.forget().await;
        let _ = sqlx::query("DELETE FROM mf_imports WHERE id = $1")
            .bind(import)
            .execute(pool)
            .await;
        return Err(e);
    }
    regroup(pool, import).await.map_err(|e| e.to_string())?;
    Ok(import)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thingiverse_licences_map_only_when_unambiguous() {
        for (name, want) in [
            ("Creative Commons - Attribution", Some("cc-by")),
            ("Creative Commons - Attribution - Share Alike", Some("cc-by-sa")),
            ("Creative Commons - Attribution - No Derivatives", Some("cc-by-nd")),
            ("Creative Commons - Attribution - Non-Commercial", Some("cc-by-nc")),
            (
                "Creative Commons - Attribution - Non-Commercial - Share Alike",
                Some("cc-by-nc-sa"),
            ),
            (
                "Creative Commons - Attribution - Non-Commercial - No Derivatives",
                Some("cc-by-nc-nd"),
            ),
            ("Creative Commons - Public Domain Dedication", Some("cc0")),
            ("BSD License", Some("bsd-3-clause")),
            ("All Rights Reserved", Some("all-rights-reserved")),
            ("GNU - GPL", None),
            ("Public Domain", None),
            ("", None),
        ] {
            assert_eq!(licence(name), want, "{name}");
        }
        assert!(Thingiverse::new("", "s", None, None).is_none());
        assert!(Thingiverse::new("id", " ", None, None).is_none());
        assert_eq!(
            format!("{:?}", Thingiverse::new("id", "secret", None, None).unwrap()),
            "Thingiverse"
        );
    }
}
