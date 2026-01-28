//! # vdb-vsr: Viewstamped Replication for `VerityDB`
//!
//! This crate implements the Viewstamped Replication (VSR) consensus protocol
//! for `VerityDB`, enabling distributed replication with guaranteed data durability
//! and consistency.
//!
//! ## Overview
//!
//! VSR is a state machine replication protocol that provides:
//! - **Linearizable consistency**: Operations appear to execute atomically
//! - **Fault tolerance**: Tolerates f failures with 2f+1 replicas
//! - **Durability**: Operations are replicated before acknowledgment
//!
//! ## Architecture
//!
//! ```text
//! Client Request
//!       │
//!       ▼
//! ┌─────────────────┐
//! │  VSR Leader     │  Assign op_number, broadcast Prepare
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │  VSR Backups    │  Validate, persist, send PrepareOK
//! └────────┬────────┘
//!          │ quorum
//!          ▼
//! ┌─────────────────┐
//! │  VSR Leader     │  Mark committed, apply_committed()
//! └────────┬────────┘
//!          │
//!          ▼
//! ┌─────────────────┐
//! │  vdb-kernel     │  Pure state transition, produce Effects
//! └─────────────────┘
//! ```
//!
//! ## Key Components
//!
//! - [`types`]: Core types (`ReplicaId`, `ViewNumber`, `OpNumber`, etc.)
//! - [`config`]: Cluster configuration and timeouts
//! - [`message`]: VSR protocol messages
//! - [`superblock`]: 4-copy atomic metadata persistence
//!
//! ## Design Principles
//!
//! 1. **Functional Core / Imperative Shell (FCIS)**: The replica state machine
//!    is pure and deterministic, enabling comprehensive simulation testing.
//!
//! 2. **Protocol-Aware Recovery (PAR)**: Safe log truncation requires a NACK
//!    quorum to distinguish "never seen" from "seen but corrupt".
//!
//! 3. **Transparent Repair**: Checksum-based corruption detection with
//!    automatic repair from healthy peers.
//!
//! 4. **Cryptographic Checkpoints**: Ed25519 signed Merkle roots provide
//!    tamper-evident state verification.
//!
//! ## Example
//!
//! ```ignore
//! use vdb_vsr::{ClusterConfig, ReplicaId, Superblock, MemorySuperblock};
//!
//! // Create a 3-node cluster
//! let replicas = vec![
//!     ReplicaId::new(0),
//!     ReplicaId::new(1),
//!     ReplicaId::new(2),
//! ];
//! let config = ClusterConfig::new(replicas);
//!
//! // Quorum is 2 of 3
//! assert_eq!(config.quorum_size(), 2);
//!
//! // Initialize superblock for replica 0
//! let storage = MemorySuperblock::new();
//! let superblock = Superblock::create(storage, ReplicaId::new(0))?;
//! ```

// Module declarations
pub mod checkpoint;
pub mod config;
pub mod framing;
pub mod idempotency;
pub mod message;
pub mod replica;
pub mod single_node;
pub mod superblock;
pub mod tcp_transport;
pub mod transport;
pub mod types;

// Multi-node modules
pub mod event_loop;
pub mod multi_node;

#[cfg(test)]
mod simulation;

// Re-exports for convenient access
pub use checkpoint::{
    Checkpoint, CheckpointBuilder, CheckpointData, MERKLE_ROOT_LENGTH, MerkleRoot,
    ReplicaSignature, compute_merkle_root,
};
pub use config::{CheckpointConfig, ClusterConfig, TimeoutConfig};
pub use idempotency::{
    DEFAULT_MIN_RETENTION, DuplicateStatus, IdempotencyConfig, IdempotencyResult, IdempotencyTable,
    MAX_ENTRIES,
};
pub use message::{
    Commit, DoViewChange, Heartbeat, Message, MessagePayload, Nack, NackReason, Prepare, PrepareOk,
    RecoveryRequest, RecoveryResponse, RepairRequest, RepairResponse, StartView, StartViewChange,
    StateTransferRequest, StateTransferResponse,
};
pub use replica::{ReplicaEvent, ReplicaOutput, ReplicaState, TimeoutKind};
pub use single_node::{Replicator, SingleNodeReplicator, SubmitResult};
pub use superblock::{
    MemorySuperblock, SUPERBLOCK_COPIES, SUPERBLOCK_COPY_SIZE, SUPERBLOCK_TOTAL_SIZE, Superblock,
    SuperblockData,
};
pub use transport::{MessageSink, NullTransport, Transport};
pub use types::{
    CommitNumber, LogEntry, MAX_REPLICAS, NONCE_LENGTH, Nonce, OpNumber, ReplicaId, ReplicaStatus,
    ViewNumber, max_failures, quorum_size,
};

// Multi-node exports
pub use event_loop::{EventLoop, EventLoopConfig, EventLoopHandle};
pub use framing::{FrameDecoder, FrameEncoder, FramingError, HEADER_SIZE};
pub use multi_node::{MultiNodeConfig, MultiNodeReplicator};
pub use tcp_transport::{ClusterAddresses, TcpTransport};

// ============================================================================
// Error Types
// ============================================================================

/// Errors that can occur during VSR operations.
#[derive(Debug, thiserror::Error)]
pub enum VsrError {
    /// The replica is not the leader for this view.
    #[error("not leader for view {view}")]
    NotLeader { view: ViewNumber },

    /// The operation has an unexpected view number.
    #[error("view mismatch: expected {expected}, got {actual}")]
    ViewMismatch {
        expected: ViewNumber,
        actual: ViewNumber,
    },

    /// The operation number is out of sequence.
    #[error("operation out of sequence: expected {expected}, got {actual}")]
    OpOutOfSequence {
        expected: OpNumber,
        actual: OpNumber,
    },

    /// A quorum could not be reached.
    #[error("quorum not reached: need {needed}, got {got}")]
    QuorumNotReached { needed: usize, got: usize },

    /// The replica is in an invalid state for this operation.
    #[error("invalid replica state: {status} (expected {expected})")]
    InvalidState {
        status: ReplicaStatus,
        expected: &'static str,
    },

    /// Log entry checksum mismatch.
    #[error("checksum mismatch for op {op_number}")]
    ChecksumMismatch { op_number: OpNumber },

    /// Storage I/O error.
    #[error("storage error: {0}")]
    Storage(#[from] std::io::Error),

    /// The replica is not a member of the cluster.
    #[error("unknown replica: {0}")]
    UnknownReplica(ReplicaId),

    /// The request nonce doesn't match.
    #[error("nonce mismatch")]
    NonceMismatch,

    /// Recovery failed.
    #[error("recovery failed: {reason}")]
    RecoveryFailed { reason: String },
}

/// Result type for VSR operations.
pub type VsrResult<T> = Result<T, VsrError>;

#[cfg(test)]
mod tests;
