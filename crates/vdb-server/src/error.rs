//! Server error types.

use thiserror::Error;
use vdb::VerityError;
use vdb_wire::WireError;

/// Result type for server operations.
pub type ServerResult<T> = Result<T, ServerError>;

/// Errors that can occur during server operations.
#[derive(Debug, Error)]
pub enum ServerError {
    /// Wire protocol error.
    #[error("wire protocol error: {0}")]
    Wire(#[from] WireError),

    /// Database error.
    #[error("database error: {0}")]
    Database(#[from] VerityError),

    /// I/O error.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// Connection closed.
    #[error("connection closed")]
    ConnectionClosed,

    /// Maximum connections reached.
    #[error("maximum connections reached: {0}")]
    MaxConnectionsReached(usize),

    /// Invalid tenant ID.
    #[error("invalid tenant ID")]
    InvalidTenant,

    /// Bind failed.
    #[error("failed to bind to {addr}: {source}")]
    BindFailed {
        addr: std::net::SocketAddr,
        source: std::io::Error,
    },

    /// TLS error.
    #[error("TLS error: {0}")]
    Tls(String),

    /// Authentication failed.
    #[error("unauthorized: {0}")]
    Unauthorized(String),

    /// Server shutdown.
    #[error("server shutdown")]
    Shutdown,

    /// Replication error.
    #[error("replication error: {0}")]
    Replication(String),
}
