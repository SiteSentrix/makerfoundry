//! The StitchHub client (MF-61, issue #1): the handshake, the outbox, and applying what shops send,
//! once each and in order. Off until `MF_STITCHHUB_URL` and `MF_STITCHHUB_KEY` are set (the key is
//! a secret, never logged). The sweep calls `sync` every minute:
//!
//! 1. `GET /v2/hello` with `X-Client: makerfoundry/<version>` and the types this version
//!    understands. A version older than the hub's minimum stops all sending (and says so on the
//!    admins' overview); deprecations are logged once a day.
//! 2. The outbox: each message whose time has come is sent (`POST /v2/messages`); a network error
//!    or a 5xx is tried again later, waiting longer each time (up to an hour); a refusal is final.
//!    A message that expires before the hub took it counts as `message.expired`.
//! 3. The inbox: `GET /v2/messages?after=<cursor>`, each message checked (`contract::parse`),
//!    applied once (by id) and in order (by `seq`, per order and sender; a later one waits for the
//!    earlier), then acknowledged (`POST /v2/messages/ack`). A message that can't be read is
//!    acknowledged and logged by its id only.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;

use super::contract::{self, Envelope, Message, MessageExpired};

/// What applies a checked message (the orders' own code: MF-62 to MF-64).
pub type Apply<'a> = &'a (dyn Fn(Envelope, Message) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Sync);

/// How long a held message may wait for an earlier one before the admins are told.
const GAP_HOURS: i64 = 24;

#[derive(Clone)]
pub struct Hub {
    url: String,
    key: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // (never the key)
        f.write_str("Hub")
    }
}

/// Why a send didn't go.
#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    /// Try again later (the network, or the hub, is down).
    Later(String),
    /// The hub refused it; trying again won't help.
    Refused(String),
    /// This MakerFoundry is older than the hub takes.
    TooOld,
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
    cursor: Option<String>,
}

/// `a.b.c` as numbers, for comparing versions.
fn version(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
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
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let understood: Vec<String> = contract::UNDERSTOOD.iter().map(|(k, v)| format!("{k}/{v}")).collect();
        self.http
            .request(method, format!("{}{path}", self.url))
            .bearer_auth(&self.key)
            .header("X-Client", format!("makerfoundry/{}", crate::VERSION))
            .header("X-Understands", understood.join(","))
    }

    /// Reads at most 1 MB of an answer as JSON.
    async fn json(answer: reqwest::Response) -> Result<Value, String> {
        let bytes = answer.bytes().await.map_err(|e| format!("hub: {}", e.without_url()))?;
        if bytes.len() > 1024 * 1024 {
            return Err("hub: an answer too large".into());
        }
        serde_json::from_slice(&bytes).map_err(|_| "hub: not JSON".to_string())
    }

    /// The handshake: `Ok(true)` when this version may talk to the hub.
    pub async fn hello(&self) -> Result<(bool, Vec<Value>), String> {
        let answer = self
            .request(reqwest::Method::GET, "/v2/hello")
            .send()
            .await
            .map_err(|e| format!("hub: {}", e.without_url()))?;
        if answer.status() == reqwest::StatusCode::UPGRADE_REQUIRED {
            return Ok((false, Vec::new()));
        }
        if !answer.status().is_success() {
            return Err(format!("hub: hello {}", answer.status()));
        }
        let hello: Hello = serde_json::from_value(Self::json(answer).await?).unwrap_or_default();
        let ok = hello
            .min_client
            .get("makerfoundry")
            .is_none_or(|min| version(crate::VERSION) >= version(min));
        Ok((ok, hello.deprecated))
    }

    pub async fn send(&self, envelope: &Value) -> Result<(), SendError> {
        let answer = self
            .request(reqwest::Method::POST, "/v2/messages")
            .json(envelope)
            .send()
            .await
            .map_err(|e| SendError::Later(format!("hub: {}", e.without_url())))?;
        let status = answer.status();
        if status.is_success() || status == reqwest::StatusCode::CONFLICT {
            // (409: the hub already has it, which is what we wanted)
            Ok(())
        } else if status == reqwest::StatusCode::UPGRADE_REQUIRED {
            Err(SendError::TooOld)
        } else if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            Err(SendError::Later(format!("hub: {status}")))
        } else {
            Err(SendError::Refused(format!("hub: {status}")))
        }
    }

    async fn fetch(&self, after: Option<&str>) -> Result<Inbox, String> {
        let mut req = self
            .request(reqwest::Method::GET, "/v2/messages")
            .query(&[("limit", "50")]);
        if let Some(a) = after {
            req = req.query(&[("after", a)]);
        }
        let answer = req.send().await.map_err(|e| format!("hub: {}", e.without_url()))?;
        if !answer.status().is_success() {
            return Err(format!("hub: messages {}", answer.status()));
        }
        Ok(serde_json::from_value(Self::json(answer).await?).unwrap_or_default())
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
            .map_err(|e| format!("hub: {}", e.without_url()))?;
        if answer.status().is_success() {
            Ok(())
        } else {
            Err(format!("hub: ack {}", answer.status()))
        }
    }
}

// ---------------------------------------------------------------- the outbox

/// Puts a message to a shop in the outbox, its `seq` the next for that order and shop, in the
/// same transaction as the change it reports: it goes when the hub can take it.
pub async fn enqueue(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    message: &Message,
    order: Option<&str>,
    to: &str,
    days: i64,
) -> Result<String, String> {
    let seq: i64 = match order {
        Some(o) => sqlx::query_scalar(
            "SELECT coalesce(max(seq), 0) + 1 FROM mf_hub_outbox WHERE order_ref = $1 AND to_party = $2",
        )
        .bind(o)
        .bind(to)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| e.to_string())?,
        None => 0,
    };
    let env = contract::make(message, order, u64::try_from(seq).unwrap_or(0), to, days)?;
    let value = serde_json::to_value(&env).map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO mf_hub_outbox (id, kind, order_ref, seq, to_party, envelope, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz)",
    )
    .bind(&env.id)
    .bind(&env.kind)
    .bind(order)
    .bind(seq)
    .bind(to)
    .bind(&value)
    .bind(&env.expires_at)
    .execute(&mut **tx)
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

type Due = (String, Value, i32);

async fn drain(pool: &PgPool, hub: &Hub, apply: Apply<'_>) -> Result<bool, sqlx::Error> {
    // expired before the hub took them: undelivered, as the hub's message.expired
    let expired: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "UPDATE mf_hub_outbox SET failed = 'expired before the hub took it'
         WHERE sent_at IS NULL AND failed IS NULL AND expires_at <= now() RETURNING id, kind, order_ref",
    )
    .fetch_all(pool)
    .await?;
    for (id, kind, order) in expired {
        let msg = Message::MessageExpired(MessageExpired { id, kind, order });
        let env = Envelope {
            id: contract::new_id().unwrap_or_default(),
            kind: "message.expired".into(),
            version: 1,
            order: None,
            seq: 0,
            from: "makerfoundry".into(),
            to: "makerfoundry".into(),
            created_at: String::new(),
            expires_at: String::new(),
            body: Value::Null,
        };
        if let Err(e) = apply(env, msg).await {
            eprintln!("stitchhub: expired: {e}");
        }
    }
    let due: Vec<Due> = sqlx::query_as(
        "SELECT id, envelope, attempts FROM mf_hub_outbox
         WHERE sent_at IS NULL AND failed IS NULL AND next_try_at <= now()
         ORDER BY created_at, seq LIMIT 100",
    )
    .fetch_all(pool)
    .await?;
    for (id, envelope, attempts) in due {
        match hub.send(&envelope).await {
            Ok(()) => {
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
                // (the hub is down: the rest wait too)
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

/// Applies one checked message from a shop, once, in its order's order. → whether it was taken
/// in (applied or held), so it can be acknowledged.
async fn receive(pool: &PgPool, env: Envelope, msg: Message, apply: Apply<'_>) -> Result<(), String> {
    let seen: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM mf_hub_seen WHERE id = $1)")
        .bind(&env.id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    if seen || env.to != "makerfoundry" {
        return Ok(());
    }
    let Some(order) = env.order.clone() else {
        let id = env.id.clone();
        apply(env, msg).await?;
        return mark_seen(pool, &id).await;
    };
    let last: i64 = sqlx::query_scalar("SELECT last_seq FROM mf_hub_seq WHERE order_ref = $1 AND from_party = $2")
        .bind(&order)
        .bind(&env.from)
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
        .bind(&order)
        .bind(&env.from)
        .bind(seq)
        .bind(&value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        return Ok(());
    }
    // the next one: apply it, then any held ones that follow
    let from = env.from.clone();
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
        .bind(&order)
        .bind(&from)
        .bind(at)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        let held: Option<Value> = sqlx::query_scalar(
            "DELETE FROM mf_hub_held WHERE order_ref = $1 AND from_party = $2 AND seq = $3 RETURNING envelope",
        )
        .bind(&order)
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

/// One round with the hub: the handshake, the outbox, then the inbox (the sweep, every minute).
pub async fn sync(pool: &PgPool, hub: &Hub, apply: Apply<'_>) -> Result<(), String> {
    match hub.hello().await {
        Ok((true, deprecated)) => {
            set_state(pool, "too_old", "no").await.map_err(|e| e.to_string())?;
            if !deprecated.is_empty() {
                let day = contract::utc(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| i64::try_from(d.as_secs()).unwrap_or(0))
                        .unwrap_or(0),
                );
                let day = day.get(..10).unwrap_or("").to_string();
                if state(pool, "deprecation_logged").await.as_deref() != Some(day.as_str()) {
                    eprintln!(
                        "stitchhub: deprecated by the hub: {}",
                        serde_json::Value::Array(deprecated)
                    );
                    set_state(pool, "deprecation_logged", &day)
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        Ok((false, _)) => {
            set_state(pool, "too_old", "yes").await.map_err(|e| e.to_string())?;
            return Err("stitchhub: this MakerFoundry is older than the hub takes; update it".into());
        }
        // (offline: the outbox waits)
        Err(e) => return Err(e),
    }
    if !drain(pool, hub, apply).await.map_err(|e| e.to_string())? {
        return Err("stitchhub: this MakerFoundry is older than the hub takes; update it".into());
    }
    let mut cursor = state(pool, "cursor").await;
    for _ in 0..20 {
        let inbox = hub.fetch(cursor.as_deref()).await?;
        if inbox.messages.is_empty() {
            break;
        }
        let mut taken = Vec::new();
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
                    // (not acknowledged: it comes again next time)
                    Err(e) => eprintln!("stitchhub: {id}: {e}"),
                },
                Err(why) => {
                    // unreadable: acknowledged so it doesn't block the rest, logged by id only
                    eprintln!("stitchhub: refused {id}: {why}");
                    taken.push(id);
                }
            }
        }
        hub.ack(&taken).await?;
        match inbox.cursor {
            Some(c) => {
                set_state(pool, "cursor", &c).await.map_err(|e| e.to_string())?;
                cursor = Some(c);
            }
            None => break,
        }
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
/// A stand-in StitchHub for tests: queues per recipient, the `makerfoundry` key `mf-test`, a
/// minimum version to set, and a switch that makes it answer 503 as if it were down.
pub mod stand_in {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use axum::extract::{Query, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::{Value, json};

    #[derive(Default)]
    pub struct Hub {
        /// Messages waiting, by recipient.
        pub queues: BTreeMap<String, Vec<Value>>,
        /// Everything sent to the hub, in order.
        pub received: Vec<Value>,
        pub down: bool,
        /// Up for the handshake, but sends fail with 503.
        pub sends_down: bool,
        pub min_makerfoundry: String,
        pub deprecated: Vec<Value>,
    }

    pub type Shared = Arc<Mutex<Hub>>;

    fn lock(s: &Shared) -> std::sync::MutexGuard<'_, Hub> {
        s.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Puts a message in MakerFoundry's queue, as if a shop had sent it.
    pub fn deliver(s: &Shared, message: Value) {
        lock(s).queues.entry("makerfoundry".into()).or_default().push(message);
    }

    fn keyed(h: &HeaderMap) -> bool {
        h.get("authorization")
            .is_some_and(|v| v.as_bytes() == b"Bearer mf-test")
    }

    pub async fn start() -> std::io::Result<(String, Shared)> {
        let shared: Shared = Arc::default();
        async fn hello(State(s): State<Shared>, h: HeaderMap) -> (StatusCode, Json<Value>) {
            let hub = lock(&s);
            if hub.down {
                return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})));
            }
            if !keyed(&h) {
                return (StatusCode::UNAUTHORIZED, Json(json!({})));
            }
            let min = if hub.min_makerfoundry.is_empty() {
                "0.0.1".to_string()
            } else {
                hub.min_makerfoundry.clone()
            };
            (
                StatusCode::OK,
                Json(json!({ "hub": "2.0.0", "min_client": { "makerfoundry": min }, "deprecated": hub.deprecated })),
            )
        }
        async fn send(State(s): State<Shared>, h: HeaderMap, Json(m): Json<Value>) -> StatusCode {
            let mut hub = lock(&s);
            if hub.down {
                return StatusCode::SERVICE_UNAVAILABLE;
            }
            if !keyed(&h) {
                return StatusCode::UNAUTHORIZED;
            }
            if hub.sends_down {
                return StatusCode::SERVICE_UNAVAILABLE;
            }
            if m["to"] == "shop:refuses" {
                return StatusCode::UNPROCESSABLE_ENTITY;
            }
            if hub.received.iter().any(|r| r["id"] == m["id"]) {
                return StatusCode::CONFLICT;
            }
            hub.received.push(m.clone());
            let to = m["to"].as_str().unwrap_or("").to_string();
            hub.queues.entry(to).or_default().push(m);
            StatusCode::ACCEPTED
        }
        async fn fetch(
            State(s): State<Shared>,
            h: HeaderMap,
            Query(q): Query<BTreeMap<String, String>>,
        ) -> (StatusCode, Json<Value>) {
            let hub = lock(&s);
            if hub.down {
                return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({})));
            }
            if !keyed(&h) {
                return (StatusCode::UNAUTHORIZED, Json(json!({})));
            }
            let limit: usize = q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(50);
            let waiting: Vec<Value> = hub.queues.get("makerfoundry").cloned().unwrap_or_default();
            let page: Vec<Value> = waiting.into_iter().take(limit).collect();
            (StatusCode::OK, Json(json!({ "messages": page, "cursor": Value::Null })))
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
            if let Some(q) = hub.queues.get_mut("makerfoundry") {
                q.retain(|m| !ids.contains(&m["id"]));
            }
            StatusCode::OK
        }
        let app = Router::new()
            .route("/v2/hello", get(hello))
            .route("/v2/messages", post(send).get(fetch))
            .route("/v2/messages/ack", post(ack))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((base, shared))
    }
}
