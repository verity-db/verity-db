//! VSR cluster configuration.
//!
//! This module defines the configuration for a VSR cluster, including
//! cluster membership, timeouts, and protocol parameters.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::types::{quorum_size, ReplicaId, MAX_REPLICAS};

// ============================================================================
// Cluster Configuration
// ============================================================================

/// Configuration for a VSR cluster.
///
/// This configuration is immutable once a cluster is formed. Cluster
/// reconfiguration requires a separate protocol (not yet implemented).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// The replica IDs in the cluster.
    ///
    /// Must be sorted and contain no duplicates.
    /// Length must be odd (2f+1) for proper quorum behavior.
    replicas: Vec<ReplicaId>,

    /// Protocol timeout configuration.
    pub timeouts: TimeoutConfig,

    /// Checkpoint configuration.
    pub checkpoint: CheckpointConfig,
}

impl ClusterConfig {
    /// Creates a new cluster configuration.
    ///
    /// # Arguments
    ///
    /// * `replicas` - List of replica IDs in the cluster
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `replicas` is empty
    /// - `replicas` has an even number of elements (must be 2f+1)
    /// - `replicas` contains duplicates
    /// - `replicas` exceeds `MAX_REPLICAS`
    pub fn new(mut replicas: Vec<ReplicaId>) -> Self {
        assert!(!replicas.is_empty(), "cluster must have at least one replica");
        assert!(
            replicas.len() % 2 == 1,
            "cluster size must be odd (2f+1) for proper quorum behavior"
        );
        assert!(
            replicas.len() <= MAX_REPLICAS,
            "cluster size exceeds MAX_REPLICAS"
        );

        // Sort and check for duplicates
        replicas.sort();
        for i in 1..replicas.len() {
            assert!(
                replicas[i - 1] != replicas[i],
                "cluster contains duplicate replica IDs"
            );
        }

        Self {
            replicas,
            timeouts: TimeoutConfig::default(),
            checkpoint: CheckpointConfig::default(),
        }
    }

    /// Creates a configuration for a single-node cluster.
    ///
    /// Single-node clusters are useful for development and testing.
    /// They provide no fault tolerance but maintain the same API.
    pub fn single_node(replica_id: ReplicaId) -> Self {
        Self {
            replicas: vec![replica_id],
            timeouts: TimeoutConfig::single_node(),
            checkpoint: CheckpointConfig::default(),
        }
    }

    /// Returns the number of replicas in the cluster.
    pub fn cluster_size(&self) -> usize {
        self.replicas.len()
    }

    /// Returns the quorum size for this cluster.
    pub fn quorum_size(&self) -> usize {
        quorum_size(self.replicas.len())
    }

    /// Returns the maximum number of failures this cluster can tolerate.
    pub fn max_failures(&self) -> usize {
        self.replicas.len() / 2
    }

    /// Returns true if this is a single-node cluster.
    pub fn is_single_node(&self) -> bool {
        self.replicas.len() == 1
    }

    /// Returns an iterator over replica IDs.
    pub fn replicas(&self) -> impl Iterator<Item = ReplicaId> + '_ {
        self.replicas.iter().copied()
    }

    /// Returns the replica ID at the given index.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn replica_at(&self, index: usize) -> ReplicaId {
        self.replicas[index]
    }

    /// Returns the index of a replica ID in the cluster.
    ///
    /// Returns `None` if the replica is not in the cluster.
    pub fn replica_index(&self, id: ReplicaId) -> Option<usize> {
        self.replicas.iter().position(|&r| r == id)
    }

    /// Returns true if the replica is a member of this cluster.
    pub fn contains(&self, id: ReplicaId) -> bool {
        self.replicas.contains(&id)
    }

    /// Determines the leader for a given view.
    ///
    /// Leadership rotates through replicas based on view number.
    pub fn leader_for_view(&self, view: crate::types::ViewNumber) -> ReplicaId {
        let index = (view.as_u64() as usize) % self.replicas.len();
        self.replicas[index]
    }

    /// Returns the other replicas (excluding the given replica).
    pub fn others(&self, exclude: ReplicaId) -> impl Iterator<Item = ReplicaId> + '_ {
        self.replicas.iter().copied().filter(move |&r| r != exclude)
    }

    /// Sets the timeout configuration.
    pub fn with_timeouts(mut self, timeouts: TimeoutConfig) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Sets the checkpoint configuration.
    pub fn with_checkpoint(mut self, checkpoint: CheckpointConfig) -> Self {
        self.checkpoint = checkpoint;
        self
    }
}

// ============================================================================
// Timeout Configuration
// ============================================================================

/// Timeout configuration for VSR protocol operations.
///
/// These timeouts control how long a replica waits before taking action.
/// Timeouts should be tuned based on network latency and failure detection
/// requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeoutConfig {
    /// Time to wait for `PrepareOK` before retrying.
    ///
    /// If a leader doesn't receive enough `PrepareOKs` within this time,
    /// it retransmits the Prepare message.
    pub prepare_timeout: Duration,

    /// Time between heartbeats from the leader.
    ///
    /// Backups expect to receive heartbeats at this interval.
    /// Missing heartbeats trigger view change consideration.
    pub heartbeat_interval: Duration,

    /// Time without heartbeat before starting view change.
    ///
    /// Should be significantly larger than `heartbeat_interval` to
    /// avoid spurious view changes due to network jitter.
    pub view_change_timeout: Duration,

    /// Time to wait for `DoViewChange` messages during view change.
    ///
    /// If a replica doesn't receive enough `DoViewChange` messages,
    /// it may abandon the view change and try a higher view.
    pub view_change_wait: Duration,

    /// Time between recovery progress checks.
    ///
    /// During recovery, a replica periodically retries state transfer
    /// requests if they haven't been answered.
    pub recovery_retry: Duration,

    /// Time between repair requests for missing/corrupt entries.
    ///
    /// When a replica detects missing or corrupt log entries, it
    /// requests repairs at this interval.
    pub repair_interval: Duration,
}

impl TimeoutConfig {
    /// Creates timeout configuration suitable for single-node operation.
    ///
    /// Single-node clusters don't need network timeouts, so we use
    /// very short intervals.
    pub fn single_node() -> Self {
        Self {
            prepare_timeout: Duration::from_millis(1),
            heartbeat_interval: Duration::from_millis(10),
            view_change_timeout: Duration::from_millis(50),
            view_change_wait: Duration::from_millis(10),
            recovery_retry: Duration::from_millis(10),
            repair_interval: Duration::from_millis(10),
        }
    }

    /// Creates timeout configuration suitable for simulation testing.
    ///
    /// Uses very short intervals for fast test execution.
    pub fn simulation() -> Self {
        Self {
            prepare_timeout: Duration::from_micros(100),
            heartbeat_interval: Duration::from_micros(500),
            view_change_timeout: Duration::from_millis(2),
            view_change_wait: Duration::from_micros(500),
            recovery_retry: Duration::from_micros(500),
            repair_interval: Duration::from_micros(500),
        }
    }

    /// Creates timeout configuration suitable for local development.
    ///
    /// Uses moderate intervals for reasonable responsiveness.
    pub fn development() -> Self {
        Self {
            prepare_timeout: Duration::from_millis(100),
            heartbeat_interval: Duration::from_millis(250),
            view_change_timeout: Duration::from_secs(1),
            view_change_wait: Duration::from_millis(500),
            recovery_retry: Duration::from_millis(500),
            repair_interval: Duration::from_millis(500),
        }
    }

    /// Creates timeout configuration suitable for production.
    ///
    /// Uses longer intervals to handle network variability and
    /// avoid spurious failures.
    pub fn production() -> Self {
        Self {
            prepare_timeout: Duration::from_millis(500),
            heartbeat_interval: Duration::from_secs(1),
            view_change_timeout: Duration::from_secs(5),
            view_change_wait: Duration::from_secs(2),
            recovery_retry: Duration::from_secs(2),
            repair_interval: Duration::from_secs(1),
        }
    }
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self::production()
    }
}

// ============================================================================
// Checkpoint Configuration
// ============================================================================

/// Configuration for VSR checkpoints.
///
/// Checkpoints provide consistent snapshots of the replicated state,
/// enabling efficient recovery and state transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointConfig {
    /// Number of operations between checkpoints.
    ///
    /// More frequent checkpoints reduce recovery time but increase
    /// storage and CPU overhead.
    pub checkpoint_interval: u64,

    /// Number of old checkpoints to retain.
    ///
    /// Old checkpoints enable state transfer to slow replicas.
    /// More retained checkpoints use more storage but allow recovery
    /// from longer outages.
    pub retain_count: usize,

    /// Whether to require Ed25519 signatures on checkpoints.
    ///
    /// Signed checkpoints provide cryptographic verification but
    /// add computational overhead.
    pub require_signatures: bool,
}

impl CheckpointConfig {
    /// Creates checkpoint configuration for testing.
    ///
    /// Uses frequent checkpoints for comprehensive test coverage.
    pub fn testing() -> Self {
        Self {
            checkpoint_interval: 10,
            retain_count: 3,
            require_signatures: false,
        }
    }
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            checkpoint_interval: 1000,
            retain_count: 5,
            require_signatures: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ViewNumber;

    #[test]
    fn cluster_config_creation() {
        let replicas = vec![
            ReplicaId::new(0),
            ReplicaId::new(1),
            ReplicaId::new(2),
        ];
        let config = ClusterConfig::new(replicas);

        assert_eq!(config.cluster_size(), 3);
        assert_eq!(config.quorum_size(), 2);
        assert_eq!(config.max_failures(), 1);
        assert!(!config.is_single_node());
    }

    #[test]
    fn single_node_cluster() {
        let config = ClusterConfig::single_node(ReplicaId::new(0));

        assert_eq!(config.cluster_size(), 1);
        assert_eq!(config.quorum_size(), 1);
        assert_eq!(config.max_failures(), 0);
        assert!(config.is_single_node());
    }

    #[test]
    fn leader_rotation() {
        let replicas = vec![
            ReplicaId::new(0),
            ReplicaId::new(1),
            ReplicaId::new(2),
        ];
        let config = ClusterConfig::new(replicas);

        assert_eq!(config.leader_for_view(ViewNumber::new(0)), ReplicaId::new(0));
        assert_eq!(config.leader_for_view(ViewNumber::new(1)), ReplicaId::new(1));
        assert_eq!(config.leader_for_view(ViewNumber::new(2)), ReplicaId::new(2));
        assert_eq!(config.leader_for_view(ViewNumber::new(3)), ReplicaId::new(0)); // wraps
    }

    #[test]
    fn replica_membership() {
        let replicas = vec![
            ReplicaId::new(1),
            ReplicaId::new(3),
            ReplicaId::new(5),
        ];
        let config = ClusterConfig::new(replicas);

        assert!(config.contains(ReplicaId::new(1)));
        assert!(config.contains(ReplicaId::new(3)));
        assert!(config.contains(ReplicaId::new(5)));
        assert!(!config.contains(ReplicaId::new(0)));
        assert!(!config.contains(ReplicaId::new(2)));
    }

    #[test]
    fn others_excludes_self() {
        let replicas = vec![
            ReplicaId::new(0),
            ReplicaId::new(1),
            ReplicaId::new(2),
        ];
        let config = ClusterConfig::new(replicas);

        let others: Vec<_> = config.others(ReplicaId::new(1)).collect();
        assert_eq!(others, vec![ReplicaId::new(0), ReplicaId::new(2)]);
    }

    #[test]
    #[should_panic(expected = "must be odd")]
    fn even_cluster_size_panics() {
        let replicas = vec![
            ReplicaId::new(0),
            ReplicaId::new(1),
        ];
        let _ = ClusterConfig::new(replicas);
    }

    #[test]
    #[should_panic(expected = "at least one replica")]
    fn empty_cluster_panics() {
        let replicas: Vec<ReplicaId> = vec![];
        let _ = ClusterConfig::new(replicas);
    }

    #[test]
    #[should_panic(expected = "duplicate")]
    fn duplicate_replicas_panics() {
        let replicas = vec![
            ReplicaId::new(0),
            ReplicaId::new(0),
            ReplicaId::new(1),
        ];
        let _ = ClusterConfig::new(replicas);
    }
}
