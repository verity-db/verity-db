//! View change protocol handlers.
//!
//! This module implements the VSR view change protocol:
//!
//! 1. **`StartViewChange`**: Backup suspects leader is dead, broadcasts to all
//! 2. **`DoViewChange`**: After receiving quorum of `StartViewChange`, send to new leader
//! 3. **`StartView`**: New leader broadcasts with authoritative log
//!
//! # Safety Properties
//!
//! - Operations committed in previous views are preserved
//! - At most one leader per view
//! - Progress guaranteed if a majority is available

use crate::message::{DoViewChange, MessagePayload, StartView, StartViewChange};
use crate::types::{ReplicaId, ReplicaStatus, ViewNumber};

use super::{ReplicaOutput, ReplicaState, msg_broadcast, msg_to};

impl ReplicaState {
    // ========================================================================
    // View Change Initiation
    // ========================================================================

    /// Initiates a view change to the next view.
    ///
    /// Called when:
    /// - Backup hasn't heard from leader (heartbeat timeout)
    /// - Backup receives `StartViewChange` for higher view
    pub(crate) fn start_view_change(mut self) -> (Self, ReplicaOutput) {
        let new_view = self.view.next();

        // Transition to new view
        self = self.transition_to_view(new_view);

        // Vote for ourselves
        self.start_view_change_votes.insert(self.replica_id);

        // Broadcast StartViewChange
        let svc = StartViewChange::new(new_view, self.replica_id);
        let msg = msg_broadcast(self.replica_id, MessagePayload::StartViewChange(svc));

        (self, ReplicaOutput::with_messages(vec![msg]))
    }

    // ========================================================================
    // StartViewChange Handler
    // ========================================================================

    /// Handles a `StartViewChange` message.
    ///
    /// If we receive a quorum of `StartViewChange` messages for the same view,
    /// we send `DoViewChange` to the new leader.
    pub(crate) fn on_start_view_change(
        mut self,
        from: ReplicaId,
        svc: StartViewChange,
    ) -> (Self, ReplicaOutput) {
        // If message is for a view we've already passed, ignore
        if svc.view < self.view {
            return (self, ReplicaOutput::empty());
        }

        // If message is for a higher view than we're tracking, start our own view change
        if svc.view > self.view {
            let (new_self, mut output) = self.start_view_change_to(svc.view);
            self = new_self;

            // Record their vote
            self.start_view_change_votes.insert(from);

            // Check if we have quorum
            let (final_self, quorum_output) = self.check_start_view_change_quorum();
            output.merge(quorum_output);

            return (final_self, output);
        }

        // Message is for current view
        if self.status != ReplicaStatus::ViewChange {
            // We're in Normal status at view N, receiving StartViewChange for view N.
            // This can happen if:
            // 1. We've already completed the view change to view N (stale message)
            // 2. Message duplication/reordering
            //
            // In either case, we should start a new view change to view N+1,
            // since someone thinks the current leader is dead.
            let (new_self, output) = self.start_view_change();
            self = new_self;

            // Note: we don't count their vote for view N since we're now in view N+1.
            // They'll need to send a new StartViewChange for view N+1.

            return (self, output);
        }

        // We're already in view change for this view, just record the vote
        self.start_view_change_votes.insert(from);
        self.check_start_view_change_quorum()
    }

    /// Starts a view change to a specific view.
    ///
    /// Called when we receive a message from a higher view and need to catch up.
    pub(crate) fn start_view_change_to(mut self, view: ViewNumber) -> (Self, ReplicaOutput) {
        // Transition to the new view
        self = self.transition_to_view(view);

        // Vote for ourselves
        self.start_view_change_votes.insert(self.replica_id);

        // Broadcast StartViewChange
        let svc = StartViewChange::new(view, self.replica_id);
        let msg = msg_broadcast(self.replica_id, MessagePayload::StartViewChange(svc));

        (self, ReplicaOutput::with_messages(vec![msg]))
    }

    /// Checks if we have a quorum of `StartViewChange` votes.
    ///
    /// If so, sends `DoViewChange` to the new leader.
    fn check_start_view_change_quorum(self) -> (Self, ReplicaOutput) {
        let quorum = self.config.quorum_size();

        if self.start_view_change_votes.len() >= quorum {
            // We have quorum, send DoViewChange to new leader
            let new_leader = self.config.leader_for_view(self.view);

            let dvc = DoViewChange::new(
                self.view,
                self.replica_id,
                self.last_normal_view,
                self.op_number,
                self.commit_number,
                self.log_tail(),
            );

            let msg = msg_to(
                self.replica_id,
                new_leader,
                MessagePayload::DoViewChange(dvc),
            );

            (self, ReplicaOutput::with_messages(vec![msg]))
        } else {
            (self, ReplicaOutput::empty())
        }
    }

    // ========================================================================
    // DoViewChange Handler (New Leader)
    // ========================================================================

    /// Handles a `DoViewChange` message (as new leader).
    ///
    /// Once we receive a quorum of `DoViewChange` messages, we:
    /// 1. Select the log with the highest `last_normal_view` and `op_number`
    /// 2. Broadcast `StartView` with the authoritative log
    /// 3. Enter normal operation as leader
    pub(crate) fn on_do_view_change(
        mut self,
        from: ReplicaId,
        dvc: DoViewChange,
    ) -> (Self, ReplicaOutput) {
        // Ignore if not for our current view
        if dvc.view != self.view {
            return (self, ReplicaOutput::empty());
        }

        // We must be the leader for this view
        if self.config.leader_for_view(self.view) != self.replica_id {
            return (self, ReplicaOutput::empty());
        }

        // Must be in ViewChange status
        if self.status != ReplicaStatus::ViewChange {
            // If we're in Normal status for this view, the view change already completed.
            // This is a stale DoViewChange message - ignore it.
            if self.status == ReplicaStatus::Normal {
                return (self, ReplicaOutput::empty());
            }
            // Otherwise (e.g., Recovering status), we shouldn't process view changes.
            return (self, ReplicaOutput::empty());
        }

        // Record the DoViewChange message
        // Avoid duplicates from the same replica
        if !self.do_view_change_msgs.iter().any(|m| m.replica == from) {
            self.do_view_change_msgs.push(dvc);
        }

        // Check if we have quorum
        self.check_do_view_change_quorum()
    }

    /// Checks if we have a quorum of `DoViewChange` messages.
    ///
    /// If so, becomes leader and broadcasts `StartView`.
    fn check_do_view_change_quorum(mut self) -> (Self, ReplicaOutput) {
        let quorum = self.config.quorum_size();

        if self.do_view_change_msgs.len() < quorum {
            return (self, ReplicaOutput::empty());
        }

        // We have quorum! Become the leader.

        // Find the DoViewChange with the highest (last_normal_view, op_number)
        // This ensures we pick the most up-to-date log.
        // Extract the values we need before moving self.
        let (best_op_number, best_log_tail, max_commit) = {
            let best_dvc = self
                .do_view_change_msgs
                .iter()
                .max_by(|a, b| {
                    (a.last_normal_view, a.op_number).cmp(&(b.last_normal_view, b.op_number))
                })
                .expect("at least quorum messages");

            let max_commit = self
                .do_view_change_msgs
                .iter()
                .map(|dvc| dvc.commit_number)
                .max()
                .unwrap_or(self.commit_number);

            (best_dvc.op_number, best_dvc.log_tail.clone(), max_commit)
        };

        // Update our state from the best DoViewChange
        self.op_number = best_op_number;

        // Merge the log tail from the best DoViewChange
        self = self.merge_log_tail(best_log_tail);

        // Apply commits up to the max known commit
        let (new_self, effects) = self.apply_commits_up_to(max_commit);
        self = new_self;

        // Enter normal status as leader
        self = self.enter_normal_status();

        // Broadcast StartView
        let start_view = StartView::new(
            self.view,
            self.op_number,
            self.commit_number,
            self.log_tail(),
        );

        let msg = msg_broadcast(self.replica_id, MessagePayload::StartView(start_view));

        (
            self,
            ReplicaOutput::with_messages_and_effects(vec![msg], effects),
        )
    }

    // ========================================================================
    // StartView Handler (Backup)
    // ========================================================================

    /// Handles a `StartView` message from the new leader.
    ///
    /// The backup:
    /// 1. Updates its log from the message
    /// 2. Applies any new commits
    /// 3. Enters normal operation
    pub(crate) fn on_start_view(mut self, from: ReplicaId, sv: StartView) -> (Self, ReplicaOutput) {
        // Must be from the leader for this view
        if from != self.config.leader_for_view(sv.view) {
            return (self, ReplicaOutput::empty());
        }

        // If we're behind, accept the new view
        if sv.view < self.view {
            return (self, ReplicaOutput::empty());
        }

        // Update to the new view if needed
        if sv.view > self.view {
            self.view = sv.view;
        }

        // Update our op_number
        self.op_number = sv.op_number;

        // Merge the log tail
        self = self.merge_log_tail(sv.log_tail);

        // Apply commits
        let (new_self, effects) = self.apply_commits_up_to(sv.commit_number);
        self = new_self;

        // Enter normal status
        self = self.enter_normal_status();

        (
            self,
            ReplicaOutput::with_messages_and_effects(vec![], effects),
        )
    }

    // ========================================================================
    // View Change Timeout
    // ========================================================================

    /// Handles view change timeout (view change taking too long).
    ///
    /// Starts a new view change to an even higher view.
    pub(crate) fn on_view_change_timeout(self) -> (Self, ReplicaOutput) {
        // Only relevant if we're in ViewChange status
        if self.status != ReplicaStatus::ViewChange {
            return (self, ReplicaOutput::empty());
        }

        // Start view change to next view
        self.start_view_change()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ClusterConfig;
    use crate::types::{CommitNumber, LogEntry, OpNumber};
    use vdb_kernel::Command;
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

    fn test_entry(op: u64, view: u64) -> LogEntry {
        LogEntry::new(
            OpNumber::new(op),
            ViewNumber::new(view),
            test_command(),
            None,
        )
    }

    #[test]
    fn start_view_change_transitions_status() {
        let config = test_config_3();
        let backup = ReplicaState::new(ReplicaId::new(1), config);

        let (backup, output) = backup.start_view_change();

        assert_eq!(backup.status(), ReplicaStatus::ViewChange);
        assert_eq!(backup.view(), ViewNumber::new(1));

        // Should broadcast StartViewChange
        assert_eq!(output.messages.len(), 1);
        assert!(matches!(
            output.messages[0].payload,
            MessagePayload::StartViewChange(_)
        ));
    }

    #[test]
    fn quorum_of_start_view_change_sends_do_view_change() {
        let config = test_config_3();
        let mut backup = ReplicaState::new(ReplicaId::new(1), config);

        // Start our own view change (votes for ourselves)
        let (new_backup, _) = backup.start_view_change();
        backup = new_backup;

        // Receive StartViewChange from replica 2
        let svc = StartViewChange::new(ViewNumber::new(1), ReplicaId::new(2));
        let (_backup, output) = backup.on_start_view_change(ReplicaId::new(2), svc);

        // Now we have quorum (ourselves + replica 2)
        // Should send DoViewChange to new leader (replica 1, which is us in view 1)
        let dvc_msg = output
            .messages
            .iter()
            .find(|m| matches!(m.payload, MessagePayload::DoViewChange(_)));
        assert!(dvc_msg.is_some());
    }

    #[test]
    fn new_leader_waits_for_quorum_of_do_view_change() {
        let config = test_config_3();

        // In view 1, replica 1 is leader
        let mut new_leader = ReplicaState::new(ReplicaId::new(1), config.clone());
        new_leader = new_leader.transition_to_view(ViewNumber::new(1));

        // Receive DoViewChange from replica 0
        let dvc0 = DoViewChange::new(
            ViewNumber::new(1),
            ReplicaId::new(0),
            ViewNumber::ZERO,
            OpNumber::ZERO,
            CommitNumber::ZERO,
            vec![],
        );

        let (new_leader, output) = new_leader.on_do_view_change(ReplicaId::new(0), dvc0);

        // Don't have quorum yet (need 2, got 1)
        assert!(output.is_empty());
        assert_eq!(new_leader.status(), ReplicaStatus::ViewChange);

        // Receive DoViewChange from replica 2
        let dvc2 = DoViewChange::new(
            ViewNumber::new(1),
            ReplicaId::new(2),
            ViewNumber::ZERO,
            OpNumber::ZERO,
            CommitNumber::ZERO,
            vec![],
        );

        let (new_leader, output) = new_leader.on_do_view_change(ReplicaId::new(2), dvc2);

        // Now have quorum, should become leader
        assert_eq!(new_leader.status(), ReplicaStatus::Normal);
        assert!(new_leader.is_leader());

        // Should broadcast StartView
        let sv_msg = output
            .messages
            .iter()
            .find(|m| matches!(m.payload, MessagePayload::StartView(_)));
        assert!(sv_msg.is_some());
    }

    #[test]
    fn new_leader_picks_best_log() {
        let config = test_config_3();

        let mut new_leader = ReplicaState::new(ReplicaId::new(1), config.clone());
        new_leader = new_leader.transition_to_view(ViewNumber::new(1));

        // Replica 0 has op 1
        let dvc0 = DoViewChange::new(
            ViewNumber::new(1),
            ReplicaId::new(0),
            ViewNumber::ZERO,
            OpNumber::new(1),
            CommitNumber::ZERO,
            vec![test_entry(1, 0)],
        );

        // Replica 2 has op 1 and op 2
        let dvc2 = DoViewChange::new(
            ViewNumber::new(1),
            ReplicaId::new(2),
            ViewNumber::ZERO,
            OpNumber::new(2),
            CommitNumber::ZERO,
            vec![test_entry(1, 0), test_entry(2, 0)],
        );

        let (new_leader, _) = new_leader.on_do_view_change(ReplicaId::new(0), dvc0);
        let (new_leader, _) = new_leader.on_do_view_change(ReplicaId::new(2), dvc2);

        // Should have picked the log from replica 2 (highest op_number)
        assert_eq!(new_leader.op_number(), OpNumber::new(2));
        assert_eq!(new_leader.log_len(), 2);
    }

    #[test]
    fn backup_accepts_start_view() {
        let config = test_config_3();

        let mut backup = ReplicaState::new(ReplicaId::new(0), config);
        backup = backup.transition_to_view(ViewNumber::new(1));

        assert_eq!(backup.status(), ReplicaStatus::ViewChange);

        // New leader (replica 1) sends StartView
        let sv = StartView::new(
            ViewNumber::new(1),
            OpNumber::new(2),
            CommitNumber::ZERO,
            vec![test_entry(1, 1), test_entry(2, 1)],
        );

        let (backup, _) = backup.on_start_view(ReplicaId::new(1), sv);

        // Should be in normal status now
        assert_eq!(backup.status(), ReplicaStatus::Normal);
        assert_eq!(backup.view(), ViewNumber::new(1));
        assert_eq!(backup.op_number(), OpNumber::new(2));
        assert_eq!(backup.log_len(), 2);
    }

    #[test]
    fn view_change_timeout_starts_higher_view() {
        let config = test_config_3();

        let mut backup = ReplicaState::new(ReplicaId::new(1), config);
        backup = backup.transition_to_view(ViewNumber::new(1));

        let (backup, output) = backup.on_view_change_timeout();

        // Should start view change to view 2
        assert_eq!(backup.view(), ViewNumber::new(2));

        // Should broadcast StartViewChange for view 2
        let svc_msg = output.messages.iter().find(|m| {
            if let MessagePayload::StartViewChange(svc) = &m.payload {
                svc.view == ViewNumber::new(2)
            } else {
                false
            }
        });
        assert!(svc_msg.is_some());
    }

    #[test]
    fn higher_view_triggers_view_change() {
        let config = test_config_3();
        let backup = ReplicaState::new(ReplicaId::new(1), config);

        // Currently in view 0
        assert_eq!(backup.view(), ViewNumber::ZERO);

        // Receive StartViewChange for view 5
        let svc = StartViewChange::new(ViewNumber::new(5), ReplicaId::new(2));
        let (backup, _) = backup.on_start_view_change(ReplicaId::new(2), svc);

        // Should jump to view 5
        assert_eq!(backup.view(), ViewNumber::new(5));
        assert_eq!(backup.status(), ReplicaStatus::ViewChange);
    }
}
