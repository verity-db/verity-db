//! Core types for Viewstamped Replication.
//!
//! This module defines the fundamental types used throughout the VSR implementation:
//! - [`ReplicaId`] - Unique identifier for a replica in the cluster
//! - [`ViewNumber`] - Monotonically increasing view number
//! - [`OpNumber`] - Operation sequence number within a view
//! - [`ReplicaStatus`] - Current status of a replica
//! - [`LogEntry`] - Entry in the replicated log

use std::fmt::{Debug, Display};

use serde::{Deserialize, Serialize};
use vdb_kernel::Command;
use vdb_types::IdempotencyId;

// ============================================================================
// Replica Identifier - Copy (single byte)
// ============================================================================

/// Maximum number of replicas in a VSR cluster.
///
/// VSR requires 2f+1 replicas to tolerate f failures. With a max of 255 replicas,
/// we can tolerate up to 127 simultaneous failures (extreme configuration).
/// Typical deployments use 3-7 replicas.
pub const MAX_REPLICAS: usize = 255;

/// Unique identifier for a replica in the cluster.
///
/// Uses `u8` internally since clusters are small (typically 3-7 nodes).
/// A replica ID is assigned during cluster formation and never changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ReplicaId(u8);

impl ReplicaId {
    /// Creates a new replica ID.
    ///
    /// # Panics
    ///
    /// Panics if `id` exceeds `MAX_REPLICAS`.
    pub fn new(id: u8) -> Self {
        debug_assert!(
            (id as usize) < MAX_REPLICAS,
            "replica ID exceeds MAX_REPLICAS"
        );
        Self(id)
    }

    /// Returns the replica ID as a `u8`.
    pub fn as_u8(&self) -> u8 {
        self.0
    }

    /// Returns the replica ID as a `usize` for indexing.
    pub fn as_usize(&self) -> usize {
        self.0 as usize
    }
}

impl Display for ReplicaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "R{}", self.0)
    }
}

impl From<u8> for ReplicaId {
    fn from(id: u8) -> Self {
        Self::new(id)
    }
}

impl From<ReplicaId> for u8 {
    fn from(id: ReplicaId) -> Self {
        id.0
    }
}

// ============================================================================
// View Number - Copy (8-byte value)
// ============================================================================

/// Monotonically increasing view number.
///
/// A view represents a period during which a specific replica is the leader.
/// View numbers only increase, never decrease. When a leader fails, the cluster
/// transitions to a higher view with a new leader.
///
/// # Invariants
///
/// - View numbers are globally ordered across the cluster
/// - A replica's current view only increases over time
/// - Operations from a lower view are rejected
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct ViewNumber(u64);

impl ViewNumber {
    /// The initial view number (view 0).
    pub const ZERO: ViewNumber = ViewNumber(0);

    /// Creates a new view number.
    pub fn new(view: u64) -> Self {
        Self(view)
    }

    /// Returns the view number as a `u64`.
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Returns the next view number (incremented by 1).
    pub fn next(&self) -> Self {
        ViewNumber(self.0.saturating_add(1))
    }

    /// Returns true if this is view zero (initial view).
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl Display for ViewNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "v{}", self.0)
    }
}

impl From<u64> for ViewNumber {
    fn from(view: u64) -> Self {
        Self(view)
    }
}

impl From<ViewNumber> for u64 {
    fn from(view: ViewNumber) -> Self {
        view.0
    }
}

// ============================================================================
// Operation Number - Copy (8-byte value)
// ============================================================================

/// Operation sequence number within the replicated log.
///
/// Operation numbers are assigned sequentially by the leader and represent
/// the position of an operation in the total order. Unlike offsets (which
/// are per-stream), operation numbers are global across all operations.
///
/// # Relationship to Offset
///
/// - `OpNumber`: Global sequence in the VSR log
/// - `Offset`: Per-stream sequence in the storage layer
///
/// A single VSR operation may affect multiple streams, each getting their
/// own offset increment, but the operation itself has one `OpNumber`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct OpNumber(u64);

impl OpNumber {
    /// The initial operation number (op 0, before any operations).
    pub const ZERO: OpNumber = OpNumber(0);

    /// Creates a new operation number.
    pub fn new(op: u64) -> Self {
        Self(op)
    }

    /// Returns the operation number as a `u64`.
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Returns the next operation number (incremented by 1).
    pub fn next(&self) -> Self {
        OpNumber(self.0.saturating_add(1))
    }

    /// Returns true if this is operation zero.
    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// Returns the number of operations between self and other.
    ///
    /// Returns 0 if other > self.
    pub fn distance_to(&self, other: OpNumber) -> u64 {
        other.0.saturating_sub(self.0)
    }
}

impl Display for OpNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "op#{}", self.0)
    }
}

impl From<u64> for OpNumber {
    fn from(op: u64) -> Self {
        Self(op)
    }
}

impl From<OpNumber> for u64 {
    fn from(op: OpNumber) -> Self {
        op.0
    }
}

// ============================================================================
// Replica Status - Copy (small enum)
// ============================================================================

/// Current status of a replica in the VSR protocol.
///
/// A replica transitions through these states based on protocol events.
/// The status determines which messages a replica can process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ReplicaStatus {
    /// Normal operation: processing client requests and replicating.
    ///
    /// In this status, the replica:
    /// - If leader: accepts client requests, sends Prepare messages
    /// - If backup: accepts Prepare messages, sends `PrepareOK`
    #[default]
    Normal,

    /// View change in progress: electing a new leader.
    ///
    /// In this status, the replica:
    /// - Participates in view change protocol
    /// - Does not process client requests
    /// - Does not accept Prepare messages from old view
    ViewChange,

    /// Recovery in progress: rebuilding state from peers.
    ///
    /// In this status, the replica:
    /// - Requests state from other replicas
    /// - Does not participate in consensus
    /// - Cannot become leader
    Recovering,
}

impl ReplicaStatus {
    /// Returns true if the replica can process client requests.
    pub fn can_process_requests(&self) -> bool {
        matches!(self, ReplicaStatus::Normal)
    }

    /// Returns true if the replica can participate in consensus.
    pub fn can_participate(&self) -> bool {
        matches!(self, ReplicaStatus::Normal | ReplicaStatus::ViewChange)
    }
}

impl Display for ReplicaStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplicaStatus::Normal => write!(f, "normal"),
            ReplicaStatus::ViewChange => write!(f, "view-change"),
            ReplicaStatus::Recovering => write!(f, "recovering"),
        }
    }
}

// ============================================================================
// Log Entry - Clone (contains Command which may have heap data)
// ============================================================================

/// An entry in the replicated log.
///
/// Each log entry contains a command to be executed by the state machine
/// along with metadata for ordering and verification.
///
/// # Invariants
///
/// - `op_number` is unique and sequential
/// - `view` is the view in which the entry was created
/// - `checksum` covers all fields for integrity verification
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Position in the replicated log.
    pub op_number: OpNumber,

    /// View in which this entry was created.
    pub view: ViewNumber,

    /// The command to execute.
    pub command: Command,

    /// Optional idempotency ID for duplicate detection.
    pub idempotency_id: Option<IdempotencyId>,

    /// CRC32 checksum of the entry for integrity verification.
    pub checksum: u32,
}

impl LogEntry {
    /// Creates a new log entry.
    ///
    /// # Arguments
    ///
    /// * `op_number` - Position in the log
    /// * `view` - View in which entry was created
    /// * `command` - Command to execute
    /// * `idempotency_id` - Optional ID for duplicate detection
    ///
    /// The checksum is computed automatically.
    pub fn new(
        op_number: OpNumber,
        view: ViewNumber,
        command: Command,
        idempotency_id: Option<IdempotencyId>,
    ) -> Self {
        let mut entry = Self {
            op_number,
            view,
            command,
            idempotency_id,
            checksum: 0,
        };
        entry.checksum = entry.compute_checksum();
        entry
    }

    /// Computes the CRC32 checksum of the entry.
    ///
    /// Covers all fields except the checksum itself.
    fn compute_checksum(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();

        // Hash op_number and view
        hasher.update(&self.op_number.as_u64().to_le_bytes());
        hasher.update(&self.view.as_u64().to_le_bytes());

        // Hash command (serialize to bytes)
        // Note: In production, this would use a stable serialization format
        if let Ok(command_bytes) = serde_json::to_vec(&self.command) {
            hasher.update(&command_bytes);
        }

        // Hash idempotency_id if present
        if let Some(id) = &self.idempotency_id {
            hasher.update(&[1u8]); // presence marker
            hasher.update(id.as_bytes());
        } else {
            hasher.update(&[0u8]); // absence marker
        }

        hasher.finalize()
    }

    /// Verifies the entry's checksum.
    ///
    /// Returns `true` if the checksum is valid.
    pub fn verify_checksum(&self) -> bool {
        self.checksum == self.compute_checksum()
    }
}

// ============================================================================
// Commit Number - Copy (tracks committed position)
// ============================================================================

/// The highest operation number known to be committed.
///
/// A commit number represents the point up to which all operations are
/// guaranteed to be durable on a quorum of replicas. Operations at or
/// below the commit number are safe to execute.
///
/// # Invariants
///
/// - Commit number only increases
/// - Commit number <= highest prepared op number
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct CommitNumber(OpNumber);

impl CommitNumber {
    /// The initial commit number (nothing committed).
    pub const ZERO: CommitNumber = CommitNumber(OpNumber::ZERO);

    /// Creates a new commit number from an operation number.
    pub fn new(op: OpNumber) -> Self {
        Self(op)
    }

    /// Returns the underlying operation number.
    pub fn as_op_number(&self) -> OpNumber {
        self.0
    }

    /// Returns the commit number as a `u64`.
    pub fn as_u64(&self) -> u64 {
        self.0.as_u64()
    }

    /// Returns true if this operation is committed.
    pub fn is_committed(&self, op: OpNumber) -> bool {
        op <= self.0
    }
}

impl Display for CommitNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "commit#{}", self.0.as_u64())
    }
}

impl From<OpNumber> for CommitNumber {
    fn from(op: OpNumber) -> Self {
        Self::new(op)
    }
}

impl From<CommitNumber> for OpNumber {
    fn from(commit: CommitNumber) -> Self {
        commit.0
    }
}

// ============================================================================
// Nonce - Copy (random value for protocol freshness)
// ============================================================================

/// Length of nonce in bytes.
pub const NONCE_LENGTH: usize = 16;

/// Random nonce for protocol message freshness.
///
/// Used in recovery and repair protocols to ensure responses correspond
/// to specific requests and prevent replay attacks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Nonce([u8; NONCE_LENGTH]);

impl Nonce {
    /// Creates a nonce from raw bytes.
    pub fn from_bytes(bytes: [u8; NONCE_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the nonce as a byte slice.
    pub fn as_bytes(&self) -> &[u8; NONCE_LENGTH] {
        &self.0
    }

    /// Generates a new random nonce.
    ///
    /// # Panics
    ///
    /// Panics if the OS CSPRNG fails.
    pub fn generate() -> Self {
        let mut bytes = [0u8; NONCE_LENGTH];
        getrandom::fill(&mut bytes).expect("CSPRNG failure is catastrophic");
        Self(bytes)
    }
}

impl Debug for Nonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Nonce({:02x}{:02x}{:02x}{:02x}...)",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

impl Default for Nonce {
    fn default() -> Self {
        Self([0u8; NONCE_LENGTH])
    }
}

// ============================================================================
// Quorum helpers
// ============================================================================

/// Calculates the quorum size for a cluster.
///
/// VSR requires a quorum of `f+1` out of `2f+1` replicas for any operation.
/// This ensures that any two quorums overlap by at least one replica.
///
/// # Arguments
///
/// * `cluster_size` - Total number of replicas in the cluster
///
/// # Returns
///
/// The minimum number of replicas required for a quorum.
///
/// # Panics
///
/// Panics if `cluster_size` is 0.
pub fn quorum_size(cluster_size: usize) -> usize {
    debug_assert!(cluster_size > 0, "cluster size must be positive");
    (cluster_size / 2) + 1
}

/// Returns the maximum number of failures tolerable for a cluster size.
///
/// For `2f+1` replicas, we can tolerate `f` failures.
///
/// # Arguments
///
/// * `cluster_size` - Total number of replicas in the cluster
pub fn max_failures(cluster_size: usize) -> usize {
    cluster_size / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replica_id_display() {
        let id = ReplicaId::new(5);
        assert_eq!(format!("{id}"), "R5");
    }

    #[test]
    fn view_number_ordering() {
        let v1 = ViewNumber::new(1);
        let v2 = ViewNumber::new(2);
        assert!(v1 < v2);
        assert_eq!(v1.next(), v2);
    }

    #[test]
    fn op_number_distance() {
        let op1 = OpNumber::new(5);
        let op2 = OpNumber::new(10);
        assert_eq!(op1.distance_to(op2), 5);
        assert_eq!(op2.distance_to(op1), 0); // saturating
    }

    #[test]
    fn quorum_calculations() {
        // 3 replicas: quorum is 2, tolerates 1 failure
        assert_eq!(quorum_size(3), 2);
        assert_eq!(max_failures(3), 1);

        // 5 replicas: quorum is 3, tolerates 2 failures
        assert_eq!(quorum_size(5), 3);
        assert_eq!(max_failures(5), 2);

        // 7 replicas: quorum is 4, tolerates 3 failures
        assert_eq!(quorum_size(7), 4);
        assert_eq!(max_failures(7), 3);
    }

    #[test]
    fn log_entry_checksum() {
        let entry = LogEntry::new(
            OpNumber::new(1),
            ViewNumber::new(0),
            Command::create_stream_with_auto_id(
                "test".into(),
                vdb_types::DataClass::NonPHI,
                vdb_types::Placement::Global,
            ),
            None,
        );

        assert!(entry.verify_checksum());

        // Tamper with entry
        let mut tampered = entry.clone();
        tampered.op_number = OpNumber::new(999);
        assert!(!tampered.verify_checksum());
    }

    #[test]
    fn replica_status_transitions() {
        assert!(ReplicaStatus::Normal.can_process_requests());
        assert!(!ReplicaStatus::ViewChange.can_process_requests());
        assert!(!ReplicaStatus::Recovering.can_process_requests());

        assert!(ReplicaStatus::Normal.can_participate());
        assert!(ReplicaStatus::ViewChange.can_participate());
        assert!(!ReplicaStatus::Recovering.can_participate());
    }

    #[test]
    fn commit_number_is_committed() {
        let commit = CommitNumber::new(OpNumber::new(5));

        assert!(commit.is_committed(OpNumber::new(3)));
        assert!(commit.is_committed(OpNumber::new(5)));
        assert!(!commit.is_committed(OpNumber::new(6)));
    }
}
