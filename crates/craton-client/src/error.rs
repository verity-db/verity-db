//! Client error types.

use thiserror::Error;
use craton_wire::{ErrorCode, WireError};

/// Result type for client operations.
pub type ClientResult<T> = Result<T, ClientError>;

/// Errors that can occur during client operations.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Connection error.
    #[error("connection error: {0}")]
    Connection(#[from] std::io::Error),

    /// Wire protocol error.
    #[error("wire protocol error: {0}")]
    Wire(#[from] WireError),

    /// Server returned an error response.
    #[error("server error ({code:?}): {message}")]
    Server { code: ErrorCode, message: String },

    /// Connection not established.
    #[error("not connected to server")]
    NotConnected,

    /// Response ID mismatch.
    #[error("response ID {received} does not match request ID {expected}")]
    ResponseMismatch { expected: u64, received: u64 },

    /// Unexpected response type.
    #[error("unexpected response type: expected {expected}, got {actual}")]
    UnexpectedResponse { expected: String, actual: String },

    /// Connection timeout.
    #[error("connection timeout")]
    Timeout,

    /// Handshake failed.
    #[error("handshake failed: {0}")]
    HandshakeFailed(String),
}

impl ClientError {
    /// Creates a server error from an error code and message.
    pub fn server(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Server {
            code,
            message: message.into(),
        }
    }
}
