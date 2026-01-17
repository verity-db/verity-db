//! vdb-vsr: Viewstamped Replication for VerityDB
//!
//! This crate provides the consensus abstraction layer for VerityDB. Commands
//! are proposed to VSR groups and committed once a quorum of replicas has
//! acknowledged.
//!
//! # Architecture
//!
//! The [`GroupReplicator`] trait defines the interface for proposing commands
//! to a replication group. Different implementations provide different
//! consistency guarantees:
//!
//! - [`SingleNodeGroupReplicator`]: Development mode, commits immediately
//! - Future: `VsrGroupReplicator` with full quorum-based replication
//!
//! # Example
//!
//! ```ignore
//! use vdb_vsr::{GroupReplicator, SingleNodeGroupReplicator};
//!
//! let replicator = SingleNodeGroupReplicator::new();
//! let committed = replicator.propose(group_id, command).await?;
//! ```

use std::fmt::Debug;

use async_trait::async_trait;
use vdb_kernel::Command;
use vdb_types::GroupId;

/// Trait for proposing commands to a replication group.
///
/// Implementations of this trait handle the consensus protocol for
/// committing commands across replicas. The `propose` method blocks
/// until the command is committed (or fails).
///
/// # Thread Safety
///
/// Implementations must be `Send + Sync` to allow sharing across
/// async tasks.
#[async_trait]
pub trait GroupReplicator: Send + Sync + Debug + Clone {
    /// Proposes a command to the specified replication group.
    ///
    /// Returns the committed command on success. The returned command
    /// is guaranteed to be durably committed and will be applied by
    /// all replicas in the group.
    ///
    /// # Errors
    ///
    /// Returns [`VsrError`] if:
    /// - The group is not found
    /// - Consensus times out
    /// - A quorum cannot be reached (future implementations)
    async fn propose(&self, group: GroupId, cmd: Command) -> Result<Command, VsrError>;
}

/// Single-node replicator for development and testing.
///
/// This implementation provides no actual replication - commands are
/// immediately "committed" since there's only one node. This is useful
/// for local development and testing without the overhead of consensus.
///
/// # Example
///
/// ```ignore
/// let replicator = SingleNodeGroupReplicator::new();
/// // Commands commit immediately
/// let committed = replicator.propose(group, cmd).await?;
/// ```
#[derive(Default, Debug, Clone)]
pub struct SingleNodeGroupReplicator;

impl SingleNodeGroupReplicator {
    /// Creates a new single-node replicator.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl GroupReplicator for SingleNodeGroupReplicator {
    async fn propose(&self, _group: GroupId, cmd: Command) -> Result<Command, VsrError> {
        tracing::debug!(?cmd, "single-node: immediately committing command");
        Ok(cmd)
    }
}

/// Errors that can occur during VSR consensus.
#[derive(thiserror::Error, Debug)]
pub enum VsrError {
    /// The specified replication group was not found.
    #[error("group not found: {0}")]
    GroupNotFound(GroupId),

    /// The consensus operation timed out.
    #[error("consensus timeout")]
    Timeout,
}

#[cfg(test)]
mod tests {
    use super::*;
    use vdb_types::{DataClass, Placement, StreamId, StreamName};

    #[tokio::test]
    async fn single_node_propose_returns_same_command() {
        let replicator = SingleNodeGroupReplicator::new();
        let group = GroupId::new(1);
        let cmd = Command::create_stream(
            StreamId::new(1),
            StreamName::new("test"),
            DataClass::NonPHI,
            Placement::Global,
        );

        let result = replicator.propose(group, cmd).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn single_node_ignores_group_id() {
        let replicator = SingleNodeGroupReplicator::new();

        // Different group IDs should all work
        for group_id in [1, 2, 100, 999] {
            let group = GroupId::new(group_id);
            let cmd = Command::create_stream(
                StreamId::new(1),
                StreamName::new("test"),
                DataClass::NonPHI,
                Placement::Global,
            );

            let result = replicator.propose(group, cmd).await;
            assert!(result.is_ok());
        }
    }

    #[test]
    fn single_node_implements_default() {
        let _replicator: SingleNodeGroupReplicator = Default::default();
    }
}
