//! Integration tests for craton-vsr.
//!
//! This module contains higher-level integration tests that exercise
//! multiple components together.

use crate::{
    ClusterConfig, CommitNumber, LogEntry, MemorySuperblock, Message, MessagePayload, OpNumber,
    Prepare, PrepareOk, ReplicaId, ReplicaStatus, Superblock, ViewNumber,
};
use craton_kernel::Command;
use craton_types::{DataClass, Placement};

// ============================================================================
// Helper Functions
// ============================================================================

fn test_command() -> Command {
    Command::create_stream_with_auto_id("test-stream".into(), DataClass::NonPHI, Placement::Global)
}

fn test_log_entry(op: u64, view: u64) -> LogEntry {
    LogEntry::new(
        OpNumber::new(op),
        ViewNumber::new(view),
        test_command(),
        None,
    )
}

// ============================================================================
// Cluster Configuration Tests
// ============================================================================

#[test]
fn three_node_cluster_quorum() {
    let config = ClusterConfig::new(vec![
        ReplicaId::new(0),
        ReplicaId::new(1),
        ReplicaId::new(2),
    ]);

    assert_eq!(config.cluster_size(), 3);
    assert_eq!(config.quorum_size(), 2);
    assert_eq!(config.max_failures(), 1);
}

#[test]
fn five_node_cluster_quorum() {
    let config = ClusterConfig::new(vec![
        ReplicaId::new(0),
        ReplicaId::new(1),
        ReplicaId::new(2),
        ReplicaId::new(3),
        ReplicaId::new(4),
    ]);

    assert_eq!(config.cluster_size(), 5);
    assert_eq!(config.quorum_size(), 3);
    assert_eq!(config.max_failures(), 2);
}

#[test]
fn seven_node_cluster_quorum() {
    let config = ClusterConfig::new(vec![
        ReplicaId::new(0),
        ReplicaId::new(1),
        ReplicaId::new(2),
        ReplicaId::new(3),
        ReplicaId::new(4),
        ReplicaId::new(5),
        ReplicaId::new(6),
    ]);

    assert_eq!(config.cluster_size(), 7);
    assert_eq!(config.quorum_size(), 4);
    assert_eq!(config.max_failures(), 3);
}

// ============================================================================
// Message Protocol Tests
// ============================================================================

#[test]
fn prepare_prepare_ok_flow() {
    let leader = ReplicaId::new(0);
    let backup = ReplicaId::new(1);
    let view = ViewNumber::new(0);
    let op = OpNumber::new(1);
    let entry = test_log_entry(1, 0);

    // Leader sends Prepare
    let prepare = Prepare::new(view, op, entry, CommitNumber::ZERO);
    let prepare_msg = Message::broadcast(leader, MessagePayload::Prepare(prepare.clone()));

    assert!(prepare_msg.is_broadcast());
    assert_eq!(prepare_msg.from, leader);

    // Backup responds with PrepareOk
    let prepare_ok = PrepareOk::new(view, op, backup);
    let prepare_ok_msg = Message::targeted(backup, leader, MessagePayload::PrepareOk(prepare_ok));

    assert!(!prepare_ok_msg.is_broadcast());
    assert_eq!(prepare_ok_msg.from, backup);
    assert_eq!(prepare_ok_msg.to, Some(leader));
}

#[test]
fn message_view_extraction() {
    let view = ViewNumber::new(42);

    let heartbeat = MessagePayload::Heartbeat(crate::Heartbeat::new(view, CommitNumber::ZERO));
    assert_eq!(heartbeat.view(), Some(view));

    let prepare = MessagePayload::Prepare(Prepare::new(
        view,
        OpNumber::new(1),
        test_log_entry(1, 42),
        CommitNumber::ZERO,
    ));
    assert_eq!(prepare.view(), Some(view));
}

// ============================================================================
// Superblock Persistence Tests
// ============================================================================

#[test]
fn superblock_survives_partial_write() {
    // Simulate a crash during superblock update
    let storage = MemorySuperblock::new();
    let mut sb = Superblock::create(storage, ReplicaId::new(0)).expect("create");

    // Make some updates
    sb.update(ViewNumber::new(1), OpNumber::new(10), CommitNumber::ZERO)
        .expect("update");
    sb.update(ViewNumber::new(2), OpNumber::new(20), CommitNumber::ZERO)
        .expect("update");

    // Get a copy of storage before the next write
    let stable_data = sb.storage().clone_data();

    // Make another update
    sb.update(ViewNumber::new(3), OpNumber::new(30), CommitNumber::ZERO)
        .expect("update");

    // Simulate crash by reverting to previous state
    let crashed_storage = MemorySuperblock::from_data(stable_data);

    // Recovery should find view 2 as the latest valid state
    let recovered = Superblock::open(crashed_storage).expect("open");
    assert_eq!(recovered.view(), ViewNumber::new(2));
    assert_eq!(recovered.op_number(), OpNumber::new(20));
}

#[test]
fn superblock_all_copies_updated() {
    let storage = MemorySuperblock::new();
    let mut sb = Superblock::create(storage, ReplicaId::new(0)).expect("create");

    // Write 5 times to cycle through all slots + 1
    for i in 1..=5 {
        sb.update(
            ViewNumber::new(i),
            OpNumber::new(i * 10),
            CommitNumber::ZERO,
        )
        .expect("update");
    }

    // Simulate reopening
    let storage2 = MemorySuperblock::from_data(sb.storage().clone_data());

    let recovered = Superblock::open(storage2).expect("open");
    assert_eq!(recovered.view(), ViewNumber::new(5));
    assert_eq!(recovered.op_number(), OpNumber::new(50));
    assert_eq!(recovered.data().sequence, 5);
}

// ============================================================================
// Log Entry Integrity Tests
// ============================================================================

#[test]
fn log_entry_checksum_verification() {
    let entry = test_log_entry(1, 0);

    // Valid entry
    assert!(entry.verify_checksum());

    // Tampered entry
    let mut tampered = entry.clone();
    tampered.op_number = OpNumber::new(999);
    assert!(!tampered.verify_checksum());
}

#[test]
fn log_entry_with_idempotency_id() {
    let id = craton_types::IdempotencyId::generate();
    let entry = LogEntry::new(
        OpNumber::new(1),
        ViewNumber::new(0),
        test_command(),
        Some(id),
    );

    assert!(entry.verify_checksum());
    assert_eq!(entry.idempotency_id, Some(id));
}

// ============================================================================
// Replica Status Tests
// ============================================================================

#[test]
fn replica_status_capabilities() {
    // Normal status can do everything
    assert!(ReplicaStatus::Normal.can_process_requests());
    assert!(ReplicaStatus::Normal.can_participate());

    // ViewChange can participate but not process requests
    assert!(!ReplicaStatus::ViewChange.can_process_requests());
    assert!(ReplicaStatus::ViewChange.can_participate());

    // Recovering can do neither
    assert!(!ReplicaStatus::Recovering.can_process_requests());
    assert!(!ReplicaStatus::Recovering.can_participate());
}

// ============================================================================
// Leader Election Tests
// ============================================================================

#[test]
fn leader_determinism() {
    let config = ClusterConfig::new(vec![
        ReplicaId::new(0),
        ReplicaId::new(1),
        ReplicaId::new(2),
    ]);

    // Same view always yields same leader
    for _ in 0..10 {
        assert_eq!(
            config.leader_for_view(ViewNumber::new(5)),
            config.leader_for_view(ViewNumber::new(5))
        );
    }
}

#[test]
fn leader_rotation_covers_all_replicas() {
    let config = ClusterConfig::new(vec![
        ReplicaId::new(0),
        ReplicaId::new(1),
        ReplicaId::new(2),
    ]);

    let mut seen = std::collections::HashSet::new();

    // After cluster_size views, all replicas should have been leader
    for view in 0..config.cluster_size() {
        let leader = config.leader_for_view(ViewNumber::new(view as u64));
        seen.insert(leader);
    }

    assert_eq!(seen.len(), config.cluster_size());
}

// ============================================================================
// Quorum Math Tests
// ============================================================================

#[test]
fn quorum_math_properties() {
    use crate::types::{max_failures, quorum_size};

    // Property: quorum_size + max_failures = cluster_size (or cluster_size + 1)
    // This ensures any two quorums overlap
    for size in (1..=21).step_by(2) {
        let q = quorum_size(size);
        let f = max_failures(size);

        // Two quorums must overlap by at least 1
        assert!(2 * q > size, "two quorums must overlap");

        // We can tolerate f failures and still have a quorum
        assert!(size - f >= q, "must have quorum after f failures");
    }
}

// ============================================================================
// Commit Number Tests
// ============================================================================

#[test]
fn commit_number_ordering() {
    let commit = CommitNumber::new(OpNumber::new(10));

    // Operations at or before commit are committed
    assert!(commit.is_committed(OpNumber::new(5)));
    assert!(commit.is_committed(OpNumber::new(10)));

    // Operations after commit are not committed
    assert!(!commit.is_committed(OpNumber::new(11)));
    assert!(!commit.is_committed(OpNumber::new(100)));
}

// ============================================================================
// View Change Message Tests
// ============================================================================

#[test]
fn do_view_change_contains_log_tail() {
    use crate::DoViewChange;

    let log_tail = vec![
        test_log_entry(5, 1),
        test_log_entry(6, 1),
        test_log_entry(7, 1),
    ];

    let dvc = DoViewChange::new(
        ViewNumber::new(2),
        ReplicaId::new(1),
        ViewNumber::new(1),
        OpNumber::new(7),
        CommitNumber::new(OpNumber::new(4)),
        log_tail.clone(),
    );

    assert_eq!(dvc.log_tail.len(), 3);
    assert_eq!(dvc.view, ViewNumber::new(2));
    assert_eq!(dvc.last_normal_view, ViewNumber::new(1));
}

// ============================================================================
// Nack Reason Tests
// ============================================================================

#[test]
fn nack_reasons_for_par() {
    use crate::NackReason;

    // NotSeen is safe for truncation
    let not_seen = NackReason::NotSeen;
    assert_eq!(format!("{not_seen}"), "not_seen");

    // SeenButCorrupt is NOT safe for truncation
    let corrupt = NackReason::SeenButCorrupt;
    assert_eq!(format!("{corrupt}"), "seen_but_corrupt");

    // Recovering replicas can't help
    let recovering = NackReason::Recovering;
    assert_eq!(format!("{recovering}"), "recovering");
}

// ============================================================================
// Config Builder Tests
// ============================================================================

#[test]
fn config_with_timeouts() {
    use crate::TimeoutConfig;

    let config = ClusterConfig::new(vec![
        ReplicaId::new(0),
        ReplicaId::new(1),
        ReplicaId::new(2),
    ])
    .with_timeouts(TimeoutConfig::simulation());

    // Simulation timeouts are very short
    assert!(config.timeouts.heartbeat_interval.as_micros() < 1000);
}

#[test]
fn config_with_checkpoint() {
    use crate::CheckpointConfig;

    let config = ClusterConfig::new(vec![
        ReplicaId::new(0),
        ReplicaId::new(1),
        ReplicaId::new(2),
    ])
    .with_checkpoint(CheckpointConfig::testing());

    assert_eq!(config.checkpoint.checkpoint_interval, 10);
    assert!(!config.checkpoint.require_signatures);
}

// ============================================================================
// Single-Node Replicator Integration Tests
// ============================================================================

#[test]
fn single_node_replicator_basic_flow() {
    use crate::{Replicator, SingleNodeReplicator};

    let storage = MemorySuperblock::new();
    let config = ClusterConfig::single_node(ReplicaId::new(0));
    let mut replicator = SingleNodeReplicator::create(config, storage).expect("create");

    // Submit create stream command
    let result = replicator.submit(test_command(), None).expect("submit");

    assert_eq!(result.op_number, OpNumber::new(1));
    assert!(!result.effects.is_empty());

    // Verify replicator state
    assert_eq!(replicator.commit_number().as_u64(), 1);
    assert_eq!(replicator.view(), ViewNumber::ZERO);
    assert_eq!(replicator.status(), ReplicaStatus::Normal);
}

#[test]
fn single_node_effects_include_storage_append() {
    use crate::{Replicator, SingleNodeReplicator};
    use craton_kernel::Effect;

    let storage = MemorySuperblock::new();
    let config = ClusterConfig::single_node(ReplicaId::new(0));
    let mut replicator = SingleNodeReplicator::create(config, storage).expect("create");

    // First, create a stream
    let create_result = replicator
        .submit(test_command(), None)
        .expect("create stream");

    // Find the StreamMetadataWrite effect
    let has_metadata_write = create_result
        .effects
        .iter()
        .any(|e| matches!(e, Effect::StreamMetadataWrite(_)));
    assert!(has_metadata_write, "should have StreamMetadataWrite effect");

    // Now append some data
    let stream_id = craton_types::StreamId::new(0); // Auto-allocated ID
    let append_cmd = craton_kernel::Command::append_batch(
        stream_id,
        vec![bytes::Bytes::from("event-1"), bytes::Bytes::from("event-2")],
        craton_types::Offset::ZERO,
    );

    let append_result = replicator.submit(append_cmd, None).expect("append batch");

    // Find the StorageAppend effect
    let storage_append = append_result
        .effects
        .iter()
        .find(|e| matches!(e, Effect::StorageAppend { .. }));
    assert!(storage_append.is_some(), "should have StorageAppend effect");

    // Verify the effect contains our events
    if let Some(Effect::StorageAppend {
        stream_id: sid,
        events,
        base_offset,
    }) = storage_append
    {
        assert_eq!(*sid, stream_id);
        assert_eq!(events.len(), 2);
        assert_eq!(*base_offset, craton_types::Offset::ZERO);
    }
}

#[test]
fn single_node_log_entries_are_sequential() {
    use crate::{Replicator, SingleNodeReplicator};

    let storage = MemorySuperblock::new();
    let config = ClusterConfig::single_node(ReplicaId::new(0));
    let mut replicator = SingleNodeReplicator::create(config, storage).expect("create");

    // Submit multiple commands
    for i in 1..=10 {
        let cmd = craton_kernel::Command::create_stream_with_auto_id(
            format!("stream-{i}").into(),
            DataClass::NonPHI,
            Placement::Global,
        );
        let result = replicator.submit(cmd, None).expect("submit");
        assert_eq!(result.op_number, OpNumber::new(i));
    }

    // Verify log entries are sequential
    for i in 1..=10 {
        let entry = replicator
            .log_entry(OpNumber::new(i))
            .expect("entry exists");
        assert_eq!(entry.op_number, OpNumber::new(i));
        assert_eq!(entry.view, ViewNumber::ZERO);
        assert!(entry.verify_checksum());
    }

    // Verify commit number advances
    assert_eq!(replicator.commit_number().as_u64(), 10);
}

#[test]
#[allow(clippy::items_after_statements)]
fn single_node_replicator_trait_is_object_safe() {
    use crate::{Replicator, SingleNodeReplicator};

    let storage = MemorySuperblock::new();
    let config = ClusterConfig::single_node(ReplicaId::new(0));
    let mut replicator = SingleNodeReplicator::create(config, storage).expect("create");

    // Should be usable through trait reference
    fn use_replicator(r: &dyn Replicator) -> ViewNumber {
        r.view()
    }

    assert_eq!(use_replicator(&replicator), ViewNumber::ZERO);

    // Mutable operations through trait
    fn submit_via_trait(r: &mut dyn Replicator, cmd: craton_kernel::Command) -> crate::OpNumber {
        r.submit(cmd, None).expect("submit").op_number
    }

    let op = submit_via_trait(&mut replicator, test_command());
    assert_eq!(op, OpNumber::new(1));
}
