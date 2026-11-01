//! The StitchHub client (MF-81; changed to the hub's own form for #53, as StitchHub's message of
//! 2026-11-01 sets out). Off until `MF_STITCHHUB_URL`, `MF_STITCHHUB_KEY` (a service key, `svk_…`)
//! and `MF_STITCHHUB_SIGNING_KEY` are set; the keys are secrets, never logged. The sweep calls
//! `sync` every minute:
//!
//! 1. The handshake: `GET /v2/hello` (public: the minimum client version and deprecations), then
//!    `POST /v2/hello` with the types MakerFoundry reads. Every call carries
//!    `Authorization: Bearer <service key>` and `X-Client: makerfoundry/<version>`; a version
//!    below the hub's minimum (426) stops all sending.
//! 2. The outbox: each message whose time has come is sent (`POST /v2/messages`), its files first
//!    (`POST /v2/files/put`, a PUT of the bytes with exactly the headers the hub gives, then
//!    `POST /v2/files/done`). It's stamped again if it waited long (the hub takes `created_at`
//!    only within 10 minutes of its clock) and `order.placed` is signed (`sign.rs`). A network
//!    error, a 429 or a 5xx is tried again later, waiting longer each time (up to an hour); any
//!    other refusal is final. The same message again answers `duplicate: true`; `id_reused` means
//!    an earlier try got through, which counts as sent.
//! 3. The inbox: `GET /v2/messages?after=<cursor>`, each message checked (`contract::parse`),
//!    applied once (by id) and in order (by `seq`, per conversation and sender), then acknowledged
//!    (`POST /v2/messages/ack`). The hub releases a conversation one message at a time, so after a
//!    poll that brought messages, MakerFoundry polls again straight away until one brings none. A
//!    message that can't be read is acknowledged and logged by its id only. The cursor is saved
//!    past a page only when every message in it was taken (the hub's `next` passes messages handed
//!    out and not acknowledged), so one that couldn't be applied comes again.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::contract::{self, Envelope, Message, MessageExpired};
use super::sign::Signer;

/// What applies a checked message (the orders' own code: MF-82 to MF-84).
pub type Apply<'a> = &'a (dyn Fn(Envelope, Message) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Sync);

/// How long a held message may wait for an earlier one before the admins are told.
const GAP_HOURS: i64 = 24;
/// A message waiting longer than this is stamped again before it's sent (the hub allows 10).
const RESTAMP_MINUTES: i64 = 8;
/// The most polls in one round (each brings at most 50 messages).
const MAX_POLLS: usize = 40;
/// The largest file the hub keeps (art; pictures are 2 MB).
pub const MAX_FILE_BYTES: usize = 10 * 1024 * 1024;

#[derive(Clone)]
pub struct Hub {
    url: String,
    key: String,
    http: reqwest::Client,
    signer: Option<Arc<Signer>>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // (never the keys)
        f.write_str("Hub")
    }
}

/// How a send went.
#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    /// Try again later (the network, or the hub, is down, or busy).
    Later(String),
    /// The hub refused it; trying again won't help.
    Refused(String),
    /// This MakerFoundry is older than the hub takes.
    TooOld,
}

/// A file a message names, to be in the hub before the message goes: `kind` `art` or `mockup`
/// (with its `type`), and where MakerFoundry keeps it (`key`, in its storage).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct OutFile {
    pub sha256: String,
    pub size: u64,
    pub kind: String,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    pub key: String,
}

/// A shop the hub can deliver to.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct HubShop {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub ready: bool,
}

#[derive(Deserialize, Debug, Default)]
struct Hello {
    #[serde(default)]
    min_client: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    deprecated: Vec<Value>,
}

#[derive(Deserialize, Debug, Default)]
struct Inbox {
    #[serde(default)]
    messages: Vec<Value>,
    #[serde(default)]
    next: Option<String>,
}

/// `a.b.c` as numbers, for comparing versions.
fn version(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

fn net(e: reqwest::Error) -> String {
    // (never the address: a signed link is a secret for its 15 minutes)
    format!("hub: {}", e.without_url())
}

impl Hub {
    pub fn new(url: &str, key: &str) -> Option<Self> {
        let url = url.trim().trim_end_matches('/');
        let key = key.trim();
        if key.is_empty() || !(url.starts_with("https://") || url.starts_with("http://")) {
            return None;
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;
        Some(Hub {
            url: url.to_string(),
            key: key.to_string(),
            http,
            signer: None,
        })
    }

    /// The same, signing `order.placed` with `signer`.
    #[must_use]
    pub fn with_signer(mut self, signer: Signer) -> Self {
        self.signer = Some(Arc::new(signer));
        self
    }

    /// MakerFoundry's public key, for the hub admin to register (not a secret).
    pub fn public_key(&self) -> Option<String> {
        self.signer.as_ref().map(|s| s.public_spki())
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.url))
            .bearer_auth(&self.key)
            .header("X-Client", format!("makerfoundry/{}", crate::VERSION))
    }

    /// Reads at most 1 MB of an answer as JSON.
    async fn json(answer: reqwest::Response) -> Result<Value, String> {
        let bytes = answer.bytes().await.map_err(net)?;
        if bytes.len() > 1024 * 1024 {
            return Err("hub: an answer too large".into());
        }
        serde_json::from_slice(&bytes).map_err(|_| "hub: not JSON".to_string())
    }

    /// The handshake: `Ok(true)` when this version may talk to the hub, with any deprecations.
    pub async fn hello(&self) -> Result<(bool, Vec<Value>), String> {
        let answer = self
            .request(reqwest::Method::GET, "/v2/hello")
            .send()
            .await
            .map_err(net)?;
        if !answer.status().is_success() {
            return Err(format!("hub: hello {}", answer.status()));
        }
        let hello: Hello = serde_json::from_value(Self::json(answer).await?).unwrap_or_default();
        let ok = hello
            .min_client
            .get("makerfoundry")
            .is_none_or(|min| version(crate::VERSION) >= version(min));
        if !ok {
            return Ok((false, hello.deprecated));
        }
        let types: serde_json::Map<String, Value> = contract::READS
            .iter()
            .map(|(k, v)| ((*k).to_string(), json!([v])))
            .collect();
        let answer = self
            .request(reqwest::Method::POST, "/v2/hello")
            .json(&json!({ "app": "makerfoundry", "version": crate::VERSION, "types": types }))
            .send()
            .await
            .map_err(net)?;
        if answer.status() == reqwest::StatusCode::UPGRADE_REQUIRED {
            return Ok((false, hello.deprecated));
        }
        if !answer.status().is_success() {
            return Err(format!("hub: hello {}", answer.status()));
        }
        Ok((true, hello.deprecated))
    }

    /// The shops that opted in (`GET /v2/shops`).
    pub async fn shops(&self) -> Result<Vec<HubShop>, String> {
        let answer = self
            .request(reqwest::Method::GET, "/v2/shops")
            .send()
            .await
            .map_err(net)?;
        if !answer.status().is_success() {
            return Err(format!("hub: shops {}", answer.status()));
        }
        let v = Self::json(answer).await?;
        let shops: Vec<HubShop> =
            serde_json::from_value(v.get("shops").cloned().unwrap_or(Value::Null)).unwrap_or_default();
        Ok(shops
            .into_iter()
            .filter(|s| contract::is_shop(&s.id))
            .take(10_000)
            .collect())
    }

    /// Sends one envelope. `Ok(true)` when the hub already had it (a repeat).
    pub async fn send(&self, envelope: &Value) -> Result<bool, SendError> {
        let answer = self
            .request(reqwest::Method::POST, "/v2/messages")
            .json(envelope)
            .send()
            .await
            .map_err(|e| SendError::Later(net(e)))?;
        let status = answer.status();
        if status.is_success() {
            let v = Self::json(answer).await.unwrap_or(Value::Null);
            return Ok(v.get("duplicate").and_then(Value::as_bool).unwrap_or(false));
        }
        if status == reqwest::StatusCode::UPGRADE_REQUIRED {
            return Err(SendError::TooOld);
        }
        if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SendError::Later(format!("hub: {status}")));
        }
        let code = Self::json(answer).await.map(|v| error_code(&v)).unwrap_or_default();
        if status == reqwest::StatusCode::CONFLICT && code == "id_reused" {
            // (an earlier try got through before its answer was lost: the hub has it)
            return Ok(true);
        }
        Err(SendError::Refused(format!("hub: {status} {code}")))
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<(reqwest::StatusCode, Value), SendError> {
        let answer = self
            .request(reqwest::Method::POST, path)
            .json(body)
            .send()
            .await
            .map_err(|e| SendError::Later(net(e)))?;
        let status = answer.status();
        let v = Self::json(answer).await.unwrap_or(Value::Null);
        if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SendError::Later(format!("hub: {path} {status}")));
        }
        if status == reqwest::StatusCode::UPGRADE_REQUIRED {
            return Err(SendError::TooOld);
        }
        if !status.is_success() {
            return Err(SendError::Refused(format!("hub: {path} {status} {}", error_code(&v))));
        }
        Ok((status, v))
    }

    /// Whether `url` is one the hub may hand out for a file: HTTPS, or the hub's own address.
    fn file_url_ok(&self, url: &str) -> bool {
        let own = url
            .strip_prefix(self.url.as_str())
            .is_some_and(|rest| rest.starts_with('/'));
        (own || url.starts_with("https://")) && url.len() <= 4096 && !url.chars().any(char::is_whitespace)
    }

    /// Puts one file in the hub for `conversation`, to `to`.
    pub async fn upload(&self, conversation: &str, to: &str, file: &OutFile, bytes: &[u8]) -> Result<(), SendError> {
        let mut ask = json!({
            "conversation": conversation, "sha256": file.sha256, "size": file.size, "kind": file.kind, "to": to,
        });
        if let Some(t) = &file.content_type {
            ask["type"] = json!(t);
        }
        let (_, put) = self.post_json("/v2/files/put", &ask).await?;
        if put.get("ready").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        let url = put.get("url").and_then(Value::as_str).unwrap_or("");
        if !self.file_url_ok(url) || put.get("method").and_then(Value::as_str).unwrap_or("PUT") != "PUT" {
            return Err(SendError::Refused("hub: files/put gave no usable address".into()));
        }
        // exactly the headers the hub gives, and never the service key
        let mut req = self.http.put(url).body(bytes.to_vec());
        if let Some(h) = put.get("headers").and_then(Value::as_object) {
            for (k, v) in h.iter().take(20) {
                if let (Some(v), false) = (v.as_str(), k.eq_ignore_ascii_case("authorization")) {
                    req = req.header(k.as_str(), v);
                }
            }
        }
        let answer = req.send().await.map_err(|e| SendError::Later(net(e)))?;
        if answer.status().is_server_error() {
            return Err(SendError::Later(format!("hub: file PUT {}", answer.status())));
        }
        if !answer.status().is_success() {
            return Err(SendError::Refused(format!("hub: file PUT {}", answer.status())));
        }
        self.post_json(
            "/v2/files/done",
            &json!({ "conversation": conversation, "sha256": file.sha256 }),
        )
        .await
        .map(|_| ())
    }

    /// A file a shop put in the hub for `conversation` (a proof's mock-up), at most `max` bytes,
    /// checked by size and SHA-256.
    pub async fn download(&self, conversation: &str, sha256: &str, size: u64, max: usize) -> Result<Vec<u8>, String> {
        let (_, got) = self
            .post_json(
                "/v2/files/get",
                &json!({ "conversation": conversation, "sha256": sha256 }),
            )
            .await
            .map_err(|e| match e {
                // (refused: the hub has no such file for this conversation, and won't have)
                SendError::Refused(why) => format!("the hub has no such file ({why})"),
                SendError::Later(why) => why,
                SendError::TooOld => "hub: this MakerFoundry is too old".to_string(),
            })?;
        let url = got.get("url").and_then(Value::as_str).unwrap_or("");
        if !self.file_url_ok(url) {
            return Err("hub: files/get gave no usable address".into());
        }
        // a signed address: no key goes with it
        let mut answer = self.http.get(url).send().await.map_err(net)?;
        if !answer.status().is_success() {
            return Err(format!("hub: file {}", answer.status()));
        }
        if answer.content_length().is_some_and(|n| n > max as u64) {
            return Err("the file is too large".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = answer.chunk().await.map_err(net)? {
            if bytes.len() + chunk.len() > max {
                return Err("the file is too large".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() as u64 != size || format!("{:x}", Sha256::digest(&bytes)) != sha256 {
            return Err("the file isn't the one described".into());
        }
        Ok(bytes)
    }

    async fn fetch(&self, after: Option<&str>) -> Result<Result<Inbox, ()>, String> {
        let mut req = self
            .request(reqwest::Method::GET, "/v2/messages")
            .query(&[("limit", "50")]);
        if let Some(a) = after {
            req = req.query(&[("after", a)]);
        }
        let answer = req.send().await.map_err(net)?;
        if answer.status() == reqwest::StatusCode::CONFLICT {
            // hello_first: the hub forgot the handshake
            return Ok(Err(()));
        }
        if !answer.status().is_success() {
            return Err(format!("hub: messages {}", answer.status()));
        }
        Ok(Ok(serde_json::from_value(Self::json(answer).await?).unwrap_or_default()))
    }

    async fn ack(&self, ids: &[String]) -> Result<(), String> {
        if ids.is_empty() {
            return Ok(());
        }
        let answer = self
            .request(reqwest::Method::POST, "/v2/messages/ack")
            .json(&json!({ "ids": ids }))
            .send()
            .await
            .map_err(net)?;
        if answer.status().is_success() {
            Ok(())
        } else {
            Err(format!("hub: ack {}", answer.status()))
        }
    }
}

/// The hub's error code (`{"error": {"code"}}`, or `{"error": "<code>"}`), short and plain.
fn error_code(v: &Value) -> String {
    let e = v.get("error").unwrap_or(&Value::Null);
    let code = e
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| e.as_str())
        .or_else(|| v.get("code").and_then(Value::as_str))
        .unwrap_or("");
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(40)
        .collect()
}

// ---------------------------------------------------------------- the outbox

/// Puts a message to a shop in the outbox, its `seq` the next for that conversation, in the same
/// transaction as the change it reports, with the files it names: it goes when the hub can take
/// it.
pub async fn enqueue(
    tx: &mut sqlx::PgConnection,
    message: &Message,
    conversation: &str,
    to: &str,
    days: i64,
    files: &[OutFile],
) -> Result<String, String> {
    let seq: i64 = sqlx::query_scalar(
        "SELECT coalesce(max(seq), 0) + 1 FROM mf_hub_outbox WHERE order_ref = $1 AND to_party = $2",
    )
    .bind(conversation)
    .bind(to)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    let env = contract::make(message, conversation, u64::try_from(seq).unwrap_or(1), to, days)?;
    let value = serde_json::to_value(&env).map_err(|e| e.to_string())?;
    let files = serde_json::to_value(files).map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO mf_hub_outbox (id, kind, order_ref, seq, to_party, envelope, files, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::timestamptz)",
    )
    .bind(&env.id)
    .bind(&env.kind)
    .bind(conversation)
    .bind(seq)
    .bind(to)
    .bind(&value)
    .bind(&files)
    .bind(env.expires_at.as_deref().unwrap_or(""))
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    Ok(env.id)
}

async fn set_state(pool: &PgPool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mf_hub_state (key, value) VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    .map(|_| ())
}

pub async fn state(pool: &PgPool, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM mf_hub_state WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

type Due = (String, Value, Value, i32, String, String, bool);

/// Puts a due message's files in the hub (those not there yet).
async fn upload_files(
    pool: &PgPool,
    hub: &Hub,
    storage: Option<&crate::storage::Storage>,
    conversation: &str,
    to: &str,
    files: &Value,
) -> Result<(), SendError> {
    let files: Vec<OutFile> = serde_json::from_value(files.clone()).unwrap_or_default();
    for f in files {
        let done: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM mf_hub_files WHERE conversation = $1 AND sha256 = $2)")
                .bind(conversation)
                .bind(&f.sha256)
                .fetch_one(pool)
                .await
                .map_err(|e| SendError::Later(e.to_string()))?;
        if done {
            continue;
        }
        let storage = storage.ok_or_else(|| SendError::Later("no storage for the files".into()))?;
        let bytes = match storage.get(&f.key).await {
            Ok(Some(b)) => b,
            Ok(None) => return Err(SendError::Refused("a file the message names is gone".into())),
            Err(e) => return Err(SendError::Later(e)),
        };
        if bytes.len() > MAX_FILE_BYTES
            || bytes.len() as u64 != f.size
            || format!("{:x}", Sha256::digest(&bytes)) != f.sha256
        {
            return Err(SendError::Refused("a file isn't the one the message names".into()));
        }
        hub.upload(conversation, to, &f, &bytes).await?;
        sqlx::query("INSERT INTO mf_hub_files (conversation, sha256) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(conversation)
            .bind(&f.sha256)
            .execute(pool)
            .await
            .map_err(|e| SendError::Later(e.to_string()))?;
    }
    Ok(())
}

async fn drain(
    pool: &PgPool,
    hub: &Hub,
    storage: Option<&crate::storage::Storage>,
    apply: Apply<'_>,
) -> Result<bool, sqlx::Error> {
    // expired before the hub took them: undelivered, as the hub's message.expired would say
    let expired: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
        "UPDATE mf_hub_outbox SET failed = 'expired before the hub took it'
         WHERE sent_at IS NULL AND failed IS NULL AND expires_at <= now()
         RETURNING id, kind, order_ref, to_party",
    )
    .fetch_all(pool)
    .await?;
    for (id, kind, conversation, to) in expired {
        let conversation = conversation.unwrap_or_default();
        let msg = Message::MessageExpired(MessageExpired {
            id,
            kind,
            conversation: conversation.clone(),
            to: Some(to),
            reason: "expired".into(),
        });
        let env = Envelope {
            id: contract::new_id().unwrap_or_default(),
            kind: "message.expired".into(),
            version: 1,
            conversation,
            seq: 1,
            from: Some("hub".into()),
            to: contract::me().into(),
            created_at: contract::now_utc(),
            expires_at: None,
            body: Value::Null,
            sig: None,
        };
        if let Err(e) = apply(env, msg).await {
            eprintln!("stitchhub: expired: {e}");
        }
    }
    let due: Vec<Due> = sqlx::query_as(
        "SELECT id, envelope, files, attempts, order_ref, to_party,
                coalesce(last_try_at, created_at) < now() - make_interval(mins => $1::int) AS stale
         FROM mf_hub_outbox
         WHERE sent_at IS NULL AND failed IS NULL AND next_try_at <= now()
         ORDER BY created_at, seq LIMIT 100",
    )
    .bind(i32::try_from(RESTAMP_MINUTES).unwrap_or(8))
    .fetch_all(pool)
    .await?;
    for (id, envelope, files, attempts, conversation, to, stale) in due {
        let sent = async {
            upload_files(pool, hub, storage, &conversation, &to, &files).await?;
            let mut env: Envelope =
                serde_json::from_value(envelope.clone()).map_err(|e| SendError::Refused(e.to_string()))?;
            if stale {
                env.created_at = contract::now_utc();
            }
            if env.kind == "order.placed" {
                let signer = hub
                    .signer
                    .as_ref()
                    .ok_or_else(|| SendError::Later("no signing key (MF_STITCHHUB_SIGNING_KEY)".into()))?;
                env.sig = None;
                env.sig = Some(signer.sign(&env));
            }
            let value = serde_json::to_value(&env).map_err(|e| SendError::Refused(e.to_string()))?;
            // (what was sent, so a quick retry repeats it exactly)
            let _ = sqlx::query("UPDATE mf_hub_outbox SET envelope = $2, last_try_at = now() WHERE id = $1")
                .bind(&id)
                .bind(&value)
                .execute(pool)
                .await;
            hub.send(&value).await
        };
        match sent.await {
            Ok(repeat) => {
                if repeat {
                    eprintln!("stitchhub: {id}: the hub already had it");
                }
                sqlx::query("UPDATE mf_hub_outbox SET sent_at = now(), attempts = attempts + 1 WHERE id = $1")
                    .bind(&id)
                    .execute(pool)
                    .await?;
            }
            Err(SendError::Later(why)) => {
                // 1, 2, 4… minutes, at most an hour
                let wait = 1i64 << attempts.clamp(0, 6);
                sqlx::query(
                    "UPDATE mf_hub_outbox SET attempts = attempts + 1,
                        next_try_at = now() + make_interval(mins => $2::int) WHERE id = $1",
                )
                .bind(&id)
                .bind(i32::try_from(wait.min(60)).unwrap_or(60))
                .execute(pool)
                .await?;
                eprintln!("stitchhub: {id}: {why}; trying again later");
                // (the hub is down or busy: the rest wait too)
                return Ok(true);
            }
            Err(SendError::Refused(why)) => {
                sqlx::query("UPDATE mf_hub_outbox SET failed = $2, attempts = attempts + 1 WHERE id = $1")
                    .bind(&id)
                    .bind(&why)
                    .execute(pool)
                    .await?;
                eprintln!("stitchhub: {id} refused: {why}");
            }
            Err(SendError::TooOld) => {
                set_state(pool, "too_old", "yes").await?;
                return Ok(false);
            }
        }
    }
    Ok(true)
}

// ---------------------------------------------------------------- the inbox

/// Applies one checked message from a shop or the hub, once, in its conversation's order.
async fn receive(pool: &PgPool, env: Envelope, msg: Message, apply: Apply<'_>) -> Result<(), String> {
    let seen: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM mf_hub_seen WHERE id = $1)")
        .bind(&env.id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    if seen {
        return Ok(());
    }
    let conversation = env.conversation.clone();
    let from = env.sender().to_string();
    let last: i64 = sqlx::query_scalar("SELECT last_seq FROM mf_hub_seq WHERE order_ref = $1 AND from_party = $2")
        .bind(&conversation)
        .bind(&from)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or(0);
    let seq = i64::try_from(env.seq).unwrap_or(i64::MAX);
    if seq <= last {
        // already applied one with this seq (a repeat under a new id): nothing to do
        return mark_seen(pool, &env.id).await;
    }
    if seq > last + 1 {
        // an earlier one hasn't come yet: hold this one
        let value = serde_json::to_value(&env).map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT INTO mf_hub_held (id, order_ref, from_party, seq, envelope) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT DO NOTHING",
        )
        .bind(&env.id)
        .bind(&conversation)
        .bind(&from)
        .bind(seq)
        .bind(&value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        return Ok(());
    }
    // the next one: apply it, then any held ones that follow
    let mut next = Some((env, msg));
    let mut at = seq;
    while let Some((env, msg)) = next.take() {
        let id = env.id.clone();
        apply(env, msg).await?;
        mark_seen(pool, &id).await?;
        sqlx::query(
            "INSERT INTO mf_hub_seq (order_ref, from_party, last_seq) VALUES ($1, $2, $3)
             ON CONFLICT (order_ref, from_party) DO UPDATE SET last_seq = $3",
        )
        .bind(&conversation)
        .bind(&from)
        .bind(at)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        let held: Option<Value> = sqlx::query_scalar(
            "DELETE FROM mf_hub_held WHERE order_ref = $1 AND from_party = $2 AND seq = $3 RETURNING envelope",
        )
        .bind(&conversation)
        .bind(&from)
        .bind(at + 1)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
        if let Some(v) = held {
            let bytes = serde_json::to_vec(&v).map_err(|e| e.to_string())?;
            next = Some(contract::parse(&bytes)?);
            at += 1;
        }
    }
    Ok(())
}

async fn mark_seen(pool: &PgPool, id: &str) -> Result<(), String> {
    sqlx::query("INSERT INTO mf_hub_seen (id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(id)
        .execute(pool)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

async fn handshake(pool: &PgPool, hub: &Hub) -> Result<(), String> {
    match hub.hello().await {
        Ok((true, deprecated)) => {
            set_state(pool, "too_old", "no").await.map_err(|e| e.to_string())?;
            if !deprecated.is_empty() {
                let day = contract::now_utc().get(..10).unwrap_or("").to_string();
                if state(pool, "deprecation_logged").await.as_deref() != Some(day.as_str()) {
                    eprintln!("stitchhub: deprecated by the hub: {}", Value::Array(deprecated));
                    set_state(pool, "deprecation_logged", &day)
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        }
        Ok((false, _)) => {
            set_state(pool, "too_old", "yes").await.map_err(|e| e.to_string())?;
            Err("stitchhub: this MakerFoundry is older than the hub takes; update it".into())
        }
        // (offline: the outbox waits)
        Err(e) => Err(e),
    }
}

/// One round with the hub: the handshake, the outbox, then the inbox until it's empty (the
/// sweep, every minute).
pub async fn sync(
    pool: &PgPool,
    hub: &Hub,
    storage: Option<&crate::storage::Storage>,
    apply: Apply<'_>,
) -> Result<(), String> {
    handshake(pool, hub).await?;
    if !drain(pool, hub, storage, apply).await.map_err(|e| e.to_string())? {
        return Err("stitchhub: this MakerFoundry is older than the hub takes; update it".into());
    }
    let mut cursor = state(pool, "cursor").await;
    let mut greeted_again = false;
    for _ in 0..MAX_POLLS {
        let inbox = match hub.fetch(cursor.as_deref()).await? {
            Ok(inbox) => inbox,
            Err(()) if !greeted_again => {
                greeted_again = true;
                handshake(pool, hub).await?;
                continue;
            }
            Err(()) => return Err("stitchhub: the hub keeps asking for the handshake".into()),
        };
        if inbox.messages.is_empty() {
            break;
        }
        let mut taken = Vec::new();
        let mut all_taken = true;
        for raw in inbox.messages {
            let id = raw
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(64)
                .collect::<String>();
            let bytes = serde_json::to_vec(&raw).map_err(|e| e.to_string())?;
            match contract::parse(&bytes) {
                Ok((env, msg)) => match receive(pool, env, msg, apply).await {
                    Ok(()) => taken.push(id),
                    // (not acknowledged: it comes again next time, as the cursor stays before it)
                    Err(e) => {
                        all_taken = false;
                        eprintln!("stitchhub: {id}: {e}");
                    }
                },
                Err(why) => {
                    // unreadable: acknowledged so it doesn't block the rest, logged by id only
                    eprintln!("stitchhub: refused {id}: {why}");
                    taken.push(id);
                }
            }
        }
        let stuck = taken.is_empty();
        hub.ack(&taken).await?;
        // the cursor moves past this page only when every message in it was taken: the hub's
        // `next` passes the messages it handed out, acknowledged or not, so one left behind is
        // fetched again from the cursor before it (those acknowledged meanwhile don't come again)
        if all_taken && let Some(c) = inbox.next.filter(|c| !c.is_empty() && c.len() <= 200) {
            set_state(pool, "cursor", &c).await.map_err(|e| e.to_string())?;
            cursor = Some(c);
        }
        if stuck {
            // (none could be applied now: they come again next round)
            break;
        }
        // (and poll again straight away: the hub releases a conversation one at a time)
    }
    let stale: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mf_hub_held WHERE held_at < now() - make_interval(hours => $1::int)")
            .bind(i32::try_from(GAP_HOURS).unwrap_or(24))
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    if stale > 0 {
        eprintln!("stitchhub: {stale} messages have waited over {GAP_HOURS} hours for an earlier one");
    }
    sqlx::query("DELETE FROM mf_hub_seen WHERE seen_at < now() - interval '30 days'")
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[doc(hidden)]
/// A stand-in StitchHub for tests, behaving as StitchHub's message of 2026-11-01 describes: the
/// service key `mf-test`, the POST handshake, `GET /v2/shops`, signed `order.placed` (checked
/// against a registered key), refusal of fields outside the contract, files through `/blob/…`
/// (PUT with the given header, no key), one message per conversation per poll, a minimum version
/// to set, and switches that make it answer 503.
pub mod stand_in {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::{Arc, Mutex};

    use axum::body::Bytes;
    use axum::extract::{Path, Query, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post, put};
    use axum::{Json, Router};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    use crate::shop_orders::contract::{self, Envelope};

    #[derive(Default)]
    pub struct Hub {
        pub base: String,
        /// Messages waiting, by recipient (`service:makerfoundry` for MakerFoundry's).
        pub queues: BTreeMap<String, Vec<Value>>,
        /// Each waiting message's place in the hub's delivery order, once it has been handed out
        /// (beside `queues`): `after=c<n>` skips the messages handed out at `n` and before, as
        /// the hub's cursor does, whether or not they were acknowledged.
        pub places: BTreeMap<String, Vec<Option<u64>>>,
        /// The last place given out.
        pub placed: u64,
        /// Everything MakerFoundry sent the hub, in order.
        pub received: Vec<Value>,
        pub down: bool,
        /// Up for the handshake, but sends fail with 503.
        pub sends_down: bool,
        pub min_makerfoundry: String,
        pub deprecated: Vec<Value>,
        pub said_hello: bool,
        /// The types MakerFoundry said it reads.
        pub reads: Value,
        /// What `GET /v2/shops` answers.
        pub shops: Vec<Value>,
        /// MakerFoundry's registered public key (base64 SPKI DER).
        pub public_key: Option<String>,
        /// Files, by SHA-256, and the (conversation, sha256) pairs marked done.
        pub blobs: BTreeMap<String, Vec<u8>>,
        pub done: BTreeSet<(String, String)>,
        /// Signed addresses handed out (to count downloads).
        pub downloads: usize,
    }

    pub type Shared = Arc<Mutex<Hub>>;

    fn lock(s: &Shared) -> std::sync::MutexGuard<'_, Hub> {
        s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Puts a message in MakerFoundry's queue, as if a shop (or the hub) had sent it.
    pub fn deliver(s: &Shared, message: Value) {
        lock(s).queue(contract::me(), message);
    }

    impl Hub {
        /// Puts `message` in `to`'s queue, at the next place.
        fn queue(&mut self, to: &str, message: Value) {
            self.queues.entry(to.into()).or_default().push(message);
            self.places.entry(to.into()).or_default().push(None);
        }
    }

    /// A file a shop put in the hub (a proof's mock-up) → its SHA-256.
    pub fn put_file(s: &Shared, conversation: &str, bytes: &[u8]) -> String {
        let sha = format!("{:x}", Sha256::digest(bytes));
        let mut hub = lock(s);
        hub.blobs.insert(sha.clone(), bytes.to_vec());
        hub.done.insert((conversation.to_string(), sha.clone()));
        sha
    }

    fn keyed(h: &HeaderMap) -> bool {
        h.get("authorization")
            .is_some_and(|v| v.as_bytes() == b"Bearer mf-test")
            && h.get("x-client")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("makerfoundry/"))
    }

    fn err(status: StatusCode, code: &str) -> (StatusCode, Json<Value>) {
        (status, Json(json!({ "error": { "code": code } })))
    }

    /// Seconds since 1970 of `YYYY-MM-DDTHH:MM:SS…Z`.
    fn secs(t: &str) -> Option<i64> {
        let n = |a: usize, b: usize| t.get(a..b)?.parse::<i64>().ok();
        let (y, m, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
        let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * m + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        Some(days * 86_400 + n(11, 13)? * 3600 + n(14, 16)? * 60 + n(17, 19)?)
    }

    pub async fn start() -> std::io::Result<(String, Shared)> {
        let shared: Shared = Arc::default();
        async fn hello_get(State(s): State<Shared>) -> (StatusCode, Json<Value>) {
            let hub = lock(&s);
            if hub.down {
                return err(StatusCode::SERVICE_UNAVAILABLE, "down");
            }
            let min = if hub.min_makerfoundry.is_empty() {
                "0.0.1".to_string()
            } else {
                hub.min_makerfoundry.clone()
            };
            (
                StatusCode::OK,
                Json(
                    json!({ "min_client": { "makerfoundry": min }, "limits": {}, "deprecated": hub.deprecated, "keys": [] }),
                ),
            )
        }
        async fn hello_post(State(s): State<Shared>, h: HeaderMap, Json(b): Json<Value>) -> (StatusCode, Json<Value>) {
            let mut hub = lock(&s);
            if hub.down {
                return err(StatusCode::SERVICE_UNAVAILABLE, "down");
            }
            if !keyed(&h) {
                return err(StatusCode::UNAUTHORIZED, "unauthorized");
            }
            if b["app"] != "makerfoundry" || !b["types"].is_object() {
                return err(StatusCode::BAD_REQUEST, "bad_hello");
            }
            hub.said_hello = true;
            hub.reads = b["types"].clone();
            (StatusCode::OK, Json(json!({ "ok": true })))
        }
        async fn shops(State(s): State<Shared>, h: HeaderMap) -> (StatusCode, Json<Value>) {
            let hub = lock(&s);
            if !keyed(&h) {
                return err(StatusCode::UNAUTHORIZED, "unauthorized");
            }
            (StatusCode::OK, Json(json!({ "shops": hub.shops })))
        }
        async fn send(State(s): State<Shared>, h: HeaderMap, Json(m): Json<Value>) -> (StatusCode, Json<Value>) {
            let mut hub = lock(&s);
            if hub.down || hub.sends_down {
                return err(StatusCode::SERVICE_UNAVAILABLE, "down");
            }
            if !keyed(&h) {
                return err(StatusCode::UNAUTHORIZED, "unauthorized");
            }
            if m["to"] == "shop:refuses" {
                return err(StatusCode::UNPROCESSABLE_ENTITY, "recipient_not_ready");
            }
            if let Some(old) = hub.received.iter().find(|r| r["id"] == m["id"]) {
                return if *old == m {
                    (
                        StatusCode::ACCEPTED,
                        Json(json!({ "queued": true, "duplicate": true, "state": "queued" })),
                    )
                } else {
                    err(StatusCode::CONFLICT, "id_reused")
                };
            }
            if hub
                .received
                .iter()
                .any(|r| r["conversation"] == m["conversation"] && r["to"] == m["to"] && r["seq"] == m["seq"])
            {
                return err(StatusCode::CONFLICT, "seq_taken");
            }
            let Ok(env) = serde_json::from_value::<Envelope>(m.clone()) else {
                return err(StatusCode::BAD_REQUEST, "bad_envelope");
            };
            if env.from.as_deref().is_some_and(|f| f != contract::me()) {
                return err(StatusCode::BAD_REQUEST, "bad_from");
            }
            let now = secs(&contract::now_utc()).unwrap_or(0);
            if secs(&env.created_at).is_none_or(|t| (t - now).abs() > 600) {
                return err(StatusCode::BAD_REQUEST, "clock");
            }
            if env.kind == "order.placed" {
                let signed = hub
                    .public_key
                    .as_deref()
                    .is_some_and(|k| crate::shop_orders::sign::verify(k, &env));
                if !signed {
                    return err(StatusCode::FORBIDDEN, "unsigned");
                }
                let allowed = ["fulfilment", "lines", "preview", "customer", "totals", "note"];
                if env
                    .body
                    .as_object()
                    .is_none_or(|o| o.keys().any(|k| !allowed.contains(&k.as_str())))
                {
                    return err(StatusCode::UNPROCESSABLE_ENTITY, "not_normalized");
                }
                let arts: Vec<String> = env.body["lines"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|l| l["decorations"].as_array().into_iter().flatten())
                    .filter_map(|d| d["art"]["sha256"].as_str().map(str::to_string))
                    .collect();
                if arts
                    .iter()
                    .any(|a| !hub.done.contains(&(env.conversation.clone(), a.clone())))
                {
                    return err(StatusCode::UNPROCESSABLE_ENTITY, "file_missing");
                }
                if contract::parse(&serde_json::to_vec(&m).unwrap_or_default()).is_err() {
                    return err(StatusCode::UNPROCESSABLE_ENTITY, "bad_field");
                }
            }
            hub.received.push(m.clone());
            hub.queue(&env.to, m);
            (StatusCode::ACCEPTED, Json(json!({ "queued": true, "state": "queued" })))
        }
        async fn fetch(
            State(s): State<Shared>,
            h: HeaderMap,
            Query(q): Query<BTreeMap<String, String>>,
        ) -> (StatusCode, Json<Value>) {
            let mut hub = lock(&s);
            if hub.down {
                return err(StatusCode::SERVICE_UNAVAILABLE, "down");
            }
            if !keyed(&h) {
                return err(StatusCode::UNAUTHORIZED, "unauthorized");
            }
            if !hub.said_hello {
                return err(StatusCode::CONFLICT, "hello_first");
            }
            let limit: usize = q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(50);
            let after: u64 = q
                .get("after")
                .and_then(|a| a.strip_prefix('c'))
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            let queue = hub.queues.get(contract::me()).cloned().unwrap_or_default();
            let mut places = hub.places.remove(contract::me()).unwrap_or_default();
            places.resize(queue.len(), None);
            // one message per conversation and sender at a time: the next waits for this one's
            // ack; one handed out before is handed out again only to a cursor before it
            let mut seen = BTreeSet::new();
            let mut page = Vec::new();
            let mut next = after;
            for (m, place) in queue.into_iter().zip(places.iter_mut()) {
                if page.len() >= limit || !seen.insert((m["conversation"].to_string(), m["from"].to_string())) {
                    continue;
                }
                let at = match *place {
                    Some(p) if p <= after => continue,
                    Some(p) => p,
                    None => {
                        hub.placed += 1;
                        *place = Some(hub.placed);
                        hub.placed
                    }
                };
                next = next.max(at);
                page.push(m);
            }
            hub.places.insert(contract::me().into(), places);
            let next = format!("c{next}");
            (StatusCode::OK, Json(json!({ "messages": page, "next": next })))
        }
        async fn ack(State(s): State<Shared>, h: HeaderMap, Json(b): Json<Value>) -> StatusCode {
            let mut hub = lock(&s);
            if hub.down {
                return StatusCode::SERVICE_UNAVAILABLE;
            }
            if !keyed(&h) {
                return StatusCode::UNAUTHORIZED;
            }
            let ids: Vec<Value> = b["ids"].as_array().cloned().unwrap_or_default();
            let queue = hub.queues.remove(contract::me()).unwrap_or_default();
            let places = hub.places.remove(contract::me()).unwrap_or_default();
            let kept: Vec<(Value, Option<u64>)> = queue
                .into_iter()
                .enumerate()
                .map(|(i, m)| (m, places.get(i).copied().flatten()))
                .filter(|(m, _)| !ids.contains(&m["id"]))
                .collect();
            hub.places
                .insert(contract::me().into(), kept.iter().map(|(_, p)| *p).collect());
            hub.queues
                .insert(contract::me().into(), kept.into_iter().map(|(m, _)| m).collect());
            StatusCode::OK
        }
        async fn files_put(State(s): State<Shared>, h: HeaderMap, Json(b): Json<Value>) -> (StatusCode, Json<Value>) {
            let hub = lock(&s);
            if !keyed(&h) {
                return err(StatusCode::UNAUTHORIZED, "unauthorized");
            }
            let (Some(conv), Some(sha)) = (b["conversation"].as_str(), b["sha256"].as_str()) else {
                return err(StatusCode::BAD_REQUEST, "bad_field");
            };
            let limit = if b["kind"] == "art" {
                10 * 1024 * 1024
            } else {
                2 * 1024 * 1024
            };
            if b["size"].as_u64().is_none_or(|n| n == 0 || n > limit)
                || !matches!(b["kind"].as_str(), Some("art" | "mockup"))
            {
                return err(StatusCode::BAD_REQUEST, "bad_field");
            }
            if hub.done.contains(&(conv.to_string(), sha.to_string())) {
                return (StatusCode::OK, Json(json!({ "ready": true })));
            }
            let url = format!("{}/blob/{sha}", hub.base);
            (
                StatusCode::OK,
                Json(
                    json!({ "url": url, "method": "PUT", "headers": { "x-upload-token": format!("t-{sha}") }, "expires_in": 900 }),
                ),
            )
        }
        async fn blob_put(State(s): State<Shared>, h: HeaderMap, Path(sha): Path<String>, body: Bytes) -> StatusCode {
            let mut hub = lock(&s);
            let token_ok = h
                .get("x-upload-token")
                .is_some_and(|v| v.as_bytes() == format!("t-{sha}").as_bytes());
            if h.get("authorization").is_some() || !token_ok {
                return StatusCode::FORBIDDEN;
            }
            hub.blobs.insert(sha, body.to_vec());
            StatusCode::OK
        }
        async fn files_done(State(s): State<Shared>, h: HeaderMap, Json(b): Json<Value>) -> (StatusCode, Json<Value>) {
            let mut hub = lock(&s);
            if !keyed(&h) {
                return err(StatusCode::UNAUTHORIZED, "unauthorized");
            }
            let (Some(conv), Some(sha)) = (b["conversation"].as_str(), b["sha256"].as_str()) else {
                return err(StatusCode::BAD_REQUEST, "bad_field");
            };
            match hub.blobs.get(sha) {
                Some(bytes) if format!("{:x}", Sha256::digest(bytes)) == sha => {
                    hub.done.insert((conv.to_string(), sha.to_string()));
                    (StatusCode::OK, Json(json!({ "ready": true })))
                }
                _ => err(StatusCode::CONFLICT, "not_uploaded"),
            }
        }
        async fn files_get(State(s): State<Shared>, h: HeaderMap, Json(b): Json<Value>) -> (StatusCode, Json<Value>) {
            let mut hub = lock(&s);
            if !keyed(&h) {
                return err(StatusCode::UNAUTHORIZED, "unauthorized");
            }
            let (Some(conv), Some(sha)) = (b["conversation"].as_str(), b["sha256"].as_str()) else {
                return err(StatusCode::BAD_REQUEST, "bad_field");
            };
            let size = match hub.blobs.get(sha) {
                Some(bytes) if hub.done.contains(&(conv.to_string(), sha.to_string())) => bytes.len(),
                _ => return err(StatusCode::NOT_FOUND, "no_file"),
            };
            hub.downloads += 1;
            let url = format!("{}/blob/{sha}", hub.base);
            (
                StatusCode::OK,
                Json(json!({ "url": url, "size": size, "type": "image/png", "expires_in": 600 })),
            )
        }
        async fn blob_get(State(s): State<Shared>, h: HeaderMap, Path(sha): Path<String>) -> (StatusCode, Vec<u8>) {
            let hub = lock(&s);
            match hub.blobs.get(&sha) {
                // a signed address: a key sent with it is a mistake
                Some(b) if h.get("authorization").is_none() => (StatusCode::OK, b.clone()),
                Some(_) => (StatusCode::FORBIDDEN, Vec::new()),
                None => (StatusCode::NOT_FOUND, Vec::new()),
            }
        }
        let app = Router::new()
            .route("/v2/hello", get(hello_get).post(hello_post))
            .route("/v2/shops", get(shops))
            .route("/v2/messages", post(send).get(fetch))
            .route("/v2/messages/ack", post(ack))
            .route("/v2/files/put", post(files_put))
            .route("/v2/files/done", post(files_done))
            .route("/v2/files/get", post(files_get))
            .route("/blob/{sha}", put(blob_put).get(blob_get))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        lock(&shared).base = base.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((base, shared))
    }
}
