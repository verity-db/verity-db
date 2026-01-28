//! # vdb-server: `VerityDB` server daemon
//!
//! This crate provides the TCP server that exposes `VerityDB` over the network
//! using the binary wire protocol defined in `vdb-wire`.
//!
//! ## Architecture
//!
//! The server uses `mio` for non-blocking I/O with a poll-based event loop.
//! This follows the project's design principle of explicit control flow
//! without async runtimes.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                      vdb-server                          │
//! │  ┌─────────────┐   ┌─────────────┐   ┌───────────────┐  │
//! │  │  Listener   │ → │ Connections │ → │  RequestRouter │  │
//! │  │  (TCP)      │   │ (mio poll)  │   │  (→ Verity)   │  │
//! │  └─────────────┘   └─────────────┘   └───────────────┘  │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! use vdb_server::{Server, ServerConfig};
//! use vdb::Verity;
//!
//! let db = Verity::open("./data")?;
//! let config = ServerConfig::new("127.0.0.1:5432");
//! let server = Server::new(config, db)?;
//! server.run()?;
//! ```

pub mod auth;
mod config;
mod connection;
mod error;
mod handler;
pub mod health;
pub mod metrics;
pub mod replication;
mod server;
#[cfg(test)]
mod tests;
pub mod tls;

pub use auth::{ApiKeyConfig, AuthMode, AuthService, AuthenticatedIdentity, JwtConfig};
pub use config::{RateLimitConfig, ReplicationMode, ServerConfig};
pub use error::{ServerError, ServerResult};
pub use health::{HealthChecker, HealthResponse, HealthStatus};
pub use replication::{CommandSubmitter, ReplicationStatus, SubmissionResult};
pub use server::Server;
pub use tls::TlsConfig;
