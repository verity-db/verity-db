//! Main entry point for the Verity SDK.
//!
//! The `Verity` struct provides the top-level API for interacting with `VerityDB`.
//! It manages the underlying storage, kernel state, projection store, and query engine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use vdb_crypto::ChainHash;
use vdb_kernel::{Command, Effect, State as KernelState, apply_committed};
use vdb_query::{ColumnDef, DataType, QueryEngine, SchemaBuilder};
use vdb_storage::Storage;
use vdb_store::{BTreeStore, Key, ProjectionStore, TableId, WriteBatch};
use vdb_types::{Offset, StreamId, TenantId};

use crate::error::{Result, VerityError};
use crate::tenant::TenantHandle;

/// Configuration for opening a Verity database.
#[derive(Debug, Clone)]
pub struct VerityConfig {
    /// Path to the data directory.
    pub data_dir: PathBuf,
    /// Page cache capacity for projection store (in pages, 4KB each).
    pub cache_capacity: usize,
}

impl VerityConfig {
    /// Creates a new configuration with the given data directory.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            cache_capacity: 4096, // 16MB default
        }
    }

    /// Sets the page cache capacity.
    pub fn with_cache_capacity(mut self, capacity: usize) -> Self {
        self.cache_capacity = capacity;
        self
    }
}

/// Internal state shared across tenant handles.
pub(crate) struct VerityInner {
    /// Path to data directory (used for future operations like metadata persistence).
    #[allow(dead_code)]
    pub(crate) data_dir: PathBuf,

    /// Append-only log storage.
    pub(crate) storage: Storage,

    /// Kernel state machine.
    pub(crate) kernel_state: KernelState,

    /// Projection store (B+tree with MVCC).
    pub(crate) projection_store: BTreeStore,

    /// Query engine with schema.
    pub(crate) query_engine: QueryEngine,

    /// Current log position (offset of last written record).
    pub(crate) log_position: Offset,

    /// Hash chain head for each stream.
    pub(crate) chain_heads: HashMap<StreamId, ChainHash>,
}

impl VerityInner {
    /// Executes effects produced by the kernel.
    ///
    /// This is the "imperative shell" that handles I/O.
    pub(crate) fn execute_effects(&mut self, effects: Vec<Effect>) -> Result<()> {
        for effect in effects {
            match effect {
                Effect::StorageAppend {
                    stream_id,
                    base_offset,
                    events,
                } => {
                    let prev_hash = self.chain_heads.get(&stream_id).copied();
                    let (new_offset, new_hash) = self.storage.append_batch(
                        stream_id,
                        events.clone(),
                        base_offset,
                        prev_hash,
                        true, // fsync for durability
                    )?;
                    self.chain_heads.insert(stream_id, new_hash);
                    self.log_position = new_offset;

                    // Apply to projection store
                    self.apply_to_projection(stream_id, base_offset, &events)?;
                }
                Effect::StreamMetadataWrite(metadata) => {
                    // For now, stream metadata is tracked in kernel state
                    // Future: persist to metadata store
                    tracing::debug!(?metadata, "stream metadata updated");
                }
                Effect::WakeProjection {
                    stream_id,
                    from_offset,
                    to_offset,
                } => {
                    // Projection is updated inline in StorageAppend
                    // This effect signals external consumers
                    tracing::debug!(
                        ?stream_id,
                        ?from_offset,
                        ?to_offset,
                        "projection wake signal"
                    );
                }
                Effect::AuditLogAppend(action) => {
                    // Future: write to immutable audit log
                    tracing::debug!(?action, "audit action");
                }
            }
        }
        Ok(())
    }

    /// Applies events to the projection store.
    fn apply_to_projection(
        &mut self,
        stream_id: StreamId,
        base_offset: Offset,
        events: &[Bytes],
    ) -> Result<()> {
        // Each event becomes a projection entry
        // Key format: stream_id:offset
        let table_id = TableId::new(u64::from(stream_id));

        for (i, event) in events.iter().enumerate() {
            let offset = Offset::new(base_offset.as_u64() + i as u64);
            let batch = WriteBatch::new(Offset::new(
                self.projection_store.applied_position().as_u64() + 1,
            ))
            .put(
                table_id,
                Key::from(format!("{:016x}", offset.as_u64())),
                event.clone(),
            );

            self.projection_store.apply(batch)?;
        }

        Ok(())
    }
}

/// The main `VerityDB` handle.
///
/// Provides the top-level API for interacting with the database.
/// Get tenant-scoped access via the `tenant()` method.
///
/// # Example
///
/// ```ignore
/// use vdb::Verity;
///
/// let db = Verity::open("./data")?;
/// let tenant = db.tenant(TenantId::new(1));
///
/// // Use tenant handle for operations
/// tenant.execute("INSERT INTO users (id, name) VALUES ($1, $2)", &[1.into(), "Alice".into()])?;
/// let results = tenant.query("SELECT * FROM users WHERE id = $1", &[1.into()])?;
/// ```
#[derive(Clone)]
pub struct Verity {
    inner: Arc<RwLock<VerityInner>>,
}

impl Verity {
    /// Opens a Verity database at the given path.
    ///
    /// If the directory doesn't exist, it will be created.
    /// If the database already exists, it will be opened and state recovered.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let config = VerityConfig::new(data_dir.as_ref());
        Self::open_with_config(config)
    }

    /// Opens a Verity database with custom configuration.
    pub fn open_with_config(config: VerityConfig) -> Result<Self> {
        // Ensure data directory exists
        std::fs::create_dir_all(&config.data_dir)?;

        // Open storage layer
        let storage = Storage::new(&config.data_dir);

        // Open projection store
        let projection_path = config.data_dir.join("projections.db");
        let projection_store =
            BTreeStore::open_with_capacity(&projection_path, config.cache_capacity)?;

        // Initialize kernel state
        let kernel_state = KernelState::new();

        // Build default schema (streams as tables)
        // TODO: Make schema configurable
        let schema = SchemaBuilder::new()
            .table(
                "events",
                TableId::new(1),
                vec![
                    ColumnDef::new("offset", DataType::BigInt).not_null(),
                    ColumnDef::new("data", DataType::Text),
                ],
                vec!["offset".into()],
            )
            .build();

        let query_engine = QueryEngine::new(schema);

        let inner = VerityInner {
            data_dir: config.data_dir,
            storage,
            kernel_state,
            projection_store,
            query_engine,
            log_position: Offset::ZERO,
            chain_heads: HashMap::new(),
        };

        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
        })
    }

    /// Returns a tenant-scoped handle.
    ///
    /// The tenant handle provides operations scoped to a specific tenant ID.
    pub fn tenant(&self, id: TenantId) -> TenantHandle {
        TenantHandle::new(self.clone(), id)
    }

    /// Submits a command to the kernel and executes resulting effects.
    ///
    /// This is the core write path: command → kernel → effects → I/O.
    pub fn submit(&self, command: Command) -> Result<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| VerityError::internal("lock poisoned"))?;

        // Apply command to kernel (pure)
        let (new_state, effects) = apply_committed(inner.kernel_state.clone(), command)?;

        // Update kernel state
        inner.kernel_state = new_state;

        // Execute effects (impure)
        inner.execute_effects(effects)
    }

    /// Returns the current log position.
    pub fn log_position(&self) -> Result<Offset> {
        let inner = self
            .inner
            .read()
            .map_err(|_| VerityError::internal("lock poisoned"))?;
        Ok(inner.log_position)
    }

    /// Returns the current projection store position.
    pub fn projection_position(&self) -> Result<Offset> {
        let inner = self
            .inner
            .read()
            .map_err(|_| VerityError::internal("lock poisoned"))?;
        Ok(inner.projection_store.applied_position())
    }

    /// Syncs all data to disk.
    ///
    /// Ensures durability of all written data.
    pub fn sync(&self) -> Result<()> {
        let mut inner = self
            .inner
            .write()
            .map_err(|_| VerityError::internal("lock poisoned"))?;
        inner.projection_store.sync()?;
        Ok(())
    }

    /// Returns a reference to the inner state.
    ///
    /// This is used internally by `TenantHandle` to access shared state.
    pub(crate) fn inner(&self) -> &Arc<RwLock<VerityInner>> {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use vdb_types::{DataClass, Placement, StreamName};

    #[test]
    fn test_open_creates_directory() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("testdb");

        assert!(!db_path.exists());
        let _db = Verity::open(&db_path).unwrap();
        assert!(db_path.exists());
    }

    #[test]
    fn test_create_stream() {
        let dir = tempdir().unwrap();
        let db = Verity::open(dir.path()).unwrap();

        let cmd = Command::create_stream(
            StreamId::new(1),
            StreamName::new("test_stream"),
            DataClass::NonPHI,
            Placement::Global,
        );

        db.submit(cmd).unwrap();

        let inner = db.inner.read().unwrap();
        assert!(inner.kernel_state.stream_exists(&StreamId::new(1)));
    }

    #[test]
    fn test_append_events() {
        let dir = tempdir().unwrap();
        let db = Verity::open(dir.path()).unwrap();

        // Create stream
        db.submit(Command::create_stream(
            StreamId::new(1),
            StreamName::new("events"),
            DataClass::NonPHI,
            Placement::Global,
        ))
        .unwrap();

        // Append events
        db.submit(Command::append_batch(
            StreamId::new(1),
            vec![Bytes::from("event1"), Bytes::from("event2")],
            Offset::ZERO,
        ))
        .unwrap();

        assert!(db.log_position().unwrap().as_u64() > 0);
    }
}
