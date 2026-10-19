//! MakerFoundry's own records in Postgres (`DATABASE_URL`): who has signed in and their sessions
//! (MF-1); listings and sales come later. Tables are made by the migrations in `migrations/`,
//! run when the service starts.

use sqlx::Row;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::cloud::Account;

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// A session unused this long ends (days).
pub const SESSION_DAYS: i32 = 30;
/// How often a session is checked with the account service (minutes), so that one ended there (a
/// password reset, signing out in the app) ends here soon after.
pub const CHECK_MINUTES: i32 = 15;

/// The database at `url`, its tables brought up to date.
pub async fn connect(url: &str) -> Result<PgPool, String> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(url)
        .await
        .map_err(|e| format!("can't reach the database: {e}"))?;
    MIGRATOR
        .run(&pool)
        .await
        .map_err(|e| format!("can't update the database's tables: {e}"))?;
    Ok(pool)
}

/// A signed-in session, as kept.
pub struct Session {
    pub account_id: i64,
    pub email: String,
    pub cloud_token: String,
    /// Due a check with the account service.
    pub stale: bool,
}

/// Notes that `account` signed in (its email may have changed since).
pub async fn remember_account(pool: &PgPool, account: &Account) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mf_accounts (account_id, email) VALUES ($1, $2)
         ON CONFLICT (account_id) DO UPDATE SET email = EXCLUDED.email, last_seen_at = now()",
    )
    .bind(account.id)
    .bind(&account.email)
    .execute(pool)
    .await
    .map(|_| ())
}

pub async fn create_session(
    pool: &PgPool,
    id_hash: &str,
    account_id: i64,
    cloud_token: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO mf_sessions (id_hash, account_id, cloud_token) VALUES ($1, $2, $3)")
        .bind(id_hash)
        .bind(account_id)
        .bind(cloud_token)
        .execute(pool)
        .await
        .map(|_| ())
}

/// The session `id_hash` stands for, while it lasts; using it keeps it going.
pub async fn session(pool: &PgPool, id_hash: &str) -> Result<Option<Session>, sqlx::Error> {
    let row = sqlx::query(
        "UPDATE mf_sessions s SET last_seen_at = now()
         FROM mf_accounts a
         WHERE s.id_hash = $1 AND a.account_id = s.account_id
           AND s.last_seen_at > now() - make_interval(days => $2)
         RETURNING s.account_id, a.email, s.cloud_token, s.checked_at < now() - make_interval(mins => $3) AS stale",
    )
    .bind(id_hash)
    .bind(SESSION_DAYS)
    .bind(CHECK_MINUTES)
    .fetch_optional(pool)
    .await?;
    row.map(|r| {
        Ok(Session {
            account_id: r.try_get("account_id")?,
            email: r.try_get("email")?,
            cloud_token: r.try_get("cloud_token")?,
            stale: r.try_get("stale")?,
        })
    })
    .transpose()
}

/// The account service still knows the session: note when, and the account's email as it is now.
pub async fn checked(pool: &PgPool, id_hash: &str, account: &Account) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mf_sessions SET checked_at = now() WHERE id_hash = $1")
        .bind(id_hash)
        .execute(pool)
        .await?;
    remember_account(pool, account).await
}

/// Ends a session → the account service's token it held, to end that one too.
pub async fn end_session(pool: &PgPool, id_hash: &str) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query("DELETE FROM mf_sessions WHERE id_hash = $1 RETURNING cloud_token")
        .bind(id_hash)
        .fetch_optional(pool)
        .await?;
    row.map(|r| r.try_get("cloud_token")).transpose()
}
