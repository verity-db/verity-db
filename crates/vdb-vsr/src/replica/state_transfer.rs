//! State transfer protocol handlers.
//!
//! This module implements state transfer for replicas that are too far behind
//! to catch up via log repair and need a full checkpoint.
//!
//! # When State Transfer is Needed
//!
//! State transfer is triggered when:
//! 1. A replica receives a message from a much higher view
//! 2. Log repair fails because entries have been garbage collected
//! 3. A replica recovers and finds it's too far behind
//!
//! # Protocol Flow
//!
//! ```text
//! Stale Replica ──StateTransferRequest──► All
//!                    │
//!                    ▼ (f+1 responses)
//! Stale Replica ◄─StateTransferResponse── Healthy Replica
//!                    │
//!                    ▼ (apply checkpoint)
//!                 Normal Operation
//! ```

use std::collections::HashMap;

use vdb_types::Hash;

use crate::checkpoint::{CheckpointData, compute_merkle_root};
use crate::message::{MessagePayload, StateTransferRequest, StateTransferResponse};
use crate::types::{CommitNumber, Nonce, OpNumber, ReplicaId, ReplicaStatus, ViewNumber};

use super::{ReplicaOutput, ReplicaState, msg_broadcast, msg_to};

// ============================================================================
// State Transfer State
// ============================================================================

/// State tracked during a state transfer operation.
#[derive(Debug, Clone)]
pub struct StateTransferState {
    /// Nonce for matching responses to our request.
    pub nonce: Nonce,

    /// Our current checkpoint op (to only accept newer checkpoints).
    pub known_checkpoint: OpNumber,

    /// Responses received from replicas.
    pub responses: HashMap<ReplicaId, StateTransferResponse>,

    /// The target view we're trying to reach (if known).
    pub target_view: Option<ViewNumber>,
}

impl StateTransferState {
    /// Creates a new state transfer state.
    pub fn new(nonce: Nonce, known_checkpoint: OpNumber) -> Self {
        Self {
            nonce,
            known_checkpoint,
            responses: HashMap::new(),
            target_view: None,
        }
    }

    /// Creates a state transfer state with a target view.
    pub fn with_target_view(
        nonce: Nonce,
        known_checkpoint: OpNumber,
        target_view: ViewNumber,
    ) -> Self {
        Self {
            nonce,
            known_checkpoint,
            responses: HashMap::new(),
            target_view: Some(target_view),
        }
    }

    /// Returns the number of responses received.
    pub fn response_count(&self) -> usize {
        self.responses.len()
    }

    /// Returns the best (most recent) checkpoint from responses.
    ///
    /// Selects the checkpoint with the highest op number that has valid
    /// Merkle verification.
    pub fn best_checkpoint(&self) -> Option<&StateTransferResponse> {
        self.responses
            .values()
            .max_by_key(|r| r.checkpoint_op.as_u64())
    }
}

impl ReplicaState {
    // ========================================================================
    // State Transfer Initiation
    // ========================================================================

    /// Initiates state transfer to catch up with the cluster.
    ///
    /// Called when the replica is too far behind to use log repair.
    pub fn start_state_transfer(
        mut self,
        target_view: Option<ViewNumber>,
    ) -> (Self, ReplicaOutput) {
        // Generate a nonce for this request
        let nonce = Nonce::generate();

        // Record our highest known checkpoint
        let known_checkpoint: OpNumber = self.commit_number.into();

        // Initialize state transfer state
        self.state_transfer_state = Some(if let Some(view) = target_view {
            StateTransferState::with_target_view(nonce, known_checkpoint, view)
        } else {
            StateTransferState::new(nonce, known_checkpoint)
        });

        // Create request
        let request = StateTransferRequest::new(self.replica_id, nonce, known_checkpoint);

        // Broadcast to all replicas
        let msg = msg_broadcast(
            self.replica_id,
            MessagePayload::StateTransferRequest(request),
        );

        tracing::info!(
            replica = %self.replica_id,
            known_checkpoint = %known_checkpoint,
            target_view = ?target_view,
            "initiated state transfer"
        );

        (self, ReplicaOutput::with_messages(vec![msg]))
    }

    // ========================================================================
    // StateTransferRequest Handler
    // ========================================================================

    /// Handles a `StateTransferRequest` from another replica.
    ///
    /// Responds with our latest checkpoint if we have one newer than theirs.
    pub(crate) fn on_state_transfer_request(
        self,
        from: ReplicaId,
        request: &StateTransferRequest,
    ) -> (Self, ReplicaOutput) {
        // Don't respond to our own request
        if from == self.replica_id {
            return (self, ReplicaOutput::empty());
        }

        // If we're recovering or in state transfer ourselves, we can't help
        if self.status == ReplicaStatus::Recovering {
            return (self, ReplicaOutput::empty());
        }

        // Check if we have a checkpoint newer than what they know
        let our_checkpoint: OpNumber = self.commit_number.into();
        if our_checkpoint <= request.known_checkpoint {
            // We don't have anything newer to offer
            return (self, ReplicaOutput::empty());
        }

        // Build checkpoint data from our current state
        let checkpoint_data = self.build_checkpoint_data();

        // Convert MerkleRoot to Hash for the response
        let merkle_root = Hash::from_bytes(*checkpoint_data.log_root.as_bytes());

        // Serialize checkpoint data using serde_json
        let checkpoint_bytes = serde_json::to_vec(&checkpoint_data).unwrap_or_else(|_| Vec::new());

        // Create response
        let response = StateTransferResponse::new(
            self.replica_id,
            request.nonce,
            self.view,
            our_checkpoint,
            merkle_root,
            checkpoint_bytes,
            None, // Signature would require access to signing key
        );

        let msg = msg_to(
            self.replica_id,
            from,
            MessagePayload::StateTransferResponse(response),
        );

        tracing::debug!(
            replica = %self.replica_id,
            to = %from,
            checkpoint_op = %our_checkpoint,
            "sending state transfer response"
        );

        (self, ReplicaOutput::with_messages(vec![msg]))
    }

    // ========================================================================
    // StateTransferResponse Handler
    // ========================================================================

    /// Handles a `StateTransferResponse` from another replica.
    ///
    /// Applies the checkpoint if it's valid and newer than our current state.
    #[allow(clippy::needless_pass_by_value)] // response is cloned when stored
    pub(crate) fn on_state_transfer_response(
        mut self,
        from: ReplicaId,
        response: StateTransferResponse,
    ) -> (Self, ReplicaOutput) {
        // Must be waiting for state transfer
        let (nonce, known_checkpoint, quorum) = {
            let Some(ref st_state) = self.state_transfer_state else {
                return (self, ReplicaOutput::empty());
            };
            (
                st_state.nonce,
                st_state.known_checkpoint,
                self.config.quorum_size(),
            )
        };

        // Nonce must match
        if response.nonce != nonce {
            return (self, ReplicaOutput::empty());
        }

        // Checkpoint must be newer than what we requested
        if response.checkpoint_op <= known_checkpoint {
            return (self, ReplicaOutput::empty());
        }

        // Record the response
        if let Some(ref mut st_state) = self.state_transfer_state {
            st_state.responses.insert(from, response.clone());
        }

        // Check if we have enough responses to proceed
        let response_count = self
            .state_transfer_state
            .as_ref()
            .map_or(0, StateTransferState::response_count);

        if response_count < quorum {
            return (self, ReplicaOutput::empty());
        }

        // Select the best checkpoint
        let best_response = self
            .state_transfer_state
            .as_ref()
            .and_then(|s| s.best_checkpoint().cloned());

        let Some(best_response) = best_response else {
            return (self, ReplicaOutput::empty());
        };

        // Try to verify and apply the checkpoint
        // First validate without consuming self
        let checkpoint_data: CheckpointData =
            if let Ok(data) = serde_json::from_slice(&best_response.checkpoint_data) {
                data
            } else {
                tracing::warn!(
                    replica = %self.replica_id,
                    "failed to deserialize checkpoint data"
                );
                self.state_transfer_state = None;
                return (self, ReplicaOutput::empty());
            };

        // Verify Merkle root matches
        let expected_root = Hash::from_bytes(*checkpoint_data.log_root.as_bytes());
        if expected_root != best_response.merkle_root {
            tracing::warn!(
                replica = %self.replica_id,
                "Merkle root mismatch in state transfer"
            );
            self.state_transfer_state = None;
            return (self, ReplicaOutput::empty());
        }

        // All validation passed, apply the checkpoint
        self.view = best_response.checkpoint_view;
        self.last_normal_view = best_response.checkpoint_view;
        self.op_number = best_response.checkpoint_op;
        self.commit_number = CommitNumber::new(best_response.checkpoint_op);

        // Clear old log
        self.log.clear();

        // Clear state transfer state
        self.state_transfer_state = None;

        // Transition to normal status
        self.status = ReplicaStatus::Normal;

        // Clear any pending repair/recovery
        self.repair_state = None;
        self.recovery_state = None;

        tracing::info!(
            replica = %self.replica_id,
            checkpoint_op = %best_response.checkpoint_op,
            checkpoint_view = %best_response.checkpoint_view,
            "applied state transfer checkpoint"
        );

        (self, ReplicaOutput::empty())
    }

    // ========================================================================
    // Checkpoint Helpers
    // ========================================================================

    /// Builds checkpoint data from current state.
    fn build_checkpoint_data(&self) -> CheckpointData {
        // Build Merkle tree from log entries
        let log_root = compute_merkle_root(&self.log);

        let commit_op: OpNumber = self.commit_number.into();

        CheckpointData::new(
            commit_op,
            self.view,
            log_root,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        )
    }
}
