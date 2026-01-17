//! # VerityDB
//!
//! Compliance-native, healthcare-focused database with transparent SQL.
//!
//! VerityDB provides a familiar SQL experience while automatically capturing
//! every mutation as an immutable event. This gives you:
//!
//! - **Full audit trail** - Every INSERT/UPDATE/DELETE is logged
//! - **Point-in-time recovery** - Replay events to any moment
//! - **HIPAA compliance** - Built-in durability guarantees
//!
//! # Quick Start
//!
//! ```ignore
//! use vdb::VerityDb;
//!
//! // Open a database (creates if not exists)
//! let db = VerityDb::open("./clinic.db").await?;
//!
//! // Run migrations
//! db.migrate(&[
//!     "CREATE TABLE patients (id INTEGER PRIMARY KEY, name TEXT, dob TEXT)",
//! ]).await?;
//!
//! // Use with any Rust ORM - events are captured automatically
//! sqlx::query("INSERT INTO patients (name, dob) VALUES (?, ?)")
//!     .bind("John Doe")
//!     .bind("1990-01-15")
//!     .execute(db.pool())
//!     .await?;
//! ```

pub mod config;

use std::{path::Path, sync::Arc};

use sqlx::SqlitePool;
use vdb_projections::{ProjectionDb, TransactionHooks, schema::SchemaCache};
use vdb_runtime::{Runtime, RuntimeHandle};
use vdb_types::{EventPersister, StreamId};
use vdb_vsr::GroupReplicator;

// Re-export commonly used types
pub use config::VerityDbConfig;
pub use vdb_types::{DataClass, Offset, Placement, Region, StreamName, TenantId};

/// The main VerityDB database handle.
///
/// `VerityDb` wraps a SQLite connection pool with automatic event capture.
/// Every write operation is logged to a durable event stream before the
/// SQLite transaction commits.
///
/// # Type Parameter
///
/// - `R`: The replication strategy ([`vdb_vsr::SingleNodeGroupReplicator`] for
///   dev, full VSR for production)
#[derive(Debug)]
pub struct VerityDb<R: GroupReplicator> {
    /// The SQLite connection pool (with hooks installed)
    db: ProjectionDb,

    /// Handle to the runtime for event persistence
    runtime_handle: RuntimeHandle<R>,

    /// Schema cache for column name lookups during event capture
    schema_cache: Arc<SchemaCache>,

    /// Stream ID for this database's events
    stream_id: StreamId,
}

impl<R: GroupReplicator + 'static> VerityDb<R> {
    //   - Create SQLite pool
    //   - Initialize schema cache from existing tables
    //   - Create or get stream for this database
    //   - Install transaction hooks
    pub async fn open(
        path: &Path,
        runtime: Arc<Runtime<R>>,
        config: VerityDbConfig,
    ) -> Result<Self, VerityDbError> {
        let db = ProjectionDb::open(path.to_str().unwrap(), None, config.pool).await?;

        let schema_cache = Arc::new(SchemaCache::from_db(&db.read_pool).await?);

        let stream_id = runtime
            .create_stream_with_auto_id(config.stream_name, config.data_class, config.placement)
            .await?;

        let runtime_handle = RuntimeHandle::new(Arc::clone(&runtime));

        let persister: Arc<dyn EventPersister> = Arc::new(runtime_handle.clone());

        let hooks = TransactionHooks::new(
            db.write_pool.clone(),
            schema_cache.clone(),
            persister,
            stream_id,
        );

        hooks.install().await?;

        Ok(Self {
            db,
            runtime_handle,
            schema_cache,
            stream_id,
        })
    }
    pub fn pool(&self) -> &SqlitePool {
        &self.db.read_pool
    }

    //   - Return reference to pool for ORM usage
    //
    // pub async fn migrate(&self, statements: &[&str]) -> Result<(), VerityDbError>
    //   - Execute each DDL statement
    //   - Update schema cache
    //   - SchemaChange events are captured by hooks
    //
    // pub async fn replay_from(&self, offset: Offset) -> Result<(), VerityDbError>
    //   - Read events from storage
    //   - Disable hooks during replay
    //   - Apply events to SQLite
}

/// Errors that can occur during VerityDb operations.
#[derive(Debug, thiserror::Error)]
pub enum VerityDbError {
    /// SQLite/sqlx error
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),

    /// Runtime error (consensus, storage)
    #[error(transparent)]
    Runtime(#[from] vdb_runtime::RuntimeError),

    #[error(transparent)]
    Projection(#[from] vdb_projections::ProjectionError),

    /// Migration failed
    #[error("migration failed: {0}")]
    MigrationFailed(String),

    /// Database not found
    #[error("database not found: {0}")]
    NotFound(String),
}

#[cfg(test)]
mod tests {
    // TODO: Add tests
}
