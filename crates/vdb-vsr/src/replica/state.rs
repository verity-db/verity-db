//! Replica state structure.
//!
//! This module defines [`ReplicaState`], the core state of a VSR replica.
//! The state is designed to be cloneable for simulation testing and
//! follows the FCIS pattern (pure, no I/O).

use std::collections::{HashMap, HashSet};

use vdb_kernel::{apply_committed, Command, Effect, State as KernelState};
use vdb_types::{Generation, IdempotencyId};

use crate::config::ClusterConfig;
use crate::message::{DoViewChange, MessagePayload, Prepare};
use crate::types::{CommitNumber, LogEntry, OpNumber, ReplicaId, ReplicaStatus, ViewNumber};

use super::recovery::RecoveryState;
use super::repair::RepairState;
use super::{msg_broadcast, ReplicaEvent, ReplicaOutput, TimeoutKind};

// ============================================================================
// Replica State
// ============================================================================

/// The state of a VSR replica.
///
/// This structure contains all mutable state for a replica. It is designed
/// to be:
/// - **Pure**: All state transitions are deterministic
/// - **Cloneable**: For simulation testing and snapshotting
/// - **Serializable**: For persistence and debugging
///
/// # State Categories
///
/// 1. **Identity**: `replica_id`, `config`
/// 2. **View State**: `view`, `status`, `last_normal_view`
/// 3. **Log State**: `log`, `op_number`, `commit_number`
/// 4. **Tracking**: `prepare_ok_tracker`, view change votes
/// 5. **Application**: `kernel_state`
#[derive(Debug, Clone)]
pub struct ReplicaState {
    // ========================================================================
    // Identity
    // ========================================================================
    /// This replica's ID.
    pub(crate) replica_id: ReplicaId,

    /// Cluster configuration.
    pub(crate) config: ClusterConfig,

    // ========================================================================
    // View State
    // ========================================================================
    /// Current view number.
    pub(crate) view: ViewNumber,

    /// Current replica status.
    pub(crate) status: ReplicaStatus,

    /// Last view in which this replica was in normal status.
    ///
    /// Used during view change to determine which replica has
    /// the most up-to-date log.
    pub(crate) last_normal_view: ViewNumber,

    // ========================================================================
    // Log State
    // ========================================================================
    /// The replicated log of committed entries.
    pub(crate) log: Vec<LogEntry>,

    /// Highest operation number in the log.
    pub(crate) op_number: OpNumber,

    /// Highest committed operation number.
    pub(crate) commit_number: CommitNumber,

    // ========================================================================
    // Leader Tracking (only used when leader)
    // ========================================================================
    /// Tracks `PrepareOK` responses for pending operations.
    ///
    /// Key: operation number, Value: set of replicas that sent `PrepareOK`.
    pub(crate) prepare_ok_tracker: HashMap<OpNumber, HashSet<ReplicaId>>,

    /// Pending client requests waiting to be prepared.
    pub(crate) pending_requests: Vec<(Command, Option<IdempotencyId>)>,

    // ========================================================================
    // View Change Tracking
    // ========================================================================
    /// `StartViewChange` votes received for current view change.
    pub(crate) start_view_change_votes: HashSet<ReplicaId>,

    /// `DoViewChange` messages received (as new leader).
    pub(crate) do_view_change_msgs: Vec<DoViewChange>,

    // ========================================================================
    // Recovery & Repair State
    // ========================================================================
    /// Current recovery generation.
    ///
    /// Incremented each time the replica recovers from a crash.
    pub(crate) generation: Generation,

    /// State tracked during recovery (if recovering).
    pub(crate) recovery_state: Option<RecoveryState>,

    /// State tracked during repair (if repairing).
    pub(crate) repair_state: Option<RepairState>,

    // ========================================================================
    // Application State
    // ========================================================================
    /// The kernel (application) state machine.
    pub(crate) kernel_state: KernelState,
}

impl ReplicaState {
    /// Creates a new replica state with initial values.
    ///
    /// The replica starts in `Normal` status at view 0. If it's the leader
    /// for view 0, it can immediately accept client requests.
    pub fn new(replica_id: ReplicaId, config: ClusterConfig) -> Self {
        debug_assert!(
            config.contains(replica_id),
            "replica must be in cluster config"
        );

        Self {
            replica_id,
            config,
            view: ViewNumber::ZERO,
            status: ReplicaStatus::Normal,
            last_normal_view: ViewNumber::ZERO,
            log: Vec::new(),
            op_number: OpNumber::ZERO,
            commit_number: CommitNumber::ZERO,
            prepare_ok_tracker: HashMap::new(),
            pending_requests: Vec::new(),
            start_view_change_votes: HashSet::new(),
            do_view_change_msgs: Vec::new(),
            generation: Generation::INITIAL,
            recovery_state: None,
            repair_state: None,
            kernel_state: KernelState::new(),
        }
    }

    // ========================================================================
    // Accessors
    // ========================================================================

    /// Returns this replica's ID.
    pub fn replica_id(&self) -> ReplicaId {
        self.replica_id
    }

    /// Returns the cluster configuration.
    pub fn config(&self) -> &ClusterConfig {
        &self.config
    }

    /// Returns the current view number.
    pub fn view(&self) -> ViewNumber {
        self.view
    }

    /// Returns the current replica status.
    pub fn status(&self) -> ReplicaStatus {
        self.status
    }

    /// Returns the highest operation number.
    pub fn op_number(&self) -> OpNumber {
        self.op_number
    }

    /// Returns the highest committed operation number.
    pub fn commit_number(&self) -> CommitNumber {
        self.commit_number
    }

    /// Returns the kernel state.
    pub fn kernel_state(&self) -> &KernelState {
        &self.kernel_state
    }

    /// Returns the number of entries in the log.
    pub fn log_len(&self) -> usize {
        self.log.len()
    }

    /// Returns a log entry by operation number.
    pub fn log_entry(&self, op: OpNumber) -> Option<&LogEntry> {
        if op.is_zero() {
            return None;
        }
        let index = op.as_u64().checked_sub(1)? as usize;
        self.log.get(index)
    }

    /// Returns true if this replica is the leader for the current view.
    pub fn is_leader(&self) -> bool {
        self.config.leader_for_view(self.view) == self.replica_id
    }

    /// Returns the leader for the current view.
    pub fn leader(&self) -> ReplicaId {
        self.config.leader_for_view(self.view)
    }

    /// Returns true if the replica can process client requests.
    pub fn can_accept_requests(&self) -> bool {
        self.status == ReplicaStatus::Normal && self.is_leader()
    }

    // ========================================================================
    // Event Processing (Main Entry Point)
    // ========================================================================

    /// Processes an event and returns the new state and output.
    ///
    /// This is the main entry point for the state machine. All state
    /// transitions go through this method.
    ///
    /// # FCIS Pattern
    ///
    /// This method is pure: it takes ownership of `self`, processes the
    /// event, and returns a new state. The caller is responsible for
    /// executing the output (sending messages, executing effects).
    pub fn process(self, event: ReplicaEvent) -> (Self, ReplicaOutput) {
        match event {
            ReplicaEvent::Message(msg) => self.on_message(msg),
            ReplicaEvent::Timeout(kind) => self.on_timeout(kind),
            ReplicaEvent::ClientRequest {
                command,
                idempotency_id,
            } => self.on_client_request(command, idempotency_id),
            ReplicaEvent::Tick => self.on_tick(),
        }
    }

    /// Handles an incoming message.
    fn on_message(self, msg: crate::Message) -> (Self, ReplicaOutput) {
        // Ignore messages from unknown replicas
        if !self.config.contains(msg.from) {
            return (self, ReplicaOutput::empty());
        }

        // Check if message is for us (if targeted)
        if let Some(to) = msg.to {
            if to != self.replica_id {
                return (self, ReplicaOutput::empty());
            }
        }

        match msg.payload {
            // Normal operation
            MessagePayload::Prepare(prepare) => self.on_prepare(msg.from, prepare),
            MessagePayload::PrepareOk(prepare_ok) => self.on_prepare_ok(msg.from, prepare_ok),
            MessagePayload::Commit(commit) => self.on_commit(msg.from, commit),
            MessagePayload::Heartbeat(heartbeat) => self.on_heartbeat(msg.from, heartbeat),

            // View change
            MessagePayload::StartViewChange(svc) => self.on_start_view_change(msg.from, svc),
            MessagePayload::DoViewChange(dvc) => self.on_do_view_change(msg.from, dvc),
            MessagePayload::StartView(sv) => self.on_start_view(msg.from, sv),

            // Recovery
            MessagePayload::RecoveryRequest(ref req) => self.on_recovery_request(msg.from, req),
            MessagePayload::RecoveryResponse(resp) => self.on_recovery_response(msg.from, resp),

            // Repair
            MessagePayload::RepairRequest(ref req) => self.on_repair_request(msg.from, req),
            MessagePayload::RepairResponse(ref resp) => self.on_repair_response(msg.from, resp),
            MessagePayload::Nack(nack) => self.on_nack(msg.from, nack),

            // State transfer (TODO: Phase 3.4)
            MessagePayload::StateTransferRequest(_)
            | MessagePayload::StateTransferResponse(_) => (self, ReplicaOutput::empty()),
        }
    }

    /// Handles a timeout event.
    fn on_timeout(self, kind: TimeoutKind) -> (Self, ReplicaOutput) {
        match kind {
            TimeoutKind::Heartbeat => self.on_heartbeat_timeout(),
            TimeoutKind::Prepare(op) => self.on_prepare_timeout(op),
            TimeoutKind::ViewChange => self.on_view_change_timeout(),
            TimeoutKind::Recovery => self.on_recovery_timeout(),
        }
    }

    /// Handles a client request (leader only).
    fn on_client_request(
        mut self,
        command: Command,
        idempotency_id: Option<IdempotencyId>,
    ) -> (Self, ReplicaOutput) {
        // Only leader can accept client requests
        if !self.can_accept_requests() {
            // Queue for later if we might become leader
            self.pending_requests.push((command, idempotency_id));
            return (self, ReplicaOutput::empty());
        }

        self.prepare_new_operation(command, idempotency_id)
    }

    /// Handles a periodic tick (for housekeeping).
    fn on_tick(self) -> (Self, ReplicaOutput) {
        // TODO: Could be used for:
        // - Sending heartbeats
        // - Retrying pending prepares
        // - Garbage collecting old state
        (self, ReplicaOutput::empty())
    }

    // ========================================================================
    // Leader Operations
    // ========================================================================

    /// Prepares a new operation (leader only).
    #[allow(clippy::needless_pass_by_value)] // Command is cloned into LogEntry
    pub(crate) fn prepare_new_operation(
        mut self,
        command: Command,
        idempotency_id: Option<IdempotencyId>,
    ) -> (Self, ReplicaOutput) {
        debug_assert!(self.is_leader(), "only leader can prepare");
        debug_assert!(
            self.status == ReplicaStatus::Normal,
            "must be in normal status"
        );

        // Assign next op number
        let op_number = self.op_number.next();
        self.op_number = op_number;

        // Create log entry
        let entry = LogEntry::new(op_number, self.view, command.clone(), idempotency_id);

        // Add to log
        self.log.push(entry.clone());

        // Initialize prepare tracker (leader counts itself)
        let mut voters = HashSet::new();
        voters.insert(self.replica_id);
        self.prepare_ok_tracker.insert(op_number, voters);

        // Create Prepare message
        let prepare = Prepare::new(self.view, op_number, entry, self.commit_number);

        // Broadcast to all backups
        let msg = msg_broadcast(self.replica_id, MessagePayload::Prepare(prepare));

        // Check if we already have quorum (single-node case)
        let (state, mut output) = self.try_commit(op_number);

        // Add the prepare message
        output.messages.insert(0, msg);

        (state, output)
    }

    /// Tries to commit operations up to the given op number.
    ///
    /// Returns the new state and any effects from committing.
    pub(crate) fn try_commit(mut self, up_to: OpNumber) -> (Self, ReplicaOutput) {
        let mut output = ReplicaOutput::empty();
        let quorum = self.config.quorum_size();

        // Find operations that have quorum
        while self.commit_number.as_op_number() < up_to {
            let next_commit = self.commit_number.as_op_number().next();

            // Check if we have quorum for this operation
            let votes = self
                .prepare_ok_tracker
                .get(&next_commit)
                .map_or(0, std::collections::HashSet::len);

            if votes < quorum {
                break; // Don't have quorum yet
            }

            // Commit this operation
            let (new_self, commit_output) = self.commit_operation(next_commit);
            self = new_self;
            output.merge(commit_output);
        }

        (self, output)
    }

    /// Commits a single operation and applies it to the kernel.
    fn commit_operation(mut self, op: OpNumber) -> (Self, ReplicaOutput) {
        debug_assert!(
            op == self.commit_number.as_op_number().next(),
            "must commit in order"
        );

        // Get the log entry
        let entry = self
            .log_entry(op)
            .expect("log entry must exist for commit")
            .clone();

        // Apply to kernel
        let result = apply_committed(self.kernel_state.clone(), entry.command);

        match result {
            Ok((new_kernel_state, effects)) => {
                self.kernel_state = new_kernel_state;
                self.commit_number = CommitNumber::new(op);

                // Clean up prepare tracker
                self.prepare_ok_tracker.remove(&op);

                // Create commit message for backups
                let commit_msg = msg_broadcast(
                    self.replica_id,
                    MessagePayload::Commit(crate::Commit::new(self.view, self.commit_number)),
                );

                let output = ReplicaOutput::with_messages_and_effects(vec![commit_msg], effects)
                    .with_committed(op);

                (self, output)
            }
            Err(e) => {
                // Kernel error - this shouldn't happen for valid commands
                // In production, we'd need to handle this gracefully
                tracing::error!(error = %e, op = %op, "kernel error during commit");
                (self, ReplicaOutput::empty())
            }
        }
    }

    // ========================================================================
    // State Management
    // ========================================================================

    /// Transitions to a new view.
    pub(crate) fn transition_to_view(mut self, new_view: ViewNumber) -> Self {
        debug_assert!(new_view > self.view, "view must increase");

        if self.status == ReplicaStatus::Normal {
            self.last_normal_view = self.view;
        }

        self.view = new_view;
        self.status = ReplicaStatus::ViewChange;

        // Clear view change state
        self.start_view_change_votes.clear();
        self.do_view_change_msgs.clear();
        self.prepare_ok_tracker.clear();

        self
    }

    /// Enters normal operation in the current view.
    pub(crate) fn enter_normal_status(mut self) -> Self {
        self.status = ReplicaStatus::Normal;
        self.last_normal_view = self.view;

        // Clear view change state
        self.start_view_change_votes.clear();
        self.do_view_change_msgs.clear();

        self
    }

    /// Returns the log tail from `commit_number+1` to `op_number`.
    pub(crate) fn log_tail(&self) -> Vec<LogEntry> {
        let start = self.commit_number.as_u64() as usize;
        self.log[start..].to_vec()
    }

    /// Adds entries to the log, replacing any conflicting entries.
    pub(crate) fn merge_log_tail(mut self, entries: Vec<LogEntry>) -> Self {
        for entry in entries {
            let index = entry.op_number.as_u64().saturating_sub(1) as usize;

            match index.cmp(&self.log.len()) {
                std::cmp::Ordering::Less => {
                    // Replace existing entry
                    self.log[index] = entry;
                }
                std::cmp::Ordering::Equal => {
                    // Append new entry
                    self.log.push(entry);
                }
                std::cmp::Ordering::Greater => {
                    // Gap in log - shouldn't happen in normal operation
                    tracing::warn!(
                        expected = self.log.len(),
                        got = index,
                        "gap in log during merge"
                    );
                }
            }

            // Update op_number if needed
            let entry_op = OpNumber::new(index as u64 + 1);
            if entry_op > self.op_number {
                self.op_number = entry_op;
            }
        }

        self
    }

    /// Applies committed entries to the kernel up to the given commit number.
    pub(crate) fn apply_commits_up_to(mut self, new_commit: CommitNumber) -> (Self, Vec<Effect>) {
        let mut all_effects = Vec::new();

        while self.commit_number < new_commit {
            let next_op = self.commit_number.as_op_number().next();

            if let Some(entry) = self.log_entry(next_op).cloned() {
                match apply_committed(self.kernel_state.clone(), entry.command) {
                    Ok((new_state, effects)) => {
                        self.kernel_state = new_state;
                        self.commit_number = CommitNumber::new(next_op);
                        all_effects.extend(effects);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, op = %next_op, "kernel error during catchup");
                        break;
                    }
                }
            } else {
                tracing::warn!(op = %next_op, "missing log entry during catchup");
                break;
            }
        }

        (self, all_effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vdb_types::{DataClass, Placement};

    fn test_config_3() -> ClusterConfig {
        ClusterConfig::new(vec![
            ReplicaId::new(0),
            ReplicaId::new(1),
            ReplicaId::new(2),
        ])
    }

    fn test_command() -> Command {
        Command::create_stream_with_auto_id("test".into(), DataClass::NonPHI, Placement::Global)
    }

    #[test]
    fn new_replica_state() {
        let config = test_config_3();
        let state = ReplicaState::new(ReplicaId::new(0), config);

        assert_eq!(state.replica_id(), ReplicaId::new(0));
        assert_eq!(state.view(), ViewNumber::ZERO);
        assert_eq!(state.status(), ReplicaStatus::Normal);
        assert_eq!(state.op_number(), OpNumber::ZERO);
        assert_eq!(state.commit_number(), CommitNumber::ZERO);
        assert!(state.is_leader()); // Replica 0 is leader for view 0
    }

    #[test]
    fn leader_determination() {
        let config = test_config_3();

        let r0 = ReplicaState::new(ReplicaId::new(0), config.clone());
        let r1 = ReplicaState::new(ReplicaId::new(1), config.clone());
        let r2 = ReplicaState::new(ReplicaId::new(2), config);

        // In view 0, replica 0 is leader
        assert!(r0.is_leader());
        assert!(!r1.is_leader());
        assert!(!r2.is_leader());

        // Transition to view 1
        let r0 = r0.transition_to_view(ViewNumber::new(1));
        let r1 = r1.transition_to_view(ViewNumber::new(1));

        // In view 1, replica 1 is leader
        assert!(!r0.is_leader());
        assert!(r1.is_leader());
    }

    #[test]
    fn prepare_new_operation() {
        let config = test_config_3();
        let state = ReplicaState::new(ReplicaId::new(0), config);

        let (state, output) = state.prepare_new_operation(test_command(), None);

        assert_eq!(state.op_number(), OpNumber::new(1));
        assert_eq!(state.log_len(), 1);
        assert!(!output.messages.is_empty()); // Should have Prepare broadcast
    }

    #[test]
    fn log_entry_retrieval() {
        let config = test_config_3();
        let state = ReplicaState::new(ReplicaId::new(0), config);

        let (state, _) = state.prepare_new_operation(test_command(), None);

        let entry = state.log_entry(OpNumber::new(1));
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().op_number, OpNumber::new(1));

        // Op 0 should return None
        assert!(state.log_entry(OpNumber::ZERO).is_none());

        // Op 2 doesn't exist yet
        assert!(state.log_entry(OpNumber::new(2)).is_none());
    }

    #[test]
    fn view_transition() {
        let config = test_config_3();
        let state = ReplicaState::new(ReplicaId::new(0), config);

        assert_eq!(state.status(), ReplicaStatus::Normal);

        let state = state.transition_to_view(ViewNumber::new(1));

        assert_eq!(state.view(), ViewNumber::new(1));
        assert_eq!(state.status(), ReplicaStatus::ViewChange);
        assert_eq!(state.last_normal_view, ViewNumber::ZERO);
    }

    #[test]
    fn enter_normal_status() {
        let config = test_config_3();
        let state = ReplicaState::new(ReplicaId::new(0), config);

        let state = state.transition_to_view(ViewNumber::new(1));
        assert_eq!(state.status(), ReplicaStatus::ViewChange);

        let state = state.enter_normal_status();
        assert_eq!(state.status(), ReplicaStatus::Normal);
        assert_eq!(state.last_normal_view, ViewNumber::new(1));
    }
}
