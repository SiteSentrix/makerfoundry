//! Accounts end to end: MakerFoundry's pages, its sessions in a real Postgres, and a stand-in for
//! StitchFoundry's account service on a local port (answering as `docs/cloud.md` there says).
//!
//! The database tests need `MF_TEST_DATABASE_URL` (a Postgres they may make schemas in); without
//! it they are skipped. CI sets it.

// The helpers below are test code too, but outside `#[test]` functions, where clippy.toml's
// allow-unwrap-in-tests does not reach.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use http_body_util::BodyExt;
use makerfoundry::AppState;
use makerfoundry::accounts::COOKIE;
use makerfoundry::cloud::Cloud;
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tower::ServiceExt;

const HOST: &str = "makerfoundry.test";

/// The stand-in account service's state.
#[derive(Default)]
struct Fake {
    /// email → (id, password)
    accounts: HashMap<String, (i64, String)>,
    /// token → account id
    sessions: HashMap<String, i64>,
    next: i64,
    /// Reset emails asked for.
    forgot: Vec<String>,
}

type Shared = Arc<Mutex<Fake>>;

fn error(status: StatusCode, message: &str) -> axum::response::Response {
    (
        status,
        Json(json!({"type": "error", "error": {"type": "x", "message": message}})),
    )
        .into_response()
}

fn token_of(headers: &HeaderMap) -> String {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_string()
}

fn signed_in(f: &mut Fake, id: i64, email: &str) -> Value {
    f.next += 1;
    let token = format!("sfs_test_{}", f.next);
    f.sessions.insert(token.clone(), id);
    json!({"token": token, "account": {"id": id, "email": email, "credits": 100}})
}

/// Starts the stand-in service → its address and state.
async fn fake_cloud() -> (String, Shared) {
    let shared: Shared = Arc::default();
    let app = Router::new()
        .route(
            "/v1/signup",
            post(|State(s): State<Shared>, Json(b): Json<Value>| async move {
                let email = b["email"].as_str().unwrap_or("").to_lowercase();
                let password = b["password"].as_str().unwrap_or("").to_string();
                if !email.contains('@') {
                    return error(StatusCode::BAD_REQUEST, "give a real email address");
                }
                if password.len() < 8 {
                    return error(StatusCode::BAD_REQUEST, "a password needs at least 8 characters");
                }
                let mut f = s.lock().unwrap();
                if f.accounts.contains_key(&email) {
                    return error(StatusCode::CONFLICT, "an account with that email already exists: sign in instead");
                }
                let id = 1000 + f.accounts.len() as i64;
                f.accounts.insert(email.clone(), (id, password));
                Json(signed_in(&mut f, id, &email)).into_response()
            }),
        )
        .route(
            "/v1/signin",
            post(|State(s): State<Shared>, Json(b): Json<Value>| async move {
                let email = b["email"].as_str().unwrap_or("").to_lowercase();
                let mut f = s.lock().unwrap();
                match f.accounts.get(&email).cloned() {
                    Some((id, p)) if b["password"] == p.as_str() => Json(signed_in(&mut f, id, &email)).into_response(),
                    _ => error(StatusCode::UNAUTHORIZED, "the email or the password is wrong"),
                }
            }),
        )
        .route(
            "/v1/signout",
            post(|State(s): State<Shared>, headers: HeaderMap| async move {
                s.lock().unwrap().sessions.remove(&token_of(&headers));
                Json(json!({}))
            }),
        )
        .route(
            "/v1/account",
            post(|State(s): State<Shared>, headers: HeaderMap| async move {
                let f = s.lock().unwrap();
                let Some(id) = f.sessions.get(&token_of(&headers)).copied() else {
                    return error(StatusCode::UNAUTHORIZED, "your sign-in has ended: sign in again");
                };
                let email = f.accounts.iter().find(|(_, (i, _))| *i == id).map(|(e, _)| e.clone()).unwrap();
                Json(json!({"id": id, "email": email, "credits": 100})).into_response()
            }),
        )
        .route(
            "/v1/password/forgot",
            post(|State(s): State<Shared>, Json(b): Json<Value>| async move {
                s.lock().unwrap().forgot.push(b["email"].as_str().unwrap_or("").to_string());
                Json(json!({"sent": true, "message": "If there's a Foundry Account with that email, a link to reset its password is on its way. It works for an hour."}))
            }),
        )
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), shared)
}

/// What the stand-in storage holds: key → (content type, bytes).
type Stored = Arc<Mutex<HashMap<String, (String, Vec<u8>)>>>;

/// A stand-in for R2's S3 API on a local port (path style: `/bucket/key`), checking each request
/// is signed → the storage MakerFoundry uses with it, and what it holds.
async fn fake_storage() -> (makerfoundry::storage::Storage, Stored) {
    use axum::extract::{Path, RawQuery};
    let stored: Stored = Arc::default();
    let signed = |q: &Option<String>| q.as_deref().unwrap_or("").contains("X-Amz-Signature=");
    let app = Router::new()
        .route(
            "/files/{*key}",
            axum::routing::put(
                move |State(s): State<Stored>,
                      Path(key): Path<String>,
                      RawQuery(q): RawQuery,
                      headers: HeaderMap,
                      body: axum::body::Bytes| async move {
                    if !signed(&q) {
                        return StatusCode::FORBIDDEN;
                    }
                    let ct = headers
                        .get(header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    s.lock().unwrap().insert(key, (ct, body.to_vec()));
                    StatusCode::OK
                },
            )
            .get(
                move |State(s): State<Stored>, Path(key): Path<String>, RawQuery(q): RawQuery| async move {
                    if !signed(&q) {
                        return StatusCode::FORBIDDEN.into_response();
                    }
                    match s.lock().unwrap().get(&key) {
                        Some((_, b)) => b.clone().into_response(),
                        None => StatusCode::NOT_FOUND.into_response(),
                    }
                },
            )
            .delete(
                move |State(s): State<Stored>, Path(key): Path<String>, RawQuery(q): RawQuery| async move {
                    if !signed(&q) {
                        return StatusCode::FORBIDDEN;
                    }
                    s.lock().unwrap().remove(&key);
                    StatusCode::NO_CONTENT
                },
            ),
        )
        .with_state(stored.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let storage = makerfoundry::storage::Storage::new(&format!("http://{addr}"), "files", "key", "secret").unwrap();
    (storage, stored)
}

/// A database of this test's own (a fresh schema), its tables made; `None` without
/// `MF_TEST_DATABASE_URL`.
async fn test_db() -> Option<PgPool> {
    let url = std::env::var("MF_TEST_DATABASE_URL").ok()?;
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).unwrap();
    let schema = format!("t{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
    let admin = PgPool::connect(&url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let opts: PgConnectOptions = url
        .parse::<PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .unwrap();
    makerfoundry::db::MIGRATOR.run(&pool).await.unwrap();
    Some(pool)
}

struct Site {
    app: Router,
    pool: PgPool,
    fake: Shared,
    stored: Stored,
}

async fn site() -> Option<Site> {
    site_with(Vec::new()).await
}

/// The test site, with `admins` (account ids: the stand-in service numbers accounts from 1000).
async fn site_with(admins: Vec<i64>) -> Option<Site> {
    site_full(admins, None).await
}

/// The test site, with `admins` and Stripe.
async fn site_full(admins: Vec<i64>, stripe: Option<makerfoundry::stripe::Stripe>) -> Option<Site> {
    site_opts(admins, stripe, false).await
}

/// The test site, open to everyone (`launched`) or not.
async fn site_opts(admins: Vec<i64>, stripe: Option<makerfoundry::stripe::Stripe>, launched: bool) -> Option<Site> {
    let pool = test_db().await?;
    let (storage, stored) = fake_storage().await;
    let (url, fake) = fake_cloud().await;
    let app = makerfoundry::app(AppState {
        cloud: Cloud::new(&url).unwrap(),
        db: Some(pool.clone()),
        storage: Some(storage),
        publishers: makerfoundry::listings::Publishers::Everyone,
        admins,
        limits: std::sync::Arc::default(),
        stripe,
        launched,
        public_url: "https://makerfoundry.test".into(),
        api_origins: vec!["https://stitchfoundry.test".into()],
    });
    Some(Site {
        app,
        pool,
        fake,
        stored,
    })
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl Answer {
    fn location(&self) -> &str {
        self.headers
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap())
            .unwrap_or("")
    }

    /// The session cookie this answer sets, as `name=value`.
    fn cookie(&self) -> String {
        let set = self
            .headers
            .get(header::SET_COOKIE)
            .expect("a cookie")
            .to_str()
            .unwrap();
        set.split(';').next().unwrap().to_string()
    }
}

impl Site {
    async fn send(&self, req: Request<Body>) -> Answer {
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes()).into_owned();
        Answer { status, headers, body }
    }

    async fn get(&self, path: &str, cookie: Option<&str>) -> Answer {
        let mut req = Request::get(path).header(header::HOST, HOST);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        self.send(req.body(Body::empty()).unwrap()).await
    }

    /// A form posted from our own page (its `Origin`), with `cookie` if given.
    async fn post(&self, path: &str, form: &str, cookie: Option<&str>) -> Answer {
        let mut req = Request::post(path)
            .header(header::HOST, HOST)
            .header(header::ORIGIN, format!("https://{HOST}"))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        self.send(req.body(Body::from(form.to_string())).unwrap()).await
    }

    async fn sessions(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM mf_sessions")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_person_makes_an_account_signs_out_and_back_in() {
    let Some(s) = site().await else { return };
    // the pages are there, signed out
    assert!(s.get("/signup", None).await.body.contains("Foundry Account"));
    assert_eq!(s.get("/account", None).await.location(), "/signin");
    // sign up → signed in, with a cookie only this site gets, over HTTPS, out of scripts' reach
    let r = s
        .post(
            "/signup",
            "email=Maker%40Example.com&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await;
    assert_eq!(
        (r.status, r.location()),
        (StatusCode::SEE_OTHER, "/account"),
        "{}",
        r.body
    );
    let set = r.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    for part in [COOKIE, "Path=/", "HttpOnly", "Secure", "SameSite=Lax"] {
        assert!(set.contains(part), "{set}");
    }
    let cookie = r.cookie();
    let page = s.get("/account", Some(&cookie)).await;
    assert!(
        page.body.contains("maker@example.com") && page.body.contains("Sign out"),
        "{}",
        page.body
    );
    assert_eq!(page.headers[header::CACHE_CONTROL], "no-store");
    assert!(
        s.get("/", Some(&cookie))
            .await
            .body
            .contains(r#"href="/account">Account</a>"#)
    );
    // the account is kept under the service's id; the cookie's id is kept only as its hash
    let (id, email): (i64, String) = sqlx::query_as("SELECT account_id, email FROM mf_accounts")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!((id, email.as_str()), (1000, "maker@example.com"));
    let id_in_cookie = cookie.split_once('=').unwrap().1;
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM mf_sessions WHERE id_hash = $1")
        .bind(id_in_cookie)
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(kept, 0, "never the id itself");
    // sign out → the session ends here and at the service, and the cookie goes
    let r = s.post("/signout", "", Some(&cookie)).await;
    assert_eq!(r.location(), "/");
    assert!(r.headers[header::SET_COOKIE].to_str().unwrap().contains("Max-Age=0"));
    assert_eq!(s.sessions().await, 0);
    assert!(s.fake.lock().unwrap().sessions.is_empty());
    assert_eq!(s.get("/account", Some(&cookie)).await.location(), "/signin");
    // a wrong password says so and signs nobody in; the right one does
    let r = s
        .post("/signin", "email=maker%40example.com&password=wrong+one", None)
        .await;
    assert!(r.body.contains("The email or the password is wrong.") && !r.headers.contains_key(header::SET_COOKIE));
    assert!(
        r.body.contains(r#"value="maker@example.com""#),
        "the email stays filled in"
    );
    let r = s
        .post("/signin", "email=maker%40example.com&password=fire+truck+1", None)
        .await;
    assert_eq!(r.location(), "/account");
    // signed in already: the sign-in page goes to the account
    assert_eq!(s.get("/signin", Some(&r.cookie())).await.location(), "/account");
}

#[tokio::test]
async fn sign_up_problems_are_explained() {
    let Some(s) = site().await else { return };
    let r = s
        .post(
            "/signup",
            "email=a%40b.c&password=fire+truck+1&again=fire+truck+2",
            None,
        )
        .await;
    assert!(r.body.contains("The two passwords aren&#39;t the same."));
    let r = s
        .post("/signup", "email=a%40b.c&password=short&again=short", None)
        .await;
    assert!(r.body.contains("A password needs at least 8 characters."), "{}", r.body);
    s.post(
        "/signup",
        "email=a%40b.c&password=fire+truck+1&again=fire+truck+1",
        None,
    )
    .await;
    let r = s
        .post(
            "/signup",
            "email=a%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await;
    assert!(r.body.contains("already a Foundry Account"));
    // what was typed comes back escaped
    let r = s
        .post("/signup", "email=%22%3E%3Cscript%3E&password=x&again=y", None)
        .await;
    assert!(r.body.contains("&quot;&gt;&lt;script&gt;") && !r.body.contains("\"><script>"));
}

#[tokio::test]
async fn a_session_ended_at_the_account_service_ends_here() {
    let Some(s) = site().await else { return };
    let r = s
        .post(
            "/signup",
            "email=a%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await;
    let cookie = r.cookie();
    // a password reset there ends every session of the account
    s.fake.lock().unwrap().sessions.clear();
    assert!(
        s.get("/account", Some(&cookie)).await.body.contains("a@b.c"),
        "until the next check"
    );
    sqlx::query("UPDATE mf_sessions SET checked_at = now() - interval '1 hour'")
        .execute(&s.pool)
        .await
        .unwrap();
    assert_eq!(s.get("/account", Some(&cookie)).await.location(), "/signin");
    assert_eq!(s.sessions().await, 0);
}

#[tokio::test]
async fn sessions_end_when_unused_and_carry_on_while_checks_pass() {
    let Some(s) = site().await else { return };
    let cookie = s
        .post(
            "/signup",
            "email=a%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    sqlx::query("UPDATE mf_sessions SET checked_at = now() - interval '1 hour'")
        .execute(&s.pool)
        .await
        .unwrap();
    assert!(
        s.get("/account", Some(&cookie)).await.body.contains("a@b.c"),
        "checked and still good"
    );
    let fresh: bool = sqlx::query_scalar("SELECT checked_at > now() - interval '1 minute' FROM mf_sessions")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert!(fresh);
    sqlx::query("UPDATE mf_sessions SET last_seen_at = now() - interval '31 days'")
        .execute(&s.pool)
        .await
        .unwrap();
    assert_eq!(
        s.get("/account", Some(&cookie)).await.location(),
        "/signin",
        "30 days unused"
    );
}

#[tokio::test]
async fn a_forgotten_password_goes_to_the_account_service() {
    let Some(s) = site().await else { return };
    let r = s.post("/forgot", "email=+maker%40example.com+", None).await;
    assert!(
        r.body.contains("Check your email") && r.body.contains("works for an hour"),
        "{}",
        r.body
    );
    assert_eq!(s.fake.lock().unwrap().forgot, ["maker@example.com"]);
}

#[tokio::test]
async fn forms_from_other_sites_are_refused() {
    let Some(s) = site().await else { return };
    let form = "email=a%40b.c&password=fire+truck+1&again=fire+truck+1";
    let req = |origin: Option<&str>| {
        let mut r = Request::post("/signup")
            .header(header::HOST, HOST)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(o) = origin {
            r = r.header(header::ORIGIN, o);
        }
        r.body(Body::from(form)).unwrap()
    };
    assert_eq!(
        s.send(req(Some("https://evil.example"))).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(s.send(req(None)).await.status, StatusCode::FORBIDDEN);
    assert!(
        s.fake.lock().unwrap().accounts.is_empty(),
        "nothing reached the service"
    );
}

#[tokio::test]
async fn without_the_account_service_people_are_told_to_try_later() {
    let Some(pool) = test_db().await else { return };
    let app = makerfoundry::app(AppState {
        cloud: Cloud::new("http://127.0.0.1:9").unwrap(),
        db: Some(pool),
        storage: None,
        publishers: makerfoundry::listings::Publishers::Only(Vec::new()),
        admins: Vec::new(),
        limits: std::sync::Arc::default(),
        stripe: None,
        launched: false,
        public_url: "https://makerfoundry.test".into(),
        api_origins: vec!["https://stitchfoundry.test".into()],
    });
    let s = Site {
        stored: Arc::default(),
        app,
        pool: test_db().await.unwrap(),
        fake: Arc::default(),
    };
    let r = s.post("/signin", "email=a%40b.c&password=fire+truck+1", None).await;
    assert!(r.body.contains("can&#39;t be reached right now"), "{}", r.body);
}

#[tokio::test]
async fn without_a_database_nobody_can_sign_in() {
    let app = makerfoundry::app(AppState {
        cloud: Cloud::new("http://127.0.0.1:9").unwrap(),
        db: None,
        storage: None,
        publishers: makerfoundry::listings::Publishers::Only(Vec::new()),
        admins: Vec::new(),
        limits: std::sync::Arc::default(),
        stripe: None,
        launched: false,
        public_url: "https://makerfoundry.test".into(),
        api_origins: vec!["https://stitchfoundry.test".into()],
    });
    let req = Request::get("/signin")
        .header(header::HOST, HOST)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(body.contains("isn&#39;t available yet"));
}

/// A small two-colour design as `format`, written by StitchFoundry's own writer.
fn sample(format: sf_formats::Format) -> Vec<u8> {
    use sf_core::{Cmd, Pattern, Point, Rgb, Thread};
    let mut p = Pattern::new();
    p.threads = vec![
        Thread {
            color: Rgb { r: 200, g: 30, b: 40 },
            name: "Rose <red>".into(),
            code: String::new(),
            chart: String::new(),
        },
        Thread {
            color: Rgb { r: 20, g: 60, b: 200 },
            name: String::new(),
            code: String::new(),
            chart: String::new(),
        },
    ];
    for i in 0..=30 {
        p.add(Point::new(f64::from(i), f64::from(i % 2) * 4.0), Cmd::Stitch);
    }
    p.add(Point::new(30.0, 0.0), Cmd::ColorChange);
    for i in 0..=30 {
        p.add(Point::new(f64::from(i), 10.0 + f64::from(i % 2) * 4.0), Cmd::Stitch);
    }
    p.add(Point::new(30.0, 14.0), Cmd::End);
    sf_formats::write(format, &p, &sf_formats::WriteOptions::default()).unwrap()
}

/// A multipart upload of `bytes` as the form's `file`, named `name`.
fn upload(name: &str, bytes: &[u8], cookie: Option<&str>, origin: &str) -> Request<Body> {
    let boundary = "----mf-test-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let mut req = Request::post("/preview")
        .header(header::HOST, HOST)
        .header(header::ORIGIN, origin)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        );
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    req.body(Body::from(body)).unwrap()
}

#[tokio::test]
async fn an_uploaded_pes_shows_its_stitches() {
    use sf_formats::Format::{Pes, Vp3};
    let Some(s) = site().await else { return };
    let ours = format!("https://{HOST}");
    // signed out: asked to sign in, and nothing is read
    assert!(s.get("/preview", None).await.body.contains("Sign in to upload"));
    assert!(
        s.send(upload("rose.pes", &sample(Pes), None, &ours))
            .await
            .body
            .contains("Sign in to upload")
    );
    let cookie = s
        .post(
            "/signup",
            "email=a%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    assert!(
        s.get("/preview", Some(&cookie))
            .await
            .body
            .contains(r#"enctype="multipart/form-data""#)
    );
    // the done-when of MF-2: an uploaded PES shows its stitches
    let r = s.send(upload("rose.pes", &sample(Pes), Some(&cookie), &ours)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        r.body.contains(r#"<svg class="stitches""#) && r.body.matches("<path stroke=").count() == 2,
        "{}",
        r.body
    );
    assert!(
        r.body.contains("<dd>PES</dd>") && r.body.contains("30.0 × 14.0 mm"),
        "{}",
        r.body
    );
    assert!(
        r.body.contains("Red 5 (#ed171f)"),
        "PES colours are Brother's thread chart: {}",
        r.body
    );
    // VP3 keeps the file's own thread names, which come back escaped
    let r = s.send(upload("rose.vp3", &sample(Vp3), Some(&cookie), &ours)).await;
    assert!(
        r.body.contains("Rose &lt;red&gt; (#c81e28)") && !r.body.contains("<red>"),
        "{}",
        r.body
    );
    // not a design, or too big: a reason, no preview
    let r = s
        .send(upload(
            "evil.pes",
            b"MZ\x90\x00 this is not a design <b>",
            Some(&cookie),
            &ours,
        ))
        .await;
    assert!(
        r.body.contains("isn&#39;t an embroidery file") && !r.body.contains("<b>"),
        "{}",
        r.body
    );
    let big = vec![0u8; makerfoundry::laser::MAX_FILE_BYTES + 200 * 1024];
    let r = s.send(upload("big.dst", &big, Some(&cookie), &ours)).await;
    assert!(
        r.status == StatusCode::PAYLOAD_TOO_LARGE || r.body.contains("up to 8 MB"),
        "{}: {}",
        r.status,
        r.body
    );
    // from another site: refused
    assert_eq!(
        s.send(upload("rose.pes", &sample(Pes), Some(&cookie), "https://evil.example"))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn uploaded_laser_files_show_their_lines() {
    let Some(s) = site().await else { return };
    let ours = format!("https://{HOST}");
    let cookie = s
        .post(
            "/signup",
            "email=a%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    // an SVG in mm with a script in it: drawn from its shapes, the script nowhere
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="60mm" height="40mm" viewBox="0 0 60 40" onload="alert(1)"><script>alert(2)</script><rect x="5" y="5" width="50" height="30" fill="none" stroke="#ff0000"/><text x="10" y="20">Name</text></svg>"##;
    let r = s.send(upload("coaster.svg", svg, Some(&cookie), &ours)).await;
    assert!(
        r.body.contains("<dd>SVG</dd>") && r.body.contains("50.0 × 30.0 mm") && r.body.contains("160 mm"),
        "{}",
        r.body
    );
    assert!(r.body.contains("Not drawn: text"), "{}", r.body);
    assert!(!r.body.contains("alert"), "{}", r.body);
    // a DXF in inches
    let dxf = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n1\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nCIRCLE\n62\n1\n10\n0\n20\n0\n40\n1\n0\nENDSEC\n0\nEOF\n";
    let r = s.send(upload("disc.dxf", dxf.as_bytes(), Some(&cookie), &ours)).await;
    assert!(
        r.body.contains("<dd>DXF</dd>") && r.body.contains("50.8 × 50.8 mm"),
        "{}",
        r.body
    );
}

#[tokio::test]
async fn an_uploaded_pdf_is_read_as_a_laser_file() {
    let Some(s) = site().await else { return };
    let ours = format!("https://{HOST}");
    let cookie = s
        .post(
            "/signup",
            "email=a%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    let r = s
        .send(upload("broken.pdf", b"%PDF-1.7\nnot a real pdf", Some(&cookie), &ours))
        .await;
    assert!(r.body.contains("PDF couldn&#39;t be read"), "{}", r.body);
}

/// A multipart form of `(field, file name, bytes)` parts, posted to `path`.
fn multipart(path: &str, parts: &[(&str, &str, Vec<u8>)], cookie: &str) -> Request<Body> {
    let boundary = "----mf-test-parts";
    let mut body = Vec::new();
    for (field, name, bytes) in parts {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Request::post(path)
        .header(header::HOST, HOST)
        .header(header::ORIGIN, format!("https://{HOST}"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .header(header::COOKIE, cookie)
        .body(Body::from(body))
        .unwrap()
}

/// A small JPEG with an EXIF block naming a place, as phones write them.
fn photo_with_location() -> Vec<u8> {
    let picture = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(800, 600, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
    }));
    let mut out = std::io::Cursor::new(Vec::new());
    picture.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    let mut bytes = out.into_inner();
    bytes.splice(2..2, b"\xff\xe1\x00\x18Exif\x00\x00GPSLatitude=41.8".iter().copied());
    bytes
}

#[tokio::test]
async fn a_maker_keeps_a_design_with_photos() {
    let Some(s) = site().await else { return };
    let cookie = s
        .post(
            "/signup",
            "email=maker%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    assert!(
        s.get("/designs", Some(&cookie))
            .await
            .body
            .contains("haven't added a design")
    );
    assert!(
        s.get("/designs/new", Some(&cookie))
            .await
            .body
            .contains(r#"name="photos""#)
    );
    // a design and two photos
    let parts = [
        ("file", "rose.pes", sample(sf_formats::Format::Pes)),
        ("photos", "sewn.jpg", photo_with_location()),
        ("photos", "close-up.jpg", photo_with_location()),
    ];
    let r = s.send(multipart("/designs/new", &parts, &cookie)).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let page_path = r.location().to_string();
    assert!(page_path.starts_with("/designs/"), "{page_path}");
    // the design file, two photos and two thumbnails are stored under keys of our own
    {
        let stored = s.stored.lock().unwrap();
        assert_eq!(stored.len(), 5, "{:?}", stored.keys().collect::<Vec<_>>());
        assert!(
            stored.keys().all(|k| !k.contains("rose") && !k.contains("sewn")),
            "no uploader names in keys"
        );
        for (key, (ct, bytes)) in stored.iter() {
            if key.starts_with("photo/") || key.starts_with("thumb/") {
                assert_eq!(ct, "image/jpeg");
                assert!(
                    !bytes.windows(11).any(|w| w == b"GPSLatitude"),
                    "where it was taken is gone"
                );
            }
        }
    }
    // its page: the stitches, the facts, the photos; and it's in the list
    let page = s.get(&page_path, Some(&cookie)).await;
    assert!(
        page.body.contains(r#"<svg class="stitches""#) && page.body.contains("<dd>PES</dd>"),
        "{}",
        page.body
    );
    assert_eq!(page.body.matches("/thumb\"").count(), 2, "{}", page.body);
    assert!(s.get("/designs", Some(&cookie)).await.body.contains("rose.pes"));
    // a photo comes back as a JPEG; the download goes to a signed link that names the file
    let thumb_path = page
        .body
        .split("src=\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .unwrap()
        .to_string();
    let thumb = s.get(&thumb_path, Some(&cookie)).await;
    assert_eq!(thumb.headers[header::CONTENT_TYPE], "image/jpeg");
    let dl = s.get(&format!("{page_path}/download"), Some(&cookie)).await;
    assert_eq!(dl.status, StatusCode::SEE_OTHER);
    assert!(
        dl.location().contains("X-Amz-Signature=") && dl.location().contains("rose.pes"),
        "{}",
        dl.location()
    );
    // another maker sees none of it
    let other = s
        .post(
            "/signup",
            "email=other%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    assert_eq!(s.get(&page_path, Some(&other)).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.get(&thumb_path, Some(&other)).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        s.get(&format!("{page_path}/download"), Some(&other)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        s.post(&format!("{page_path}/delete"), "", Some(&other)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        s.get(&page_path, None).await.location(),
        format!("/signin?next={page_path}")
    );
    // deleting it takes its files too
    let r = s.post(&format!("{page_path}/delete"), "", Some(&cookie)).await;
    assert_eq!(r.location(), "/designs");
    assert!(s.stored.lock().unwrap().is_empty());
    assert_eq!(s.get(&page_path, Some(&cookie)).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bad_photos_and_too_many_are_refused_and_nothing_is_kept() {
    let Some(s) = site().await else { return };
    let cookie = s
        .post(
            "/signup",
            "email=maker%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    let design = ("file", "rose.pes", sample(sf_formats::Format::Pes));
    let r = s
        .send(multipart(
            "/designs/new",
            &[design.clone(), ("photos", "x.jpg", b"<svg onload=alert(1)>".to_vec())],
            &cookie,
        ))
        .await;
    assert!(
        r.body.contains("Photo 1: this isn&#39;t a JPEG, PNG or WebP photo."),
        "{}",
        r.body
    );
    let mut parts = vec![design.clone()];
    for i in 0..=makerfoundry::designs::MAX_PHOTOS {
        parts.push(("photos", if i == 0 { "a.jpg" } else { "b.jpg" }, photo_with_location()));
    }
    let r = s.send(multipart("/designs/new", &parts, &cookie)).await;
    assert!(r.body.contains("up to 6 photos"), "{}", r.body);
    let r = s
        .send(multipart(
            "/designs/new",
            &[("file", "notes.txt", b"hello".to_vec())],
            &cookie,
        ))
        .await;
    assert!(
        r.body.contains("notes.txt: this isn&#39;t an embroidery file"),
        "{}",
        r.body
    );
    assert!(s.stored.lock().unwrap().is_empty());
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM mf_designs")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn without_storage_designs_can_only_be_previewed() {
    let Some(pool) = test_db().await else { return };
    let (url, _fake) = fake_cloud().await;
    let app = makerfoundry::app(AppState {
        cloud: Cloud::new(&url).unwrap(),
        db: Some(pool.clone()),
        storage: None,
        publishers: makerfoundry::listings::Publishers::Only(Vec::new()),
        admins: Vec::new(),
        limits: std::sync::Arc::default(),
        stripe: None,
        launched: false,
        public_url: "https://makerfoundry.test".into(),
        api_origins: vec!["https://stitchfoundry.test".into()],
    });
    let s = Site {
        app,
        pool,
        fake: Arc::default(),
        stored: Arc::default(),
    };
    let cookie = s
        .post(
            "/signup",
            "email=a%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    let r = s.get("/designs/new", Some(&cookie)).await;
    assert!(
        r.body.contains("isn't switched on yet") && r.body.contains("/preview"),
        "{}",
        r.body
    );
}

/// The id in a `/designs/<id>` address.
fn design_id(path: &str) -> i64 {
    path.trim_start_matches("/designs/").parse().unwrap()
}

/// Adds a design as the maker signed in with `cookie` → its id.
async fn add_design(s: &Site, cookie: &str, format: sf_formats::Format, name: &str) -> i64 {
    let parts = [
        ("file", name, sample(format)),
        ("photos", "sewn.jpg", photo_with_location()),
    ];
    let r = s.send(multipart("/designs/new", &parts, cookie)).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    design_id(r.location())
}

#[tokio::test]
async fn a_free_design_is_published_found_and_downloaded() {
    let Some(s) = site().await else { return };
    let cookie = s
        .post(
            "/signup",
            "email=ann%40example.com&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    let id = add_design(&s, &cookie, sf_formats::Format::Pes, "rose.pes").await;
    let manage = format!("/designs/{id}");
    // publishing needs a profile, details, and the maker's word that it's theirs to share
    let r = s.post(&format!("{manage}/publish"), "confirm=yes", Some(&cookie)).await;
    assert!(r.body.contains("maker profile first"), "{}", r.body);
    let r = s
        .post(
            "/account/maker",
            "name=Ann%27s+Stitches&handle=Ann+S&bio=",
            Some(&cookie),
        )
        .await;
    assert!(r.body.contains("lower-case"), "{}", r.body);
    let r = s
        .post(
            "/account/maker",
            "name=Ann%27s+Stitches&handle=ann-s&bio=Hoops+%26+more",
            Some(&cookie),
        )
        .await;
    assert_eq!(r.location(), "/account");
    let r = s.post(&format!("{manage}/publish"), "confirm=yes", Some(&cookie)).await;
    assert!(r.body.contains("title and a licence first"), "{}", r.body);
    let r = s.post(&format!("{manage}/edit"), "title=Rose+Bouquet&description=Roses+for+a+4x4+hoop.%0A%0AStitch+on+cotton.&tags=Flowers%2C+Roses%2C+flowers&licence=nope", Some(&cookie)).await;
    assert!(r.body.contains("Choose a licence."), "{}", r.body);
    let r = s.post(&format!("{manage}/edit"), "title=Rose+Bouquet&description=Roses+for+a+4x4+hoop.%0A%0AStitch+on+cotton.&tags=Flowers%2C+Roses%2C+flowers&licence=personal", Some(&cookie)).await;
    assert_eq!(r.location(), manage);
    // before publishing, nobody else can see it
    assert_eq!(s.get(&format!("/d/{id}"), None).await.status, StatusCode::NOT_FOUND);
    let r = s.post(&format!("{manage}/publish"), "", Some(&cookie)).await;
    assert!(r.body.contains("yours to share"), "{}", r.body);
    let r = s.post(&format!("{manage}/publish"), "confirm=yes", Some(&cookie)).await;
    assert_eq!(r.location(), format!("/d/{id}/rose-bouquet"));
    // its public page, for anyone: title, maker, description, tags, licence, photo; never an email
    let page = s.get(&format!("/d/{id}/rose-bouquet"), None).await;
    assert_eq!(page.status, StatusCode::OK);
    for want in [
        "<h1>Rose Bouquet</h1>",
        r#"href="/makers/ann-s">Ann&#39;s Stitches</a>"#,
        "<p>Roses for a 4x4 hoop.</p>",
        "/browse?tag=flowers",
        "Personal use",
        "Don&#39;t share",
        "/thumb\"",
        "Download free",
    ] {
        assert!(page.body.contains(want), "{want}: {}", page.body);
    }
    assert!(!page.body.contains("ann@example.com"));
    // found by words (stemmed: "rose" finds "Roses"), by tag, by kind; not by the wrong kind
    for query in [
        "/browse",
        "/browse?q=rose",
        "/browse?q=bouquet+cotton",
        "/browse?tag=roses",
        "/browse?kind=embroidery",
        "/browse?sort=popular",
    ] {
        assert!(s.get(query, None).await.body.contains("Rose Bouquet"), "{query}");
    }
    for query in ["/browse?q=tulip", "/browse?kind=laser", "/browse?tag=tulips"] {
        assert!(!s.get(query, None).await.body.contains("Rose Bouquet"), "{query}");
    }
    // on its maker's page and the home page
    let maker = s.get("/makers/ann-s", None).await;
    assert!(
        maker.body.contains("Ann&#39;s Stitches")
            && maker.body.contains("Hoops &amp; more")
            && maker.body.contains("Rose Bouquet")
    );
    assert!(s.get("/", None).await.body.contains("Newest designs"));
    // downloaded by anyone, counted, saved under its title
    let dl = s.get(&format!("/d/{id}/download"), None).await;
    assert_eq!(dl.status, StatusCode::SEE_OTHER);
    assert!(
        dl.location().contains("rose-bouquet.pes") && dl.location().contains("X-Amz-Signature="),
        "{}",
        dl.location()
    );
    assert!(s.get(&format!("/d/{id}"), None).await.body.contains("1 download"));
    // a published design's photos are public; the maker's own download isn't counted
    let thumb = page
        .body
        .split("src=\"")
        .find(|p| p.starts_with("/designs/"))
        .and_then(|p| p.split('"').next())
        .unwrap()
        .to_string();
    assert_eq!(
        s.get(&thumb, None).await.headers[header::CACHE_CONTROL],
        "public, max-age=3600"
    );
    s.get(&format!("{manage}/download"), Some(&cookie)).await;
    let n: i64 = sqlx::query_scalar("SELECT downloads FROM mf_designs WHERE id = $1")
        .bind(id)
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    // a published design isn't deleted by mistake; unpublished, it's hidden again
    assert!(
        s.post(&format!("{manage}/delete"), "", Some(&cookie))
            .await
            .body
            .contains("Unpublish the design before deleting it.")
    );
    s.post(&format!("{manage}/unpublish"), "", Some(&cookie)).await;
    assert_eq!(s.get(&format!("/d/{id}"), None).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.get(&thumb, None).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        s.get(&format!("/d/{id}/download"), None).await.status,
        StatusCode::NOT_FOUND
    );
    assert!(!s.get("/browse", None).await.body.contains("Rose Bouquet"));
    // the maker still sees it, marked a draft
    assert!(
        s.get(&format!("/d/{id}"), Some(&cookie))
            .await
            .body
            .contains("only you can see this page")
    );
}

#[tokio::test]
async fn others_cant_change_a_design_or_take_a_maker_address() {
    let Some(s) = site().await else { return };
    let ann = s
        .post(
            "/signup",
            "email=ann%40example.com&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    let bob = s
        .post(
            "/signup",
            "email=bob%40example.com&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    let id = add_design(&s, &ann, sf_formats::Format::Dst, "star.dst").await;
    s.post("/account/maker", "name=Ann&handle=ann-s", Some(&ann)).await;
    let r = s.post("/account/maker", "name=Bob&handle=ann-s", Some(&bob)).await;
    assert!(r.body.contains("That address is taken"), "{}", r.body);
    for (path, form) in [
        ("edit", "title=Mine&licence=cc0"),
        ("publish", "confirm=yes"),
        ("unpublish", ""),
    ] {
        let r = s.post(&format!("/designs/{id}/{path}"), form, Some(&bob)).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{path}");
    }
    let title: String = sqlx::query_scalar("SELECT title FROM mf_designs WHERE id = $1")
        .bind(id)
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(title, "");
}

#[tokio::test]
async fn publishing_waits_for_the_launch_unless_allowed() {
    let Some(pool) = test_db().await else { return };
    let (url, fake) = fake_cloud().await;
    let (storage, stored) = fake_storage().await;
    let app = makerfoundry::app(AppState {
        cloud: Cloud::new(&url).unwrap(),
        db: Some(pool.clone()),
        storage: Some(storage),
        publishers: makerfoundry::listings::Publishers::Only(vec![]),
        admins: Vec::new(),
        limits: std::sync::Arc::default(),
        stripe: None,
        launched: false,
        public_url: "https://makerfoundry.test".into(),
        api_origins: vec!["https://stitchfoundry.test".into()],
    });
    let s = Site {
        app,
        pool,
        fake,
        stored,
    };
    let cookie = s
        .post(
            "/signup",
            "email=a%40b.c&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    let id = add_design(&s, &cookie, sf_formats::Format::Pes, "rose.pes").await;
    s.post("/account/maker", "name=Ann&handle=ann-s", Some(&cookie)).await;
    s.post(&format!("/designs/{id}/edit"), "title=Rose&licence=cc0", Some(&cookie))
        .await;
    let r = s
        .post(&format!("/designs/{id}/publish"), "confirm=yes", Some(&cookie))
        .await;
    assert!(
        r.body.contains("Publishing opens when MakerFoundry launches"),
        "{}",
        r.body
    );
    assert_eq!(s.get(&format!("/d/{id}"), None).await.status, StatusCode::NOT_FOUND);
}

/// A published design by a maker signed in with `cookie` → its id.
async fn published(s: &Site, cookie: &str, handle: &str, title: &str) -> i64 {
    let id = add_design(s, cookie, sf_formats::Format::Pes, "logo.pes").await;
    s.post("/account/maker", &format!("name=Maker&handle={handle}"), Some(cookie))
        .await;
    s.post(
        &format!("/designs/{id}/edit"),
        &format!("title={title}&licence=commercial"),
        Some(cookie),
    )
    .await;
    let r = s
        .post(&format!("/designs/{id}/publish"), "confirm=yes", Some(cookie))
        .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    id
}

#[tokio::test]
async fn a_takedown_is_done_in_minutes() {
    // the first account made is the admin
    let Some(s) = site_with(vec![1000]).await else { return };
    let admin = s
        .post(
            "/signup",
            "email=admin%40example.com&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    let maker = s
        .post(
            "/signup",
            "email=maker%40example.com&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    let id = published(&s, &maker, "copycat", "Famous+Mouse").await;
    // anyone can report it, from its page
    let page = s.get(&format!("/d/{id}"), None).await;
    assert!(page.body.contains(&format!("/d/{id}/report")), "{}", page.body);
    let r = s
        .post(
            &format!("/d/{id}/report"),
            "reason=trademark&details=That+is+a+famous+cartoon+mouse.",
            None,
        )
        .await;
    assert!(r.body.contains("Your report is in"), "{}", r.body);
    assert!(
        s.post(&format!("/d/{id}/report"), "reason=nope", None)
            .await
            .body
            .contains("Choose what the problem is")
    );
    // and its copyright owner sends a notice, every element the law asks for
    let notice = format!(
        "name=Rights+Holder&email=legal%40example.com&address=1+Main+St%0ASpringfield&phone=555-0100&original=Our+mouse+character&designs=https%3A%2F%2Fmakerfoundry.app%2Fd%2F{id}%2Ffamous-mouse&good_faith=yes&accurate=yes&signature=Rights+Holder"
    );
    assert!(
        s.post("/dmca/notice", &notice.replace("&accurate=yes", ""), None)
            .await
            .body
            .contains("Both statements are needed")
    );
    assert!(
        s.post("/dmca/notice", &notice.replace(&format!("%2Fd%2F{id}"), "%2Fx"), None)
            .await
            .body
            .contains("at least one design")
    );
    assert!(
        s.post("/dmca/notice", &notice, None)
            .await
            .body
            .contains("Notice received")
    );
    // the queue is the admins' alone
    assert_eq!(s.get("/admin", Some(&maker)).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.get("/admin", None).await.status, StatusCode::NOT_FOUND);
    let queue = s.get("/admin", Some(&admin)).await;
    assert!(
        queue.body.contains("A copyright notice (DMCA)")
            && queue.body.contains("Rights Holder")
            && queue.body.contains("famous cartoon mouse"),
        "{}",
        queue.body
    );
    // one click: down
    let rid = queue
        .body
        .split("/admin/reports/")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap()
        .to_string();
    assert_eq!(
        s.post(&format!("/admin/reports/{rid}/takedown"), "", Some(&maker))
            .await
            .status,
        StatusCode::NOT_FOUND,
        "not by the maker"
    );
    let r = s
        .post(&format!("/admin/reports/{rid}/takedown"), "", Some(&admin))
        .await;
    assert_eq!(r.location(), "/admin");
    // gone from every public place, both reports answered, the action logged
    assert_eq!(s.get(&format!("/d/{id}"), None).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        s.get(&format!("/d/{id}/download"), None).await.status,
        StatusCode::NOT_FOUND
    );
    assert!(!s.get("/browse", None).await.body.contains("Famous Mouse"));
    assert!(!s.get("/makers/copycat", None).await.body.contains("Famous Mouse"));
    assert!(s.get("/admin", Some(&admin)).await.body.contains("Nothing waiting"));
    let logged: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mf_moderation_log WHERE action = 'takedown' AND design_id = $1")
            .bind(id)
            .fetch_one(&s.pool)
            .await
            .unwrap();
    assert_eq!(logged, 1);
    // the maker sees why, can't publish it again, and may send a counter-notice
    let manage = s.get(&format!("/designs/{id}"), Some(&maker)).await;
    assert!(
        manage.body.contains("Taken down after a report") && manage.body.contains("counter-notice"),
        "{}",
        manage.body
    );
    let r = s
        .post(&format!("/designs/{id}/publish"), "confirm=yes", Some(&maker))
        .await;
    assert!(
        r.body.contains("can&#39;t be published again") || r.body.contains("can't be published again"),
        "{}",
        r.body
    );
    s.post(&format!("/designs/{id}/unpublish"), "", Some(&maker)).await;
    let status: String = sqlx::query_scalar("SELECT status FROM mf_designs WHERE id = $1")
        .bind(id)
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(status, "removed", "unpublishing doesn't undo a takedown");
    let counter = "name=Maker+Person&address=2+Elm+St&phone=555-0101&email=maker%40example.com&mistake=yes&consent=yes&signature=Maker+Person";
    assert!(
        s.post(
            &format!("/designs/{id}/counter"),
            &counter.replace("&consent=yes", ""),
            Some(&maker)
        )
        .await
        .body
        .contains("Both statements")
    );
    assert!(
        s.post(&format!("/designs/{id}/counter"), counter, Some(&maker))
            .await
            .body
            .contains("Counter-notice received")
    );
    // after the waiting time, the admin restores it
    let queue = s.get("/admin", Some(&admin)).await;
    assert!(
        queue.body.contains("A counter-notice from the maker") && queue.body.contains("Restore the design"),
        "{}",
        queue.body
    );
    let rid = queue
        .body
        .split("/admin/reports/")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap()
        .to_string();
    s.post(&format!("/admin/reports/{rid}/restore"), "", Some(&admin)).await;
    assert_eq!(s.get(&format!("/d/{id}"), None).await.status, StatusCode::OK);
}

#[tokio::test]
async fn reports_and_sign_ups_are_rate_limited() {
    let Some(s) = site().await else { return };
    let maker = s
        .post(
            "/signup",
            "email=maker%40example.com&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await
        .cookie();
    let id = published(&s, &maker, "maker-one", "Rose").await;
    for _ in 0..makerfoundry::limits::REPORT.max {
        assert!(
            s.post(&format!("/d/{id}/report"), "reason=spam", None)
                .await
                .body
                .contains("Your report is in")
        );
    }
    let r = s.post(&format!("/d/{id}/report"), "reason=spam", None).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    // one sign-up made above; the rest of the hour's allowance, then no more
    for i in 1..makerfoundry::limits::SIGNUP.max {
        s.post(
            "/signup",
            &format!("email=p{i}%40example.com&password=fire+truck+1&again=fire+truck+1"),
            None,
        )
        .await;
    }
    let r = s
        .post(
            "/signup",
            "email=late%40example.com&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await;
    assert!(r.body.contains("Too many tries"), "{}", r.body);
    assert!(!s.fake.lock().unwrap().accounts.contains_key("late@example.com"));
}

#[tokio::test]
async fn the_copyright_page_says_how_to_send_a_notice() {
    let Some(s) = site().await else { return };
    let page = s.get("/dmca", None).await;
    assert!(
        page.body.contains("512(c)(3)")
            && page.body.contains("/dmca/notice")
            && page.body.contains("repeatedly infringe")
    );
    assert!(
        s.get("/", None).await.body.contains(r#"href="/dmca""#),
        "linked from every page"
    );
}

#[tokio::test]
async fn sessions_past_their_days_are_forgotten() {
    let Some(s) = site().await else { return };
    s.post(
        "/signup",
        "email=a%40b.c&password=fire+truck+1&again=fire+truck+1",
        None,
    )
    .await;
    s.post(
        "/signup",
        "email=d%40e.f&password=fire+truck+1&again=fire+truck+1",
        None,
    )
    .await;
    sqlx::query("UPDATE mf_sessions SET last_seen_at = now() - interval '31 days' WHERE account_id = 1000")
        .execute(&s.pool)
        .await
        .unwrap();
    assert_eq!(makerfoundry::db::forget_old_sessions(&s.pool).await.unwrap(), 1);
    assert_eq!(s.sessions().await, 1, "the recent one stays");
}

// ---------------------------------------------------------------- MF-4: selling

const STRIPE_KEY: &str = "sk_test_fake";
const WEBHOOK_SECRET: &str = "whsec_fake";

/// The stand-in Stripe's state.
#[derive(Default)]
struct StripeFake {
    /// Express account → whether Stripe lets it take payments.
    accounts: HashMap<String, bool>,
    /// Idempotency key → the account made with it.
    made: HashMap<String, String>,
    /// Checkout Session → (its form, paid).
    sessions: HashMap<String, (HashMap<String, String>, bool)>,
    /// Every form posted, by path.
    posts: Vec<(String, HashMap<String, String>)>,
    refunds: Vec<HashMap<String, String>>,
}

type StripeShared = Arc<Mutex<StripeFake>>;

/// A stand-in for Stripe's API (the few calls MakerFoundry makes, answered as
/// https://docs.stripe.com/api says) → MakerFoundry's client for it, and its state.
async fn fake_stripe() -> (makerfoundry::stripe::Stripe, StripeShared) {
    use axum::Form;
    use axum::extract::Path;
    use axum::routing::get;
    let shared: StripeShared = Arc::default();
    let authorised = |h: &HeaderMap| token_of(h) == STRIPE_KEY;
    let app = Router::new()
        .route(
            "/v1/accounts",
            post(move |State(s): State<StripeShared>, h: HeaderMap, Form(f): Form<HashMap<String, String>>| async move {
                assert!(authorised(&h));
                let key = h.get("idempotency-key").unwrap().to_str().unwrap().to_string();
                let mut st = s.lock().unwrap();
                st.posts.push(("/v1/accounts".into(), f));
                let n = st.made.len();
                let id = st.made.entry(key).or_insert_with(|| format!("acct_test{n}")).clone();
                st.accounts.entry(id.clone()).or_insert(false);
                Json(json!({"id": id, "object": "account"}))
            }),
        )
        .route(
            "/v1/account_links",
            post(|State(s): State<StripeShared>, Form(f): Form<HashMap<String, String>>| async move {
                let url = format!("https://connect.stripe.com/setup/e/{}/x", f["account"]);
                s.lock().unwrap().posts.push(("/v1/account_links".into(), f));
                Json(json!({"object": "account_link", "url": url}))
            }),
        )
        .route(
            "/v1/accounts/{id}",
            get(|State(s): State<StripeShared>, Path(id): Path<String>| async move {
                let on = s.lock().unwrap().accounts.get(&id).copied().unwrap_or(false);
                Json(json!({"id": id, "details_submitted": true, "charges_enabled": on, "payouts_enabled": on}))
            }),
        )
        .route(
            "/v1/accounts/{id}/login_links",
            post(|Path(id): Path<String>| async move {
                Json(json!({"object": "login_link", "url": format!("https://connect.stripe.com/express/{id}")}))
            }),
        )
        .route(
            "/v1/checkout/sessions",
            post(move |State(s): State<StripeShared>, h: HeaderMap, Form(f): Form<HashMap<String, String>>| async move {
                assert!(authorised(&h));
                let mut st = s.lock().unwrap();
                let id = format!("cs_test_{}", st.sessions.len() + 1);
                st.sessions.insert(id.clone(), (f.clone(), false));
                st.posts.push(("/v1/checkout/sessions".into(), f));
                Json(json!({"id": id, "url": format!("https://checkout.stripe.com/c/pay/{id}"), "payment_status": "unpaid"}))
            }),
        )
        .route(
            "/v1/checkout/sessions/{id}",
            get(|State(s): State<StripeShared>, Path(id): Path<String>| async move {
                let st = s.lock().unwrap();
                let (f, paid) = st.sessions.get(&id).cloned().unwrap();
                Json(session_json(&id, &f, paid))
            }),
        )
        .route(
            "/v1/refunds",
            post(|State(s): State<StripeShared>, Form(f): Form<HashMap<String, String>>| async move {
                s.lock().unwrap().refunds.push(f);
                Json(json!({"id": "re_test_1", "object": "refund", "status": "succeeded"}))
            }),
        )
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let stripe = makerfoundry::stripe::Stripe::new(
        &format!("http://{addr}"),
        STRIPE_KEY,
        vec![WEBHOOK_SECRET.into()],
        "https://makerfoundry.test",
    )
    .unwrap();
    (stripe, shared)
}

/// A Checkout Session as Stripe describes it, made from the form that made it.
fn session_json(id: &str, f: &HashMap<String, String>, paid: bool) -> Value {
    json!({
        "id": id,
        "object": "checkout.session",
        "payment_status": if paid { "paid" } else { "unpaid" },
        "amount_total": f["line_items[0][price_data][unit_amount]"].parse::<i64>().unwrap(),
        "currency": "usd",
        "payment_intent": format!("pi_{id}"),
        "metadata": {"order": f["metadata[order]"], "app": f["metadata[app]"]},
    })
}

#[tokio::test]
async fn a_shared_stripe_accounts_other_payments_are_left_alone() {
    let Some((s, st, _sam, fox)) = selling_site(Vec::new()).await else {
        return;
    };
    let bea = s.sign_up("bea%40example.com").await;
    s.post(&format!("/d/{fox}/buy"), "", Some(&bea)).await;
    let form = st.lock().unwrap().sessions["cs_test_1"].0.clone();
    assert_eq!(form["metadata[app]"], "makerfoundry");
    assert_eq!(form["payment_intent_data[metadata][app]"], "makerfoundry");
    // StitchFoundry's credit pack, paid in the same Stripe account: its event comes here too
    let credits = json!({"id": "cs_sf_1", "object": "checkout.session", "payment_status": "paid", "amount_total": 1000,
        "currency": "usd", "payment_intent": "pi_sf_1", "metadata": {"account": "1000", "credits": "1000", "pack": "small"}});
    assert_eq!(
        s.stripe_event(WEBHOOK_SECRET, "evt_sf_1", "checkout.session.completed", credits)
            .await
            .status,
        StatusCode::OK
    );
    let refund = json!({"id": "ch_sf_1", "object": "charge", "payment_intent": "pi_sf_1", "refunded": true});
    assert_eq!(
        s.stripe_event(WEBHOOK_SECRET, "evt_sf_2", "charge.refunded", refund)
            .await
            .status,
        StatusCode::OK
    );
    // a session that names one of our orders but isn't labelled ours changes nothing either
    let mut unlabelled = session_json("cs_test_1", &form, true);
    unlabelled["metadata"]["app"] = json!("stitchfoundry");
    s.stripe_event(WEBHOOK_SECRET, "evt_sf_3", "checkout.session.completed", unlabelled)
        .await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_ledger").await, 0);
    assert_eq!(
        s.scalar("SELECT count(*) FROM mf_orders WHERE status = 'pending'")
            .await,
        1
    );
    // ours, labelled, is paid
    s.stripe_event(
        WEBHOOK_SECRET,
        "evt_mf_1",
        "checkout.session.completed",
        session_json("cs_test_1", &form, true),
    )
    .await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_ledger").await, 3);
}

impl Site {
    /// A Stripe event sent to the webhook, signed with `secret`.
    async fn stripe_event(&self, secret: &str, id: &str, kind: &str, object: Value) -> Answer {
        let body = json!({"id": id, "object": "event", "type": kind, "data": {"object": object}}).to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let sig = makerfoundry::stripe::signature(secret, now, body.as_bytes());
        self.send(
            Request::post("/stripe/webhook")
                .header(header::HOST, HOST)
                .header(header::CONTENT_TYPE, "application/json")
                .header("stripe-signature", format!("t={now},v1={sig}"))
                .body(Body::from(body))
                .unwrap(),
        )
        .await
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.unwrap()
    }

    async fn sign_up(&self, email: &str) -> String {
        self.post(
            "/signup",
            &format!("email={email}&password=fire+truck+1&again=fire+truck+1"),
            None,
        )
        .await
        .cookie()
    }
}

/// A seller (account 1000) whose Stripe account can take payments, with a published design
/// priced $5 → the site, the stand-in Stripe, the seller's cookie and the design.
async fn selling_site(admins: Vec<i64>) -> Option<(Site, StripeShared, String, i64)> {
    let (stripe, st) = fake_stripe().await;
    let s = site_full(admins, Some(stripe)).await?;
    let seller = s.sign_up("sam%40example.com").await;
    let id = published(&s, &seller, "sams-shop", "Fox+Patch").await;
    // a price needs selling set up first
    let r = s
        .post(
            &format!("/designs/{id}/edit"),
            "title=Fox+Patch&licence=personal&price=5",
            Some(&seller),
        )
        .await;
    assert!(r.body.contains("first set up selling"), "{}", r.body);
    // setting up: a Stripe Express account, paid monthly, then Stripe's own pages
    let r = s.post("/account/selling/start", "", Some(&seller)).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert!(
        r.location()
            .starts_with("https://connect.stripe.com/setup/e/acct_test0"),
        "{}",
        r.location()
    );
    {
        let f = st.lock().unwrap();
        let made = &f.posts.iter().find(|(p, _)| p == "/v1/accounts").unwrap().1;
        assert_eq!(made["type"], "express");
        assert_eq!(made["settings[payouts][schedule][interval]"], "monthly");
    }
    // pressed again: the same account
    s.post("/account/selling/start", "", Some(&seller)).await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_sellers").await, 1);
    // back from Stripe, not finished yet
    assert_eq!(
        s.get("/account/selling/return", Some(&seller)).await.location(),
        "/account/selling"
    );
    assert!(
        s.get("/account/selling", Some(&seller))
            .await
            .body
            .contains("Carry on with Stripe")
    );
    // Stripe says it's done, by a webhook from the connected accounts' endpoint
    st.lock().unwrap().accounts.insert("acct_test0".into(), true);
    let account = json!({"id": "acct_test0", "object": "account", "details_submitted": true, "charges_enabled": true, "payouts_enabled": true});
    let r = s
        .stripe_event("whsec_wrong", "evt_acct_0", "account.updated", account.clone())
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(
        s.get("/account/selling", Some(&seller))
            .await
            .body
            .contains("Carry on with Stripe")
    );
    let r = s
        .stripe_event(WEBHOOK_SECRET, "evt_acct_1", "account.updated", account)
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        s.get("/account/selling", Some(&seller))
            .await
            .body
            .contains("You can sell")
    );
    let r = s
        .post(
            &format!("/designs/{id}/edit"),
            "title=Fox+Patch&licence=personal&price=%245.00",
            Some(&seller),
        )
        .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    Some((s, st, seller, id))
}

#[tokio::test]
async fn a_design_is_sold_paid_for_delivered_and_refunded() {
    let Some((s, st, seller, id)) = selling_site(vec![1002]).await else {
        return;
    };
    let page = s.get(&format!("/d/{id}"), None).await.body;
    assert!(page.contains("Sign in to buy · $5"), "{page}");
    let buyer = s.sign_up("bea%40example.com").await;
    let admin = s.sign_up("ada%40example.com").await;
    // not bought: no file, just the page with the button
    let r = s.get(&format!("/d/{id}/download"), Some(&buyer)).await;
    assert_eq!(r.location(), format!("/d/{id}/fox-patch"));
    assert!(s.get(&format!("/d/{id}"), Some(&buyer)).await.body.contains("Buy · $5"));
    // buying: an order with its amounts fixed, then Stripe's checkout page
    let r = s.post(&format!("/d/{id}/buy"), "", Some(&buyer)).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location(), "https://checkout.stripe.com/c/pay/cs_test_1");
    let form = st.lock().unwrap().sessions["cs_test_1"].0.clone();
    assert_eq!(form["line_items[0][price_data][unit_amount]"], "500");
    // 10% ($0.50) and the card fee ($0.45) to MakerFoundry, $4.05 to the seller's Stripe account
    assert_eq!(form["payment_intent_data[application_fee_amount]"], "95");
    assert_eq!(form["payment_intent_data[transfer_data][destination]"], "acct_test0");
    let order = form["metadata[order]"].clone();
    // back from Stripe before the payment is confirmed: no download yet
    let lib = s.get(&format!("/library?order={order}"), Some(&buyer)).await.body;
    assert!(lib.contains("Waiting for the payment"), "{lib}");
    assert!(
        !s.get(&format!("/d/{id}/download"), Some(&buyer))
            .await
            .location()
            .contains("/files/")
    );
    // a forged event changes nothing
    let paid = session_json("cs_test_1", &form, true);
    let r = s
        .stripe_event("whsec_wrong", "evt_1", "checkout.session.completed", paid.clone())
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(s.scalar("SELECT count(*) FROM mf_ledger").await, 0);
    // a session paid for less than the price changes nothing
    let mut short = paid.clone();
    short["amount_total"] = json!(100);
    assert_eq!(
        s.stripe_event(WEBHOOK_SECRET, "evt_short", "checkout.session.completed", short)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(s.scalar("SELECT count(*) FROM mf_ledger").await, 0);
    // Stripe's signed event: paid, and the sale in the ledger, to the cent
    let r = s
        .stripe_event(WEBHOOK_SECRET, "evt_1", "checkout.session.completed", paid.clone())
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let ledger: Vec<(i64, String, i64)> = sqlx::query_as("SELECT account_id, kind, cents FROM mf_ledger ORDER BY kind")
        .fetch_all(&s.pool)
        .await
        .unwrap();
    assert_eq!(
        ledger,
        [
            (0, "fee".to_string(), 50),
            (0, "processing".to_string(), 45),
            (1000, "seller".to_string(), 405)
        ]
    );
    // sent again, or as another event for the same payment: handled once
    s.stripe_event(WEBHOOK_SECRET, "evt_1", "checkout.session.completed", paid.clone())
        .await;
    s.stripe_event(
        WEBHOOK_SECRET,
        "evt_2",
        "checkout.session.async_payment_succeeded",
        paid.clone(),
    )
    .await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_ledger").await, 3);
    assert_eq!(s.scalar("SELECT count(*) FROM mf_stripe_events").await, 4);
    // delivered: the buyer downloads it, from their library too
    let r = s.get(&format!("/d/{id}/download"), Some(&buyer)).await;
    assert!(r.location().contains("/files/"), "{}", r.location());
    assert!(
        s.get(&format!("/d/{id}"), Some(&buyer))
            .await
            .body
            .contains("You bought this")
    );
    assert!(
        s.get("/library", Some(&buyer))
            .await
            .body
            .contains(&format!(r#"href="/d/{id}/download""#))
    );
    // buying again goes to the library
    assert_eq!(
        s.post(&format!("/d/{id}/buy"), "", Some(&buyer)).await.location(),
        "/library"
    );
    // the seller's sales
    let sales = s.get("/account/sales", Some(&seller)).await.body;
    assert!(
        sales.contains("$5 − fee $0.50 − card fee $0.45 = <strong>$4.05</strong>"),
        "{sales}"
    );
    // unpublished, it's still the buyer's; and it can't be deleted
    s.post(&format!("/designs/{id}/unpublish"), "", Some(&seller)).await;
    assert!(
        s.get(&format!("/d/{id}/download"), Some(&buyer))
            .await
            .location()
            .contains("/files/")
    );
    let r = s.post(&format!("/designs/{id}/delete"), "", Some(&seller)).await;
    assert!(r.body.contains("People have bought this design"), "{}", r.body);
    // an admin refunds it in full: the sale reversed in the ledger, and no more download
    assert_eq!(s.get("/admin/orders", Some(&buyer)).await.status, StatusCode::NOT_FOUND);
    let r = s.post(&format!("/admin/orders/{order}/refund"), "", Some(&admin)).await;
    assert!(r.body.contains("refunded"), "{}", r.body);
    {
        let f = st.lock().unwrap();
        assert_eq!(f.refunds[0]["payment_intent"], "pi_cs_test_1");
        assert_eq!(f.refunds[0]["reverse_transfer"], "true");
        assert_eq!(f.refunds[0]["refund_application_fee"], "true");
    }
    assert_eq!(s.scalar("SELECT count(*) FROM mf_ledger").await, 6);
    assert_eq!(s.scalar("SELECT sum(cents)::bigint FROM mf_ledger").await, 0);
    // Stripe's own word of the refund afterwards changes nothing more
    let charge = json!({"id": "ch_1", "object": "charge", "payment_intent": "pi_cs_test_1", "refunded": true});
    assert_eq!(
        s.stripe_event(WEBHOOK_SECRET, "evt_3", "charge.refunded", charge)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(s.scalar("SELECT count(*) FROM mf_ledger").await, 6);
    assert!(
        !s.get(&format!("/d/{id}/download"), Some(&buyer))
            .await
            .location()
            .contains("/files/")
    );
    assert!(s.get("/library", Some(&buyer)).await.body.contains("Refunded"));
    assert!(
        s.get("/account/sales", Some(&seller))
            .await
            .body
            .contains("<dd>$0</dd>")
    );
}

#[tokio::test]
async fn coming_back_from_stripe_delivers_without_waiting_for_the_webhook() {
    let Some((s, st, seller, id)) = selling_site(Vec::new()).await else {
        return;
    };
    let buyer = s.sign_up("bea%40example.com").await;
    s.post(&format!("/d/{id}/buy"), "", Some(&buyer)).await;
    // the seller can't buy their own; someone else can't see the buyer's order
    assert_eq!(
        s.post(&format!("/d/{id}/buy"), "", Some(&seller)).await.location(),
        format!("/d/{id}")
    );
    let order = st.lock().unwrap().sessions["cs_test_1"].0["metadata[order]"].clone();
    st.lock().unwrap().sessions.get_mut("cs_test_1").unwrap().1 = true;
    s.get(&format!("/library?order={order}"), Some(&seller)).await;
    assert_eq!(
        s.scalar("SELECT count(*) FROM mf_orders WHERE status = 'paid'").await,
        0
    );
    let lib = s.get(&format!("/library?order={order}"), Some(&buyer)).await.body;
    assert!(lib.contains(&format!(r#"href="/d/{id}/download""#)), "{lib}");
    assert_eq!(
        s.scalar("SELECT sum(cents)::bigint FROM mf_ledger WHERE reason = 'sale'")
            .await,
        500
    );
    // and the webhook arriving later adds nothing
    let form = st.lock().unwrap().sessions["cs_test_1"].0.clone();
    s.stripe_event(
        WEBHOOK_SECRET,
        "evt_9",
        "checkout.session.completed",
        session_json("cs_test_1", &form, true),
    )
    .await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_ledger").await, 3);
}

#[tokio::test]
async fn without_stripe_every_design_is_free() {
    let Some(s) = site().await else { return };
    let cookie = s.sign_up("ann%40example.com").await;
    let id = published(&s, &cookie, "anns", "Rose").await;
    assert!(
        s.get("/account/selling", Some(&cookie))
            .await
            .body
            .contains("isn't switched on")
    );
    let r = s
        .post(
            &format!("/designs/{id}/edit"),
            "title=Rose&licence=personal&price=3",
            Some(&cookie),
        )
        .await;
    assert!(r.body.contains("first set up selling"), "{}", r.body);
    assert_eq!(
        s.post("/stripe/webhook", "{}", None).await.status,
        StatusCode::NOT_FOUND
    );
    assert!(s.get(&format!("/d/{id}"), None).await.body.contains("Download free"));
}

// ---------------------------------------------------------------- MF-6: the launch

#[tokio::test]
async fn before_the_launch_the_legal_drafts_are_for_admins_only() {
    let Some(s) = site_with(vec![1000]).await else { return };
    let admin = s.sign_up("ada%40example.com").await;
    let other = s.sign_up("bea%40example.com").await;
    for path in ["/terms", "/privacy", "/seller-terms"] {
        let public = s.get(path, None).await;
        assert_eq!(public.status, StatusCode::OK);
        assert!(public.body.contains("hasn't opened yet"), "{}", public.body);
        assert!(!public.body.contains("OWNER"));
        assert!(!s.get(path, Some(&other)).await.body.contains("OWNER"));
        let draft = s.get(path, Some(&admin)).await.body;
        assert!(
            draft.contains("A draft for the owner") && draft.contains("<mark>[OWNER"),
            "{draft}"
        );
    }
    // no agreement asked for, search engines kept out, and still "coming soon"
    assert!(!s.get("/signup", None).await.body.contains("name=\"agree\""));
    assert_eq!(s.scalar("SELECT count(*) FROM mf_agreements").await, 0);
    assert_eq!(s.get("/robots.txt", None).await.body, "User-agent: *\nDisallow: /\n");
    assert_eq!(s.get("/sitemap.xml", None).await.status, StatusCode::NOT_FOUND);
    assert!(s.get("/", None).await.body.contains("Coming soon"));
}

#[tokio::test]
async fn once_launched_people_agree_to_the_terms_and_the_site_is_found() {
    let Some(s) = site_opts(Vec::new(), None, true).await else {
        return;
    };
    let terms = s.get("/terms", None).await.body;
    assert!(terms.contains("<h1>Terms of Service: MakerFoundry</h1>"), "{terms}");
    // signing up asks for agreement to the Terms and the Privacy Policy, and keeps it
    let form = s.get("/signup", None).await.body;
    assert!(form.contains(r#"name="agree""#) && form.contains(r#"href="/terms""#));
    let r = s
        .post(
            "/signup",
            "email=ann%40example.com&password=fire+truck+1&again=fire+truck+1",
            None,
        )
        .await;
    assert!(r.body.contains("agree to the Terms"), "{}", r.body);
    let cookie = s
        .post(
            "/signup",
            "email=ann%40example.com&password=fire+truck+1&again=fire+truck+1&agree=yes",
            None,
        )
        .await
        .cookie();
    let agreed: Vec<(String, String)> =
        sqlx::query_as("SELECT document, version FROM mf_agreements WHERE account_id = 1000 ORDER BY document")
            .fetch_all(&s.pool)
            .await
            .unwrap();
    assert_eq!(
        agreed,
        [
            (
                "privacy".to_string(),
                makerfoundry::legal::PRIVACY.version().to_string()
            ),
            ("terms".to_string(), makerfoundry::legal::TERMS.version().to_string()),
        ]
    );
    // publishing is agreeing to the Seller Terms
    let id = published(&s, &cookie, "anns", "Rose+Spray").await;
    assert_eq!(
        s.scalar("SELECT count(*) FROM mf_agreements WHERE document = 'seller-terms'")
            .await,
        1
    );
    // the home page invites people in; search engines are welcome, with a map
    let home = s.get("/", None).await.body;
    assert!(
        home.contains("Browse designs") && !home.contains("Coming soon"),
        "{home}"
    );
    let robots = s.get("/robots.txt", None).await.body;
    assert!(robots.contains("Disallow: /account") && robots.contains("Sitemap: https://makerfoundry.test/sitemap.xml"));
    let map = s.get("/sitemap.xml", None).await.body;
    assert!(
        map.contains(&format!("<loc>https://makerfoundry.test/d/{id}/rose-spray</loc>")),
        "{map}"
    );
    assert!(map.contains("<loc>https://makerfoundry.test/makers/anns</loc>"));
}

// ---------------------------------------------------------------- MF-7: publishing from the apps

impl Site {
    /// A request to the apps' API: `body` as JSON, with the app's Foundry Account `token`.
    async fn api(&self, method: &str, path: &str, token: Option<&str>, origin: Option<&str>, body: &Value) -> Answer {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, HOST)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        if let Some(o) = origin {
            req = req.header(header::ORIGIN, o);
        }
        self.send(req.body(Body::from(body.to_string())).unwrap()).await
    }
}

#[tokio::test]
async fn an_app_sends_a_design_which_opens_as_a_draft_to_finish_on_the_site() {
    use base64::Engine;
    let Some(s) = site().await else { return };
    let b64 = base64::engine::general_purpose::STANDARD;
    let cookie = s.sign_up("ann%40example.com").await;
    // the app's own sign-in to the same Foundry Account
    s.fake.lock().unwrap().sessions.insert("sfs_app".into(), 1000);
    let design = json!({
        "file_name": "C:\\Users\\ann\\rose.pes",
        "file": b64.encode(sample(sf_formats::Format::Pes)),
        "photos": [b64.encode(photo_with_location())],
        "app": "StitchFoundry 0.9.0",
    });
    // without a sign-in, or with one that has ended (or only the site's cookie): refused
    for token in [None, Some("sfs_nope")] {
        let r = s.api("POST", "/api/v1/designs", token, None, &design).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{}", r.body);
        assert!(r.body.contains("Foundry Account"), "{}", r.body);
    }
    let mut req = Request::post("/api/v1/designs").header(header::HOST, HOST);
    req = req.header(header::COOKIE, &cookie);
    assert_eq!(
        s.send(req.body(Body::from(design.to_string())).unwrap()).await.status,
        StatusCode::UNAUTHORIZED
    );
    // a file that isn't a design, or isn't base64
    let mut junk = design.clone();
    junk["file"] = json!(b64.encode(b"not a design at all"));
    let r = s.api("POST", "/api/v1/designs", Some("sfs_app"), None, &junk).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.body);
    junk["file"] = json!("%%%");
    assert_eq!(
        s.api("POST", "/api/v1/designs", Some("sfs_app"), None, &junk)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(s.scalar("SELECT count(*) FROM mf_designs").await, 0);
    // sent: a draft of Ann's, read, with its photo
    let r = s.api("POST", "/api/v1/designs", Some("sfs_app"), None, &design).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.body);
    let v: Value = serde_json::from_str(&r.body).unwrap();
    let id = v["id"].as_i64().unwrap();
    assert_eq!(v["url"], format!("https://makerfoundry.test/designs/{id}"));
    assert_eq!(
        (v["kind"].as_str(), v["format"].as_str()),
        (Some("embroidery"), Some("PES"))
    );
    let (owner, name, status): (i64, String, String) =
        sqlx::query_as("SELECT account_id, file_name, status FROM mf_designs WHERE id = $1")
            .bind(id)
            .fetch_one(&s.pool)
            .await
            .unwrap();
    assert_eq!((owner, name.as_str(), status.as_str()), (1000, "rose.pes", "draft"));
    assert_eq!(s.scalar("SELECT count(*) FROM mf_design_photos").await, 1);
    assert_eq!(s.stored.lock().unwrap().len(), 3);
    // the app opens its page: signed in, the draft to finish; signed out, sign in and come back
    let page = s.get(&format!("/designs/{id}"), Some(&cookie)).await;
    assert!(page.body.contains("Edit details"), "{}", page.body);
    let r = s.get(&format!("/designs/{id}"), None).await;
    assert_eq!(r.location(), format!("/signin?next=/designs/{id}"));
    let form = s.get(&format!("/signin?next=/designs/{id}"), None).await.body;
    assert!(
        form.contains(&format!(r#"name="next" value="/designs/{id}""#)),
        "{form}"
    );
    let r = s
        .post(
            "/signin",
            &format!("email=ann%40example.com&password=fire+truck+1&next=%2Fdesigns%2F{id}"),
            None,
        )
        .await;
    assert_eq!(r.location(), format!("/designs/{id}"));
    // only to pages of this site
    for bad in ["%2F%2Fevil.example", "https%3A%2F%2Fevil.example", "%2F%5Cevil.example"] {
        let r = s
            .post(
                "/signin",
                &format!("email=ann%40example.com&password=fire+truck+1&next={bad}"),
                None,
            )
            .await;
        assert_eq!(r.location(), "/account", "{bad}");
    }
}

#[tokio::test]
async fn the_web_apps_may_call_the_api_from_their_own_sites_only() {
    let Some(s) = site().await else { return };
    let ok = s
        .api(
            "OPTIONS",
            "/api/v1/designs",
            None,
            Some("https://stitchfoundry.test"),
            &json!({}),
        )
        .await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT);
    assert_eq!(
        ok.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://stitchfoundry.test"
    );
    assert!(
        ok.headers[header::ACCESS_CONTROL_ALLOW_HEADERS]
            .to_str()
            .unwrap()
            .contains("authorization")
    );
    assert!(ok.headers.get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none());
    let evil = s
        .api(
            "OPTIONS",
            "/api/v1/designs",
            None,
            Some("https://evil.example"),
            &json!({}),
        )
        .await;
    assert_eq!(evil.status, StatusCode::FORBIDDEN);
    assert!(evil.headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    // answers carry it too, errors included, so the web app can show the message
    let r = s
        .api(
            "POST",
            "/api/v1/designs",
            None,
            Some("https://stitchfoundry.test"),
            &json!({}),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        r.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://stitchfoundry.test"
    );
    let about = s.api("GET", "/api/v1", None, None, &json!({})).await;
    assert!(about.body.contains(r#""designs":"/api/v1/designs""#), "{}", about.body);
}

// ---------------------------------------------------------------- MF-8: finishing the first version

#[tokio::test]
async fn browsing_finds_free_or_paid_designs_and_unpaid_checkouts_expire() {
    let Some((s, st, seller, paid)) = selling_site(Vec::new()).await else {
        return;
    };
    let free = published(&s, &seller, "sams-shop", "Owl+Patch").await;
    let free_list = s.get("/browse?price=free", None).await.body;
    assert!(free_list.contains(&format!("/d/{free}/")) && !free_list.contains(&format!("/d/{paid}/")));
    let paid_list = s.get("/browse?price=paid", None).await.body;
    assert!(paid_list.contains(&format!("/d/{paid}/")) && !paid_list.contains(&format!("/d/{free}/")));
    assert!(paid_list.contains(r#"<option value="paid" selected>"#), "{paid_list}");
    let both = s.get("/browse", None).await.body;
    assert!(both.contains(&format!("/d/{free}/")) && both.contains(&format!("/d/{paid}/")));
    // two purchases started and never paid for: Stripe's page lasts 2 hours
    let buyer = s.sign_up("bea%40example.com").await;
    s.post(&format!("/d/{paid}/buy"), "", Some(&buyer)).await;
    let form = st.lock().unwrap().sessions["cs_test_1"].0.clone();
    let expires: i64 = form["expires_at"].parse().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!((expires - now - 7200).abs() < 60, "{expires}");
    s.post(&format!("/d/{paid}/buy"), "", Some(&buyer)).await;
    // Stripe says the first page expired
    let r = s
        .stripe_event(
            WEBHOOK_SECRET,
            "evt_x",
            "checkout.session.expired",
            json!({"id": "cs_test_1", "object": "checkout.session"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        s.scalar("SELECT count(*) FROM mf_orders WHERE status = 'expired'")
            .await,
        1
    );
    // the second, a day on, expires anyway
    assert_eq!(makerfoundry::selling::expire_orders(&s.pool).await.unwrap(), 0);
    sqlx::query("UPDATE mf_orders SET created_at = now() - interval '25 hours' WHERE status = 'pending'")
        .execute(&s.pool)
        .await
        .unwrap();
    assert_eq!(makerfoundry::selling::expire_orders(&s.pool).await.unwrap(), 1);
    assert_eq!(
        s.scalar("SELECT count(*) FROM mf_orders WHERE status = 'pending'")
            .await,
        0
    );
    // neither counts as a sale, and the design can still be deleted once unpublished
    assert!(
        s.get("/account/sales", Some(&seller))
            .await
            .body
            .contains("No sales yet")
    );
    s.post(&format!("/designs/{paid}/unpublish"), "", Some(&seller)).await;
    let r = s.post(&format!("/designs/{paid}/delete"), "", Some(&seller)).await;
    assert_eq!(r.location(), "/designs", "{}", r.body);
}

#[tokio::test]
async fn designs_are_found_by_the_hoop_they_fit_and_the_material_they_cut() {
    let Some(s) = site().await else { return };
    let cookie = s.sign_up("ann%40example.com").await;
    // the sample PES is 60 × 60 mm
    let rose = published(&s, &cookie, "anns", "Rose").await;
    assert!(
        s.get(&format!("/d/{rose}"), None)
            .await
            .body
            .contains("fits a 4×4 in hoop")
    );
    // a laser design, 300 × 200 mm, made for 3 mm plywood
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="300mm" height="200mm" viewBox="0 0 300 200"><rect x="1" y="1" width="298" height="198" fill="none" stroke="red"/></svg>"#;
    let r = s
        .send(multipart("/designs/new", &[("file", "box.svg", svg.to_vec())], &cookie))
        .await;
    let boxed = design_id(r.location());
    let edit = format!("/designs/{boxed}/edit");
    assert!(
        s.get(&edit, Some(&cookie))
            .await
            .body
            .contains(r#"<select name="material">"#)
    );
    let r = s
        .post(
            &edit,
            "title=Box&licence=personal&price=0&material=plywood&thickness=sixty",
            Some(&cookie),
        )
        .await;
    assert!(r.body.contains("thickness in mm"), "{}", r.body);
    let r = s
        .post(
            &edit,
            "title=Box&licence=personal&price=0&material=gold&thickness=3",
            Some(&cookie),
        )
        .await;
    assert!(r.body.contains("Choose a material"), "{}", r.body);
    let r = s
        .post(
            &edit,
            "title=Box&licence=personal&price=0&material=plywood&thickness=3",
            Some(&cookie),
        )
        .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    s.post(&format!("/designs/{boxed}/publish"), "confirm=yes", Some(&cookie))
        .await;
    assert!(
        s.get(&format!("/d/{boxed}"), None)
            .await
            .body
            .contains("for 3 mm plywood")
    );
    // an embroidery design has no material, whatever is sent
    s.post(
        &format!("/designs/{rose}/edit"),
        "title=Rose&licence=personal&price=0&material=acrylic",
        Some(&cookie),
    )
    .await;
    assert!(
        !s.get(&format!("/designs/{rose}/edit"), Some(&cookie))
            .await
            .body
            .contains("Material")
    );
    // browsing
    let has = |body: &str, id: i64| body.contains(&format!("/d/{id}/"));
    let fits = s.get("/browse?hoop=4x4", None).await.body;
    assert!(has(&fits, rose) && !has(&fits, boxed));
    assert!(fits.contains(r#"<option value="4x4" selected>"#));
    let ply = s.get("/browse?material=plywood", None).await.body;
    assert!(has(&ply, boxed) && !has(&ply, rose));
    let acrylic = s.get("/browse?material=acrylic", None).await.body;
    assert!(!has(&acrylic, boxed) && !has(&acrylic, rose));
    // nonsense is ignored
    let all = s.get("/browse?hoop=99x99&material=gold", None).await.body;
    assert!(has(&all, boxed) && has(&all, rose));
}

#[tokio::test]
async fn a_design_has_more_formats_each_checked_and_downloadable() {
    let Some(s) = site().await else { return };
    let cookie = s.sign_up("ann%40example.com").await;
    let rose = published(&s, &cookie, "anns", "Rose").await;
    let other = add_design(&s, &cookie, sf_formats::Format::Pes, "other.pes").await;
    let add = |name: &'static str, bytes: Vec<u8>| {
        multipart(&format!("/designs/{rose}/files"), &[("file", name, bytes)], &cookie)
    };
    // the same design as a DST: read, checked and kept
    let r = s.send(add("rose.dst", sample(sf_formats::Format::Dst))).await;
    assert_eq!(r.location(), format!("/designs/{rose}"), "{}", r.body);
    // a laser file isn't another format of an embroidery design, and junk isn't a file at all
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10mm" height="10mm"><rect width="5" height="5" stroke="red" fill="none"/></svg>"#;
    assert!(
        s.send(add("rose.svg", svg.to_vec()))
            .await
            .body
            .contains("This is an embroidery design")
    );
    let junk = s.send(add("rose.jef", b"not a design at all".to_vec())).await;
    assert!(
        junk.body.contains("rose.jef"),
        "{} {} {}",
        junk.status,
        junk.location(),
        junk.body
    );
    // one file a format
    assert!(
        s.send(add("again.pes", sample(sf_formats::Format::Pes)))
            .await
            .body
            .contains("already has a PES file")
    );
    assert!(
        s.send(add("again.dst", sample(sf_formats::Format::Dst)))
            .await
            .body
            .contains("already has a DST file")
    );
    let fid: i64 = sqlx::query_scalar("SELECT id FROM mf_design_files WHERE design_id = $1")
        .bind(rose)
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(s.scalar("SELECT count(*) FROM mf_design_files").await, 1);
    let manage = s.get(&format!("/designs/{rose}"), Some(&cookie)).await.body;
    assert!(
        manage.contains(&format!(r#"href="/d/{rose}/download?file={fid}">DST</a>"#)),
        "{manage}"
    );
    // on the public page, both formats, each downloadable
    let page = s.get(&format!("/d/{rose}"), None).await.body;
    assert!(
        page.contains(&format!(r#"<a href="/d/{rose}/download">PES</a>"#)),
        "{page}"
    );
    assert!(page.contains(&format!(r#"<a href="/d/{rose}/download?file={fid}">DST</a>"#)));
    let r = s.get(&format!("/d/{rose}/download?file={fid}"), None).await;
    assert!(
        r.location().contains("/files/") && r.location().contains("rose.dst"),
        "{}",
        r.location()
    );
    // a file of another design is not this design's
    assert_eq!(
        s.get(&format!("/d/{other}/download?file={fid}"), Some(&cookie))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    // others can't add or remove files
    let bea = s.sign_up("bea%40example.com").await;
    let r = s
        .send(multipart(
            &format!("/designs/{rose}/files"),
            &[("file", "x.dst", sample(sf_formats::Format::Dst))],
            &bea,
        ))
        .await;
    assert_eq!(r.location(), "/designs");
    s.post(&format!("/designs/{rose}/files/{fid}/delete"), "", Some(&bea))
        .await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_design_files").await, 1);
    // removed by its maker: gone from storage too
    let stored = s.stored.lock().unwrap().len();
    s.post(&format!("/designs/{rose}/files/{fid}/delete"), "", Some(&cookie))
        .await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_design_files").await, 0);
    assert_eq!(s.stored.lock().unwrap().len(), stored - 1);
    assert!(!s.get(&format!("/d/{rose}"), None).await.body.contains("download?file="));
    // a deleted design takes its other files with it
    s.send(add("rose.dst", sample(sf_formats::Format::Dst))).await;
    s.post(&format!("/designs/{rose}/unpublish"), "", Some(&cookie)).await;
    let before = s.stored.lock().unwrap().len();
    s.post(&format!("/designs/{rose}/delete"), "", Some(&cookie)).await;
    // the first file, its photo and thumbnail, and the DST
    assert_eq!(s.stored.lock().unwrap().len(), before - 4);
    assert_eq!(s.scalar("SELECT count(*) FROM mf_design_files").await, 0);
}

#[tokio::test]
async fn a_paid_designs_other_formats_are_for_its_buyers() {
    let Some((s, st, seller, id)) = selling_site(Vec::new()).await else {
        return;
    };
    let r = s
        .send(multipart(
            &format!("/designs/{id}/files"),
            &[("file", "fox.dst", sample(sf_formats::Format::Dst))],
            &seller,
        ))
        .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    let fid: i64 = sqlx::query_scalar("SELECT id FROM mf_design_files")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    let buyer = s.sign_up("bea%40example.com").await;
    // before buying: the formats named, not linked, and no file
    let page = s.get(&format!("/d/{id}"), Some(&buyer)).await.body;
    assert!(
        page.contains("<li>DST</li>") && !page.contains(&format!("download?file={fid}")),
        "{page}"
    );
    assert!(
        !s.get(&format!("/d/{id}/download?file={fid}"), Some(&buyer))
            .await
            .location()
            .contains("/files/")
    );
    // bought: every format
    s.post(&format!("/d/{id}/buy"), "", Some(&buyer)).await;
    let form = st.lock().unwrap().sessions["cs_test_1"].0.clone();
    s.stripe_event(
        WEBHOOK_SECRET,
        "evt_p",
        "checkout.session.completed",
        session_json("cs_test_1", &form, true),
    )
    .await;
    assert!(
        s.get(&format!("/d/{id}"), Some(&buyer))
            .await
            .body
            .contains(&format!("download?file={fid}"))
    );
    assert!(
        s.get(&format!("/d/{id}/download?file={fid}"), Some(&buyer))
            .await
            .location()
            .contains("/files/")
    );
    // and once bought, the seller can't take a format away
    s.post(&format!("/designs/{id}/files/{fid}/delete"), "", Some(&seller))
        .await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_design_files").await, 1);
    assert!(
        !s.get(&format!("/designs/{id}"), Some(&seller))
            .await
            .body
            .contains("/files/{fid}/delete")
    );
}

// ---------------------------------------------------------------- MF-9: converted downloads

#[tokio::test]
async fn an_embroidery_design_downloads_converted_to_the_other_formats() {
    let Some(s) = site().await else { return };
    let cookie = s.sign_up("ann%40example.com").await;
    let rose = published(&s, &cookie, "anns", "Rose").await;
    let page = s.get(&format!("/d/{rose}"), None).await.body;
    assert!(
        page.contains(&format!(r#"<a href="/d/{rose}/download?as=dst">DST</a>"#)),
        "{page}"
    );
    assert!(!page.contains("download?as=pes") && !page.contains("download?as=dsz"));
    // made on the spot, as a real DST
    let r = s.get(&format!("/d/{rose}/download?as=dst"), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        r.headers[header::CONTENT_DISPOSITION],
        r#"attachment; filename="rose.dst""#
    );
    let resp = s
        .app
        .clone()
        .oneshot(
            Request::get(format!("/d/{rose}/download?as=jef"))
                .header(header::HOST, HOST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let back = sf_formats::read(sf_formats::Format::Jef, &bytes).unwrap();
    let original = sf_formats::read(sf_formats::Format::Pes, &sample(sf_formats::Format::Pes)).unwrap();
    // the same design in the other format: its stitches (long ones split where the format's
    // stitches are shorter) over the same size, to a tenth of a millimetre
    let sewn = |p: &sf_core::Pattern| p.stitches.iter().filter(|s| s.cmd == sf_core::Cmd::Stitch).count();
    assert!(sewn(&back) >= sewn(&original));
    let size = |p: &sf_core::Pattern| {
        let xs = p.stitches.iter().map(|s| s.x);
        let ys = p.stitches.iter().map(|s| s.y);
        (
            xs.clone().fold(f64::MIN, f64::max) - xs.fold(f64::MAX, f64::min),
            ys.clone().fold(f64::MIN, f64::max) - ys.fold(f64::MAX, f64::min),
        )
    };
    let ((w1, h1), (w2, h2)) = (size(&original), size(&back));
    assert!(
        (w1 - w2).abs() < 0.11 && (h1 - h2).abs() < 0.11,
        "{w1}x{h1} became {w2}x{h2}"
    );
    assert_eq!(
        s.scalar(&format!("SELECT downloads FROM mf_designs WHERE id = {rose}"))
            .await,
        2
    );
    // only formats that can be written
    for bad in ["dsz", "svg", "x.dst", ""] {
        assert_eq!(
            s.get(&format!("/d/{rose}/download?as={bad}"), None).await.status,
            StatusCode::NOT_FOUND,
            "{bad}"
        );
    }
    // a DST of its own: no longer converted to DST
    s.send(multipart(
        &format!("/designs/{rose}/files"),
        &[("file", "rose.dst", sample(sf_formats::Format::Dst))],
        &cookie,
    ))
    .await;
    assert!(
        !s.get(&format!("/d/{rose}"), None)
            .await
            .body
            .contains("download?as=dst")
    );
    // saving the details without the box (another form) leaves it on; unticking it turns it off
    s.post(
        &format!("/designs/{rose}/edit"),
        "title=Rose&licence=personal&price=0",
        Some(&cookie),
    )
    .await;
    assert!(
        s.get(&format!("/d/{rose}"), None)
            .await
            .body
            .contains("download?as=jef")
    );
    let form = s.get(&format!("/designs/{rose}/edit"), Some(&cookie)).await.body;
    assert!(form.contains(r#"name="convert" value="yes" checked"#), "{form}");
    s.post(
        &format!("/designs/{rose}/edit"),
        "title=Rose&licence=personal&price=0&convert_shown=yes",
        Some(&cookie),
    )
    .await;
    assert!(!s.get(&format!("/d/{rose}"), None).await.body.contains("download?as="));
    assert_eq!(
        s.get(&format!("/d/{rose}/download?as=jef"), None).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_paid_designs_conversions_are_for_its_buyers() {
    let Some((s, _st, _seller, id)) = selling_site(Vec::new()).await else {
        return;
    };
    let buyer = s.sign_up("bea%40example.com").await;
    let page = s.get(&format!("/d/{id}"), Some(&buyer)).await.body;
    assert!(
        page.contains("<li>DST</li>") && !page.contains("download?as=dst"),
        "{page}"
    );
    let r = s.get(&format!("/d/{id}/download?as=dst"), Some(&buyer)).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location(), format!("/d/{id}/fox-patch"));
}

// ---------------------------------------------------------------- MF-10: saved designs and collections

#[tokio::test]
async fn people_save_designs_and_keep_collections_public_or_private() {
    let Some(s) = site().await else { return };
    let ann = s.sign_up("ann%40example.com").await;
    let rose = published(&s, &ann, "anns", "Rose").await;
    let owl = published(&s, &ann, "anns", "Owl").await;
    let bea = s.sign_up("bea%40example.com").await;
    let cal = s.sign_up("cal%40example.com").await;
    // signed out: sign in first
    assert!(
        s.get(&format!("/d/{rose}"), None)
            .await
            .body
            .contains("Sign in to save")
    );
    assert_eq!(
        s.post(&format!("/d/{rose}/save"), "", None).await.location(),
        format!("/signin?next=/d/{rose}")
    );
    // Bea saves the rose to her Saved list (twice is once)
    let r = s.post(&format!("/d/{rose}/save"), "collection=", Some(&bea)).await;
    assert_eq!(r.location(), format!("/d/{rose}/rose"));
    s.post(&format!("/d/{rose}/save"), "", Some(&bea)).await;
    let page = s.get(&format!("/d/{rose}"), Some(&bea)).await.body;
    assert!(
        page.contains("saved by 1 person") && page.contains("In your <a href="),
        "{page}"
    );
    // a public collection of her own, with the owl in it
    let r = s
        .post("/collections", "name=Christmas++ideas&public=yes", Some(&bea))
        .await;
    let christmas: i64 = r.location().trim_start_matches("/collections/").parse().unwrap();
    s.post(
        &format!("/d/{owl}/save"),
        &format!("collection={christmas}"),
        Some(&bea),
    )
    .await;
    let public = s.get(&format!("/collections/{christmas}"), None).await;
    assert_eq!(public.status, StatusCode::OK);
    assert!(public.body.contains("<h1>Christmas ideas</h1>") && public.body.contains(&format!("/d/{owl}/")));
    // her Saved list is hers alone
    let saved: i64 = sqlx::query_scalar("SELECT id FROM mf_collections WHERE is_saved")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert!(
        s.get(&format!("/collections/{saved}"), Some(&bea))
            .await
            .body
            .contains(&format!("/d/{rose}/"))
    );
    assert_eq!(
        s.get(&format!("/collections/{saved}"), None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        s.get(&format!("/collections/{saved}"), Some(&ann)).await.status,
        StatusCode::NOT_FOUND
    );
    // nobody else adds to, takes from, renames or deletes it
    s.post(
        &format!("/d/{rose}/save"),
        &format!("collection={christmas}"),
        Some(&cal),
    )
    .await;
    s.post(&format!("/collections/{christmas}/remove/{owl}"), "", Some(&cal))
        .await;
    s.post(&format!("/collections/{christmas}/edit"), "name=Mine", Some(&cal))
        .await;
    s.post(&format!("/collections/{christmas}/delete"), "", Some(&cal))
        .await;
    let items: Vec<i64> = sqlx::query_scalar("SELECT design_id FROM mf_collection_items WHERE collection_id = $1")
        .bind(christmas)
        .fetch_all(&s.pool)
        .await
        .unwrap();
    assert_eq!(items, [owl]);
    assert!(
        s.get(&format!("/collections/{christmas}"), None)
            .await
            .body
            .contains("Christmas ideas")
    );
    // Cal saves the rose too: the most saved comes first
    s.post(&format!("/d/{rose}/save"), "", Some(&cal)).await;
    assert!(
        s.get(&format!("/d/{rose}"), None)
            .await
            .body
            .contains("saved by 2 people")
    );
    let browse = s.get("/browse?sort=saved", None).await.body;
    let (r_at, o_at) = (
        browse.find(&format!("/d/{rose}/")).unwrap(),
        browse.find(&format!("/d/{owl}/")).unwrap(),
    );
    assert!(r_at < o_at);
    let browse = s.get("/browse", None).await.body;
    assert!(browse.find(&format!("/d/{owl}/")).unwrap() < browse.find(&format!("/d/{rose}/")).unwrap());
    // an unpublished design drops out of collections, and can't be saved
    s.post(&format!("/designs/{owl}/unpublish"), "", Some(&ann)).await;
    assert!(
        !s.get(&format!("/collections/{christmas}"), None)
            .await
            .body
            .contains(&format!("/d/{owl}/"))
    );
    assert_eq!(
        s.post(&format!("/d/{owl}/save"), "", Some(&cal)).await.location(),
        "/browse"
    );
    // a maker's public collections show on their page; private ones don't
    s.post("/collections", "name=Favourites&public=yes", Some(&ann)).await;
    s.post("/collections", "name=Secret", Some(&ann)).await;
    let maker = s.get("/makers/anns", None).await.body;
    assert!(maker.contains("Favourites") && !maker.contains("Secret"), "{maker}");
    // Bea makes hers private, then deletes it; her Saved list can't be deleted
    s.post(&format!("/collections/{christmas}/edit"), "name=Christmas", Some(&bea))
        .await;
    assert_eq!(
        s.get(&format!("/collections/{christmas}"), None).await.status,
        StatusCode::NOT_FOUND
    );
    s.post(&format!("/collections/{christmas}/delete"), "", Some(&bea))
        .await;
    s.post(&format!("/collections/{saved}/delete"), "", Some(&bea)).await;
    let left: Vec<String> = sqlx::query_scalar("SELECT name FROM mf_collections WHERE account_id = 1001")
        .fetch_all(&s.pool)
        .await
        .unwrap();
    assert_eq!(left, ["Saved"]);
    let mine = s.get("/collections", Some(&bea)).await.body;
    assert!(mine.contains("Saved") && mine.contains("1 design"), "{mine}");
}

// ---------------------------------------------------------------- MF-11: makes

#[tokio::test]
async fn people_post_photos_of_what_they_made_and_can_take_them_down() {
    let Some(s) = site().await else { return };
    let ann = s.sign_up("ann%40example.com").await;
    let rose = published(&s, &ann, "anns", "Rose").await;
    let bea = s.sign_up("bea%40example.com").await;
    let cal = s.sign_up("cal%40example.com").await;
    assert!(
        s.get(&format!("/d/{rose}"), None)
            .await
            .body
            .contains("Sign in to post your make")
    );
    let make = |cookie: &str, confirm: &str, photo: Vec<u8>| {
        multipart(
            &format!("/d/{rose}/makes"),
            &[
                ("photo", "sewn.jpg", photo),
                ("note", "", "On linen, with <b>rayon</b> thread".as_bytes().to_vec()),
                ("confirm", "", confirm.as_bytes().to_vec()),
            ],
            cookie,
        )
    };
    // a maker profile first: it's the name shown
    assert!(
        s.send(make(&bea, "yes", photo_with_location()))
            .await
            .body
            .contains("maker profile first")
    );
    s.post("/account/maker", "name=Bea+Bakes&handle=bea", Some(&bea)).await;
    // her word that it's her own photo, and a real photo
    assert!(
        s.send(make(&bea, "", photo_with_location()))
            .await
            .body
            .contains("Confirm")
    );
    assert!(
        s.send(make(&bea, "yes", b"not a photo".to_vec()))
            .await
            .body
            .contains("The photo:")
    );
    assert_eq!(s.scalar("SELECT count(*) FROM mf_makes").await, 0);
    let stored = s.stored.lock().unwrap().len();
    let r = s.send(make(&bea, "yes", photo_with_location())).await;
    assert_eq!(r.location(), format!("/d/{rose}/rose"), "{}", r.body);
    assert_eq!(s.stored.lock().unwrap().len(), stored + 2);
    let mid: i64 = sqlx::query_scalar("SELECT id FROM mf_makes")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    // on the design's page, for everyone, the note escaped
    let page = s.get(&format!("/d/{rose}"), None).await.body;
    assert!(
        page.contains("1 make") && page.contains(&format!(r#"src="/makes/{mid}/thumb""#)),
        "{page}"
    );
    assert!(
        page.contains("On linen, with &lt;b&gt;rayon&lt;/b&gt; thread")
            && page.contains(r#"<a href="/makers/bea">Bea Bakes</a>"#)
    );
    let photo = s.get(&format!("/makes/{mid}/large"), None).await;
    assert_eq!(
        (photo.status, photo.headers[header::CONTENT_TYPE].to_str().unwrap()),
        (StatusCode::OK, "image/jpeg")
    );
    assert_eq!(
        s.get(&format!("/makes/{mid}/huge"), None).await.status,
        StatusCode::NOT_FOUND
    );
    // Cal can't take it down; Ann (the design's maker) can
    s.post(&format!("/makes/{mid}/delete"), "", Some(&cal)).await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_makes").await, 1);
    assert!(
        !s.get(&format!("/d/{rose}"), Some(&cal))
            .await
            .body
            .contains("/delete\"")
    );
    assert!(
        s.get(&format!("/d/{rose}"), Some(&ann))
            .await
            .body
            .contains(&format!("/makes/{mid}/delete"))
    );
    s.post(&format!("/makes/{mid}/delete"), "", Some(&ann)).await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_makes").await, 0);
    assert_eq!(s.stored.lock().unwrap().len(), stored);
    // Bea's own, taken down by her; an unpublished design's makes aren't shown
    s.send(make(&bea, "yes", photo_with_location())).await;
    let mid: i64 = sqlx::query_scalar("SELECT id FROM mf_makes")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    s.post(&format!("/designs/{rose}/unpublish"), "", Some(&ann)).await;
    assert_eq!(
        s.get(&format!("/makes/{mid}/thumb"), None).await.status,
        StatusCode::NOT_FOUND
    );
    s.post(&format!("/designs/{rose}/publish"), "confirm=yes", Some(&ann))
        .await;
    s.post(&format!("/makes/{mid}/delete"), "", Some(&bea)).await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_makes").await, 0);
    // a deleted design takes its makes' photos with it
    s.send(make(&bea, "yes", photo_with_location())).await;
    s.post(&format!("/designs/{rose}/unpublish"), "", Some(&ann)).await;
    let before = s.stored.lock().unwrap().len();
    s.post(&format!("/designs/{rose}/delete"), "", Some(&ann)).await;
    // the file, its photo and thumbnail, and the make's photo and thumbnail
    assert_eq!(s.stored.lock().unwrap().len(), before - 5);
}

#[tokio::test]
async fn a_reported_make_is_taken_down_from_the_admin_queue() {
    let Some(s) = site_with(vec![1003]).await else { return };
    let ann = s.sign_up("ann%40example.com").await;
    let rose = published(&s, &ann, "anns", "Rose").await;
    let bea = s.sign_up("bea%40example.com").await;
    let cal = s.sign_up("cal%40example.com").await;
    let ada = s.sign_up("ada%40example.com").await;
    s.post("/account/maker", "name=Bea&handle=bea", Some(&bea)).await;
    let post = || {
        multipart(
            &format!("/d/{rose}/makes"),
            &[
                ("photo", "a.jpg", photo_with_location()),
                ("confirm", "", b"yes".to_vec()),
            ],
            &bea,
        )
    };
    s.send(post()).await;
    let mid: i64 = sqlx::query_scalar("SELECT id FROM mf_makes")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    // others see a Report link; its poster sees Take down instead
    assert!(
        s.get(&format!("/d/{rose}"), Some(&cal))
            .await
            .body
            .contains(&format!("/makes/{mid}/report"))
    );
    assert!(
        !s.get(&format!("/d/{rose}"), Some(&bea))
            .await
            .body
            .contains(&format!("/makes/{mid}/report"))
    );
    assert!(
        s.get(&format!("/makes/{mid}/report"), None)
            .await
            .body
            .contains("Report a make")
    );
    let r = s.post(&format!("/makes/{mid}/report"), "details=x", None).await;
    assert!(r.body.contains("Choose what the problem is"), "{}", r.body);
    let r = s
        .post(
            &format!("/makes/{mid}/report"),
            "reason=not-theirs&details=My+photo+from+my+blog",
            None,
        )
        .await;
    assert!(r.body.contains("Your report is in"), "{}", r.body);
    // in the queue, with the photo and its own action
    let queue = s.get("/admin", Some(&ada)).await.body;
    assert!(
        queue.contains("It isn&#39;t their photo") && queue.contains(&format!(r#"src="/makes/{mid}/thumb""#)),
        "{queue}"
    );
    let rid: i64 = sqlx::query_scalar("SELECT id FROM mf_reports")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert!(queue.contains(&format!("/admin/reports/{rid}/remove-make")));
    // only admins act on it
    assert_eq!(
        s.post(&format!("/admin/reports/{rid}/remove-make"), "", Some(&cal))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(s.scalar("SELECT count(*) FROM mf_makes").await, 1);
    let stored = s.stored.lock().unwrap().len();
    s.post(&format!("/admin/reports/{rid}/remove-make"), "", Some(&ada))
        .await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_makes").await, 0);
    assert_eq!(s.stored.lock().unwrap().len(), stored - 2);
    assert_eq!(
        s.scalar("SELECT count(*) FROM mf_reports WHERE status = 'actioned'")
            .await,
        1
    );
    assert_eq!(
        s.scalar("SELECT count(*) FROM mf_moderation_log WHERE action = 'remove-make'")
            .await,
        1
    );
    // a report of a make its poster has since taken down: just closed
    s.send(post()).await;
    let mid: i64 = sqlx::query_scalar("SELECT id FROM mf_makes")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    s.post(&format!("/makes/{mid}/report"), "reason=spam", Some(&cal)).await;
    s.post(&format!("/makes/{mid}/delete"), "", Some(&bea)).await;
    let queue = s.get("/admin", Some(&ada)).await.body;
    assert!(
        queue.contains("taken down already") && !queue.contains("remove-make"),
        "{queue}"
    );
    assert_eq!(
        s.get(&format!("/makes/{mid}/report"), None).await.status,
        StatusCode::NOT_FOUND
    );
}

// ---------------------------------------------------------------- MF-13: following makers

#[tokio::test]
async fn following_a_maker_shows_their_new_designs() {
    let Some(s) = site().await else { return };
    let ann = s.sign_up("ann%40example.com").await;
    let rose = published(&s, &ann, "anns", "Rose").await;
    let cal = s.sign_up("cal%40example.com").await;
    let owl = published(&s, &cal, "cals", "Owl").await;
    let bea = s.sign_up("bea%40example.com").await;
    assert!(s.get("/makers/anns", None).await.body.contains("Sign in to follow"));
    assert!(
        s.get("/following", Some(&bea))
            .await
            .body
            .contains("don't follow any makers")
    );
    // Bea follows Ann (twice is once); Ann can't follow herself
    assert_eq!(
        s.post("/makers/anns/follow", "", Some(&bea)).await.location(),
        "/makers/anns"
    );
    s.post("/makers/anns/follow", "", Some(&bea)).await;
    s.post("/makers/anns/follow", "", Some(&ann)).await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_follows").await, 1);
    let page = s.get("/makers/anns", Some(&bea)).await.body;
    assert!(
        page.contains("1 follower") && page.contains("/makers/anns/unfollow"),
        "{page}"
    );
    assert!(!s.get("/makers/anns", Some(&ann)).await.body.contains("/follow\""));
    // her Following page: Ann's designs, not Cal's
    let feed = s.get("/following", Some(&bea)).await.body;
    assert!(
        feed.contains(&format!("/d/{rose}/")) && !feed.contains(&format!("/d/{owl}/")),
        "{feed}"
    );
    assert!(feed.contains(r#"<a href="/makers/anns">Maker</a>"#));
    assert_eq!(s.get("/following", None).await.location(), "/signin?next=/following");
    // a form from elsewhere, or a maker who isn't there, changes nothing
    let mut req = Request::post("/makers/cals/follow").header(header::HOST, HOST);
    req = req
        .header(header::ORIGIN, "https://evil.example")
        .header(header::COOKIE, &bea);
    assert_eq!(
        s.send(req.body(Body::empty()).unwrap()).await.status,
        StatusCode::FORBIDDEN
    );
    s.post("/makers/nobody-here/follow", "", Some(&bea)).await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_follows").await, 1);
    // unfollowed
    s.post("/makers/anns/unfollow", "", Some(&bea)).await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_follows").await, 0);
    assert!(
        s.get("/following", Some(&bea))
            .await
            .body
            .contains("don't follow any makers")
    );
}

// ---------------------------------------------------------------- MF-14: comments

/// A comment `text` (form-encoded) posted on design `id` by the person signed in with `cookie`.
async fn say(s: &Site, id: i64, cookie: &str, text: &str) -> Answer {
    s.post(&format!("/d/{id}/comments"), &format!("body={text}"), Some(cookie))
        .await
}

#[tokio::test]
async fn people_comment_on_designs_and_bad_comments_come_down() {
    let Some(s) = site_with(vec![1003]).await else { return };
    let ann = s.sign_up("ann%40example.com").await;
    let rose = published(&s, &ann, "anns", "Rose").await;
    let bea = s.sign_up("bea%40example.com").await;
    let cal = s.sign_up("cal%40example.com").await;
    let ada = s.sign_up("ada%40example.com").await;
    assert!(
        s.get(&format!("/d/{rose}"), None)
            .await
            .body
            .contains("Sign in to comment")
    );
    // a maker profile first; then plain text, links and tags included
    assert!(say(&s, rose, &bea, "Hello").await.body.contains("maker profile first"));
    s.post("/account/maker", "name=Bea&handle=bea", Some(&bea)).await;
    assert!(say(&s, rose, &bea, "+++").await.body.contains("Write something first"));
    let r = say(
        &s,
        rose,
        &bea,
        "Which+stabilizer%3F+See+%3Ca+href%3D%22https%3A%2F%2Fspam.example%22%3Ehere%3C%2Fa%3E",
    )
    .await;
    assert!(r.location().starts_with(&format!("/d/{rose}/rose#c")), "{}", r.body);
    let cid: i64 = sqlx::query_scalar("SELECT id FROM mf_comments")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    let page = s.get(&format!("/d/{rose}"), None).await.body;
    assert!(
        page.contains("1 comment")
            && page.contains("Which stabilizer? See &lt;a href=&quot;https://spam.example&quot;&gt;here&lt;/a&gt;"),
        "{page}"
    );
    assert!(!page.contains(r#"<a href="https://spam.example""#));
    // the maker answers, marked as the maker
    s.post("/account/maker", "name=Ann&handle=anns", Some(&ann)).await;
    say(&s, rose, &ann, "Tear-away+works+best").await;
    assert!(
        s.get(&format!("/d/{rose}"), None)
            .await
            .body
            .contains(r#"<span class="badge">the maker</span>"#)
    );
    // Cal can't take Bea's down, but can report it
    s.post(&format!("/comments/{cid}/delete"), "", Some(&cal)).await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_comments").await, 2);
    assert!(
        s.get(&format!("/d/{rose}"), Some(&cal))
            .await
            .body
            .contains(&format!("/comments/{cid}/report"))
    );
    assert!(
        s.post(&format!("/comments/{cid}/report"), "", Some(&cal))
            .await
            .body
            .contains("Choose what the problem is")
    );
    assert!(
        s.post(&format!("/comments/{cid}/report"), "reason=spam", Some(&cal))
            .await
            .body
            .contains("Your report is in")
    );
    let queue = s.get("/admin", Some(&ada)).await.body;
    let rid: i64 = sqlx::query_scalar("SELECT id FROM mf_reports")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert!(
        queue.contains("Which stabilizer?") && queue.contains(&format!("/admin/reports/{rid}/remove-comment")),
        "{queue}"
    );
    assert_eq!(
        s.post(&format!("/admin/reports/{rid}/remove-comment"), "", Some(&cal))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    s.post(&format!("/admin/reports/{rid}/remove-comment"), "", Some(&ada))
        .await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_comments").await, 1);
    assert_eq!(
        s.scalar("SELECT count(*) FROM mf_moderation_log WHERE action = 'remove-comment'")
            .await,
        1
    );
    // the design's maker takes down any comment on it; its author their own
    s.post("/account/maker", "name=Cal&handle=cal", Some(&cal)).await;
    say(&s, rose, &cal, "Nice").await;
    let cals: i64 = sqlx::query_scalar("SELECT id FROM mf_comments WHERE account_id = 1002")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    s.post(&format!("/comments/{cals}/delete"), "", Some(&ann)).await;
    let anns: i64 = sqlx::query_scalar("SELECT id FROM mf_comments WHERE account_id = 1000")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    s.post(&format!("/comments/{anns}/delete"), "", Some(&ann)).await;
    assert_eq!(s.scalar("SELECT count(*) FROM mf_comments").await, 0);
    // twenty an hour
    for _ in 0..makerfoundry::limits::COMMENT.max {
        say(&s, rose, &bea, "again").await;
    }
    assert!(say(&s, rose, &bea, "again").await.body.contains("commented a lot"));
}

// ---------------------------------------------------------------- MF-15: your data

#[tokio::test]
async fn a_maker_downloads_their_data_and_deletes_it_and_buyers_keep_their_designs() {
    let Some((s, st, sam, fox)) = selling_site(Vec::new()).await else {
        return;
    };
    // Bea buys the fox, and has a design of her own
    let bea = s.sign_up("bea%40example.com").await;
    s.post(&format!("/d/{fox}/buy"), "", Some(&bea)).await;
    let form = st.lock().unwrap().sessions["cs_test_1"].0.clone();
    s.stripe_event(
        WEBHOOK_SECRET,
        "evt_d",
        "checkout.session.completed",
        session_json("cs_test_1", &form, true),
    )
    .await;
    let cat = published(&s, &bea, "bea", "Cat").await;
    // Sam: a free design too, a collection, a follow, a comment and a make; Bea follows Sam
    let owl = published(&s, &sam, "sams-shop", "Owl").await;
    s.post("/collections", "name=Ideas&public=yes", Some(&sam)).await;
    s.post(&format!("/d/{cat}/save"), "", Some(&sam)).await;
    s.post("/makers/bea/follow", "", Some(&sam)).await;
    s.post("/makers/sams-shop/follow", "", Some(&bea)).await;
    say(&s, cat, &sam, "Lovely").await;
    s.send(multipart(
        &format!("/d/{cat}/makes"),
        &[
            ("photo", "a.jpg", photo_with_location()),
            ("confirm", "", b"yes".to_vec()),
        ],
        &sam,
    ))
    .await;
    // the copy
    assert!(
        s.get("/account/data", Some(&sam))
            .await
            .body
            .contains("Download a copy")
    );
    let r = s.get("/account/data.json", Some(&sam)).await;
    assert_eq!(
        r.headers[header::CONTENT_DISPOSITION],
        r#"attachment; filename="makerfoundry-data.json""#
    );
    let v: Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(v["account"]["email"], "sam@example.com");
    assert_eq!(v["maker_profile"][0]["handle"], "sams-shop");
    assert_eq!(v["designs"].as_array().unwrap().len(), 2);
    assert_eq!(v["sales"][0]["seller_cents"], 405);
    assert_eq!(v["comments"][0]["body"], "Lovely");
    assert_eq!(v["makes"].as_array().unwrap().len(), 1);
    assert_eq!(v["following"][0]["handle"], "bea");
    assert_eq!(v["collections"].as_array().unwrap().len(), 2);
    assert_eq!(
        s.get("/account/data.json", None).await.location(),
        "/signin?next=/account/data"
    );
    // deleting needs the word
    assert!(
        s.post("/account/data/delete", "confirm=yes", Some(&sam))
            .await
            .body
            .contains("Type delete")
    );
    let stored = s.stored.lock().unwrap().len();
    let r = s.post("/account/data/delete", "confirm=DELETE", Some(&sam)).await;
    assert!(r.body.contains("Your MakerFoundry data is deleted"), "{}", r.body);
    assert!(r.headers[header::SET_COOKIE].to_str().unwrap().contains("Max-Age=0"));
    assert_eq!(s.get("/account", Some(&sam)).await.location(), "/signin");
    // gone: the owl and its file and photos, the make's photos, the rest of Sam's records
    assert_eq!(s.stored.lock().unwrap().len(), stored - 5);
    for (sql, n) in [
        (format!("SELECT count(*) FROM mf_designs WHERE id = {owl}"), 0),
        ("SELECT count(*) FROM mf_makes".to_string(), 0),
        ("SELECT count(*) FROM mf_comments".to_string(), 0),
        (
            "SELECT count(*) FROM mf_collections WHERE account_id = 1000".to_string(),
            0,
        ),
        ("SELECT count(*) FROM mf_follows".to_string(), 0),
        ("SELECT count(*) FROM mf_makers WHERE account_id = 1000".to_string(), 0),
        (
            "SELECT count(*) FROM mf_sessions WHERE account_id = 1000".to_string(),
            0,
        ),
        (
            "SELECT count(*) FROM mf_accounts WHERE account_id = 1000 AND email = ''".to_string(),
            1,
        ),
        // kept: the sale, its ledger, and the fox, unpublished, for its buyer
        ("SELECT count(*) FROM mf_orders WHERE status = 'paid'".to_string(), 1),
        ("SELECT count(*) FROM mf_ledger".to_string(), 3),
        (
            format!("SELECT count(*) FROM mf_designs WHERE id = {fox} AND status = 'draft'"),
            1,
        ),
    ] {
        assert_eq!(s.scalar(&sql).await, n, "{sql}");
    }
    assert!(
        s.get(&format!("/d/{fox}/download"), Some(&bea))
            .await
            .location()
            .contains("/files/")
    );
    assert!(
        s.get("/library", Some(&bea))
            .await
            .body
            .contains(&format!("/d/{fox}/download"))
    );
    // Bea's own things are untouched
    assert!(s.get(&format!("/d/{cat}"), None).await.status == StatusCode::OK);
}

// ---------------------------------------------------------------- everything people type is escaped

#[tokio::test]
async fn hostile_text_in_every_field_never_reaches_a_page_as_html() {
    let Some(s) = site_with(vec![1002]).await else { return };
    const X: &str = "%3Cscript%3Ex()%3C%2Fscript%3E%22%27";
    let eve = s.sign_up("eve%40example.com").await;
    let bob = s.sign_up("bob%40example.com").await;
    let ada = s.sign_up("ada%40example.com").await;
    // a design whose file name, title, description, tags, maker name and bio are all script
    let r = s
        .send(multipart(
            "/designs/new",
            &[
                ("file", "<script>x()</script>.pes", sample(sf_formats::Format::Pes)),
                ("photos", "p.jpg", photo_with_location()),
            ],
            &eve,
        ))
        .await;
    let id = design_id(r.location());
    s.post("/account/maker", &format!("name={X}&handle=eve&bio={X}"), Some(&eve))
        .await;
    s.post(
        &format!("/designs/{id}/edit"),
        &format!("title={X}&description={X}&tags={X}&licence=personal&price=0"),
        Some(&eve),
    )
    .await;
    s.post(&format!("/designs/{id}/publish"), "confirm=yes", Some(&eve))
        .await;
    let r = s
        .send(multipart(
            &format!("/designs/{id}/files"),
            &[("file", "<script>.dst", sample(sf_formats::Format::Dst))],
            &eve,
        ))
        .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.body);
    // a collection, a comment, a make and reports, all script
    s.post("/collections", &format!("name={X}&public=yes"), Some(&eve))
        .await;
    let cid: i64 = sqlx::query_scalar("SELECT id FROM mf_collections")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    s.post(&format!("/d/{id}/save"), &format!("collection={cid}"), Some(&eve))
        .await;
    s.post("/account/maker", &format!("name={X}&handle=bob"), Some(&bob))
        .await;
    say(&s, id, &bob, X).await;
    s.send(multipart(
        &format!("/d/{id}/makes"),
        &[
            ("photo", "a.jpg", photo_with_location()),
            ("note", "", b"<script>x()</script>".to_vec()),
            ("confirm", "", b"yes".to_vec()),
        ],
        &bob,
    ))
    .await;
    let mid: i64 = sqlx::query_scalar("SELECT id FROM mf_makes")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    let comment: i64 = sqlx::query_scalar("SELECT id FROM mf_comments")
        .fetch_one(&s.pool)
        .await
        .unwrap();
    s.post(
        &format!("/d/{id}/report"),
        &format!("reason=other&details={X}&email={X}"),
        None,
    )
    .await;
    s.post(
        &format!("/makes/{mid}/report"),
        &format!("reason=other&details={X}"),
        None,
    )
    .await;
    s.post(
        &format!("/comments/{comment}/report"),
        &format!("reason=other&details={X}"),
        None,
    )
    .await;
    s.post(
        "/dmca/notice",
        &format!("name={X}&email=a%40b.c&address={X}&phone={X}&original={X}&designs=%2Fd%2F{id}&good_faith=yes&accurate=yes&signature={X}"),
        None,
    )
    .await;
    // a report of the design, the make and the comment, and a copyright notice
    assert_eq!(s.scalar("SELECT count(*) FROM mf_reports").await, 4);
    // forms sent back with a problem show what was typed, escaped
    let again = s
        .post(
            &format!("/designs/{id}/edit"),
            &format!("title={X}&licence=nope"),
            Some(&eve),
        )
        .await;
    let signup = s.post("/signup", &format!("email={X}&password=a&again=b"), None).await;
    let mut pages = vec![again.body, signup.body];
    for path in [
        "/".to_string(),
        "/browse".into(),
        format!("/browse?q={X}&tag={X}"),
        format!("/d/{id}"),
        format!("/designs/{id}"),
        format!("/designs/{id}/edit"),
        "/designs".into(),
        "/makers/eve".into(),
        format!("/collections/{cid}"),
        "/collections".into(),
        format!("/makes/{mid}/report"),
        format!("/comments/{comment}/report"),
        "/admin".into(),
        "/account".into(),
        "/following".into(),
        "/library".into(),
        "/account/sales".into(),
    ] {
        for who in [None, Some(&eve), Some(&bob), Some(&ada)] {
            pages.push(s.get(&path, who.map(String::as_str)).await.body);
        }
    }
    let raw = pages.iter().filter(|p| p.contains("<script")).count();
    assert_eq!(raw, 0, "{}", pages.iter().find(|p| p.contains("<script")).unwrap());
    // and it was all there, escaped
    assert!(
        pages
            .iter()
            .any(|p| p.contains("&lt;script&gt;x()&lt;/script&gt;&quot;&#39;"))
    );
}

// ---------------------------------------------------------------- MF-16: the admins' overview

#[tokio::test]
async fn admins_see_the_sites_numbers_and_nobody_else_does() {
    let Some((s, st, _sam, fox)) = selling_site(vec![1002]).await else {
        return;
    };
    let bea = s.sign_up("bea%40example.com").await;
    let ada = s.sign_up("ada%40example.com").await;
    s.post(&format!("/d/{fox}/buy"), "", Some(&bea)).await;
    let form = st.lock().unwrap().sessions["cs_test_1"].0.clone();
    s.stripe_event(
        WEBHOOK_SECRET,
        "evt_o",
        "checkout.session.completed",
        session_json("cs_test_1", &form, true),
    )
    .await;
    s.post(&format!("/d/{fox}/report"), "reason=spam", None).await;
    assert_eq!(s.get("/admin/overview", Some(&bea)).await.status, StatusCode::NOT_FOUND);
    assert_eq!(s.get("/admin/overview", None).await.status, StatusCode::NOT_FOUND);
    let page = s.get("/admin/overview", Some(&ada)).await.body;
    for fact in [
        "<dt>Accounts</dt><dd>3</dd>",
        "<dt>Published</dt><dd>1</dd>",
        "<dt>Paid</dt><dd>1</dd>",
        "<dt>Sales</dt><dd>1</dd>",
        "<dt>Paid by buyers</dt><dd>$5</dd>",
        "<dt>To sellers</dt><dd>$4.05</dd>",
        "<dt>MakerFoundry&#39;s fees</dt><dd>$0.50</dd>",
        "<dt>Card fees</dt><dd>$0.45</dd>",
        "<dt>Fees this month</dt><dd>$0.50</dd>",
        "<dt>Reports</dt><dd>1</dd>",
    ] {
        assert!(page.contains(fact), "{fact} in {page}");
    }
}

/// Open in StitchFoundry: a free, published embroidery design's page links to StitchFoundry's web
/// app with its file's address, and the web app gets the file itself, with CORS; nothing that isn't
/// free and public is sent, whoever asks.
#[tokio::test]
async fn a_free_design_opens_in_stitchfoundrys_web_app() {
    let Some(s) = site().await else { return };
    let cookie = s.sign_up("ann%40example.com").await;
    let id = published(&s, &cookie, "anns-shop", "Rose").await;
    let page = s.get(&format!("/d/{id}/rose"), None).await;
    let file = format!("https%3A%2F%2Fmakerfoundry.test%2Fapi%2Fv1%2Fdesigns%2F{id}%2Ffile%2Frose.pes");
    assert!(
        page.body.contains(&format!(
            r#"href="https://stitchwright.github.io/StitchFoundry/?open={file}""#
        )),
        "{}",
        page.body
    );
    assert!(page.body.contains("Open in StitchFoundry"));
    // the web app's fetch: the file, from the web app's site
    let path = format!("/api/v1/designs/{id}/file/rose.pes");
    let r = s
        .api("GET", &path, None, Some("https://stitchfoundry.test"), &json!({}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.body);
    assert!(r.body.starts_with("#PES"), "a PES file");
    assert_eq!(
        r.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://stitchfoundry.test"
    );
    assert_eq!(
        r.headers[header::CONTENT_DISPOSITION],
        r#"attachment; filename="rose.pes""#
    );
    assert_eq!(r.headers[header::ACCESS_CONTROL_EXPOSE_HEADERS], "content-disposition");
    assert!(r.headers.get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none());
    assert_eq!(
        s.scalar(&format!("SELECT downloads::bigint FROM mf_designs WHERE id = {id}"))
            .await,
        1
    );
    // another site's page gets the file but may not read it
    let evil = s
        .api("GET", &path, None, Some("https://evil.example"), &json!({}))
        .await;
    assert!(evil.headers.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    // a draft isn't sent, even to its maker (cookies aren't read), nor a design that isn't there
    let draft = add_design(&s, &cookie, sf_formats::Format::Pes, "tulip.pes").await;
    let mut req = Request::get(format!("/api/v1/designs/{draft}/file/tulip.pes")).header(header::HOST, HOST);
    req = req.header(header::COOKIE, &cookie);
    let r = s.send(req.body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "{}", r.body);
    assert!(r.body.contains("isn't on MakerFoundry"), "{}", r.body);
    assert_eq!(
        s.api("GET", "/api/v1/designs/999999/file/x.pes", None, None, &json!({}))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    // a draft's page has no button
    assert!(
        !s.get(&format!("/designs/{draft}"), Some(&cookie))
            .await
            .body
            .contains("Open in StitchFoundry")
    );
}

/// A design for sale isn't opened in StitchFoundry: it's bought on MakerFoundry first.
#[tokio::test]
async fn a_design_for_sale_isnt_sent_to_stitchfoundry() {
    let Some((s, _st, seller, id)) = selling_site(vec![]).await else {
        return;
    };
    let r = s
        .api(
            "GET",
            &format!("/api/v1/designs/{id}/file/fox-patch.pes"),
            None,
            Some("https://stitchfoundry.test"),
            &json!({}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.body);
    assert!(r.body.contains("for sale"), "{}", r.body);
    assert_eq!(
        r.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://stitchfoundry.test"
    );
    // its maker opens it through a link made for them (see the buyers' test)
    let page = s.get(&format!("/d/{id}/fox-patch"), Some(&seller)).await;
    assert!(page.body.contains(&format!(r#"href="/d/{id}/open""#)), "{}", page.body);
    assert!(
        !s.get(&format!("/d/{id}/fox-patch"), None)
            .await
            .body
            .contains("Open in StitchFoundry")
    );
}

/// Open in StitchFoundry for a paid design: only its buyers (and maker) get the button, which
/// makes a ten-minute link to that design's file; the web app's fetch with it gets the file.
#[tokio::test]
async fn a_paid_design_opens_in_stitchfoundry_for_its_buyers() {
    let Some((s, st, _seller, id)) = selling_site(Vec::new()).await else {
        return;
    };
    let bea = s.sign_up("bea%40example.com").await;
    let cal = s.sign_up("cal%40example.com").await;
    // not bought: no button, and the open route sends them back to the page
    assert!(
        !s.get(&format!("/d/{id}"), Some(&bea))
            .await
            .body
            .contains("Open in StitchFoundry")
    );
    assert_eq!(
        s.get(&format!("/d/{id}/open"), Some(&bea)).await.location(),
        format!("/d/{id}")
    );
    assert_eq!(
        s.get(&format!("/d/{id}/open"), None).await.location(),
        format!("/signin?next=/d/{id}")
    );
    assert_eq!(s.scalar("SELECT count(*) FROM mf_open_links").await, 0);
    // bought
    s.post(&format!("/d/{id}/buy"), "", Some(&bea)).await;
    let form = st.lock().unwrap().sessions["cs_test_1"].0.clone();
    s.stripe_event(
        WEBHOOK_SECRET,
        "evt_op",
        "checkout.session.completed",
        session_json("cs_test_1", &form, true),
    )
    .await;
    assert!(
        s.get(&format!("/d/{id}"), Some(&bea))
            .await
            .body
            .contains(&format!(r#"href="/d/{id}/open""#))
    );
    let r = s.get(&format!("/d/{id}/open"), Some(&bea)).await;
    let prefix = format!(
        "https://stitchwright.github.io/StitchFoundry/?open=https%3A%2F%2Fmakerfoundry.test%2Fapi%2Fv1%2Fdesigns%2F{id}%2Ffile%2Ffox-patch.pes%3Ft%3D"
    );
    assert!(r.location().starts_with(&prefix), "{}", r.location());
    let token = r.location().trim_start_matches(&prefix).to_string();
    assert_eq!(token.len(), 48);
    // the web app's fetch with the link: the file
    let path = format!("/api/v1/designs/{id}/file/fox-patch.pes");
    let ok = s
        .api(
            "GET",
            &format!("{path}?t={token}"),
            None,
            Some("https://stitchfoundry.test"),
            &json!({}),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.body);
    assert!(ok.body.starts_with("#PES"));
    // without it, with a wrong one, or for another design: refused
    for bad in [String::new(), "0".repeat(48), "nothex".into()] {
        let r = s.api("GET", &format!("{path}?t={bad}"), None, None, &json!({})).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{bad}: {}", r.body);
    }
    let other = published(&s, &cal, "cals", "Owl").await;
    let r = s
        .api(
            "GET",
            &format!("/api/v1/designs/{other}/file/owl.pes?t={token}"),
            None,
            None,
            &json!({}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "a free design is anyone's anyway");
    sqlx::query("UPDATE mf_designs SET price_cents = 300 WHERE id = $1")
        .bind(other)
        .execute(&s.pool)
        .await
        .unwrap();
    let r = s
        .api(
            "GET",
            &format!("/api/v1/designs/{other}/file/owl.pes?t={token}"),
            None,
            None,
            &json!({}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "the link is only for its design");
    // ten minutes on, it's gone
    sqlx::query("UPDATE mf_open_links SET expires_at = now() - interval '1 second'")
        .execute(&s.pool)
        .await
        .unwrap();
    let r = s.api("GET", &format!("{path}?t={token}"), None, None, &json!({})).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(makerfoundry::api::forget_old_links(&s.pool).await.unwrap(), 1);
}
