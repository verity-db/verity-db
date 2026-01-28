//! Single-node VSR replicator.
//!
//! This module implements a degenerate case of VSR where there is only one
//! replica. This simplifies the protocol significantly:
//!
//! - No network communication needed
//! - No quorum requirements (quorum of 1 = self)
//! - Immediate commit after local persistence
//! - Always the leader (view 0 forever)
//!
//! Despite the simplifications, `SingleNodeReplicator` maintains the same
//! API as multi-node VSR, making it suitable for:
//!
//! - Development and testing
//! - Single-node deployments where replication isn't needed
//! - Integration testing before adding multi-node complexity
//!
//! # Architecture
//!
//! ```text
//! Command
//!    │
//!    ▼
//! ┌──────────────────────┐
//! │ SingleNodeReplicator │
//! │  1. Assign OpNumber  │
//! │  2. Create LogEntry  │
//! │  3. Persist to log   │
//! │  4. Update superblock│
//! │  5. Apply to kernel  │
//! └──────────┬───────────┘
//!            │
//!            ▼
//!      Vec<Effect>
//! ```

use std::io::{Read, Seek, Write};

use tracing::{debug, info, instrument};
use vdb_kernel::{Command, Effect, KernelError, State, apply_committed};
use vdb_types::IdempotencyId;

use crate::VsrError;
use crate::config::ClusterConfig;
use crate::superblock::{Superblock, SuperblockData};
use crate::types::{CommitNumber, LogEntry, OpNumber, ReplicaId, ReplicaStatus, ViewNumber};

// ============================================================================
// Replicator Trait
// ============================================================================

/// Common interface for VSR replicators.
///
/// This trait abstracts over single-node and multi-node implementations,
/// allowing code to work with either transparently.
pub trait Replicator {
    /// Submits a command for replication and execution.
    ///
    /// The command is assigned an operation number, replicated (for multi-node),
    /// and once committed, applied to the kernel state machine.
    ///
    /// # Returns
    ///
    /// - `Ok(effects)` - The command was committed; effects should be executed
    /// - `Err(VsrError)` - Replication or kernel application failed
    ///
    /// # Idempotency
    ///
    /// If the same `idempotency_id` is submitted twice, the replicator returns
    /// the cached result (if still retained) instead of re-executing.
    fn submit(
        &mut self,
        command: Command,
        idempotency_id: Option<IdempotencyId>,
    ) -> Result<SubmitResult, VsrError>;

    /// Returns the current kernel state.
    fn state(&self) -> &State;

    /// Returns the current view number.
    fn view(&self) -> ViewNumber;

    /// Returns the highest committed operation number.
    fn commit_number(&self) -> CommitNumber;

    /// Returns the replica's current status.
    fn status(&self) -> ReplicaStatus;

    /// Returns the cluster configuration.
    fn config(&self) -> &ClusterConfig;
}

/// Result of a successful command submission.
#[derive(Debug)]
pub struct SubmitResult {
    /// The operation number assigned to this command.
    pub op_number: OpNumber,

    /// The effects produced by the kernel.
    ///
    /// The caller is responsible for executing these effects.
    pub effects: Vec<Effect>,

    /// Whether this was a duplicate submission (idempotency hit).
    pub was_duplicate: bool,
}

// ============================================================================
// Single-Node Replicator
// ============================================================================

/// A single-node VSR replicator.
///
/// This is a degenerate case of VSR optimized for single-node operation.
/// All commands are immediately committed after local persistence.
///
/// # Usage
///
/// ```ignore
/// use vdb_vsr::{SingleNodeReplicator, ClusterConfig, ReplicaId, MemorySuperblock};
/// use vdb_kernel::{Command, State};
///
/// // Create single-node cluster
/// let config = ClusterConfig::single_node(ReplicaId::new(0));
/// let storage = MemorySuperblock::new();
///
/// let mut replicator = SingleNodeReplicator::create(config, storage)?;
///
/// // Submit a command
/// let command = Command::create_stream_with_auto_id(
///     "events".into(),
///     DataClass::NonPHI,
///     Placement::Global,
/// );
/// let result = replicator.submit(command, None)?;
///
/// // Execute the effects
/// for effect in result.effects {
///     runtime.execute(effect)?;
/// }
/// ```
#[derive(Debug)]
pub struct SingleNodeReplicator<S> {
    /// Cluster configuration (single-node).
    config: ClusterConfig,

    /// Durable metadata (view, `op_number`, `commit_number`, etc.).
    superblock: Superblock<S>,

    /// Current kernel state.
    state: State,

    /// In-memory log of committed entries.
    ///
    /// For single-node, entries are immediately committed on append.
    /// This log enables recovery and debugging.
    log: Vec<LogEntry>,

    /// Highest operation number in the log.
    op_number: OpNumber,

    /// Highest committed operation number (equals `op_number` for single-node).
    commit_number: CommitNumber,

    /// Current replica status.
    status: ReplicaStatus,
}

impl<S: Read + Write + Seek> SingleNodeReplicator<S> {
    /// Creates a new single-node replicator with fresh state.
    ///
    /// This initializes:
    /// - Empty kernel state
    /// - Fresh superblock (sequence 0)
    /// - Empty operation log
    ///
    /// # Arguments
    ///
    /// * `config` - Must be a single-node cluster configuration
    /// * `storage` - Storage backend for the superblock
    ///
    /// # Errors
    ///
    /// Returns an error if superblock initialization fails.
    ///
    /// # Panics
    ///
    /// Panics if `config` is not a single-node cluster.
    #[instrument(skip(storage))]
    pub fn create(config: ClusterConfig, storage: S) -> Result<Self, VsrError> {
        assert!(
            config.is_single_node(),
            "SingleNodeReplicator requires single-node cluster"
        );

        let replica_id = config.replica_at(0);
        let superblock = Superblock::create(storage, replica_id)?;

        info!(replica_id = %replica_id, "created single-node replicator");

        Ok(Self {
            config,
            superblock,
            state: State::new(),
            log: Vec::new(),
            op_number: OpNumber::ZERO,
            commit_number: CommitNumber::ZERO,
            status: ReplicaStatus::Normal,
        })
    }

    /// Opens an existing single-node replicator from storage.
    ///
    /// This recovers:
    /// - Superblock state (view, op_number, commit_number)
    /// - Kernel state must be rebuilt by replaying the log
    ///
    /// # Recovery Process
    ///
    /// 1. Open superblock (selects highest valid copy)
    /// 2. Kernel state starts empty
    /// 3. Caller should replay committed commands to rebuild state
    ///
    /// # Arguments
    ///
    /// * `config` - Must match the persisted cluster configuration
    /// * `storage` - Storage backend containing the superblock
    ///
    /// # Errors
    ///
    /// Returns an error if no valid superblock is found.
    #[instrument(skip(storage))]
    pub fn open(config: ClusterConfig, storage: S) -> Result<Self, VsrError> {
        assert!(
            config.is_single_node(),
            "SingleNodeReplicator requires single-node cluster"
        );

        let superblock = Superblock::open(storage)?;

        // Extract values before moving superblock
        let replica_id = superblock.replica_id();
        let view = superblock.view();
        let op_number = superblock.op_number();
        let commit_number = superblock.commit_number();

        info!(
            replica_id = %replica_id,
            view = %view,
            op_number = %op_number,
            commit_number = %commit_number,
            "opened single-node replicator"
        );

        Ok(Self {
            config,
            superblock,
            state: State::new(), // Will be rebuilt by replaying log
            log: Vec::new(),     // Will be loaded separately
            op_number,
            commit_number,
            status: ReplicaStatus::Normal,
        })
    }

    /// Returns a reference to the superblock data.
    pub fn superblock_data(&self) -> &SuperblockData {
        self.superblock.data()
    }

    /// Returns the replica ID.
    pub fn replica_id(&self) -> ReplicaId {
        self.superblock.replica_id()
    }

    /// Returns the number of entries in the log.
    pub fn log_len(&self) -> usize {
        self.log.len()
    }

    /// Returns a reference to a log entry by operation number.
    pub fn log_entry(&self, op: OpNumber) -> Option<&LogEntry> {
        if op.is_zero() {
            return None;
        }
        let index = op.as_u64().checked_sub(1)? as usize;
        self.log.get(index)
    }

    /// Rebuilds kernel state by replaying commands.
    ///
    /// Call this after `open()` to restore state from the log.
    ///
    /// # Arguments
    ///
    /// * `commands` - Iterator of (`OpNumber`, Command) pairs in order
    ///
    /// # Errors
    ///
    /// Returns an error if any command fails to apply.
    pub fn replay<I>(&mut self, commands: I) -> Result<(), VsrError>
    where
        I: IntoIterator<Item = (OpNumber, Command)>,
    {
        for (op, command) in commands {
            debug!(op_number = %op, "replaying command");

            // Apply to kernel (ignore effects during replay)
            let (new_state, _effects) =
                apply_committed(self.state.clone(), command).map_err(VsrError::from)?;

            self.state = new_state;
        }

        Ok(())
    }

    /// Assigns the next operation number.
    fn next_op_number(&self) -> OpNumber {
        self.op_number.next()
    }
}

impl<S: Read + Write + Seek> Replicator for SingleNodeReplicator<S> {
    #[instrument(skip(self, command), fields(op_number))]
    fn submit(
        &mut self,
        command: Command,
        idempotency_id: Option<IdempotencyId>,
    ) -> Result<SubmitResult, VsrError> {
        // Verify we're in normal status
        if self.status != ReplicaStatus::Normal {
            return Err(VsrError::InvalidState {
                status: self.status,
                expected: "Normal",
            });
        }

        // 1. Assign operation number
        let op_number = self.next_op_number();
        tracing::Span::current().record("op_number", op_number.as_u64());

        debug!(command = ?command, "submitting command");

        // 2. Create log entry
        let entry = LogEntry::new(
            op_number,
            ViewNumber::ZERO, // Single-node is always view 0
            command.clone(),
            idempotency_id,
        );

        // 3. Verify entry checksum
        debug_assert!(entry.verify_checksum(), "log entry checksum invalid");

        // 4. Persist log entry (append to in-memory log)
        self.log.push(entry);

        // 5. Update superblock (for single-node, immediately committed)
        self.op_number = op_number;
        self.commit_number = CommitNumber::new(op_number);

        self.superblock
            .update(ViewNumber::ZERO, self.op_number, self.commit_number)?;

        // 6. Apply to kernel
        let (new_state, effects) =
            apply_committed(self.state.clone(), command).map_err(VsrError::from)?;

        self.state = new_state;

        debug!(effects_count = effects.len(), "command committed");

        Ok(SubmitResult {
            op_number,
            effects,
            was_duplicate: false,
        })
    }

    fn state(&self) -> &State {
        &self.state
    }

    fn view(&self) -> ViewNumber {
        ViewNumber::ZERO // Single-node is always view 0
    }

    fn commit_number(&self) -> CommitNumber {
        self.commit_number
    }

    fn status(&self) -> ReplicaStatus {
        self.status
    }

    fn config(&self) -> &ClusterConfig {
        &self.config
    }
}

// ============================================================================
// Error Conversion
// ============================================================================

impl From<KernelError> for VsrError {
    fn from(err: KernelError) -> Self {
        VsrError::RecoveryFailed {
            reason: format!("kernel error: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemorySuperblock;
    use vdb_types::{DataClass, Placement};

    fn test_config() -> ClusterConfig {
        ClusterConfig::single_node(ReplicaId::new(0))
    }

    fn create_stream_command(name: &str) -> Command {
        Command::create_stream_with_auto_id(name.into(), DataClass::NonPHI, Placement::Global)
    }

    #[test]
    fn single_node_create_and_submit() {
        let storage = MemorySuperblock::new();
        let mut replicator = SingleNodeReplicator::create(test_config(), storage).expect("create");

        assert_eq!(replicator.view(), ViewNumber::ZERO);
        assert_eq!(replicator.commit_number(), CommitNumber::ZERO);
        assert_eq!(replicator.status(), ReplicaStatus::Normal);
        assert!(replicator.state().stream_count() == 0);

        // Submit a command
        let result = replicator
            .submit(create_stream_command("test-stream"), None)
            .expect("submit");

        assert_eq!(result.op_number, OpNumber::new(1));
        assert!(!result.was_duplicate);
        assert!(!result.effects.is_empty()); // Should have StreamMetadataWrite effect

        // State should be updated
        assert_eq!(replicator.state().stream_count(), 1);
        assert_eq!(replicator.commit_number().as_u64(), 1);
        assert_eq!(replicator.log_len(), 1);
    }

    #[test]
    fn single_node_multiple_commands() {
        let storage = MemorySuperblock::new();
        let mut replicator = SingleNodeReplicator::create(test_config(), storage).expect("create");

        // Submit multiple commands
        for i in 1..=5 {
            let result = replicator
                .submit(create_stream_command(&format!("stream-{i}")), None)
                .expect("submit");

            assert_eq!(result.op_number, OpNumber::new(i));
        }

        assert_eq!(replicator.state().stream_count(), 5);
        assert_eq!(replicator.commit_number().as_u64(), 5);
        assert_eq!(replicator.log_len(), 5);
    }

    #[test]
    fn single_node_log_entry_retrieval() {
        let storage = MemorySuperblock::new();
        let mut replicator = SingleNodeReplicator::create(test_config(), storage).expect("create");

        replicator
            .submit(create_stream_command("stream-1"), None)
            .expect("submit");
        replicator
            .submit(create_stream_command("stream-2"), None)
            .expect("submit");

        // Retrieve log entries
        let entry1 = replicator.log_entry(OpNumber::new(1)).expect("entry 1");
        assert_eq!(entry1.op_number, OpNumber::new(1));
        assert!(entry1.verify_checksum());

        let entry2 = replicator.log_entry(OpNumber::new(2)).expect("entry 2");
        assert_eq!(entry2.op_number, OpNumber::new(2));
        assert!(entry2.verify_checksum());

        // Op 0 returns None
        assert!(replicator.log_entry(OpNumber::ZERO).is_none());

        // Op 3 doesn't exist yet
        assert!(replicator.log_entry(OpNumber::new(3)).is_none());
    }

    #[test]
    fn single_node_open_recovery() {
        // Create and submit some commands
        let storage = MemorySuperblock::new();
        let mut replicator = SingleNodeReplicator::create(test_config(), storage).expect("create");

        replicator
            .submit(create_stream_command("stream-1"), None)
            .expect("submit");
        replicator
            .submit(create_stream_command("stream-2"), None)
            .expect("submit");

        // Get the superblock data for "reopening"
        let storage_data = replicator.superblock.storage().clone_data();

        // Simulate restart by opening with persisted storage
        let storage2 = MemorySuperblock::from_data(storage_data);
        let replicator2 = SingleNodeReplicator::open(test_config(), storage2).expect("open");

        // Superblock state should be recovered
        assert_eq!(replicator2.superblock_data().op_number, OpNumber::new(2));
        assert_eq!(
            replicator2.superblock_data().commit_number,
            CommitNumber::new(OpNumber::new(2))
        );

        // Kernel state is empty (needs replay)
        assert_eq!(replicator2.state().stream_count(), 0);

        // Log is empty (would be loaded from persistent storage)
        assert_eq!(replicator2.log_len(), 0);
    }

    #[test]
    fn single_node_with_idempotency_id() {
        let storage = MemorySuperblock::new();
        let mut replicator = SingleNodeReplicator::create(test_config(), storage).expect("create");

        let id = IdempotencyId::generate();

        let result = replicator
            .submit(create_stream_command("stream-1"), Some(id))
            .expect("submit");

        assert_eq!(result.op_number, OpNumber::new(1));

        // The log entry should contain the idempotency ID
        let entry = replicator.log_entry(OpNumber::new(1)).expect("entry");
        assert_eq!(entry.idempotency_id, Some(id));
    }

    #[test]
    fn superblock_updates_on_each_submit() {
        let storage = MemorySuperblock::new();
        let mut replicator = SingleNodeReplicator::create(test_config(), storage).expect("create");

        // Initial sequence
        let initial_seq = replicator.superblock_data().sequence;

        replicator
            .submit(create_stream_command("stream-1"), None)
            .expect("submit");

        // Sequence should increase
        assert!(replicator.superblock_data().sequence > initial_seq);

        replicator
            .submit(create_stream_command("stream-2"), None)
            .expect("submit");

        // Sequence should increase again
        assert!(replicator.superblock_data().sequence > initial_seq + 1);
    }

    #[test]
    #[should_panic(expected = "single-node cluster")]
    fn rejects_multi_node_config() {
        let config = ClusterConfig::new(vec![
            ReplicaId::new(0),
            ReplicaId::new(1),
            ReplicaId::new(2),
        ]);

        let storage = MemorySuperblock::new();
        let _ = SingleNodeReplicator::create(config, storage);
    }

    #[test]
    fn kernel_error_propagates() {
        let storage = MemorySuperblock::new();
        let mut replicator = SingleNodeReplicator::create(test_config(), storage).expect("create");

        // Create a stream
        replicator
            .submit(create_stream_command("stream-1"), None)
            .expect("submit");

        // Try to append to non-existent stream
        let bad_command = Command::append_batch(
            vdb_types::StreamId::new(999), // Non-existent
            vec![bytes::Bytes::from("data")],
            vdb_types::Offset::ZERO,
        );

        let result = replicator.submit(bad_command, None);
        assert!(result.is_err());
    }
}
