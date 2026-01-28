//! VSR protocol messages.
//!
//! This module defines all messages used in the Viewstamped Replication protocol:
//!
//! ## Normal Operation
//! - [`Prepare`] - Leader → Backup: Replicate this operation
//! - [`PrepareOk`] - Backup → Leader: I've persisted the operation
//! - [`Commit`] - Leader → Backup: Operations up to this point are committed
//! - [`Heartbeat`] - Leader → Backup: I'm still alive
//!
//! ## View Change
//! - [`StartViewChange`] - Backup → All: I think the leader is dead
//! - [`DoViewChange`] - Backup → New Leader: Here's my state for the new view
//! - [`StartView`] - New Leader → All: New view is starting
//!
//! ## Recovery & Repair
//! - [`RecoveryRequest`] - Recovering → All: I need to recover
//! - [`RecoveryResponse`] - Healthy → Recovering: Here's recovery info
//! - [`RepairRequest`] - Replica → All: I need specific log entries
//! - [`RepairResponse`] - Replica → Requester: Here are the entries
//! - [`Nack`] - Replica → Requester: I don't have what you asked for
//! - [`StateTransferRequest`] - Replica → All: I need a checkpoint
//! - [`StateTransferResponse`] - Replica → Requester: Here's a checkpoint

use serde::{Deserialize, Serialize};
use vdb_types::Hash;

use crate::types::{CommitNumber, LogEntry, Nonce, OpNumber, ReplicaId, ViewNumber};

// ============================================================================
// Message Envelope
// ============================================================================

/// A VSR protocol message with routing information.
///
/// All messages are wrapped in this envelope which provides the sender's
/// identity. The receiver uses this for routing responses and validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// The replica that sent this message.
    pub from: ReplicaId,

    /// The intended recipient (if targeted).
    ///
    /// `None` for broadcast messages.
    pub to: Option<ReplicaId>,

    /// The message payload.
    pub payload: MessagePayload,
}

impl Message {
    /// Creates a new targeted message.
    pub fn targeted(from: ReplicaId, to: ReplicaId, payload: MessagePayload) -> Self {
        Self {
            from,
            to: Some(to),
            payload,
        }
    }

    /// Creates a new broadcast message.
    pub fn broadcast(from: ReplicaId, payload: MessagePayload) -> Self {
        Self {
            from,
            to: None,
            payload,
        }
    }

    /// Returns true if this message is a broadcast.
    pub fn is_broadcast(&self) -> bool {
        self.to.is_none()
    }

    /// Returns true if this message is targeted at a specific replica.
    pub fn is_targeted(&self) -> bool {
        self.to.is_some()
    }
}

// ============================================================================
// Message Payload
// ============================================================================

/// The payload of a VSR protocol message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessagePayload {
    // === Normal Operation ===
    /// Leader → Backup: Replicate this operation.
    Prepare(Prepare),

    /// Backup → Leader: I've persisted the operation.
    PrepareOk(PrepareOk),

    /// Leader → Backup: Operations up to this point are committed.
    Commit(Commit),

    /// Leader → Backup: I'm still alive (no new operations).
    Heartbeat(Heartbeat),

    // === View Change ===
    /// Backup → All: I think the leader is dead.
    StartViewChange(StartViewChange),

    /// Backup → New Leader: Here's my state for the new view.
    DoViewChange(DoViewChange),

    /// New Leader → All: New view is starting.
    StartView(StartView),

    // === Recovery ===
    /// Recovering → All: I need to recover.
    RecoveryRequest(RecoveryRequest),

    /// Healthy → Recovering: Here's recovery info.
    RecoveryResponse(RecoveryResponse),

    // === Repair ===
    /// Replica → All: I need specific log entries.
    RepairRequest(RepairRequest),

    /// Replica → Requester: Here are the entries.
    RepairResponse(RepairResponse),

    /// Replica → Requester: I don't have what you asked for.
    Nack(Nack),

    // === State Transfer ===
    /// Replica → All: I need a checkpoint.
    StateTransferRequest(StateTransferRequest),

    /// Replica → Requester: Here's a checkpoint.
    StateTransferResponse(StateTransferResponse),
}

impl MessagePayload {
    /// Returns the view number associated with this message, if any.
    pub fn view(&self) -> Option<ViewNumber> {
        match self {
            MessagePayload::Prepare(m) => Some(m.view),
            MessagePayload::PrepareOk(m) => Some(m.view),
            MessagePayload::Commit(m) => Some(m.view),
            MessagePayload::Heartbeat(m) => Some(m.view),
            MessagePayload::StartViewChange(m) => Some(m.view),
            MessagePayload::DoViewChange(m) => Some(m.view),
            MessagePayload::StartView(m) => Some(m.view),
            MessagePayload::RecoveryResponse(m) => Some(m.view),
            MessagePayload::StateTransferResponse(m) => Some(m.checkpoint_view),
            // Messages without view context
            MessagePayload::RecoveryRequest(_)
            | MessagePayload::RepairRequest(_)
            | MessagePayload::RepairResponse(_)
            | MessagePayload::Nack(_)
            | MessagePayload::StateTransferRequest(_) => None,
        }
    }

    /// Returns a human-readable name for the message type.
    pub fn name(&self) -> &'static str {
        match self {
            MessagePayload::Prepare(_) => "Prepare",
            MessagePayload::PrepareOk(_) => "PrepareOk",
            MessagePayload::Commit(_) => "Commit",
            MessagePayload::Heartbeat(_) => "Heartbeat",
            MessagePayload::StartViewChange(_) => "StartViewChange",
            MessagePayload::DoViewChange(_) => "DoViewChange",
            MessagePayload::StartView(_) => "StartView",
            MessagePayload::RecoveryRequest(_) => "RecoveryRequest",
            MessagePayload::RecoveryResponse(_) => "RecoveryResponse",
            MessagePayload::RepairRequest(_) => "RepairRequest",
            MessagePayload::RepairResponse(_) => "RepairResponse",
            MessagePayload::Nack(_) => "Nack",
            MessagePayload::StateTransferRequest(_) => "StateTransferRequest",
            MessagePayload::StateTransferResponse(_) => "StateTransferResponse",
        }
    }
}

// ============================================================================
// Normal Operation Messages
// ============================================================================

/// Leader → Backup: Replicate this operation.
///
/// The leader sends Prepare messages to all backups for each client request.
/// Backups must persist the operation before responding with `PrepareOk`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prepare {
    /// Current view number.
    pub view: ViewNumber,

    /// Operation number being prepared.
    pub op_number: OpNumber,

    /// The log entry to replicate.
    pub entry: LogEntry,

    /// Current commit number.
    ///
    /// Backups use this to learn about commits they may have missed.
    pub commit_number: CommitNumber,
}

impl Prepare {
    /// Creates a new Prepare message.
    pub fn new(
        view: ViewNumber,
        op_number: OpNumber,
        entry: LogEntry,
        commit_number: CommitNumber,
    ) -> Self {
        debug_assert_eq!(
            entry.op_number, op_number,
            "entry op_number must match message op_number"
        );
        debug_assert_eq!(entry.view, view, "entry view must match message view");

        Self {
            view,
            op_number,
            entry,
            commit_number,
        }
    }
}

/// Backup → Leader: I've persisted the operation.
///
/// Backups send `PrepareOk` after durably storing the operation. The leader
/// commits the operation after receiving `PrepareOk` from a quorum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareOk {
    /// Current view number.
    pub view: ViewNumber,

    /// Operation number being acknowledged.
    pub op_number: OpNumber,

    /// Replica sending the `PrepareOk`.
    pub replica: ReplicaId,
}

impl PrepareOk {
    /// Creates a new `PrepareOk` message.
    pub fn new(view: ViewNumber, op_number: OpNumber, replica: ReplicaId) -> Self {
        Self {
            view,
            op_number,
            replica,
        }
    }
}

///// Leader → Backup: Operations up to this point are committed.
///
/// The leader sends Commit messages to inform backups of newly committed
/// operations. This is often piggybacked on Prepare messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    /// Current view number.
    pub view: ViewNumber,

    /// New commit number.
    pub commit_number: CommitNumber,
}

impl Commit {
    /// Creates a new Commit message.
    pub fn new(view: ViewNumber, commit_number: CommitNumber) -> Self {
        Self {
            view,
            commit_number,
        }
    }
}

///// Leader → Backup: I'm still alive.
///
/// The leader sends periodic heartbeats to maintain its leadership.
/// If backups don't receive heartbeats, they initiate view change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Heartbeat {
    /// Current view number.
    pub view: ViewNumber,

    /// Current commit number.
    pub commit_number: CommitNumber,
}

impl Heartbeat {
    /// Creates a new Heartbeat message.
    pub fn new(view: ViewNumber, commit_number: CommitNumber) -> Self {
        Self {
            view,
            commit_number,
        }
    }
}

// ============================================================================
// View Change Messages
// ============================================================================

///// Backup → All: I think the leader is dead.
///
/// A backup sends `StartViewChange` when it suspects the leader has failed
/// (e.g., heartbeat timeout). View change proceeds if a quorum agrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartViewChange {
    /// The new view number being proposed.
    pub view: ViewNumber,

    /// Replica initiating the view change.
    pub replica: ReplicaId,
}

impl StartViewChange {
    /// Creates a new `StartViewChange` message.
    pub fn new(view: ViewNumber, replica: ReplicaId) -> Self {
        Self { view, replica }
    }
}

/// Backup → New Leader: Here's my state for the new view.
///
/// After receiving enough `StartViewChange` messages, a backup sends
/// `DoViewChange` to the new leader with its log state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoViewChange {
    /// The new view number.
    pub view: ViewNumber,

    /// Replica sending its state.
    pub replica: ReplicaId,

    /// The last normal view this replica participated in.
    ///
    /// Used to determine which replica has the most up-to-date log.
    pub last_normal_view: ViewNumber,

    /// Highest operation number in this replica's log.
    pub op_number: OpNumber,

    /// Highest committed operation number known to this replica.
    pub commit_number: CommitNumber,

    /// Log entries from `commit_number+1` to `op_number`.
    ///
    /// The new leader uses these to reconstruct the log.
    pub log_tail: Vec<LogEntry>,
}

impl DoViewChange {
    /// Creates a new `DoViewChange` message.
    pub fn new(
        view: ViewNumber,
        replica: ReplicaId,
        last_normal_view: ViewNumber,
        op_number: OpNumber,
        commit_number: CommitNumber,
        log_tail: Vec<LogEntry>,
    ) -> Self {
        Self {
            view,
            replica,
            last_normal_view,
            op_number,
            commit_number,
            log_tail,
        }
    }
}

/// New Leader → All: New view is starting.
///
/// The new leader sends `StartView` after collecting `DoViewChange` messages
/// from a quorum and reconstructing the authoritative log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartView {
    /// The new view number.
    pub view: ViewNumber,

    /// Highest operation number in the new view.
    pub op_number: OpNumber,

    /// Highest committed operation number.
    pub commit_number: CommitNumber,

    /// Log entries that backups may be missing.
    ///
    /// Contains entries from `commit_number+1` to `op_number`.
    pub log_tail: Vec<LogEntry>,
}

impl StartView {
    /// Creates a new `StartView` message.
    pub fn new(
        view: ViewNumber,
        op_number: OpNumber,
        commit_number: CommitNumber,
        log_tail: Vec<LogEntry>,
    ) -> Self {
        Self {
            view,
            op_number,
            commit_number,
            log_tail,
        }
    }
}

// ============================================================================
// Recovery Messages
// ============================================================================

/// Recovering → All: I need to recover.
///
/// A replica sends `RecoveryRequest` after restarting when it needs to
/// recover its state from other replicas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryRequest {
    /// Replica requesting recovery.
    pub replica: ReplicaId,

    /// Random nonce to match responses to this request.
    pub nonce: Nonce,

    /// Last known operation number before crash.
    ///
    /// Helps other replicas determine how much state to send.
    pub known_op_number: OpNumber,
}

impl RecoveryRequest {
    /// Creates a new `RecoveryRequest` message.
    pub fn new(replica: ReplicaId, nonce: Nonce, known_op_number: OpNumber) -> Self {
        Self {
            replica,
            nonce,
            known_op_number,
        }
    }
}

/// Healthy → Recovering: Here's recovery info.
///
/// Healthy replicas respond to `RecoveryRequest` with their current state.
/// The recovering replica uses responses from a quorum to recover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryResponse {
    /// Current view number.
    pub view: ViewNumber,

    /// Replica sending the response.
    pub replica: ReplicaId,

    /// Nonce from the request (for matching).
    pub nonce: Nonce,

    /// Highest operation number.
    pub op_number: OpNumber,

    /// Highest committed operation number.
    pub commit_number: CommitNumber,

    /// Log entries the recovering replica may need.
    ///
    /// Only includes entries after the request's `known_op_number`.
    pub log_suffix: Vec<LogEntry>,
}

impl RecoveryResponse {
    /// Creates a new `RecoveryResponse` message.
    pub fn new(
        view: ViewNumber,
        replica: ReplicaId,
        nonce: Nonce,
        op_number: OpNumber,
        commit_number: CommitNumber,
        log_suffix: Vec<LogEntry>,
    ) -> Self {
        Self {
            view,
            replica,
            nonce,
            op_number,
            commit_number,
            log_suffix,
        }
    }
}

// ============================================================================
// Repair Messages
// ============================================================================

/// Replica → All: I need specific log entries.
///
/// A replica sends `RepairRequest` when it detects missing or corrupt
/// entries in its log. This enables transparent repair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairRequest {
    /// Replica requesting repair.
    pub replica: ReplicaId,

    /// Random nonce to match responses.
    pub nonce: Nonce,

    /// Range of operation numbers needed (inclusive start, exclusive end).
    pub op_range_start: OpNumber,
    pub op_range_end: OpNumber,
}

impl RepairRequest {
    /// Creates a new `RepairRequest` message.
    pub fn new(
        replica: ReplicaId,
        nonce: Nonce,
        op_range_start: OpNumber,
        op_range_end: OpNumber,
    ) -> Self {
        debug_assert!(
            op_range_start < op_range_end,
            "repair range must be non-empty"
        );
        Self {
            replica,
            nonce,
            op_range_start,
            op_range_end,
        }
    }

    /// Returns the number of operations requested.
    pub fn count(&self) -> u64 {
        self.op_range_end.as_u64() - self.op_range_start.as_u64()
    }
}

/// Replica → Requester: Here are the entries.
///
/// A replica responds to `RepairRequest` with the requested log entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairResponse {
    /// Replica sending the response.
    pub replica: ReplicaId,

    /// Nonce from the request (for matching).
    pub nonce: Nonce,

    /// The requested log entries.
    pub entries: Vec<LogEntry>,
}

impl RepairResponse {
    /// Creates a new `RepairResponse` message.
    pub fn new(replica: ReplicaId, nonce: Nonce, entries: Vec<LogEntry>) -> Self {
        Self {
            replica,
            nonce,
            entries,
        }
    }
}

/// Replica → Requester: I don't have what you asked for.
///
/// A replica sends Nack when it cannot fulfill a `RepairRequest`. This is
/// critical for PAR (Protocol-Aware Recovery) to safely truncate the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nack {
    /// Replica sending the nack.
    pub replica: ReplicaId,

    /// Nonce from the request (for matching).
    pub nonce: Nonce,

    /// The reason for the nack.
    pub reason: NackReason,

    /// Highest operation this replica has seen.
    ///
    /// Helps distinguish "not seen" from "seen but lost".
    pub highest_seen: OpNumber,
}

impl Nack {
    /// Creates a new Nack message.
    pub fn new(
        replica: ReplicaId,
        nonce: Nonce,
        reason: NackReason,
        highest_seen: OpNumber,
    ) -> Self {
        Self {
            replica,
            nonce,
            reason,
            highest_seen,
        }
    }
}

/// Reason why a replica cannot fulfill a repair request.
///
/// This distinction is critical for PAR: we can only safely truncate
/// the log if enough replicas confirm they never saw the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NackReason {
    /// The operation was never received by this replica.
    ///
    /// Safe to truncate if a quorum of replicas report `NotSeen`.
    NotSeen,

    /// The operation was received but is now corrupt or lost.
    ///
    /// NOT safe to truncate - the operation may have been committed.
    SeenButCorrupt,

    /// The replica is in recovery and cannot help.
    Recovering,
}

impl std::fmt::Display for NackReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NackReason::NotSeen => write!(f, "not_seen"),
            NackReason::SeenButCorrupt => write!(f, "seen_but_corrupt"),
            NackReason::Recovering => write!(f, "recovering"),
        }
    }
}

// ============================================================================
// State Transfer Messages
// ============================================================================

/// Replica → All: I need a checkpoint.
///
/// A replica sends `StateTransferRequest` when it's too far behind to
/// catch up via log repair and needs a full checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateTransferRequest {
    /// Replica requesting state transfer.
    pub replica: ReplicaId,

    /// Random nonce to match responses.
    pub nonce: Nonce,

    /// Highest checkpoint this replica has.
    ///
    /// Allows responders to send only newer checkpoints.
    pub known_checkpoint: OpNumber,
}

impl StateTransferRequest {
    /// Creates a new `StateTransferRequest` message.
    pub fn new(replica: ReplicaId, nonce: Nonce, known_checkpoint: OpNumber) -> Self {
        Self {
            replica,
            nonce,
            known_checkpoint,
        }
    }
}

/// Replica → Requester: Here's a checkpoint.
///
/// A replica responds to `StateTransferRequest` with a checkpoint.
/// The checkpoint includes a Merkle root for verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateTransferResponse {
    /// Replica sending the response.
    pub replica: ReplicaId,

    /// Nonce from the request (for matching).
    pub nonce: Nonce,

    /// View at which the checkpoint was taken.
    pub checkpoint_view: ViewNumber,

    /// Operation number of the checkpoint.
    pub checkpoint_op: OpNumber,

    /// Merkle root of the checkpoint state (BLAKE3).
    pub merkle_root: Hash,

    /// Serialized checkpoint data.
    ///
    /// The format depends on the state machine implementation.
    pub checkpoint_data: Vec<u8>,

    /// Ed25519 signature over the checkpoint (if signed checkpoints are enabled).
    pub signature: Option<Vec<u8>>,
}

impl StateTransferResponse {
    /// Creates a new `StateTransferResponse` message.
    pub fn new(
        replica: ReplicaId,
        nonce: Nonce,
        checkpoint_view: ViewNumber,
        checkpoint_op: OpNumber,
        merkle_root: Hash,
        checkpoint_data: Vec<u8>,
        signature: Option<Vec<u8>>,
    ) -> Self {
        Self {
            replica,
            nonce,
            checkpoint_view,
            checkpoint_op,
            merkle_root,
            checkpoint_data,
            signature,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vdb_kernel::Command;
    use vdb_types::DataClass;

    fn test_entry() -> LogEntry {
        LogEntry::new(
            OpNumber::new(1),
            ViewNumber::new(0),
            Command::create_stream_with_auto_id(
                "test".into(),
                DataClass::NonPHI,
                vdb_types::Placement::Global,
            ),
            None,
        )
    }

    #[test]
    fn message_envelope_targeted() {
        let msg = Message::targeted(
            ReplicaId::new(0),
            ReplicaId::new(1),
            MessagePayload::Heartbeat(Heartbeat::new(ViewNumber::new(0), CommitNumber::ZERO)),
        );

        assert!(!msg.is_broadcast());
        assert!(msg.is_targeted());
        assert_eq!(msg.to, Some(ReplicaId::new(1)));
    }

    #[test]
    fn message_envelope_broadcast() {
        let msg = Message::broadcast(
            ReplicaId::new(0),
            MessagePayload::StartViewChange(StartViewChange::new(
                ViewNumber::new(1),
                ReplicaId::new(0),
            )),
        );

        assert!(msg.is_broadcast());
        assert!(!msg.is_targeted());
        assert_eq!(msg.to, None);
    }

    #[test]
    fn prepare_message_invariants() {
        let entry = test_entry();
        let prepare = Prepare::new(
            ViewNumber::new(0),
            OpNumber::new(1),
            entry.clone(),
            CommitNumber::ZERO,
        );

        assert_eq!(prepare.view, entry.view);
        assert_eq!(prepare.op_number, entry.op_number);
    }

    #[test]
    fn repair_request_count() {
        let request = RepairRequest::new(
            ReplicaId::new(0),
            Nonce::default(),
            OpNumber::new(5),
            OpNumber::new(10),
        );

        assert_eq!(request.count(), 5);
    }

    #[test]
    fn nack_reason_display() {
        assert_eq!(format!("{}", NackReason::NotSeen), "not_seen");
        assert_eq!(
            format!("{}", NackReason::SeenButCorrupt),
            "seen_but_corrupt"
        );
        assert_eq!(format!("{}", NackReason::Recovering), "recovering");
    }

    #[test]
    fn message_payload_view() {
        let heartbeat =
            MessagePayload::Heartbeat(Heartbeat::new(ViewNumber::new(5), CommitNumber::ZERO));
        assert_eq!(heartbeat.view(), Some(ViewNumber::new(5)));

        let repair = MessagePayload::RepairRequest(RepairRequest::new(
            ReplicaId::new(0),
            Nonce::default(),
            OpNumber::new(1),
            OpNumber::new(2),
        ));
        assert_eq!(repair.view(), None);
    }

    #[test]
    fn message_payload_name() {
        let heartbeat =
            MessagePayload::Heartbeat(Heartbeat::new(ViewNumber::ZERO, CommitNumber::ZERO));
        assert_eq!(heartbeat.name(), "Heartbeat");

        let prepare = MessagePayload::Prepare(Prepare::new(
            ViewNumber::ZERO,
            OpNumber::new(1),
            test_entry(),
            CommitNumber::ZERO,
        ));
        assert_eq!(prepare.name(), "Prepare");
    }

    #[test]
    fn do_view_change_construction() {
        let dvc = DoViewChange::new(
            ViewNumber::new(2),
            ReplicaId::new(1),
            ViewNumber::new(1),
            OpNumber::new(5),
            CommitNumber::new(OpNumber::new(3)),
            vec![test_entry()],
        );

        assert_eq!(dvc.view, ViewNumber::new(2));
        assert_eq!(dvc.last_normal_view, ViewNumber::new(1));
        assert_eq!(dvc.log_tail.len(), 1);
    }
}
