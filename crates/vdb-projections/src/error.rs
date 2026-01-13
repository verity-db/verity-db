//! Error types for the projection engine.

use vdb_types::Offset;

/// Errors that can occur in the projection engine.
#[derive(thiserror::Error, Debug)]
pub enum ProjectionError {
    /// Underlying SQLite/sqlx database error.
    #[error(transparent)]
    Database(#[from] sqlx::Error),

    /// Checkpoint offset doesn't match expected value during replay.
    /// This indicates events were applied out of order or a gap exists.
    #[error("checkpoint mismatch. expected {expected}, received {actual} ")]
    CheckpointMismatch { expected: Offset, actual: Offset },

    /// Encountered a SQLite type that cannot be mapped to [`SqlValue`](crate::SqlValue).
    #[error("unsupported sqlite type: {type_name}")]
    UnsupportedSqliteType { type_name: String },

    /// Failed to decode a SQLite value to its Rust representation.
    #[error("failed to decode {type_name}: {source}")]
    DecodeError {
        type_name: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Attempted to create a [`SqlStatement`](crate::SqlStatement) from non-DDL SQL.
    /// Only CREATE, ALTER, and DROP statements are allowed for schema changes.
    #[error("expected DDL statement, got: {statement}")]
    InvalidDdlStatement { statement: String },
}
