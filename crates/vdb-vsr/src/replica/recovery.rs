//! Recovery protocol handlers.
//!
//! This module implements Protocol-Aware Recovery (PAR), which enables safe
//! recovery after crashes while preventing data loss.
//!
//! # Recovery Protocol
//!
//! 1. Recovering replica broadcasts `RecoveryRequest` with a nonce
//! 2. Healthy replicas respond with `RecoveryResponse` containing their state
//! 3. Recovering replica waits for a quorum of responses
//! 4. Applies the best state (highest view, highest `op_number`)
//! 5. Increments generation and enters normal operation
//!
//! # Generation-Based Recovery
//!
//! Each recovery event increments the generation number. This provides:
//! - Natural audit checkpoints
//! - Detection of "zombie" replicas from old generations
//! - Explicit tracking of recovery events
//!
//! # Safety Properties
//!
//! - A replica must receive a quorum of `RecoveryResponse` before recovering
//! - Generation must be strictly increasing
//! - Recovering replicas cannot participate in consensus

use std::collections::HashMap;

use vdb_types::Generation;

use crate::message::{MessagePayload, RecoveryRequest, RecoveryResponse};
use crate::types::{Nonce, OpNumber, ReplicaId, ReplicaStatus};

use super::{msg_broadcast, msg_to, ReplicaOutput, ReplicaState};

// ============================================================================
// Recovery State
// ============================================================================

/// State tracked during recovery.
#[derive(Debug, Clone)]
pub struct RecoveryState {
    /// Nonce for matching responses to our request.
    pub nonce: Nonce,

    /// Current generation (will be incremented on successful recovery).
    pub generation: Generation,

    /// Responses received so far.
    pub responses: HashMap<ReplicaId, RecoveryResponse>,
}

impl RecoveryState {
    /// Creates a new recovery state.
    pub fn new(nonce: Nonce, generation: Generation) -> Self {
        Self {
            nonce,
            generation,
            responses: HashMap::new(),
        }
    }

    /// Returns the number of responses received.
    pub fn response_count(&self) -> usize {
        self.responses.len()
    }
}

impl ReplicaState {
    // ========================================================================
    // Recovery Initiation
    // ========================================================================

    /// Initiates recovery after a restart.
    ///
    /// This should be called when a replica starts up and needs to recover
    /// its state from other replicas.
    ///
    /// Returns the new state and a `RecoveryRequest` to broadcast.
    pub fn start_recovery(mut self, generation: Generation) -> (Self, ReplicaOutput) {
        debug_assert!(
            self.status != ReplicaStatus::Normal,
            "cannot start recovery while in normal status"
        );

        // Transition to recovering status
        self.status = ReplicaStatus::Recovering;

        // Generate a nonce for this recovery attempt
        let nonce = Nonce::generate();

        // Initialize recovery state
        self.recovery_state = Some(RecoveryState::new(nonce, generation));

        // Create recovery request
        let request = RecoveryRequest::new(self.replica_id, nonce, self.op_number);

        // Broadcast to all replicas
        let msg = msg_broadcast(self.replica_id, MessagePayload::RecoveryRequest(request));

        (self, ReplicaOutput::with_messages(vec![msg]))
    }

    // ========================================================================
    // RecoveryRequest Handler (Healthy Replica)
    // ========================================================================

    /// Handles a `RecoveryRequest` from a recovering replica.
    ///
    /// Healthy replicas respond with their current state to help
    /// the recovering replica catch up.
    pub(crate) fn on_recovery_request(
        self,
        from: ReplicaId,
        request: &RecoveryRequest,
    ) -> (Self, ReplicaOutput) {
        // Don't respond if we're recovering ourselves
        if self.status == ReplicaStatus::Recovering {
            return (self, ReplicaOutput::empty());
        }

        // Don't respond to our own request
        if from == self.replica_id {
            return (self, ReplicaOutput::empty());
        }

        // Build log suffix starting from their known op_number + 1
        let log_suffix = self.log_suffix_from(request.known_op_number);

        // Create response
        let response = RecoveryResponse::new(
            self.view,
            self.replica_id,
            request.nonce,
            self.op_number,
            self.commit_number,
            log_suffix,
        );

        let msg = msg_to(self.replica_id, from, MessagePayload::RecoveryResponse(response));

        (self, ReplicaOutput::with_messages(vec![msg]))
    }

    // ========================================================================
    // RecoveryResponse Handler (Recovering Replica)
    // ========================================================================

    /// Handles a `RecoveryResponse` from a healthy replica.
    ///
    /// Collects responses until we have a quorum, then applies the
    /// best state and completes recovery.
    pub(crate) fn on_recovery_response(
        mut self,
        from: ReplicaId,
        response: RecoveryResponse,
    ) -> (Self, ReplicaOutput) {
        // Must be recovering
        if self.status != ReplicaStatus::Recovering {
            return (self, ReplicaOutput::empty());
        }

        // Must have recovery state
        let Some(ref mut recovery_state) = self.recovery_state else {
            return (self, ReplicaOutput::empty());
        };

        // Nonce must match our current recovery request
        if response.nonce != recovery_state.nonce {
            return (self, ReplicaOutput::empty());
        }

        // Record the response
        recovery_state.responses.insert(from, response);

        // Check if we have a quorum
        self.check_recovery_quorum()
    }

    /// Checks if we have a quorum of recovery responses.
    ///
    /// If so, applies the best state and completes recovery.
    fn check_recovery_quorum(mut self) -> (Self, ReplicaOutput) {
        let quorum = self.config.quorum_size();

        let Some(ref recovery_state) = self.recovery_state else {
            return (self, ReplicaOutput::empty());
        };

        if recovery_state.response_count() < quorum {
            return (self, ReplicaOutput::empty());
        }

        // We have quorum! Find the best state.
        // "Best" = highest view, then highest op_number
        let best_response = recovery_state
            .responses
            .values()
            .max_by(|a, b| (a.view, a.op_number).cmp(&(b.view, b.op_number)))
            .expect("at least quorum responses")
            .clone();

        let new_generation = recovery_state.generation.next();

        // Apply the best state
        self.view = best_response.view;
        self.op_number = best_response.op_number;
        self.commit_number = best_response.commit_number;

        // Merge log entries from the response
        self = self.merge_log_tail(best_response.log_suffix);

        // Increment generation
        self.generation = new_generation;

        // Clear recovery state and enter normal operation
        self.recovery_state = None;
        self.status = ReplicaStatus::Normal;
        self.last_normal_view = self.view;

        tracing::info!(
            replica = %self.replica_id,
            view = %self.view,
            op = %self.op_number,
            commit = %self.commit_number,
            generation = %self.generation,
            "recovery complete"
        );

        (self, ReplicaOutput::empty())
    }

    // ========================================================================
    // Recovery Timeout Handler
    // ========================================================================

    /// Handles recovery timeout.
    ///
    /// If recovery is taking too long, retry with a new nonce.
    pub(crate) fn on_recovery_timeout(mut self) -> (Self, ReplicaOutput) {
        if self.status != ReplicaStatus::Recovering {
            return (self, ReplicaOutput::empty());
        }

        let Some(ref recovery_state) = self.recovery_state else {
            return (self, ReplicaOutput::empty());
        };

        let generation = recovery_state.generation;

        // Restart recovery with a new nonce
        tracing::debug!(
            replica = %self.replica_id,
            responses = recovery_state.response_count(),
            "recovery timeout, retrying"
        );

        // Clear old state and restart
        self.recovery_state = None;
        self.start_recovery(generation)
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Returns log entries starting after the given `op_number`.
    fn log_suffix_from(&self, after: OpNumber) -> Vec<crate::types::LogEntry> {
        let start_index = after.as_u64() as usize;
        if start_index >= self.log.len() {
            return Vec::new();
        }
        self.log[start_index..].to_vec()
    }

    /// Returns true if this replica is in recovery status.
    pub fn is_recovering(&self) -> bool {
        self.status == ReplicaStatus::Recovering
    }

    /// Returns the current recovery state, if any.
    pub fn recovery_state(&self) -> Option<&RecoveryState> {
        self.recovery_state.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClusterConfig;
    use crate::types::{CommitNumber, LogEntry, OpNumber, ViewNumber};
    use vdb_kernel::Command;
    use vdb_types::{DataClass, Generation, Placement};

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

    fn test_entry(op: u64, view: u64) -> LogEntry {
        LogEntry::new(
            OpNumber::new(op),
            ViewNumber::new(view),
            test_command(),
            None,
        )
    }

    #[test]
    fn start_recovery_broadcasts_request() {
        let config = test_config_3();
        let mut replica = ReplicaState::new(ReplicaId::new(1), config);
        replica.status = ReplicaStatus::ViewChange; // Not normal

        let (replica, output) = replica.start_recovery(Generation::INITIAL);

        assert_eq!(replica.status(), ReplicaStatus::Recovering);
        assert!(replica.recovery_state().is_some());

        // Should have broadcast RecoveryRequest
        assert_eq!(output.messages.len(), 1);
        assert!(matches!(
            output.messages[0].payload,
            MessagePayload::RecoveryRequest(_)
        ));
    }

    #[test]
    fn healthy_replica_responds_to_recovery_request() {
        let config = test_config_3();
        let mut healthy = ReplicaState::new(ReplicaId::new(0), config);

        // Add some state
        healthy.log.push(test_entry(1, 0));
        healthy.op_number = OpNumber::new(1);
        healthy.commit_number = CommitNumber::new(OpNumber::new(1));

        // Receive recovery request
        let request = RecoveryRequest::new(ReplicaId::new(1), Nonce::generate(), OpNumber::ZERO);

        let (_healthy, output) = healthy.on_recovery_request(ReplicaId::new(1), &request);

        // Should respond with RecoveryResponse
        assert_eq!(output.messages.len(), 1);
        let msg = &output.messages[0];
        assert!(matches!(msg.payload, MessagePayload::RecoveryResponse(_)));
        assert_eq!(msg.to, Some(ReplicaId::new(1)));
    }

    #[test]
    fn recovering_replica_ignores_recovery_request() {
        let config = test_config_3();
        let mut replica = ReplicaState::new(ReplicaId::new(1), config);
        replica.status = ReplicaStatus::Recovering;

        let request = RecoveryRequest::new(ReplicaId::new(2), Nonce::generate(), OpNumber::ZERO);

        let (_replica, output) = replica.on_recovery_request(ReplicaId::new(2), &request);

        // Should not respond
        assert!(output.is_empty());
    }

    #[test]
    fn recovery_completes_on_quorum() {
        let config = test_config_3();
        let mut recovering = ReplicaState::new(ReplicaId::new(2), config);
        recovering.status = ReplicaStatus::ViewChange;

        // Start recovery
        let (mut recovering, _) = recovering.start_recovery(Generation::INITIAL);
        let nonce = recovering.recovery_state().unwrap().nonce;

        // Receive response from replica 0
        let response0 = RecoveryResponse::new(
            ViewNumber::new(1),
            ReplicaId::new(0),
            nonce,
            OpNumber::new(5),
            CommitNumber::new(OpNumber::new(3)),
            vec![test_entry(1, 1), test_entry(2, 1)],
        );
        let (new_recovering, _) = recovering.on_recovery_response(ReplicaId::new(0), response0);
        recovering = new_recovering;

        // Should still be recovering (need quorum of 2)
        assert_eq!(recovering.status(), ReplicaStatus::Recovering);

        // Receive response from replica 1
        let response1 = RecoveryResponse::new(
            ViewNumber::new(1),
            ReplicaId::new(1),
            nonce,
            OpNumber::new(5),
            CommitNumber::new(OpNumber::new(3)),
            vec![test_entry(1, 1), test_entry(2, 1)],
        );
        let (recovered, _) = recovering.on_recovery_response(ReplicaId::new(1), response1);

        // Now should be normal
        assert_eq!(recovered.status(), ReplicaStatus::Normal);
        assert_eq!(recovered.view(), ViewNumber::new(1));
        assert_eq!(recovered.generation, Generation::new(1));
    }

    #[test]
    fn recovery_picks_best_state() {
        let config = test_config_3();
        let mut recovering = ReplicaState::new(ReplicaId::new(2), config);
        recovering.status = ReplicaStatus::ViewChange;

        let (mut recovering, _) = recovering.start_recovery(Generation::INITIAL);
        let nonce = recovering.recovery_state().unwrap().nonce;

        // Response with lower view
        let response0 = RecoveryResponse::new(
            ViewNumber::new(1),
            ReplicaId::new(0),
            nonce,
            OpNumber::new(10),
            CommitNumber::new(OpNumber::new(8)),
            vec![],
        );
        let (new_recovering, _) = recovering.on_recovery_response(ReplicaId::new(0), response0);
        recovering = new_recovering;

        // Response with higher view (should be picked)
        let response1 = RecoveryResponse::new(
            ViewNumber::new(2),
            ReplicaId::new(1),
            nonce,
            OpNumber::new(5),
            CommitNumber::new(OpNumber::new(3)),
            vec![],
        );
        let (recovered, _) = recovering.on_recovery_response(ReplicaId::new(1), response1);

        // Should have picked view 2 (higher view wins)
        assert_eq!(recovered.view(), ViewNumber::new(2));
        assert_eq!(recovered.op_number(), OpNumber::new(5));
    }

    #[test]
    fn wrong_nonce_ignored() {
        let config = test_config_3();
        let mut recovering = ReplicaState::new(ReplicaId::new(2), config);
        recovering.status = ReplicaStatus::ViewChange;

        let (recovering, _) = recovering.start_recovery(Generation::INITIAL);

        // Response with wrong nonce
        let response = RecoveryResponse::new(
            ViewNumber::new(1),
            ReplicaId::new(0),
            Nonce::generate(), // Different nonce
            OpNumber::new(5),
            CommitNumber::ZERO,
            vec![],
        );

        let (recovering, _) = recovering.on_recovery_response(ReplicaId::new(0), response);

        // Should still be recovering (response ignored)
        assert_eq!(recovering.status(), ReplicaStatus::Recovering);
        assert_eq!(recovering.recovery_state().unwrap().response_count(), 0);
    }
}
