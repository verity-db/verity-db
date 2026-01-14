//! vdb-runtime: Orchestrator for VerityDB
//!
//! The runtime is the "imperative shell" that coordinates all VerityDB
//! components. It implements the request lifecycle:
//!
//! 1. Receive request (create_stream, append, etc.)
//! 2. Route to appropriate VSR group via directory
//! 3. Propose command to VSR consensus
//! 4. On commit: apply to kernel, execute effects
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        Runtime                               │
//! │  ┌─────────┐   ┌───────────┐   ┌────────┐   ┌─────────────┐ │
//! │  │Directory│ → │Replicator │ → │ Kernel │ → │   Effect    │ │
//! │  │(routing)│   │(consensus)│   │ (pure) │   │  Executor   │ │
//! │  └─────────┘   └───────────┘   └────────┘   └─────────────┘ │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use vdb_runtime::Runtime;
//! use vdb_vsr::SingleNodeGroupReplicator;
//!
//! let runtime = Runtime::new(
//!     State::new(),
//!     directory,
//!     SingleNodeGroupReplicator::new(),
//!     storage,
//! );
//!
//! runtime.create_stream(stream_id, name, DataClass::PHI, placement).await?;
//! runtime.append(stream_id, events, Offset::new(0)).await?;
//! ```

pub mod handle;

use bytes::Bytes;
use tokio::sync::RwLock;
use vdb_directory::Directory;
use vdb_kernel::{Command, Effect, State, apply_committed};
use vdb_storage::Storage;
use vdb_types::{DataClass, Offset, Placement, StreamId, StreamMetadata, StreamName};
use vdb_vsr::GroupReplicator;

/// The VerityDB runtime orchestrator.
///
/// Generic over `R: GroupReplicator` to allow different consensus
/// implementations (single-node for dev, VSR for production).
///
/// # Fields
///
/// - `state`: The kernel's in-memory state (stream metadata)
/// - `directory`: Routes streams to replication groups by placement
/// - `replicator`: Consensus layer for committing commands
/// - `storage`: Durable append-only event log
#[derive(Debug)]
pub struct Runtime<R: GroupReplicator> {
    /// The kernel's in-memory state.
    pub state: RwLock<State>,
    /// Routes placements to replication groups.
    pub directory: Directory,
    /// Consensus layer for committing commands.
    pub replicator: R,
    /// Durable storage for events.
    pub storage: Storage,
}

impl<R> Runtime<R>
where
    R: GroupReplicator,
{
    /// Creates a new runtime with the given components.
    pub fn new(
        state: RwLock<State>,
        directory: Directory,
        replicator: R,
        storage: Storage,
    ) -> Self {
        Self {
            state,
            directory,
            replicator,
            storage,
        }
    }

    /// Creates a new event stream.
    ///
    /// This will:
    /// 1. Build a CreateStream command
    /// 2. Route to the appropriate VSR group based on placement
    /// 3. Propose the command through consensus
    /// 4. Apply the committed command to the kernel
    /// 5. Execute resulting effects (metadata write, audit log)
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] if:
    /// - The placement region is not configured in the directory
    /// - Consensus fails
    /// - A stream with the same ID already exists
    pub async fn create_stream(
        &self,
        stream_id: StreamId,
        stream_name: StreamName,
        data_class: DataClass,
        placement: Placement,
    ) -> Result<(), RuntimeError> {
        let cmd = Command::create_stream(stream_id, stream_name, data_class, placement.clone());

        let group = self.directory.group_for_placement(&placement)?;

        let committed_cmd = self.replicator.propose(group, cmd).await?;

        let effects = {
            let mut guard = self.state.write().await;
            let state = std::mem::take(&mut *guard);
            let (new_state, effects) = apply_committed(state, committed_cmd)?;
            *guard = new_state;
            effects
        };

        self.execute_effects(effects).await?;

        Ok(())
    }

    pub async fn append_raw(
        &self,
        stream_id: StreamId,
        events: Vec<Bytes>,
    ) -> Result<Offset, RuntimeError> {
        // 1. Read current offset and placement from state
        let (current_offset, placement) = {
            let guard = self.state.read().await;
            let stream = guard
                .get_stream(&stream_id)
                .ok_or(RuntimeError::StreamNotFound)?;
            (stream.current_offset, stream.placement.clone())
        };

        // 2. Build Command::append_batch with that offset
        let cmd = Command::append_batch(stream_id, events, current_offset);

        // 3. Propose through VSR
        let group = self.directory.group_for_placement(&placement)?;
        let commited_cmd = self.replicator.propose(group, cmd).await?;

        // 4. Apply to kernel, update state
        let (effects, new_offset) = {
            let mut guard = self.state.write().await;
            let state = std::mem::take(&mut *guard);
            let (new_state, effects) = apply_committed(state, commited_cmd)?;
            let new_offset = new_state
                .get_stream(&stream_id)
                .map(|s| s.current_offset)
                .unwrap_or(current_offset);
            *guard = new_state;
            (effects, new_offset)
        };

        // 5. Execute effects
        self.execute_effects(effects).await?;

        // 6. Return new offset
        Ok(new_offset)
    }

    /// Appends events to an existing stream.
    ///
    /// Uses optimistic concurrency control via `expected_offset`. The append
    /// will fail if the stream's current offset doesn't match.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError`] if:
    /// - The stream doesn't exist
    /// - The expected offset doesn't match (concurrent write)
    /// - Consensus fails
    /// - Storage write fails
    pub async fn append(
        &self,
        stream_id: StreamId,
        events: Vec<Bytes>,
        expected_offset: Offset,
    ) -> Result<(), RuntimeError> {
        let cmd = Command::append_batch(stream_id, events, expected_offset);

        let effects = {
            let mut guard = self.state.write().await;
            let state = std::mem::take(&mut *guard);
            let StreamMetadata { placement, .. } = state
                .get_stream(&stream_id)
                .ok_or(RuntimeError::StreamNotFound)?;

            let group = self.directory.group_for_placement(placement)?;

            let committed_cmd = self.replicator.propose(group, cmd).await?;

            let (new_state, effects) = apply_committed(state, committed_cmd)?;
            *guard = new_state;
            effects
        };

        self.execute_effects(effects).await?;

        Ok(())
    }

    /// Executes effects produced by the kernel.
    ///
    /// Effects are side effects that must be performed after a command
    /// is applied: storage writes, projection notifications, audit logging.
    async fn execute_effects(&self, effects: Vec<Effect>) -> Result<(), RuntimeError> {
        for effect in effects {
            match effect {
                Effect::StorageAppend {
                    stream_id,
                    base_offset,
                    events,
                } => {
                    self.storage
                        .append_batch(stream_id, events, base_offset, true)
                        .await?;
                }
                Effect::StreamMetadataWrite(stream_metadata) => {
                    tracing::debug!(
                        ?stream_metadata,
                        "StreamMetadataWrite effect received (persistence not yet implemented)"
                    );
                }
                Effect::WakeProjection {
                    stream_id,
                    from_offset,
                    to_offset,
                } => {
                    tracing::debug!(
                        %stream_id,
                        %from_offset,
                        %to_offset,
                        "WakeProjection effect received (projections not yet implemented)"
                    );
                }
                Effect::AuditLogAppend(audit_action) => {
                    tracing::debug!(
                        ?audit_action,
                        "AuditLogAppend effect received (audit log not yet implemented)"
                    );
                }
            }
        }
        Ok(())
    }
}

/// Errors that can occur during runtime operations.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// Error from the directory (e.g., region not found).
    #[error(transparent)]
    DirectoryError(#[from] vdb_directory::DirectoryError),

    /// Error from VSR consensus.
    #[error(transparent)]
    VsrError(#[from] vdb_vsr::VsrError),

    /// Error from the kernel (e.g., stream already exists).
    #[error(transparent)]
    KernelError(#[from] vdb_kernel::KernelError),

    /// Error from storage (e.g., I/O failure).
    #[error(transparent)]
    StorageError(#[from] vdb_storage::StorageError),

    /// The requested stream was not found.
    #[error("stream not found")]
    StreamNotFound,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use vdb_types::{GroupId, Region};
    use vdb_vsr::SingleNodeGroupReplicator;

    async fn setup_runtime() -> (Runtime<SingleNodeGroupReplicator>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = Storage::new(temp_dir.path());
        let directory = Directory::new(GroupId::new(0))
            .with_region(Region::APSoutheast2, GroupId::new(1))
            .with_region(Region::USEast1, GroupId::new(2));
        let replicator = SingleNodeGroupReplicator::new();
        let state = RwLock::new(State::new());

        let runtime = Runtime::new(state, directory, replicator, storage);
        (runtime, temp_dir)
    }

    #[tokio::test]
    async fn create_stream_succeeds() {
        let (runtime, _dir) = setup_runtime().await;

        let result = runtime
            .create_stream(
                StreamId::new(1),
                StreamName::new("test-stream"),
                DataClass::PHI,
                Placement::Region(Region::APSoutheast2),
            )
            .await;

        assert!(result.is_ok());
        assert!(runtime.state.read().await.stream_exists(&StreamId::new(1)));
    }

    #[tokio::test]
    async fn create_stream_with_global_placement() {
        let (runtime, _dir) = setup_runtime().await;

        let result = runtime
            .create_stream(
                StreamId::new(1),
                StreamName::new("global-stream"),
                DataClass::NonPHI,
                Placement::Global,
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn create_duplicate_stream_fails() {
        let (runtime, _dir) = setup_runtime().await;

        // Create first stream
        runtime
            .create_stream(
                StreamId::new(1),
                StreamName::new("test"),
                DataClass::NonPHI,
                Placement::Global,
            )
            .await
            .unwrap();

        // Try to create duplicate
        let result = runtime
            .create_stream(
                StreamId::new(1),
                StreamName::new("test"),
                DataClass::NonPHI,
                Placement::Global,
            )
            .await;

        assert!(matches!(result, Err(RuntimeError::KernelError(_))));
    }

    #[tokio::test]
    async fn append_to_existing_stream_succeeds() {
        let (runtime, _dir) = setup_runtime().await;

        // Create stream first
        runtime
            .create_stream(
                StreamId::new(1),
                StreamName::new("test"),
                DataClass::NonPHI,
                Placement::Global,
            )
            .await
            .unwrap();

        // Append events
        let events = vec![
            Bytes::from("event-1"),
            Bytes::from("event-2"),
            Bytes::from("event-3"),
        ];
        let result = runtime
            .append(StreamId::new(1), events, Offset::new(0))
            .await;

        assert!(result.is_ok());

        // Verify offset updated
        let guard = runtime.state.read().await;
        let stream = guard.get_stream(&StreamId::new(1)).unwrap();
        assert_eq!(stream.current_offset.as_i64(), 3);
    }

    #[tokio::test]
    async fn append_to_nonexistent_stream_fails() {
        let (runtime, _dir) = setup_runtime().await;

        let events = vec![Bytes::from("event")];
        let result = runtime
            .append(StreamId::new(999), events, Offset::new(0))
            .await;

        assert!(matches!(result, Err(RuntimeError::StreamNotFound)));
    }

    #[tokio::test]
    async fn append_with_wrong_offset_fails() {
        let (runtime, _dir) = setup_runtime().await;

        // Create stream
        runtime
            .create_stream(
                StreamId::new(1),
                StreamName::new("test"),
                DataClass::NonPHI,
                Placement::Global,
            )
            .await
            .unwrap();

        // Append with wrong expected offset
        let events = vec![Bytes::from("event")];
        let result = runtime
            .append(StreamId::new(1), events, Offset::new(5))
            .await;

        assert!(matches!(result, Err(RuntimeError::KernelError(_))));
    }

    #[tokio::test]
    async fn create_stream_with_unknown_region_fails() {
        let (runtime, _dir) = setup_runtime().await;

        let result = runtime
            .create_stream(
                StreamId::new(1),
                StreamName::new("test"),
                DataClass::PHI,
                Placement::Region(Region::custom("unknown-region")),
            )
            .await;

        assert!(matches!(result, Err(RuntimeError::DirectoryError(_))));
    }
}
