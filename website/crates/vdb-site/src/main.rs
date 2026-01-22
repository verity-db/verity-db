//! `VerityDB` Website
//!
//! Marketing website for `VerityDB` - a compliance-first, verifiable database.

use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "vdb_site=debug,tower_http=debug".parse().expect("valid filter")))
        .with(fmt::layer())
        .init();

    tracing::info!("Starting VerityDB website server");

    vdb_site::run().await;
}
