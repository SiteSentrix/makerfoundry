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
                Json(json!({"sent": true, "message": "If there's a StitchFoundry account with that email, a link to reset its password is on its way. It works for an hour."}))
            }),
        )
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), shared)
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
}

async fn site() -> Option<Site> {
    let pool = test_db().await?;
    let (url, fake) = fake_cloud().await;
    let app = makerfoundry::app(AppState {
        cloud: Cloud::new(&url).unwrap(),
        db: Some(pool.clone()),
    });
    Some(Site { app, pool, fake })
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
        let body = String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
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
    assert!(s.get("/signup", None).await.body.contains("StitchFoundry account"));
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
    assert!(s.get("/", Some(&cookie)).await.body.contains("Your account"));
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
    assert!(r.body.contains("already a StitchFoundry account"));
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
    });
    let s = Site {
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
    });
    let req = Request::get("/signin")
        .header(header::HOST, HOST)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(body.contains("isn&#39;t available yet"));
}
