//! vdb-projections: SQLCipher projection runtime for VerityDB
//!
//! Projections transform the event log into queryable SQLite views.
//! Each projection maintains its own encrypted SQLite database.
//!
//! Features:
//! - SQLCipher encryption (via sqlx + libsqlite3-sys)
//! - Optimized read/write connection pools
//! - Checkpoint tracking (last_offset, checksum)
//! - Snapshot/restore for fast recovery
//! - ProjectionRunner for continuous event application

pub mod checkpoint;
pub mod error;
pub mod event;
pub mod events;
pub mod pool;
pub mod realtime;
pub mod schema;

pub use error::ProjectionError;
pub use events::{ChangeEvent, ColumnName, RowId, SqlStatement, SqlValue, TableName};
pub use pool::{PoolConfig, ProjectionDb};
// pub use realtime::{TableUpdate, UpdateAction};
