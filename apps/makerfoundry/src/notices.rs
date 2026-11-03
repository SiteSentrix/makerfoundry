//! Email notices (issue #31): a daily digest of comments and makes on your designs, remixes of your
//! models and new followers, sent through Resend's API
//! (<https://resend.com/docs/api-reference/emails/send-email>). Off until `MF_RESEND_KEY` is set,
//! and then off for each member until they turn it on (Account › Email notices). Notices are only
//! noted for members who turned it on; a digest goes at most once a day; every email has a
//! one-click unsubscribe link that works without signing in (RFC 8058). A notice names people by
//! their maker name only, never by an email address.
//!
//! A staging deploy sets `MF_MAIL_CATCHER_URL` (a Mailpit test mailbox,
//! <https://mailpit.axllent.org/docs/api-v1/view.html#post-/api/v1/send>): notices are then on and
//! every one goes to that mailbox's send API instead of Resend, so no real person gets staging
//! mail and the end-to-end tests can read it; Resend's key is never sent there.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::PgPool;

use crate::AppState;
use crate::accounts::{current_user, forbidden, private, same_origin};
use crate::pages::escape;

/// The most notices one digest lists.
const MAX_LISTED: i64 = 100;

/// The sending key and the sender.
#[derive(Clone)]
pub struct Notices {
    key: String,
    from: String,
    api: String,
    /// A staging test mailbox's send address and its sign-in (`MF_MAIL_CATCHER_URL`), in place of
    /// Resend.
    catcher: Option<Catcher>,
    http: reqwest::Client,
}

/// Mailpit's send API, and the user name and password its address carried.
#[derive(Clone)]
struct Catcher {
    send: reqwest::Url,
    user: Option<(String, String)>,
}

/// `%XX` escapes decoded (a password may carry `@` or `:` written so); anything else as it is.
fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = b
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (c, hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What stands in for the sending key with a test mailbox and no `MF_RESEND_KEY`: it only makes
/// the unsubscribe links (a staging deploy's), and is never sent anywhere.
const CATCHER_KEY: &str = "mail-catcher";

impl std::fmt::Debug for Notices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // (never the key)
        f.write_str("Notices")
    }
}

impl Notices {
    pub fn new(key: &str, from: Option<&str>, api: Option<&str>) -> Option<Self> {
        let key = key.trim();
        if key.is_empty() {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .ok()?;
        Some(Notices {
            key: key.to_string(),
            from: from.unwrap_or("MakerFoundry <notices@makerfoundry.app>").to_string(),
            api: api
                .unwrap_or("https://api.resend.com")
                .trim_end_matches('/')
                .to_string(),
            catcher: None,
            http,
        })
    }

    /// Notices for a staging deploy: every email goes to the Mailpit test mailbox at `address`
    /// (`http://mailpit.railway.internal:8025`, or `http://user:password@host:port`), never to
    /// Resend. `key`, if set, only makes the unsubscribe links.
    pub fn to_catcher(key: &str, from: Option<&str>, address: &str) -> Result<Self, String> {
        const WHAT: &str =
            "MF_MAIL_CATCHER_URL is the test mailbox's address, like http://mailpit.railway.internal:8025";
        let mut url = reqwest::Url::parse(address.trim()).map_err(|_| WHAT)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none_or(str::is_empty)
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(WHAT.into());
        }
        let user =
            (!url.username().is_empty()).then(|| (decode(url.username()), decode(url.password().unwrap_or_default())));
        url.set_username("").map_err(|_| WHAT)?;
        url.set_password(None).map_err(|_| WHAT)?;
        let path = format!("{}/api/v1/send", url.path().trim_end_matches('/'));
        url.set_path(&path);
        let key = if key.trim().is_empty() { CATCHER_KEY } else { key };
        let mut notices = Notices::new(key, from, None).ok_or(WHAT)?;
        notices.catcher = Some(Catcher { send: url, user });
        Ok(notices)
    }

    /// Where a test mailbox's emails go (no password), when there is one.
    pub fn catcher(&self) -> Option<String> {
        self.catcher.as_ref().map(|c| c.send.to_string())
    }

    /// The token in an account's unsubscribe link: an HMAC of its id with the sending key.
    fn token(&self, account: i64) -> String {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(self.key.as_bytes()) else {
            return String::new();
        };
        mac.update(format!("unsubscribe:{account}").as_bytes());
        mac.finalize()
            .into_bytes()
            .iter()
            .take(16)
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    async fn send(&self, to: &str, subject: &str, text: &str, unsubscribe: &str) -> Result<(), String> {
        if let Some(c) = &self.catcher {
            return self.send_to_catcher(c, to, subject, text, unsubscribe).await;
        }
        let body = serde_json::json!({
            "from": self.from,
            "to": [to],
            "subject": subject,
            "text": text,
            "headers": {
                "List-Unsubscribe": format!("<{unsubscribe}>"),
                "List-Unsubscribe-Post": "List-Unsubscribe=One-Click",
            },
        });
        let answer = self
            .http
            .post(format!("{}/emails", self.api))
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("resend: {}", e.without_url()))?;
        if answer.status().is_success() {
            Ok(())
        } else {
            Err(format!("resend: {}", answer.status()))
        }
    }
}

impl Notices {
    /// `send`, to a staging test mailbox in Mailpit's shape (no Resend key goes with it).
    async fn send_to_catcher(
        &self,
        c: &Catcher,
        to: &str,
        subject: &str,
        text: &str,
        unsubscribe: &str,
    ) -> Result<(), String> {
        let (name, email) = match self.from.rsplit_once('<') {
            Some((name, rest)) => (name.trim().trim_matches('"'), rest.trim_end_matches('>').trim()),
            None => ("", self.from.trim()),
        };
        let body = serde_json::json!({
            "From": { "Email": email, "Name": name },
            "To": [{ "Email": to, "Name": "" }],
            "Subject": subject,
            "Text": text,
            "Headers": {
                "List-Unsubscribe": format!("<{unsubscribe}>"),
                "List-Unsubscribe-Post": "List-Unsubscribe=One-Click",
            },
        });
        let mut request = self.http.post(c.send.clone()).json(&body);
        if let Some((user, password)) = &c.user {
            request = request.basic_auth(user, Some(password));
        }
        let answer = request
            .send()
            .await
            .map_err(|e| format!("mail catcher: {}", e.without_url()))?;
        if answer.status().is_success() {
            Ok(())
        } else {
            Err(format!("mail catcher: {}", answer.status()))
        }
    }
}

/// Notes a notice for `recipient` about what `actor` did (`comment`, `make`, `remix`, `follow`),
/// if notices are on for the site and for them, and it isn't their own doing.
pub async fn record(state: &AppState, pool: &PgPool, recipient: i64, actor: i64, kind: &str, design: Option<i64>) {
    if state.notices.is_none() || recipient == actor {
        return;
    }
    // a remix or a follow is noted once (following again, or publishing again, isn't news)
    if let Err(e) = sqlx::query(
        "INSERT INTO mf_notices (account_id, kind, design_id, actor_account)
         SELECT $1, $2, $3, $4 WHERE EXISTS (SELECT 1 FROM mf_notice_prefs WHERE account_id = $1 AND digest)
           AND NOT ($2 IN ('remix', 'follow') AND EXISTS (SELECT 1 FROM mf_notices WHERE account_id = $1
                AND kind = $2 AND design_id IS NOT DISTINCT FROM $3 AND actor_account = $4))",
    )
    .bind(recipient)
    .bind(kind)
    .bind(design)
    .bind(actor)
    .execute(pool)
    .await
    {
        eprintln!("notices: {e}");
    }
}

/// Notes model `child`'s remix for the makers of its published parents, once it's published.
pub async fn remix(state: &AppState, pool: &PgPool, child: i64, actor: i64) {
    if state.notices.is_none() {
        return;
    }
    let makers: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT p.account_id FROM mf_remixes r JOIN mf_designs p ON p.id = r.parent_id
         JOIN mf_designs c ON c.id = r.child_id
         WHERE r.child_id = $1 AND c.status = 'published' AND p.status = 'published'",
    )
    .bind(child)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for maker in makers {
        record(state, pool, maker, actor, "remix", Some(child)).await;
    }
}

/// The owner of design `id`.
pub async fn owner(pool: &PgPool, id: i64) -> Option<i64> {
    sqlx::query_scalar("SELECT account_id FROM mf_designs WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// One line of a digest: kind, the actor's maker name, the design's id and title.
type Line = (String, Option<String>, Option<i64>, Option<String>);

/// Sends each member with waiting notices one digest, at most once a day (the sweep).
pub async fn send_digests(pool: &PgPool, notices: &Notices, public_url: &str) -> Result<(), sqlx::Error> {
    let due: Vec<(i64, String)> = sqlx::query_as(
        "SELECT p.account_id, a.email FROM mf_notice_prefs p JOIN mf_accounts a ON a.account_id = p.account_id
         WHERE p.digest AND a.email <> '' AND (p.last_sent_at IS NULL OR p.last_sent_at < now() - interval '24 hours')
           AND EXISTS (SELECT 1 FROM mf_notices n WHERE n.account_id = p.account_id AND n.sent_at IS NULL)
         LIMIT 500",
    )
    .fetch_all(pool)
    .await?;
    for (account, email) in due {
        let lines: Vec<Line> = sqlx::query_as(
            "SELECT n.kind, m.name, d.id, d.title FROM mf_notices n
             LEFT JOIN mf_makers m ON m.account_id = n.actor_account LEFT JOIN mf_designs d ON d.id = n.design_id
             WHERE n.account_id = $1 AND n.sent_at IS NULL ORDER BY n.created_at LIMIT $2",
        )
        .bind(account)
        .bind(MAX_LISTED)
        .fetch_all(pool)
        .await?;
        let text_lines: Vec<String> = lines
            .iter()
            .map(|(kind, who, id, title)| {
                let who = who.as_deref().unwrap_or("Someone");
                let what = match (id, title) {
                    (Some(id), Some(t)) => format!("{t}: {public_url}/d/{id}/{}", crate::listings::slug(t)),
                    _ => "one of your designs".into(),
                };
                match kind.as_str() {
                    "comment" => format!("- {who} commented on {what}"),
                    "make" => format!("- {who} posted a make of {what}"),
                    "remix" => format!("- {who} published a remix: {what}"),
                    _ => format!("- {who} started following you"),
                }
            })
            .collect();
        let unsubscribe = format!("{public_url}/notices/unsubscribe/{account}/{}", notices.token(account));
        let text = format!(
            "Hello,\n\nHere's what happened on MakerFoundry since your last update:\n\n{}\n\nTo stop these emails: {unsubscribe}\nTo change them: {public_url}/account/notices\n",
            text_lines.join("\n")
        );
        match notices
            .send(&email, "Your MakerFoundry update", &text, &unsubscribe)
            .await
        {
            Ok(()) => {
                sqlx::query("UPDATE mf_notices SET sent_at = now() WHERE account_id = $1 AND sent_at IS NULL")
                    .bind(account)
                    .execute(pool)
                    .await?;
                sqlx::query("UPDATE mf_notice_prefs SET last_sent_at = now() WHERE account_id = $1")
                    .bind(account)
                    .execute(pool)
                    .await?;
            }
            // (tried again at the next sweep)
            Err(e) => eprintln!("notices: digest for {account}: {e}"),
        }
    }
    // sent notices are kept 30 days (so a remix or a follow isn't noted twice), then forgotten
    sqlx::query("DELETE FROM mf_notices WHERE sent_at < now() - interval '30 days'")
        .execute(pool)
        .await?;
    Ok(())
}

async fn on_for(pool: &PgPool, account: i64) -> bool {
    sqlx::query_scalar("SELECT digest FROM mf_notice_prefs WHERE account_id = $1")
        .bind(account)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// `GET /account/notices`.
pub async fn page(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if state.notices.is_none() {
        return crate::designs::not_found();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin?next=/account/notices").into_response();
    };
    let on = on_for(pool, user.account_id).await;
    private(
        "Email notices · MakerFoundry",
        &format!(
            r#"<section class="form"><h1>Email notices</h1>
<p class="lead">A daily email when people comment on or make your designs, remix your models, or follow you. Off unless you turn it on.</p>
<form method="post" action="/account/notices">
<label class="choice"><input type="checkbox" name="digest" value="yes"{checked}> <span>Email me a daily update</span></label>
<button type="submit">Save</button>
</form>
<p><a href="/account">Your account</a></p></section>"#,
            checked = if on { " checked" } else { "" }
        ),
        true,
    )
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub struct PrefsForm {
    digest: String,
}

/// `POST /account/notices`: on or off; off forgets waiting notices.
pub async fn save(State(state): State<AppState>, headers: HeaderMap, axum::Form(f): axum::Form<PrefsForm>) -> Response {
    if !same_origin(&headers) {
        return forbidden();
    }
    if state.notices.is_none() {
        return crate::designs::not_found();
    }
    let (Some(user), Some(pool)) = (current_user(&state, &headers).await, state.db.as_ref()) else {
        return Redirect::to("/signin?next=/account/notices").into_response();
    };
    set(pool, user.account_id, f.digest == "yes").await;
    Redirect::to("/account/notices").into_response()
}

async fn set(pool: &PgPool, account: i64, on: bool) {
    let _ = sqlx::query(
        "INSERT INTO mf_notice_prefs (account_id, digest) VALUES ($1, $2)
         ON CONFLICT (account_id) DO UPDATE SET digest = $2",
    )
    .bind(account)
    .bind(on)
    .execute(pool)
    .await;
    if !on {
        let _ = sqlx::query("DELETE FROM mf_notices WHERE account_id = $1")
            .bind(account)
            .execute(pool)
            .await;
    }
}

/// `GET /notices/unsubscribe/<account>/<token>`: a page with one button (mail programs that
/// follow links don't unsubscribe anyone by looking).
pub async fn unsubscribe_page(State(state): State<AppState>, Path((account, token)): Path<(i64, String)>) -> Response {
    let Some(n) = state.notices.as_ref().filter(|n| n.token(account) == token) else {
        return crate::designs::not_found();
    };
    let _ = n;
    private(
        "Unsubscribe · MakerFoundry",
        &format!(
            r#"<section class="form"><h1>Stop the emails?</h1><form method="post" action="/notices/unsubscribe/{account}/{token}"><button type="submit">Unsubscribe</button></form></section>"#,
            token = escape(&token)
        ),
        false,
    )
}

/// `POST /notices/unsubscribe/<account>/<token>`: one click, no sign-in (RFC 8058's
/// `List-Unsubscribe-Post` sends this too).
pub async fn unsubscribe(State(state): State<AppState>, Path((account, token)): Path<(i64, String)>) -> Response {
    let (Some(n), Some(pool)) = (state.notices.as_ref(), state.db.as_ref()) else {
        return crate::designs::not_found();
    };
    if n.token(account) != token {
        return crate::designs::not_found();
    }
    set(pool, account, false).await;
    private(
        "Unsubscribed · MakerFoundry",
        r#"<section class="form"><h1>Unsubscribed</h1><p class="lead">You won't get these emails any more. You can turn them on again from your account.</p></section>"#,
        false,
    )
}

#[doc(hidden)]
/// A stand-in for Resend in tests: keeps every email sent (its JSON).
pub mod stand_in {
    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::extract::State;
    use axum::routing::post;

    pub type Sent = Arc<Mutex<Vec<serde_json::Value>>>;

    pub async fn start() -> std::io::Result<(String, Sent)> {
        let sent: Sent = Arc::default();
        async fn email(State(sent): State<Sent>, body: String) -> &'static str {
            if let Ok(v) = serde_json::from_str(&body) {
                sent.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(v);
            }
            r#"{"id": "test"}"#
        }
        // Mailpit's send API, as a staging deploy's test mailbox: kept as
        // `{"mailpit": <body>, "authorization": <header or null>}`
        async fn mailpit(State(sent): State<Sent>, headers: axum::http::HeaderMap, body: String) -> &'static str {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                let auth = headers
                    .get("authorization")
                    .and_then(|h| h.to_str().ok())
                    .map(str::to_string);
                sent.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(serde_json::json!({ "mailpit": v, "authorization": auth }));
            }
            r#"{"ID": "test"}"#
        }
        let app = Router::new()
            .route("/emails", post(email))
            .route("/api/v1/send", post(mailpit))
            .with_state(sent.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((base, sent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_mailbox_address_is_read_and_checked() {
        let n = Notices::to_catcher("", None, "http://mailpit.railway.internal:8025").unwrap();
        assert_eq!(
            n.catcher().as_deref(),
            Some("http://mailpit.railway.internal:8025/api/v1/send")
        );
        assert_eq!(n.key, CATCHER_KEY);
        let n = Notices::to_catcher("k", None, "https://qa:p%40ss@mail.example.com/").unwrap();
        assert_eq!(n.catcher().as_deref(), Some("https://mail.example.com/api/v1/send"));
        assert_eq!(
            n.catcher.as_ref().and_then(|c| c.user.clone()),
            Some(("qa".to_string(), "p@ss".to_string()))
        );
        for bad in [
            "",
            "mailpit:8025",
            "ftp://mailpit",
            "http://mailpit?x=1",
            "http://mailpit#x",
        ] {
            assert!(Notices::to_catcher("", None, bad).is_err(), "{bad}");
        }
        assert!(Notices::new("k", None, None).unwrap().catcher().is_none());
    }

    #[tokio::test]
    async fn notices_go_to_the_test_mailbox_without_resends_key() {
        let (base, sent) = stand_in::start().await.unwrap();
        let address = base.replace("http://", "http://qa:secret@");
        let n = Notices::to_catcher("re_live_key", None, &address).unwrap();
        n.send("ann@test.example", "Your update", "Hello", "https://mf.test/u/1/x")
            .await
            .unwrap();
        let sent = sent.lock().unwrap();
        let v = &sent[0];
        assert_eq!(
            v["authorization"], "Basic cWE6c2VjcmV0",
            "base64(qa:secret), not Resend's key"
        );
        let m = &v["mailpit"];
        assert_eq!(m["From"]["Email"], "notices@makerfoundry.app");
        assert_eq!(m["From"]["Name"], "MakerFoundry");
        assert_eq!(m["To"][0]["Email"], "ann@test.example");
        assert_eq!(m["Subject"], "Your update");
        assert_eq!(m["Headers"]["List-Unsubscribe"], "<https://mf.test/u/1/x>");
        assert!(!m.to_string().contains("re_live_key"));
    }
}
