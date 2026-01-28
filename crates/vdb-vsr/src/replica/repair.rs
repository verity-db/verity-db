//! Repair protocol handlers.
//!
//! This module implements transparent repair and Protocol-Aware Recovery (PAR)
//! NACK handling for safe log truncation.
//!
//! # Transparent Repair
//!
//! When a replica detects missing or corrupt log entries:
//! 1. Sends `RepairRequest` for the needed range
//! 2. Healthy replicas respond with `RepairResponse` containing entries
//! 3. Corrupt replicas respond with `Nack` indicating they can't help
//! 4. Apply received entries after checksum verification
//!
//! # Protocol-Aware Recovery (PAR)
//!
//! PAR ensures safe log truncation by distinguishing between:
//! - **`NotSeen`**: The operation was never received (safe to truncate)
//! - **`SeenButCorrupt`**: The operation was seen but is now corrupt (NOT safe)
//!
//! To safely truncate an operation, we need a NACK quorum (f+1 replicas)
//! where ALL NACKs report `NotSeen`. If ANY replica reports `SeenButCorrupt`,
//! the operation may have been committed and cannot be truncated.
//!
//! This is critical for preventing data loss: without PAR, a replica might
//! truncate an operation that was committed but not yet replicated to
//! enough replicas.

use std::collections::HashMap;

use crate::message::{MessagePayload, Nack, NackReason, RepairRequest, RepairResponse};
use crate::types::{Nonce, OpNumber, ReplicaId, ReplicaStatus};

use super::{msg_broadcast, msg_to, ReplicaOutput, ReplicaState};

// ============================================================================
// Repair State
// ============================================================================

/// State tracked during a repair operation.
#[derive(Debug, Clone)]
pub struct RepairState {
    /// Nonce for matching responses to our request.
    pub nonce: Nonce,

    /// Range of operations being repaired (inclusive start, exclusive end).
    pub op_range_start: OpNumber,
    pub op_range_end: OpNumber,

    /// Repair responses received.
    pub responses: HashMap<ReplicaId, RepairResponse>,

    /// NACKs received (for PAR).
    pub nacks: HashMap<ReplicaId, Nack>,
}

impl RepairState {
    /// Creates a new repair state.
    pub fn new(nonce: Nonce, op_range_start: OpNumber, op_range_end: OpNumber) -> Self {
        debug_assert!(
            op_range_start < op_range_end,
            "repair range must be non-empty"
        );
        Self {
            nonce,
            op_range_start,
            op_range_end,
            responses: HashMap::new(),
            nacks: HashMap::new(),
        }
    }

    /// Returns the total number of replies (responses + nacks).
    pub fn total_replies(&self) -> usize {
        self.responses.len() + self.nacks.len()
    }

    /// Returns the number of NACKs with `NotSeen` reason.
    pub fn not_seen_count(&self) -> usize {
        self.nacks
            .values()
            .filter(|n| n.reason == NackReason::NotSeen)
            .count()
    }

    /// Returns true if any NACK indicates the operation was seen.
    pub fn any_seen(&self) -> bool {
        self.nacks
            .values()
            .any(|n| n.reason == NackReason::SeenButCorrupt)
    }

    /// Returns true if we have enough `NotSeen` NACKs for safe truncation.
    ///
    /// Requires f+1 replicas to report `NotSeen`, where f is max failures.
    pub fn can_safely_truncate(&self, max_failures: usize) -> bool {
        let nack_quorum = max_failures + 1;
        self.not_seen_count() >= nack_quorum && !self.any_seen()
    }
}

impl ReplicaState {
    // ========================================================================
    // Repair Initiation
    // ========================================================================

    /// Initiates repair for a range of operations.
    ///
    /// Returns the new state and a `RepairRequest` to broadcast.
    pub fn start_repair(
        mut self,
        op_range_start: OpNumber,
        op_range_end: OpNumber,
    ) -> (Self, ReplicaOutput) {
        if op_range_start >= op_range_end {
            return (self, ReplicaOutput::empty());
        }

        // Generate a nonce for this repair request
        let nonce = Nonce::generate();

        // Initialize repair state
        self.repair_state = Some(RepairState::new(nonce, op_range_start, op_range_end));

        // Create repair request
        let request = RepairRequest::new(self.replica_id, nonce, op_range_start, op_range_end);

        // Broadcast to all replicas
        let msg = msg_broadcast(self.replica_id, MessagePayload::RepairRequest(request));

        tracing::debug!(
            replica = %self.replica_id,
            start = %op_range_start,
            end = %op_range_end,
            "initiated repair"
        );

        (self, ReplicaOutput::with_messages(vec![msg]))
    }

    // ========================================================================
    // RepairRequest Handler
    // ========================================================================

    /// Handles a `RepairRequest` from another replica.
    ///
    /// Responds with either:
    /// - `RepairResponse` if we have the requested entries
    /// - `Nack` if we don't have them (with reason)
    pub(crate) fn on_repair_request(
        self,
        from: ReplicaId,
        request: &RepairRequest,
    ) -> (Self, ReplicaOutput) {
        // Don't respond to our own request
        if from == self.replica_id {
            return (self, ReplicaOutput::empty());
        }

        // If we're recovering, send a NACK
        if self.status == ReplicaStatus::Recovering {
            let nack = Nack::new(
                self.replica_id,
                request.nonce,
                NackReason::Recovering,
                self.op_number,
            );
            let msg = msg_to(self.replica_id, from, MessagePayload::Nack(nack));
            return (self, ReplicaOutput::with_messages(vec![msg]));
        }

        // Try to fulfill the request
        let mut entries = Vec::new();
        let mut can_fulfill = true;
        let mut highest_seen = self.op_number;

        let mut op = request.op_range_start;
        while op < request.op_range_end {
            match self.log_entry(op) {
                Some(entry) if entry.verify_checksum() => {
                    entries.push(entry.clone());
                }
                Some(_) => {
                    // Entry exists but is corrupt
                    can_fulfill = false;
                    break;
                }
                None => {
                    // Entry doesn't exist
                    can_fulfill = false;
                    // Check if we ever saw this op
                    if op > self.op_number {
                        // We never saw this operation
                    } else {
                        // We should have this but don't - it's corrupt/lost
                        highest_seen = highest_seen.max(op);
                    }
                    break;
                }
            }
            op = op.next();
        }

        if can_fulfill {
            // Send the entries
            let response = RepairResponse::new(self.replica_id, request.nonce, entries);
            let msg = msg_to(self.replica_id, from, MessagePayload::RepairResponse(response));
            (self, ReplicaOutput::with_messages(vec![msg]))
        } else {
            // Send NACK with appropriate reason
            let reason = if request.op_range_start > self.op_number {
                // We never received these operations
                NackReason::NotSeen
            } else {
                // We had them but they're corrupt/lost
                NackReason::SeenButCorrupt
            };

            let nack = Nack::new(self.replica_id, request.nonce, reason, highest_seen);
            let msg = msg_to(self.replica_id, from, MessagePayload::Nack(nack));
            (self, ReplicaOutput::with_messages(vec![msg]))
        }
    }

    // ========================================================================
    // RepairResponse Handler
    // ========================================================================

    /// Handles a `RepairResponse` from another replica.
    ///
    /// Applies the received entries after verification.
    pub(crate) fn on_repair_response(
        mut self,
        from: ReplicaId,
        response: &RepairResponse,
    ) -> (Self, ReplicaOutput) {
        // Extract repair state info we need, then release the borrow
        let (nonce, range_start, range_end) = {
            let Some(ref repair_state) = self.repair_state else {
                return (self, ReplicaOutput::empty());
            };
            (repair_state.nonce, repair_state.op_range_start, repair_state.op_range_end)
        };

        // Nonce must match
        if response.nonce != nonce {
            return (self, ReplicaOutput::empty());
        }

        // Record the response (re-borrow mutably)
        if let Some(ref mut repair_state) = self.repair_state {
            repair_state.responses.insert(from, response.clone());
        }

        // Collect entries to apply (to avoid borrow issues)
        let entries_to_apply: Vec<_> = response
            .entries
            .iter()
            .filter(|entry| {
                if !entry.verify_checksum() {
                    tracing::warn!(
                        op = %entry.op_number,
                        from = %from,
                        "received corrupt entry in repair response"
                    );
                    return false;
                }
                entry.op_number >= range_start && entry.op_number < range_end
            })
            .cloned()
            .collect();

        // Apply entries
        for entry in entries_to_apply {
            self = self.merge_log_tail(vec![entry]);
        }

        // Check if repair is complete
        self.check_repair_complete()
    }

    // ========================================================================
    // Nack Handler (PAR)
    // ========================================================================

    /// Handles a `Nack` from another replica.
    ///
    /// Tracks NACKs for PAR - safe truncation requires a quorum of `NotSeen`.
    pub(crate) fn on_nack(mut self, from: ReplicaId, nack: Nack) -> (Self, ReplicaOutput) {
        // Must have repair state
        let Some(ref mut repair_state) = self.repair_state else {
            return (self, ReplicaOutput::empty());
        };

        // Nonce must match
        if nack.nonce != repair_state.nonce {
            return (self, ReplicaOutput::empty());
        }

        tracing::debug!(
            replica = %self.replica_id,
            from = %from,
            reason = %nack.reason,
            highest_seen = %nack.highest_seen,
            "received NACK"
        );

        // Record the NACK
        repair_state.nacks.insert(from, nack);

        // Check if repair is complete
        self.check_repair_complete()
    }

    /// Checks if repair is complete.
    ///
    /// Repair is complete when we either:
    /// 1. Have all the entries we need
    /// 2. Have enough NACKs to make a PAR decision
    fn check_repair_complete(mut self) -> (Self, ReplicaOutput) {
        let Some(ref repair_state) = self.repair_state else {
            return (self, ReplicaOutput::empty());
        };

        // Check if we have all entries
        let mut have_all = true;
        let mut op = repair_state.op_range_start;
        while op < repair_state.op_range_end {
            if self.log_entry(op).is_none() {
                have_all = false;
                break;
            }
            op = op.next();
        }

        if have_all {
            tracing::info!(
                replica = %self.replica_id,
                start = %repair_state.op_range_start,
                end = %repair_state.op_range_end,
                "repair complete"
            );
            self.repair_state = None;
            return (self, ReplicaOutput::empty());
        }

        // Check if we have enough information for a PAR decision
        let cluster_size = self.config.cluster_size();
        let max_failures = self.config.max_failures();

        // We've heard from everyone (or enough)
        if repair_state.total_replies() >= cluster_size - 1 {
            if repair_state.any_seen() {
                // At least one replica saw these operations - cannot truncate
                // This is a data integrity issue - we need manual intervention
                tracing::error!(
                    replica = %self.replica_id,
                    start = %repair_state.op_range_start,
                    end = %repair_state.op_range_end,
                    "repair failed: some replicas saw operations, cannot truncate"
                );
                self.repair_state = None;
            } else if repair_state.can_safely_truncate(max_failures) {
                // All NACKs are NotSeen - safe to consider these operations as never committed
                tracing::warn!(
                    replica = %self.replica_id,
                    start = %repair_state.op_range_start,
                    end = %repair_state.op_range_end,
                    nack_count = repair_state.not_seen_count(),
                    "PAR: safely abandoning uncommitted operations"
                );
                // The operations were never committed, so we don't need to track them
                self.repair_state = None;
            }
        }

        (self, ReplicaOutput::empty())
    }

    // ========================================================================
    // Accessors
    // ========================================================================

    /// Returns the current repair state, if any.
    pub fn repair_state(&self) -> Option<&RepairState> {
        self.repair_state.as_ref()
    }

    /// Returns true if a repair is in progress.
    pub fn is_repairing(&self) -> bool {
        self.repair_state.is_some()
    }

    /// Detects gaps or corruption in the log and initiates repair if needed.
    ///
    /// Returns true if repair was initiated.
    pub fn check_and_repair(self) -> (Self, ReplicaOutput, bool) {
        // Don't repair if already repairing or recovering
        if self.is_repairing() || self.is_recovering() {
            return (self, ReplicaOutput::empty(), false);
        }

        // Find gaps in the log
        let mut gap_start: Option<OpNumber> = None;
        let mut op = OpNumber::new(1);

        while op <= self.op_number {
            match self.log_entry(op) {
                Some(entry) if entry.verify_checksum() => {
                    // Entry is valid
                    if let Some(start) = gap_start {
                        // End of gap found
                        let (new_self, output) = self.start_repair(start, op);
                        return (new_self, output, true);
                    }
                }
                _ => {
                    // Entry is missing or corrupt
                    if gap_start.is_none() {
                        gap_start = Some(op);
                    }
                }
            }
            op = op.next();
        }

        // Check if there's a gap at the end
        if let Some(start) = gap_start {
            let end = self.op_number.next();
            let (new_self, output) = self.start_repair(start, end);
            return (new_self, output, true);
        }

        (self, ReplicaOutput::empty(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClusterConfig;
    use crate::types::{LogEntry, OpNumber, ViewNumber};
    use vdb_kernel::Command;
    use vdb_types::{DataClass, Placement};

    fn test_config_3() -> ClusterConfig {
        ClusterConfig::new(vec![
            ReplicaId::new(0),
            ReplicaId::new(1),
            ReplicaId::new(2),
        ])
    }

    fn test_config_5() -> ClusterConfig {
        ClusterConfig::new(vec![
            ReplicaId::new(0),
            ReplicaId::new(1),
            ReplicaId::new(2),
            ReplicaId::new(3),
            ReplicaId::new(4),
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
    fn start_repair_broadcasts_request() {
        let config = test_config_3();
        let replica = ReplicaState::new(ReplicaId::new(0), config);

        let (replica, output) = replica.start_repair(OpNumber::new(1), OpNumber::new(5));

        assert!(replica.is_repairing());
        assert_eq!(output.messages.len(), 1);
        assert!(matches!(
            output.messages[0].payload,
            MessagePayload::RepairRequest(_)
        ));
    }

    #[test]
    fn healthy_replica_responds_with_entries() {
        let config = test_config_3();
        let mut healthy = ReplicaState::new(ReplicaId::new(0), config);

        // Add entries
        healthy.log.push(test_entry(1, 0));
        healthy.log.push(test_entry(2, 0));
        healthy.log.push(test_entry(3, 0));
        healthy.op_number = OpNumber::new(3);

        // Receive repair request
        let request = RepairRequest::new(
            ReplicaId::new(1),
            Nonce::generate(),
            OpNumber::new(1),
            OpNumber::new(4),
        );

        let (_healthy, output) = healthy.on_repair_request(ReplicaId::new(1), &request);

        assert_eq!(output.messages.len(), 1);
        let msg = &output.messages[0];
        assert!(matches!(msg.payload, MessagePayload::RepairResponse(_)));

        if let MessagePayload::RepairResponse(ref resp) = msg.payload {
            assert_eq!(resp.entries.len(), 3);
        }
    }

    #[test]
    fn replica_nacks_when_missing_entries() {
        let config = test_config_3();
        let healthy = ReplicaState::new(ReplicaId::new(0), config);
        // No entries in log

        // Request entries we don't have
        let request = RepairRequest::new(
            ReplicaId::new(1),
            Nonce::generate(),
            OpNumber::new(5),
            OpNumber::new(10),
        );

        let (_healthy, output) = healthy.on_repair_request(ReplicaId::new(1), &request);

        assert_eq!(output.messages.len(), 1);
        let msg = &output.messages[0];
        assert!(matches!(msg.payload, MessagePayload::Nack(_)));

        if let MessagePayload::Nack(ref nack) = msg.payload {
            assert_eq!(nack.reason, NackReason::NotSeen);
        }
    }

    #[test]
    fn recovering_replica_nacks_with_recovering_reason() {
        let config = test_config_3();
        let mut replica = ReplicaState::new(ReplicaId::new(0), config);
        replica.status = ReplicaStatus::Recovering;

        let request = RepairRequest::new(
            ReplicaId::new(1),
            Nonce::generate(),
            OpNumber::new(1),
            OpNumber::new(5),
        );

        let (_replica, output) = replica.on_repair_request(ReplicaId::new(1), &request);

        if let MessagePayload::Nack(ref nack) = output.messages[0].payload {
            assert_eq!(nack.reason, NackReason::Recovering);
        }
    }

    #[test]
    fn repair_applies_received_entries() {
        let config = test_config_3();
        let replica = ReplicaState::new(ReplicaId::new(0), config);

        // Start repair
        let (replica, _) = replica.start_repair(OpNumber::new(1), OpNumber::new(3));
        let nonce = replica.repair_state().unwrap().nonce;

        // Receive response with entries
        let response = RepairResponse::new(
            ReplicaId::new(1),
            nonce,
            vec![test_entry(1, 0), test_entry(2, 0)],
        );

        let (replica, _) = replica.on_repair_response(ReplicaId::new(1), &response);

        // Entries should be applied
        assert!(replica.log_entry(OpNumber::new(1)).is_some());
        assert!(replica.log_entry(OpNumber::new(2)).is_some());
    }

    #[test]
    fn par_requires_quorum_of_not_seen() {
        let config = test_config_5();
        let max_failures = config.max_failures();
        let replica = ReplicaState::new(ReplicaId::new(0), config);

        // Start repair
        let (mut replica, _) = replica.start_repair(OpNumber::new(10), OpNumber::new(15));
        let nonce = replica.repair_state().unwrap().nonce;

        // Receive NotSeen from replicas 1 and 2 (need 3 for f+1 in 5-node cluster)
        for i in 1..=2 {
            let nack = Nack::new(
                ReplicaId::new(i),
                nonce,
                NackReason::NotSeen,
                OpNumber::new(5),
            );
            let (new_replica, _) = replica.on_nack(ReplicaId::new(i), nack);
            replica = new_replica;
        }

        // Still repairing - don't have quorum
        assert!(replica.is_repairing());

        // Third NotSeen gives us quorum
        let nack = Nack::new(
            ReplicaId::new(3),
            nonce,
            NackReason::NotSeen,
            OpNumber::new(5),
        );
        let (replica, _) = replica.on_nack(ReplicaId::new(3), nack);

        // Still need more responses to complete (cluster_size - 1)
        // But PAR state should be tracking correctly
        let repair_state = replica.repair_state().unwrap();
        assert!(repair_state.can_safely_truncate(max_failures));
    }

    #[test]
    fn par_blocks_truncation_when_seen_but_corrupt() {
        // Test the RepairState PAR logic directly (the replica auto-clears state
        // when it has enough information to make a PAR decision)
        let nonce = Nonce::generate();
        let mut repair_state = RepairState::new(nonce, OpNumber::new(5), OpNumber::new(10));

        // max_failures for a 3-node cluster is 1
        let max_failures = 1;

        // One replica says NotSeen
        let nack1 = Nack::new(
            ReplicaId::new(1),
            nonce,
            NackReason::NotSeen,
            OpNumber::new(3),
        );
        repair_state.nacks.insert(ReplicaId::new(1), nack1);

        // With just NotSeen, truncation could be safe (need f+1 = 2 for quorum)
        assert!(!repair_state.any_seen());
        assert!(!repair_state.can_safely_truncate(max_failures)); // Only 1, need 2

        // Another says SeenButCorrupt - blocks truncation!
        let nack2 = Nack::new(
            ReplicaId::new(2),
            nonce,
            NackReason::SeenButCorrupt,
            OpNumber::new(7),
        );
        repair_state.nacks.insert(ReplicaId::new(2), nack2);

        // Now any_seen() is true, so truncation is blocked
        assert!(repair_state.any_seen());
        assert!(!repair_state.can_safely_truncate(max_failures));
    }

    #[test]
    fn repair_state_tracks_counts() {
        let nonce = Nonce::generate();
        let mut state = RepairState::new(nonce, OpNumber::new(1), OpNumber::new(10));

        // Add some NACKs
        state.nacks.insert(
            ReplicaId::new(1),
            Nack::new(ReplicaId::new(1), nonce, NackReason::NotSeen, OpNumber::new(0)),
        );
        state.nacks.insert(
            ReplicaId::new(2),
            Nack::new(ReplicaId::new(2), nonce, NackReason::NotSeen, OpNumber::new(0)),
        );
        state.nacks.insert(
            ReplicaId::new(3),
            Nack::new(ReplicaId::new(3), nonce, NackReason::Recovering, OpNumber::new(0)),
        );

        assert_eq!(state.not_seen_count(), 2);
        assert_eq!(state.total_replies(), 3);
        assert!(!state.any_seen());

        // Add SeenButCorrupt
        state.nacks.insert(
            ReplicaId::new(4),
            Nack::new(ReplicaId::new(4), nonce, NackReason::SeenButCorrupt, OpNumber::new(5)),
        );

        assert!(state.any_seen());
    }
}
