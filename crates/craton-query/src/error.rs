//! Error types for query operations.

use craton_store::StoreError;

/// Errors that can occur during query parsing and execution.
#[derive(thiserror::Error, Debug)]
pub enum QueryError {
    /// SQL syntax or parsing error.
    #[error("parse error: {0}")]
    ParseError(String),

    /// Table not found in schema.
    #[error("table '{0}' not found")]
    TableNotFound(String),

    /// Column not found in table.
    #[error("column '{column}' not found in table '{table}'")]
    ColumnNotFound { table: String, column: String },

    /// Query parameter not provided.
    #[error("parameter ${0} not provided")]
    ParameterNotFound(usize),

    /// Type mismatch between expected and actual value.
    #[error("type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    /// SQL feature not supported.
    #[error("unsupported feature: {0}")]
    UnsupportedFeature(String),

    /// Underlying store error.
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    /// JSON serialization/deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Result type for query operations.
pub type Result<T> = std::result::Result<T, QueryError>;
