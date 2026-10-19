//! Runs MakerFoundry's web service. Settings come from the environment (docs/deploy.md):
//! `PORT` (default 8080), which Railway sets; `DATABASE_URL`, MakerFoundry's Postgres (without it
//! nobody can sign in); `STITCHFOUNDRY_CLOUD_URL`, the account service (default
//! `https://cloud.stitchfoundry.app`).

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::process::ExitCode;

const DEFAULT_PORT: u16 = 8080;

#[tokio::main]
async fn main() -> ExitCode {
    let port = match std::env::var("PORT") {
        Ok(text) => match text.trim().parse::<u16>() {
            Ok(port) => port,
            Err(_) => {
                eprintln!("PORT must be a port number (1-65535), got {text:?}");
                return ExitCode::FAILURE;
            }
        },
        Err(_) => DEFAULT_PORT,
    };
    // `::` takes IPv4 too on Linux, and Railway's private network is IPv6. Hosts without IPv6
    // get IPv4 only.
    let ipv6 = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    let ipv4 = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    let (listener, addr) = match tokio::net::TcpListener::bind(ipv6).await {
        Ok(listener) => (listener, ipv6),
        Err(err6) => match tokio::net::TcpListener::bind(ipv4).await {
            Ok(listener) => (listener, ipv4),
            Err(err4) => {
                eprintln!("can't listen on {ipv6} ({err6}) or {ipv4} ({err4})");
                return ExitCode::FAILURE;
            }
        },
    };
    let cloud_url =
        std::env::var("STITCHFOUNDRY_CLOUD_URL").unwrap_or_else(|_| makerfoundry::cloud::DEFAULT_URL.to_string());
    let cloud = match makerfoundry::cloud::Cloud::new(&cloud_url) {
        Ok(cloud) => cloud,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let db = match std::env::var("DATABASE_URL").ok().filter(|u| !u.trim().is_empty()) {
        Some(url) => match makerfoundry::db::connect(&url).await {
            Ok(pool) => Some(pool),
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
        },
        None => {
            eprintln!("no DATABASE_URL: the site runs, but nobody can sign in");
            None
        }
    };
    eprintln!(
        "makerfoundry {} listening on {addr}; accounts at {cloud_url}",
        makerfoundry::VERSION
    );
    let storage = match makerfoundry::storage::Storage::from_env(|k| std::env::var(k).ok()) {
        Ok(storage) => storage,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    if storage.is_none() {
        eprintln!("no R2 settings: designs can be previewed but not kept");
    }
    let publishers = makerfoundry::listings::Publishers::from_setting(std::env::var("MF_PUBLISHERS").ok().as_deref());
    let admins: Vec<i64> = std::env::var("MF_ADMINS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let limits = std::sync::Arc::new(makerfoundry::limits::Limits::default());
    let state = makerfoundry::AppState {
        cloud,
        db,
        storage,
        publishers,
        admins,
        limits,
    };
    // every 10 minutes: forget rate-limit entries over a day old and sessions past their 30 days
    // (what the privacy policy says is kept, and for how long)
    let (sweep_limits, sweep_db) = (state.limits.clone(), state.db.clone());
    tokio::spawn(async move {
        let mut every = tokio::time::interval(std::time::Duration::from_secs(600));
        loop {
            every.tick().await;
            sweep_limits.sweep();
            if let Some(pool) = &sweep_db
                && let Err(e) = makerfoundry::db::forget_old_sessions(pool).await
            {
                eprintln!("sessions: {e}");
            }
        }
    });
    match axum::serve(listener, makerfoundry::app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("server stopped: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Resolves on Ctrl-C or SIGTERM (what Railway sends on a redeploy), so requests in flight finish.
async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    eprintln!("shutting down");
}
