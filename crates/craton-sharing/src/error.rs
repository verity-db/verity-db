//! Error types for data sharing operations.

use thiserror::Error;

/// Result type for sharing operations.
pub type SharingResult<T> = Result<T, SharingError>;

/// Errors that can occur during data sharing operations.
#[derive(Debug, Error)]
pub enum SharingError {
    /// Token has expired.
    #[error("access token expired")]
    TokenExpired,

    /// Token has been revoked.
    #[error("access token revoked")]
    TokenRevoked,

    /// Token not found.
    #[error("access token not found")]
    TokenNotFound,

    /// Scope violation - requested data outside allowed scope.
    #[error("scope violation: {0}")]
    ScopeViolation(String),

    /// Invalid transformation rule.
    #[error("invalid transformation rule: {0}")]
    InvalidTransformation(String),

    /// Consent not found for the operation.
    #[error("consent not found for operation: {0}")]
    ConsentNotFound(String),

    /// Consent has been withdrawn.
    #[error("consent withdrawn: {0}")]
    ConsentWithdrawn(String),

    /// Export generation failed.
    #[error("export failed: {0}")]
    ExportFailed(String),

    /// Serialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Cryptographic operation failed.
    #[error("crypto error: {0}")]
    Crypto(String),
}
