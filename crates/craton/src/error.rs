//! Error types for the Craton SDK.
//!
//! This module provides a unified error type that wraps errors from the
//! underlying subsystems: kernel, storage, projection store, and query engine.

use thiserror::Error;
use craton_kernel::KernelError;
use craton_query::QueryError;
use craton_storage::StorageError;
use craton_store::StoreError;
use craton_types::{Offset, StreamId, TenantId};

/// Result type for Craton operations.
pub type Result<T> = std::result::Result<T, CratonError>;

/// Errors that can occur during Craton operations.
#[derive(Debug, Error)]
pub enum CratonError {
    /// Error from the kernel (state machine).
    #[error("kernel error: {0}")]
    Kernel(#[from] KernelError),

    /// Error from the storage layer (append-only log).
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    /// Error from the projection store (B+tree).
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    /// Error from the query engine (SQL parsing/execution).
    #[error("query error: {0}")]
    Query(#[from] QueryError),

    /// Tenant not found.
    #[error("tenant not found: {0:?}")]
    TenantNotFound(TenantId),

    /// Stream not found.
    #[error("stream not found: {0}")]
    StreamNotFound(StreamId),

    /// Table not found in schema.
    #[error("table not found: {0}")]
    TableNotFound(String),

    /// Position is ahead of current log position.
    #[error("position {requested} is ahead of current log position {current}")]
    PositionAhead { requested: Offset, current: Offset },

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),

    /// I/O error.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// Projection store is not caught up to the log.
    #[error("projection store at position {store_pos}, log at {log_pos}")]
    ProjectionLag { store_pos: Offset, log_pos: Offset },

    /// Internal error (should not occur in normal operation).
    #[error("internal error: {0}")]
    Internal(String),
}

impl CratonError {
    /// Creates an internal error with the given message.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }

    /// Creates a configuration error with the given message.
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }
}
