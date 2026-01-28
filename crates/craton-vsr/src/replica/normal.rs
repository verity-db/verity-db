//! Normal operation protocol handlers.
//!
//! This module implements the handlers for normal VSR operation:
//! - Prepare (leader → backups)
//! - `PrepareOK` (backup → leader)
//! - Commit (leader → backups)
//! - Heartbeat (leader → backups)

use crate::message::{Commit, Heartbeat, MessagePayload, Prepare, PrepareOk};
use crate::types::{OpNumber, ReplicaId, ReplicaStatus};

use super::{ReplicaOutput, ReplicaState, msg_to};

impl ReplicaState {
    // ========================================================================
    // Prepare Handler (Backup)
    // ========================================================================

    /// Handles a Prepare message from the leader.
    ///
    /// The backup:
    /// 1. Validates the message (view, op number)
    /// 2. Adds the entry to its log
    /// 3. Sends `PrepareOK` back to the leader
    /// 4. Applies any newly committed operations
    pub(crate) fn on_prepare(mut self, from: ReplicaId, prepare: Prepare) -> (Self, ReplicaOutput) {
        // Must be in normal status
        if self.status != ReplicaStatus::Normal {
            return (self, ReplicaOutput::empty());
        }

        // Message must be from the leader
        if from != self.leader() {
            return (self, ReplicaOutput::empty());
        }

        // View must match
        if prepare.view != self.view {
            // If message is from a higher view, we need to catch up
            if prepare.view > self.view {
                tracing::info!(
                    replica = %self.replica_id,
                    our_view = %self.view,
                    msg_view = %prepare.view,
                    "received Prepare from higher view, initiating view change"
                );

                // Check how far behind we are - if just one view, do view change
                // If multiple views behind, consider state transfer
                let view_gap = prepare.view.as_u64().saturating_sub(self.view.as_u64());
                if view_gap > 3 {
                    // We're very far behind - initiate state transfer
                    let (new_self, output) = self.start_state_transfer(Some(prepare.view));
                    return (new_self, output);
                }
                // Otherwise, start a view change to the message's view
                let (new_self, output) = self.start_view_change_to(prepare.view);
                return (new_self, output);
            }
            // Message from lower view - ignore
            return (self, ReplicaOutput::empty());
        }

        // Op number must be next expected or a reasonable gap
        let expected_op = self.op_number.next();
        if prepare.op_number < expected_op {
            // Already have this operation, but still send PrepareOK
            // (leader might have missed our previous response)
            let prepare_ok = PrepareOk::new(self.view, prepare.op_number, self.replica_id);
            let msg = msg_to(self.replica_id, from, MessagePayload::PrepareOk(prepare_ok));
            return (self, ReplicaOutput::with_messages(vec![msg]));
        }

        if prepare.op_number > expected_op {
            // Gap in sequence - we're missing operations
            // Initiate repair to get the missing entries
            tracing::debug!(
                replica = %self.replica_id,
                expected = %expected_op,
                got = %prepare.op_number,
                gap_size = prepare.op_number.distance_to(expected_op),
                "gap in Prepare sequence, initiating repair"
            );

            // Start repair for the missing range [expected_op, prepare.op_number)
            let (new_self, repair_output) = self.start_repair(expected_op, prepare.op_number);

            // Return with repair messages - we'll process this Prepare when repair completes
            return (new_self, repair_output);
        }

        // Validate log entry
        if !prepare.entry.verify_checksum() {
            tracing::warn!(op = %prepare.op_number, "Prepare entry failed checksum");
            return (self, ReplicaOutput::empty());
        }

        // Add entry to log
        self.log.push(prepare.entry);
        self.op_number = prepare.op_number;

        // Send PrepareOK
        let prepare_ok = PrepareOk::new(self.view, prepare.op_number, self.replica_id);
        let msg = msg_to(self.replica_id, from, MessagePayload::PrepareOk(prepare_ok));

        // Apply any new commits from the leader's commit_number
        let (new_self, effects) = self.apply_commits_up_to(prepare.commit_number);

        let mut output = ReplicaOutput::with_messages(vec![msg]);
        output.effects = effects;

        (new_self, output)
    }

    // ========================================================================
    // PrepareOK Handler (Leader)
    // ========================================================================

    /// Handles a `PrepareOK` message from a backup.
    ///
    /// The leader:
    /// 1. Validates the message
    /// 2. Records the vote
    /// 3. Commits if quorum is reached
    pub(crate) fn on_prepare_ok(
        mut self,
        from: ReplicaId,
        prepare_ok: PrepareOk,
    ) -> (Self, ReplicaOutput) {
        // Must be leader
        if !self.is_leader() {
            return (self, ReplicaOutput::empty());
        }

        // Must be in normal status
        if self.status != ReplicaStatus::Normal {
            return (self, ReplicaOutput::empty());
        }

        // View must match
        if prepare_ok.view != self.view {
            return (self, ReplicaOutput::empty());
        }

        // Operation must be pending (not yet committed)
        if prepare_ok.op_number <= self.commit_number.as_op_number() {
            return (self, ReplicaOutput::empty());
        }

        // Operation must exist in our log
        if prepare_ok.op_number > self.op_number {
            return (self, ReplicaOutput::empty());
        }

        // Record the vote
        let voters = self
            .prepare_ok_tracker
            .entry(prepare_ok.op_number)
            .or_default();
        voters.insert(from);

        // Try to commit
        self.try_commit(prepare_ok.op_number)
    }

    // ========================================================================
    // Commit Handler (Backup)
    // ========================================================================

    /// Handles a Commit message from the leader.
    ///
    /// The backup applies committed operations it hasn't yet executed.
    pub(crate) fn on_commit(self, from: ReplicaId, commit: Commit) -> (Self, ReplicaOutput) {
        // Must be in normal status
        if self.status != ReplicaStatus::Normal {
            return (self, ReplicaOutput::empty());
        }

        // Message must be from the leader
        if from != self.leader() {
            return (self, ReplicaOutput::empty());
        }

        // View must match
        if commit.view != self.view {
            return (self, ReplicaOutput::empty());
        }

        // Apply commits
        if commit.commit_number > self.commit_number {
            let (new_self, effects) = self.apply_commits_up_to(commit.commit_number);
            return (
                new_self,
                ReplicaOutput::with_messages_and_effects(vec![], effects),
            );
        }

        (self, ReplicaOutput::empty())
    }

    // ========================================================================
    // Heartbeat Handler (Backup)
    // ========================================================================

    /// Handles a Heartbeat message from the leader.
    ///
    /// Heartbeats serve as:
    /// 1. Liveness signal (leader is alive)
    /// 2. Commit notification (piggybacks `commit_number`)
    pub(crate) fn on_heartbeat(
        self,
        from: ReplicaId,
        heartbeat: Heartbeat,
    ) -> (Self, ReplicaOutput) {
        // Must be in normal status
        if self.status != ReplicaStatus::Normal {
            return (self, ReplicaOutput::empty());
        }

        // Message must be from the leader
        if from != self.leader() {
            return (self, ReplicaOutput::empty());
        }

        // View must match
        if heartbeat.view != self.view {
            // If higher view, we might need to catch up
            if heartbeat.view > self.view {
                tracing::debug!(
                    our_view = %self.view,
                    msg_view = %heartbeat.view,
                    "received Heartbeat from higher view"
                );
            }
            return (self, ReplicaOutput::empty());
        }

        // Apply any commits we're behind on
        if heartbeat.commit_number > self.commit_number {
            let (new_self, effects) = self.apply_commits_up_to(heartbeat.commit_number);
            return (
                new_self,
                ReplicaOutput::with_messages_and_effects(vec![], effects),
            );
        }

        (self, ReplicaOutput::empty())
    }

    // ========================================================================
    // Timeout Handlers
    // ========================================================================

    /// Handles heartbeat timeout (backup hasn't heard from leader).
    ///
    /// Initiates view change by sending `StartViewChange`.
    pub(crate) fn on_heartbeat_timeout(self) -> (Self, ReplicaOutput) {
        // Only backups care about heartbeat timeout
        if self.is_leader() {
            return (self, ReplicaOutput::empty());
        }

        // Must be in normal status to initiate view change
        if self.status != ReplicaStatus::Normal {
            return (self, ReplicaOutput::empty());
        }

        // Start view change to next view
        self.start_view_change()
    }

    /// Handles prepare timeout (leader didn't get quorum).
    ///
    /// Retransmits the Prepare message.
    pub(crate) fn on_prepare_timeout(self, op: OpNumber) -> (Self, ReplicaOutput) {
        // Only leader cares about prepare timeout
        if !self.is_leader() {
            return (self, ReplicaOutput::empty());
        }

        // Must be in normal status
        if self.status != ReplicaStatus::Normal {
            return (self, ReplicaOutput::empty());
        }

        // Operation must still be pending
        if op <= self.commit_number.as_op_number() {
            return (self, ReplicaOutput::empty());
        }

        // Get the log entry
        let entry = match self.log_entry(op) {
            Some(e) => e.clone(),
            None => return (self, ReplicaOutput::empty()),
        };

        // Retransmit Prepare
        let prepare = Prepare::new(self.view, op, entry, self.commit_number);
        let msg = super::msg_broadcast(self.replica_id, MessagePayload::Prepare(prepare));

        (self, ReplicaOutput::with_messages(vec![msg]))
    }

    // ========================================================================
    // Leader Heartbeat Generation
    // ========================================================================

    /// Generates a heartbeat message (for leader to call on tick).
    pub fn generate_heartbeat(&self) -> Option<crate::Message> {
        if !self.is_leader() || self.status != ReplicaStatus::Normal {
            return None;
        }

        let heartbeat = Heartbeat::new(self.view, self.commit_number);
        Some(super::msg_broadcast(
            self.replica_id,
            MessagePayload::Heartbeat(heartbeat),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClusterConfig;
    use crate::types::{CommitNumber, LogEntry, OpNumber, ViewNumber};
    use craton_kernel::Command;
    use craton_types::{DataClass, Placement};

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
    fn leader_prepare_broadcasts_to_backups() {
        let config = test_config_3();
        let leader = ReplicaState::new(ReplicaId::new(0), config);

        let (_leader, output) = leader.prepare_new_operation(test_command(), None);

        // Should have broadcast Prepare message
        assert!(!output.messages.is_empty());

        let prepare_msg = output
            .messages
            .iter()
            .find(|m| matches!(m.payload, MessagePayload::Prepare(_)));
        assert!(prepare_msg.is_some());
        assert!(prepare_msg.unwrap().is_broadcast());
    }

    #[test]
    fn backup_responds_with_prepare_ok() {
        let config = test_config_3();
        let backup = ReplicaState::new(ReplicaId::new(1), config);

        // Leader sends Prepare
        let prepare = Prepare::new(
            ViewNumber::ZERO,
            OpNumber::new(1),
            test_entry(1, 0),
            CommitNumber::ZERO,
        );

        let (backup, output) = backup.on_prepare(ReplicaId::new(0), prepare);

        // Backup should respond with PrepareOK
        assert_eq!(output.messages.len(), 1);

        let msg = &output.messages[0];
        assert!(matches!(msg.payload, MessagePayload::PrepareOk(_)));
        assert_eq!(msg.to, Some(ReplicaId::new(0))); // Sent to leader

        // Backup should have the entry in its log
        assert_eq!(backup.log_len(), 1);
        assert_eq!(backup.op_number(), OpNumber::new(1));
    }

    #[test]
    fn leader_commits_on_quorum() {
        let config = test_config_3();
        let mut leader = ReplicaState::new(ReplicaId::new(0), config);

        // Prepare an operation
        let (new_leader, _) = leader.prepare_new_operation(test_command(), None);
        leader = new_leader;

        // Leader counts itself as one vote
        // Need one more for quorum of 2

        // Backup 1 sends PrepareOK
        let prepare_ok = PrepareOk::new(ViewNumber::ZERO, OpNumber::new(1), ReplicaId::new(1));
        let (leader, output) = leader.on_prepare_ok(ReplicaId::new(1), prepare_ok);

        // Should have committed
        assert_eq!(leader.commit_number().as_u64(), 1);

        // Should have broadcast Commit
        let commit_msg = output
            .messages
            .iter()
            .find(|m| matches!(m.payload, MessagePayload::Commit(_)));
        assert!(commit_msg.is_some());

        // Should have effects from kernel
        assert!(!output.effects.is_empty());
    }

    #[test]
    fn backup_applies_commits_from_heartbeat() {
        let config = test_config_3();
        let mut backup = ReplicaState::new(ReplicaId::new(1), config);

        // Add an entry to backup's log (simulating successful Prepare)
        let entry = test_entry(1, 0);
        backup.log.push(entry);
        backup.op_number = OpNumber::new(1);

        // Leader sends heartbeat with commit
        let heartbeat = Heartbeat::new(ViewNumber::ZERO, CommitNumber::new(OpNumber::new(1)));
        let (backup, output) = backup.on_heartbeat(ReplicaId::new(0), heartbeat);

        // Backup should have committed
        assert_eq!(backup.commit_number().as_u64(), 1);

        // Should have effects from applying the command
        assert!(!output.effects.is_empty());
    }

    #[test]
    fn backup_ignores_prepare_from_non_leader() {
        let config = test_config_3();
        let backup = ReplicaState::new(ReplicaId::new(1), config);

        // Non-leader sends Prepare
        let prepare = Prepare::new(
            ViewNumber::ZERO,
            OpNumber::new(1),
            test_entry(1, 0),
            CommitNumber::ZERO,
        );

        let (backup, output) = backup.on_prepare(ReplicaId::new(2), prepare); // From replica 2, not leader

        // Should be ignored
        assert!(output.is_empty());
        assert_eq!(backup.log_len(), 0);
    }

    #[test]
    fn backup_ignores_prepare_from_lower_view() {
        let config = test_config_3();
        let mut backup = ReplicaState::new(ReplicaId::new(1), config);
        // Simulate backup already being in view 5
        backup = backup.transition_to_view(ViewNumber::new(5));
        backup = backup.enter_normal_status();

        // Someone sends Prepare from old view 0
        let prepare = Prepare::new(
            ViewNumber::new(0), // Lower/old view
            OpNumber::new(1),
            test_entry(1, 0),
            CommitNumber::ZERO,
        );

        let (backup, output) = backup.on_prepare(ReplicaId::new(0), prepare);

        // Should be ignored - stale message from old view
        assert!(output.is_empty());
        assert_eq!(backup.log_len(), 0);
    }

    #[test]
    fn backup_initiates_state_transfer_from_much_higher_view() {
        let config = test_config_3();
        let backup = ReplicaState::new(ReplicaId::new(1), config);

        // Leader sends Prepare from view 99 (much higher than our view 0)
        let prepare = Prepare::new(
            ViewNumber::new(99), // Much higher view
            OpNumber::new(1),
            test_entry(1, 99),
            CommitNumber::ZERO,
        );

        let (backup, output) = backup.on_prepare(ReplicaId::new(0), prepare);

        // Should initiate state transfer (produces broadcast messages)
        assert!(!output.is_empty());
        assert!(backup.state_transfer_state.is_some());
    }

    #[test]
    fn backup_initiates_view_change_from_slightly_higher_view() {
        let config = test_config_3();
        let backup = ReplicaState::new(ReplicaId::new(1), config);

        // Leader sends Prepare from view 2 (slightly higher than our view 0)
        let prepare = Prepare::new(
            ViewNumber::new(2), // Slightly higher view
            OpNumber::new(1),
            test_entry(1, 2),
            CommitNumber::ZERO,
        );

        let (backup, output) = backup.on_prepare(ReplicaId::new(0), prepare);

        // Should initiate view change (produces broadcast messages)
        assert!(!output.is_empty());
        assert_eq!(backup.status(), crate::ReplicaStatus::ViewChange);
    }

    #[test]
    fn leader_generates_heartbeat() {
        let config = test_config_3();
        let leader = ReplicaState::new(ReplicaId::new(0), config.clone());
        let backup = ReplicaState::new(ReplicaId::new(1), config);

        // Leader can generate heartbeat
        let heartbeat = leader.generate_heartbeat();
        assert!(heartbeat.is_some());

        // Backup cannot generate heartbeat
        let no_heartbeat = backup.generate_heartbeat();
        assert!(no_heartbeat.is_none());
    }

    #[test]
    fn heartbeat_timeout_triggers_view_change() {
        let config = test_config_3();
        let backup = ReplicaState::new(ReplicaId::new(1), config);

        let (backup, output) = backup.on_heartbeat_timeout();

        // Should have started view change
        assert_eq!(backup.status(), ReplicaStatus::ViewChange);
        assert_eq!(backup.view(), ViewNumber::new(1));

        // Should have broadcast StartViewChange
        let svc_msg = output
            .messages
            .iter()
            .find(|m| matches!(m.payload, MessagePayload::StartViewChange(_)));
        assert!(svc_msg.is_some());
    }

    #[test]
    fn leader_retransmits_on_prepare_timeout() {
        let config = test_config_3();
        let leader = ReplicaState::new(ReplicaId::new(0), config);

        // Prepare an operation
        let (leader, _) = leader.prepare_new_operation(test_command(), None);

        // Simulate prepare timeout
        let (_, output) = leader.on_prepare_timeout(OpNumber::new(1));

        // Should retransmit Prepare
        let prepare_msg = output
            .messages
            .iter()
            .find(|m| matches!(m.payload, MessagePayload::Prepare(_)));
        assert!(prepare_msg.is_some());
    }
}
