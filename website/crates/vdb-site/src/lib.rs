//! `VerityDB` Website Library
//!
//! Core library for the `VerityDB` marketing website.

/// Build version for cache busting (git commit hash or timestamp).
pub const BUILD_VERSION: &str = env!("BUILD_VERSION");

pub mod content;
#[cfg(debug_assertions)]
pub mod dev_tools;
pub mod handlers;
pub mod router;
pub mod state;
pub mod templates;

use std::net::SocketAddr;

use tokio::net::TcpListener;
use tracing::info;

use crate::{router::create_router, state::AppState};

/// Run the website server.
///
/// The server address can be configured via environment variables:
/// - `HOST`: The host to bind to (default: "127.0.0.1")
/// - `PORT`: The port to bind to (default: "3000")
pub async fn run() {
    let state = AppState::new();

    #[cfg(debug_assertions)]
    let state = {
        let state_with_reloader = state.with_reloader();
        dev_tools::spawn_file_watcher(state_with_reloader.clone());
        state_with_reloader
    };

    let app = create_router(state);

    let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".into())
        .parse()
        .expect("PORT must be a valid u16");
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .expect("HOST:PORT must be a valid socket address");
    let listener = TcpListener::bind(addr).await.expect("failed to bind to address");

    info!("Listening on http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
    info!("Shutting down gracefully...");
}
