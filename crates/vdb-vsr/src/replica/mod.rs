//! VSR replica state machine.
//!
//! This module implements the core Viewstamped Replication protocol as a
//! pure, deterministic state machine following the FCIS pattern.
//!
//! # Architecture
//!
//! The replica state machine is completely pure:
//! - Takes messages/events as input
//! - Produces new state, outgoing messages, and effects as output
//! - No I/O, no clocks, no randomness
//!
//! This enables comprehensive simulation testing under `vdb-sim`.
//!
//! # Protocol Overview
//!
//! ## Normal Operation
//!
//! ```text
//! Client ──Request──► Leader
//!                       │
//!                       ├──Prepare──► Backup₁
//!                       ├──Prepare──► Backup₂
//!                       │              │
//!                       │◄─PrepareOK───┤
//!                       │◄─PrepareOK───┘
//!                       │
//!                       ├──Commit───► All
//!                       │
//! Client ◄──Reply─────┘
//! ```
//!
//! ## View Change
//!
//! ```text
//! Backup ──StartViewChange──► All (on leader timeout)
//!           │
//!           ▼ (quorum)
//! Backup ──DoViewChange──► New Leader
//!           │
//!           ▼ (quorum)
//! New Leader ──StartView──► All
//! ```
//!
//! # Key Types
//!
//! - [`ReplicaState`]: The core state machine state
//! - [`ReplicaOutput`]: Output from processing an event
//! - [`ReplicaEvent`]: Events that can trigger state transitions

mod normal;
mod recovery;
mod repair;
mod state;
mod state_transfer;
mod view_change;

pub use recovery::RecoveryState;
pub use repair::RepairState;
pub use state::*;
pub use state_transfer::StateTransferState;

use crate::message::{Message, MessagePayload};
use crate::types::{OpNumber, ReplicaId};
use vdb_kernel::Effect;

// ============================================================================
// Replica Output
// ============================================================================

/// Output produced by the replica state machine.
///
/// This is the result of processing an event. The caller (runtime) is
/// responsible for:
/// 1. Sending the outgoing messages via the transport
/// 2. Executing the effects (storage writes, etc.)
/// 3. Updating any external state based on the result
#[derive(Debug, Default)]
pub struct ReplicaOutput {
    /// Messages to send to other replicas.
    pub messages: Vec<Message>,

    /// Effects to execute (storage writes, projections, etc.).
    pub effects: Vec<Effect>,

    /// If Some, a client request was committed at this op number.
    pub committed_op: Option<OpNumber>,
}

impl ReplicaOutput {
    /// Creates an empty output (no messages, no effects).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Creates output with only messages.
    pub fn with_messages(messages: Vec<Message>) -> Self {
        Self {
            messages,
            effects: Vec::new(),
            committed_op: None,
        }
    }

    /// Creates output with messages and effects.
    pub fn with_messages_and_effects(messages: Vec<Message>, effects: Vec<Effect>) -> Self {
        Self {
            messages,
            effects,
            committed_op: None,
        }
    }

    /// Adds a committed operation to the output.
    pub fn with_committed(mut self, op: OpNumber) -> Self {
        self.committed_op = Some(op);
        self
    }

    /// Returns true if there are no messages or effects.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.effects.is_empty() && self.committed_op.is_none()
    }

    /// Merges another output into this one.
    pub fn merge(&mut self, other: ReplicaOutput) {
        self.messages.extend(other.messages);
        self.effects.extend(other.effects);
        if other.committed_op.is_some() {
            self.committed_op = other.committed_op;
        }
    }
}

// ============================================================================
// Replica Event
// ============================================================================

/// Events that can trigger replica state transitions.
#[derive(Debug, Clone)]
pub enum ReplicaEvent {
    /// Received a message from another replica.
    Message(Message),

    /// A timeout fired.
    Timeout(TimeoutKind),

    /// Client submitted a request (leader only).
    ClientRequest {
        /// The command to execute.
        command: vdb_kernel::Command,
        /// Optional idempotency ID.
        idempotency_id: Option<vdb_types::IdempotencyId>,
    },

    /// Tick event for periodic housekeeping.
    Tick,
}

/// Types of timeouts that can fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeoutKind {
    /// Heartbeat timeout (backup hasn't heard from leader).
    Heartbeat,

    /// Prepare timeout (leader hasn't received enough `PrepareOK`s).
    Prepare(OpNumber),

    /// View change timeout (view change taking too long).
    ViewChange,

    /// Recovery timeout (recovery taking too long).
    Recovery,
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Creates a message from this replica to a specific target.
pub(crate) fn msg_to(from: ReplicaId, to: ReplicaId, payload: MessagePayload) -> Message {
    Message::targeted(from, to, payload)
}

/// Creates a broadcast message from this replica.
pub(crate) fn msg_broadcast(from: ReplicaId, payload: MessagePayload) -> Message {
    Message::broadcast(from, payload)
}
