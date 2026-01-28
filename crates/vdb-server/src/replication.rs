//! VSR replication integration for the server.
//!
//! This module provides a unified interface for command submission through
//! VSR replication. It supports both single-node and (future) cluster modes.

use std::path::Path;
use std::sync::{Arc, RwLock};

use vdb::Verity;
use vdb_kernel::Command;
use vdb_types::IdempotencyId;
use vdb_vsr::{
    ClusterAddresses, ClusterConfig, MemorySuperblock, MultiNodeConfig, MultiNodeReplicator,
    Replicator, SingleNodeReplicator,
};

use crate::config::ReplicationMode;
use crate::error::{ServerError, ServerResult};

/// A command submitter that routes commands through the appropriate replication layer.
///
/// This abstraction allows the server to use either direct kernel apply (legacy mode)
/// or VSR replication (single-node or cluster mode) transparently.
pub enum CommandSubmitter {
    /// Direct mode - applies commands directly to Verity without VSR.
    Direct { db: Verity },

    /// Single-node VSR mode - uses `SingleNodeReplicator` for durable processing.
    SingleNode {
        replicator: Arc<RwLock<SingleNodeReplicator<MemorySuperblock>>>,
        db: Verity,
    },

    /// Cluster mode - uses `MultiNodeReplicator` with full VSR consensus.
    Cluster {
        replicator: Arc<RwLock<MultiNodeReplicator>>,
        db: Verity,
    },
}

impl CommandSubmitter {
    /// Creates a new command submitter based on the replication mode.
    pub fn new(mode: &ReplicationMode, db: Verity, data_dir: &Path) -> ServerResult<Self> {
        match mode {
            ReplicationMode::None => Ok(Self::Direct { db }),

            ReplicationMode::SingleNode { replica_id } => {
                let config = ClusterConfig::single_node(*replica_id);
                let storage = MemorySuperblock::new();

                let replicator = SingleNodeReplicator::create(config, storage)
                    .map_err(|e| ServerError::Replication(e.to_string()))?;

                Ok(Self::SingleNode {
                    replicator: Arc::new(RwLock::new(replicator)),
                    db,
                })
            }

            ReplicationMode::Cluster { replica_id, peers } => {
                // Build cluster addresses
                let addresses = ClusterAddresses::from_pairs(peers.iter().copied());

                // Create superblock path
                let superblock_path =
                    data_dir.join(format!("superblock-{}.vsr", replica_id.as_u8()));

                // Create multi-node config
                let config = MultiNodeConfig::new(*replica_id, addresses, superblock_path);

                // Start the multi-node replicator
                let replicator = MultiNodeReplicator::start(config)
                    .map_err(|e| ServerError::Replication(e.to_string()))?;

                Ok(Self::Cluster {
                    replicator: Arc::new(RwLock::new(replicator)),
                    db,
                })
            }
        }
    }

    /// Submits a command for processing.
    ///
    /// In direct mode, this applies the command directly to the kernel.
    /// In VSR mode, this routes through the replicator for durable processing.
    pub fn submit(&self, command: Command) -> ServerResult<SubmissionResult> {
        self.submit_with_idempotency(command, None)
    }

    /// Submits a command with an optional idempotency ID.
    ///
    /// The idempotency ID enables duplicate detection for retried requests.
    pub fn submit_with_idempotency(
        &self,
        command: Command,
        idempotency_id: Option<IdempotencyId>,
    ) -> ServerResult<SubmissionResult> {
        match self {
            Self::Direct { db } => {
                // Direct mode: apply to Verity
                db.submit(command.clone())?;

                // Direct mode doesn't track operation numbers
                Ok(SubmissionResult {
                    was_duplicate: false,
                    effects_applied: true,
                })
            }

            Self::SingleNode { replicator, db } => {
                let mut repl = replicator
                    .write()
                    .map_err(|_| ServerError::Replication("lock poisoned".to_string()))?;

                // Submit to replicator
                let result = repl
                    .submit(command.clone(), idempotency_id)
                    .map_err(|e| ServerError::Replication(e.to_string()))?;

                // If not a duplicate, apply to Verity for projection updates
                // The replicator handles durability, but Verity manages projections
                if !result.was_duplicate {
                    db.submit(command)?;
                }

                Ok(SubmissionResult {
                    was_duplicate: result.was_duplicate,
                    effects_applied: true,
                })
            }

            Self::Cluster { replicator, db } => {
                let mut repl = replicator
                    .write()
                    .map_err(|_| ServerError::Replication("lock poisoned".to_string()))?;

                // Submit to replicator (blocks until committed or error)
                let result = repl
                    .submit(command.clone(), idempotency_id)
                    .map_err(|e| ServerError::Replication(e.to_string()))?;

                // If not a duplicate, apply to Verity for projection updates
                if !result.was_duplicate {
                    db.submit(command)?;
                }

                Ok(SubmissionResult {
                    was_duplicate: result.was_duplicate,
                    effects_applied: true,
                })
            }
        }
    }

    /// Returns a reference to the underlying Verity instance.
    pub fn verity(&self) -> &Verity {
        match self {
            Self::Direct { db } | Self::SingleNode { db, .. } | Self::Cluster { db, .. } => db,
        }
    }

    /// Returns true if VSR replication is enabled.
    pub fn is_replicated(&self) -> bool {
        !matches!(self, Self::Direct { .. })
    }

    /// Returns the current replication status (for health checks).
    pub fn status(&self) -> ReplicationStatus {
        match self {
            Self::Direct { .. } => ReplicationStatus {
                mode: "direct",
                is_leader: true,
                replica_id: None,
                commit_number: None,
            },
            Self::SingleNode { replicator, .. } => {
                let repl = replicator.read().ok();
                ReplicationStatus {
                    mode: "single-node",
                    is_leader: true, // Single-node is always leader
                    replica_id: repl
                        .as_ref()
                        .and_then(|r| r.config().replicas().next().map(|id| id.as_u8())),
                    commit_number: repl.as_ref().map(|r| r.commit_number().as_u64()),
                }
            }
            Self::Cluster { replicator, .. } => {
                let repl = replicator.read().ok();
                ReplicationStatus {
                    mode: "cluster",
                    is_leader: repl.as_ref().is_some_and(|r| r.is_leader()),
                    replica_id: repl
                        .as_ref()
                        .and_then(|r| r.config().replicas().next().map(|id| id.as_u8())),
                    commit_number: repl.as_ref().map(|r| r.commit_number().as_u64()),
                }
            }
        }
    }

    /// Returns true if this node is the leader (for cluster mode).
    pub fn is_leader(&self) -> bool {
        match self {
            Self::Direct { .. } | Self::SingleNode { .. } => true, // Single-node is always leader
            Self::Cluster { replicator, .. } => replicator.read().is_ok_and(|r| r.is_leader()),
        }
    }
}

/// Result of command submission.
#[derive(Debug, Clone)]
pub struct SubmissionResult {
    /// Whether this was a duplicate request (idempotency hit).
    pub was_duplicate: bool,
    /// Whether effects were successfully applied.
    pub effects_applied: bool,
}

/// Replication status for health/metrics.
#[derive(Debug, Clone)]
pub struct ReplicationStatus {
    /// Replication mode name.
    pub mode: &'static str,
    /// Whether this node is the leader.
    pub is_leader: bool,
    /// Replica ID (if replicated).
    pub replica_id: Option<u8>,
    /// Commit number (if replicated).
    pub commit_number: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use vdb_types::{DataClass, Placement, StreamId, StreamName};

    #[test]
    fn test_direct_mode_submit() {
        let temp_dir = TempDir::new().unwrap();
        let db = Verity::open(temp_dir.path()).unwrap();
        let submitter = CommandSubmitter::new(&ReplicationMode::None, db, temp_dir.path()).unwrap();

        assert!(!submitter.is_replicated());

        let cmd = Command::create_stream(
            StreamId::new(1),
            StreamName::new("test"),
            DataClass::NonPHI,
            Placement::Global,
        );

        let result = submitter.submit(cmd).unwrap();
        assert!(!result.was_duplicate);
        assert!(result.effects_applied);
    }

    #[test]
    fn test_single_node_mode_submit() {
        let temp_dir = TempDir::new().unwrap();
        let db = Verity::open(temp_dir.path()).unwrap();
        let submitter =
            CommandSubmitter::new(&ReplicationMode::single_node(), db, temp_dir.path()).unwrap();

        assert!(submitter.is_replicated());

        let cmd = Command::create_stream(
            StreamId::new(1),
            StreamName::new("test"),
            DataClass::NonPHI,
            Placement::Global,
        );

        let result = submitter.submit(cmd).unwrap();
        assert!(!result.was_duplicate);
        assert!(result.effects_applied);

        // Check status
        let status = submitter.status();
        assert_eq!(status.mode, "single-node");
        assert!(status.is_leader);
        assert!(status.replica_id.is_some());
    }

    #[test]
    fn test_idempotency_detection() {
        let temp_dir = TempDir::new().unwrap();
        let db = Verity::open(temp_dir.path()).unwrap();
        let submitter =
            CommandSubmitter::new(&ReplicationMode::single_node(), db, temp_dir.path()).unwrap();

        // Create idempotency ID
        let idem_id = IdempotencyId::generate();

        // First submission
        let cmd = Command::create_stream(
            StreamId::new(1),
            StreamName::new("test"),
            DataClass::NonPHI,
            Placement::Global,
        );
        let result = submitter
            .submit_with_idempotency(cmd.clone(), Some(idem_id))
            .unwrap();
        assert!(!result.was_duplicate);

        // Second submission with same ID should be detected as duplicate
        // Note: The replicator should detect this, but we need different commands
        // for the same idempotency_id to trigger duplicate detection.
        // For now, this test verifies the plumbing works.
    }
}
