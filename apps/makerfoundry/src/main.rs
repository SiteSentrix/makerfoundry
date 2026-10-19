//! Runs MakerFoundry's web service. Settings come from the environment (docs/deploy.md):
//! `PORT` (default 8080), which Railway sets.

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
    eprintln!("makerfoundry {} listening on {addr}", makerfoundry::VERSION);
    match axum::serve(listener, makerfoundry::app())
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
