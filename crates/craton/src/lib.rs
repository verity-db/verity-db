//! # Craton
//!
//! Compliance-native database for regulated industries.
//!
//! Craton is built on a replicated append-only log with deterministic
//! projection to a custom storage engine. This provides:
//!
//! - **Correctness by design** - Ordered log → deterministic apply → snapshot
//! - **Full audit trail** - Every mutation is captured in the immutable log
//! - **Point-in-time recovery** - Replay from any offset
//! - **Compliance by construction** - Built-in durability and encryption
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                          Craton                             │
//! │  ┌─────────┐   ┌───────────┐   ┌──────────┐   ┌──────────┐ │
//! │  │   Log   │ → │  Kernel   │ → │  Store   │ → │  Query   │ │
//! │  │(append) │   │(pure FSM) │   │(B+tree)  │   │  (SQL)   │ │
//! │  └─────────┘   └───────────┘   └──────────┘   └──────────┘ │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Quick Start
//!
//! ```ignore
//! use craton::{Craton, TenantId, DataClass};
//!
//! // Open database
//! let db = Craton::open("./data")?;
//!
//! // Get tenant handle
//! let tenant = db.tenant(TenantId::new(1));
//!
//! // Create a stream
//! let stream_id = tenant.create_stream("events", DataClass::NonPHI)?;
//!
//! // Append events
//! tenant.append(stream_id, vec![b"event1".to_vec(), b"event2".to_vec()])?;
//!
//! // Query (point-in-time support)
//! let results = tenant.query("SELECT * FROM events LIMIT 10", &[])?;
//! ```
//!
//! # Modules
//!
//! - **SDK Layer**: [`Craton`], [`TenantHandle`] - Main API
//! - **Foundation**: Types, crypto, storage primitives
//! - **Query**: SQL subset for compliance lookups

mod error;
mod tenant;
mod craton;

// SDK Layer - Main API
pub use error::{Result, CratonError};
pub use tenant::TenantHandle;
pub use craton::{Craton, CratonConfig};

// Re-export core types from craton-types
pub use craton_types::{
    DataClass, GroupId, Offset, Placement, Region, StreamId, StreamMetadata, StreamName, TenantId,
};

// Re-export crypto primitives
pub use craton_crypto::{ChainHash, chain_hash};

// Re-export field-level encryption
pub use craton_crypto::{FieldKey, ReversibleToken, Token, decrypt_field, encrypt_field, tokenize};

// Re-export anonymization utilities
pub use craton_crypto::{
    DatePrecision, GeoLevel, KAnonymityResult, MaskStyle, check_k_anonymity, generalize_age,
    generalize_numeric, generalize_zip, mask, redact, truncate_date,
};

// Re-export storage types
pub use craton_storage::{Record, Storage, StorageError};

// Re-export kernel types
pub use craton_kernel::{Command, Effect, KernelError, State, apply_committed};

// Re-export directory
pub use craton_directory::{Directory, DirectoryError};

// Re-export query types for SQL operations
pub use craton_query::{
    ColumnDef, ColumnName, DataType, QueryEngine, QueryError, QueryResult, Row, Schema,
    SchemaBuilder, TableDef, TableName, Value,
};

// Re-export store types for advanced usage
pub use craton_store::{BTreeStore, Key, ProjectionStore, StoreError, TableId, WriteBatch, WriteOp};
