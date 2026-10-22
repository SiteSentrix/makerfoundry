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

/// Deletes sessions unused for `SESSION_DAYS` (they no longer work; this forgets them too).
pub async fn forget_old_sessions(pool: &PgPool) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM mf_sessions WHERE last_seen_at < now() - make_interval(days => $1)")
        .bind(SESSION_DAYS)
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
}

/// Ends every session made before `unix` (seconds) → how many: after a breach, everyone signs in
/// again (`MF_END_SESSIONS_BEFORE`).
pub async fn end_sessions_before(pool: &PgPool, unix: i64) -> Result<u64, sqlx::Error> {
    sqlx::query("DELETE FROM mf_sessions WHERE created_at < to_timestamp($1)")
        .bind(unix as f64)
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
}

/// `text` as Unix seconds: digits, or a UTC date `YYYY-MM-DD` with an optional `THH:MM[:SS]` and
/// `Z`.
pub fn parse_time(text: &str) -> Option<i64> {
    let t = text.trim();
    if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
        return t.parse().ok();
    }
    let t = t.strip_suffix('Z').unwrap_or(t);
    let (date, time) = t.split_once('T').unwrap_or((t, "00:00:00"));
    let d: Vec<i64> = date.split('-').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let h: Vec<i64> = time.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let ([y, m, day], [hh, mm, rest @ ..]) = (d.as_slice(), h.as_slice()) else {
        return None;
    };
    let ss = rest.first().copied().unwrap_or(0);
    if !((1970..=9999).contains(y)
        && (1..=12).contains(m)
        && (1..=31).contains(day)
        && (0..24).contains(hh)
        && (0..60).contains(mm)
        && (0..61).contains(&ss))
    {
        return None;
    }
    // days from 1970-01-01 to the date (Howard Hinnant's days_from_civil)
    let (y, m) = if *m <= 2 { (y - 1, m + 9) } else { (*y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// Ends a session → the account service's token it held, to end that one too.
pub async fn end_session(pool: &PgPool, id_hash: &str) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query("DELETE FROM mf_sessions WHERE id_hash = $1 RETURNING cloud_token")
        .bind(id_hash)
        .fetch_optional(pool)
        .await?;
    row.map(|r| r.try_get("cloud_token")).transpose()
}
