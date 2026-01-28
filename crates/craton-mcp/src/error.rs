//! MCP server error types.

use thiserror::Error;

/// Result type for MCP operations.
pub type McpResult<T> = Result<T, McpError>;

/// Errors that can occur during MCP operations.
#[derive(Debug, Error)]
pub enum McpError {
    /// Invalid or missing access token.
    #[error("authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Token has expired.
    #[error("token expired")]
    TokenExpired,

    /// Token has been revoked.
    #[error("token revoked")]
    TokenRevoked,

    /// Token usage limit exceeded.
    #[error("token usage limit exceeded")]
    UsageLimitExceeded,

    /// Access denied due to scope restrictions.
    #[error("scope violation: {0}")]
    ScopeViolation(String),

    /// Query validation failed (potential data exfiltration).
    #[error("query rejected: {0}")]
    QueryRejected(String),

    /// Rate limit exceeded.
    #[error("rate limit exceeded")]
    RateLimitExceeded,

    /// Invalid tool parameters.
    #[error("invalid parameters: {0}")]
    InvalidParameters(String),

    /// Unknown tool requested.
    #[error("unknown tool: {0}")]
    UnknownTool(String),

    /// Database error.
    #[error("database error: {0}")]
    Database(#[from] craton::CratonError),

    /// Query parsing or execution error.
    #[error("query error: {0}")]
    Query(String),

    /// Serialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Internal server error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl McpError {
    /// Returns the JSON-RPC error code for this error.
    ///
    /// Error codes follow JSON-RPC 2.0 conventions:
    /// - -32700: Parse error
    /// - -32600: Invalid request
    /// - -32601: Method not found
    /// - -32602: Invalid params
    /// - -32603: Internal error
    /// - -32000 to -32099: Server error (reserved for implementation-defined errors)
    pub fn error_code(&self) -> i32 {
        match self {
            Self::AuthenticationFailed(_) => -32001,
            Self::TokenExpired => -32002,
            Self::TokenRevoked => -32003,
            Self::UsageLimitExceeded => -32004,
            Self::ScopeViolation(_) => -32005,
            Self::QueryRejected(_) => -32006,
            Self::RateLimitExceeded => -32007,
            Self::InvalidParameters(_) => -32602,
            Self::UnknownTool(_) => -32601,
            Self::Database(_) => -32010,
            Self::Query(_) => -32011,
            Self::Serialization(_) => -32012,
            Self::Internal(_) => -32603,
        }
    }

    /// Returns true if this error should be logged at error level.
    pub fn is_server_error(&self) -> bool {
        matches!(
            self,
            Self::Database(_) | Self::Internal(_) | Self::Serialization(_)
        )
    }
}

impl From<serde_json::Error> for McpError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e.to_string())
    }
}
