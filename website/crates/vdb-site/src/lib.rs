//! `VerityDB` Website Library
//!
//! Core library for the `VerityDB` marketing website.

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
pub async fn run() {
    let state = AppState::new();

    #[cfg(debug_assertions)]
    let state = {
        let state_with_reloader = state.with_reloader();
        dev_tools::spawn_file_watcher(state_with_reloader.clone());
        state_with_reloader
    };

    let app = create_router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
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
