//! Unit tests for craton-types

use crate::{
    AppliedIndex, Checkpoint, CheckpointPolicy, Generation, GroupId, HASH_LENGTH, Hash,
    IDEMPOTENCY_ID_LENGTH, IdempotencyId, Offset, RecordHeader, RecordKind, RecoveryReason,
    RecoveryRecord, Region, StreamId, StreamName, TenantId, Timestamp,
};

// ============================================================================
// ID Type Tests
// ============================================================================

#[test]
fn stream_id_from_u64_roundtrip() {
    let id = StreamId::new(42);
    let raw: u64 = id.into();
    assert_eq!(raw, 42);
}

#[test]
fn offset_addition() {
    let a = Offset::new(10);
    let b = Offset::new(5);
    assert_eq!((a + b).as_u64(), 15);
}

#[test]
fn offset_add_assign() {
    let mut a = Offset::new(10);
    a += Offset::new(5);
    assert_eq!(a.as_u64(), 15);
}

#[test]
fn offset_subtraction() {
    let a = Offset::new(10);
    let b = Offset::new(3);
    assert_eq!((a - b).as_u64(), 7);
}

// ============================================================================
// StreamName Tests
// ============================================================================

#[test]
fn stream_name_from_str() {
    let name = StreamName::new("test-stream");
    assert_eq!(name.as_str(), "test-stream");
}

#[test]
fn stream_name_from_string() {
    let name = StreamName::new(String::from("test-stream"));
    assert_eq!(name.as_str(), "test-stream");
}

// ============================================================================
// Region Tests
// ============================================================================

#[test]
fn region_display_known_regions() {
    assert_eq!(Region::USEast1.to_string(), "us-east-1");
    assert_eq!(Region::APSoutheast2.to_string(), "ap-southeast-2");
}

#[test]
fn region_display_custom() {
    let region = Region::custom("eu-west-1");
    assert_eq!(region.to_string(), "eu-west-1");
}

// ============================================================================
// Hash Tests
// ============================================================================

#[test]
fn hash_genesis_is_all_zeros() {
    let genesis = Hash::GENESIS;
    assert!(genesis.is_genesis());
    assert_eq!(genesis.as_bytes(), &[0u8; HASH_LENGTH]);
}

#[test]
fn hash_from_bytes_roundtrip() {
    let bytes = [42u8; HASH_LENGTH];
    let hash = Hash::from_bytes(bytes);
    assert_eq!(hash.as_bytes(), &bytes);
    assert!(!hash.is_genesis());
}

#[test]
fn hash_display_shows_hex() {
    let bytes = [0xab; HASH_LENGTH];
    let hash = Hash::from_bytes(bytes);
    let display = hash.to_string();
    assert_eq!(display.len(), 64); // 32 bytes * 2 hex chars
    assert!(display.chars().all(|c| c == 'a' || c == 'b'));
}

#[test]
fn hash_debug_shows_truncated() {
    let bytes = [0xab; HASH_LENGTH];
    let hash = Hash::from_bytes(bytes);
    let debug = format!("{:?}", hash);
    assert!(debug.starts_with("Hash(abababab"));
    assert!(debug.ends_with("...)"));
}

// ============================================================================
// Timestamp Tests
// ============================================================================

#[test]
fn timestamp_epoch_is_zero() {
    assert_eq!(Timestamp::EPOCH.as_nanos(), 0);
    assert_eq!(Timestamp::EPOCH.as_secs(), 0);
}

#[test]
fn timestamp_from_nanos_roundtrip() {
    let ts = Timestamp::from_nanos(1_000_000_000);
    assert_eq!(ts.as_nanos(), 1_000_000_000);
    assert_eq!(ts.as_secs(), 1);
}

#[test]
fn timestamp_now_is_reasonable() {
    let ts = Timestamp::now();
    // Should be after 2020 (1577836800 seconds since epoch)
    assert!(ts.as_secs() > 1577836800);
}

#[test]
fn timestamp_monotonic_increases() {
    let first = Timestamp::from_nanos(100);
    let second = Timestamp::now_monotonic(Some(first));
    assert!(second > first);
}

#[test]
fn timestamp_monotonic_handles_clock_skew() {
    // Simulate a timestamp far in the future
    let future = Timestamp::from_nanos(u64::MAX - 1000);
    let next = Timestamp::now_monotonic(Some(future));
    // Should be at least future + 1
    assert!(next > future);
}

#[test]
fn timestamp_display_format() {
    let ts = Timestamp::from_nanos(1_234_567_890);
    let display = ts.to_string();
    assert_eq!(display, "1.234567890");
}

// ============================================================================
// RecordKind Tests
// ============================================================================

#[test]
fn record_kind_byte_roundtrip() {
    for kind in [
        RecordKind::Data,
        RecordKind::Checkpoint,
        RecordKind::Tombstone,
    ] {
        let byte = kind.as_byte();
        let recovered = RecordKind::from_byte(byte);
        assert_eq!(recovered, Some(kind));
    }
}

#[test]
fn record_kind_invalid_byte_returns_none() {
    assert_eq!(RecordKind::from_byte(3), None);
    assert_eq!(RecordKind::from_byte(255), None);
}

#[test]
fn record_kind_default_is_data() {
    assert_eq!(RecordKind::default(), RecordKind::Data);
}

// ============================================================================
// RecordHeader Tests
// ============================================================================

#[test]
fn record_header_genesis_detection() {
    let header = RecordHeader::new(
        Offset::ZERO,
        Hash::GENESIS,
        Timestamp::EPOCH,
        100,
        RecordKind::Data,
    );
    assert!(header.is_genesis());
}

#[test]
fn record_header_non_genesis() {
    let header = RecordHeader::new(
        Offset::new(1),
        Hash::from_bytes([1u8; HASH_LENGTH]),
        Timestamp::from_nanos(1000),
        100,
        RecordKind::Data,
    );
    assert!(!header.is_genesis());
}

// ============================================================================
// AppliedIndex Tests
// ============================================================================

#[test]
fn applied_index_genesis() {
    let idx = AppliedIndex::genesis();
    assert_eq!(idx.offset, Offset::ZERO);
    assert!(idx.hash.is_genesis());
}

#[test]
fn applied_index_default_is_genesis() {
    assert_eq!(AppliedIndex::default(), AppliedIndex::genesis());
}

// ============================================================================
// Checkpoint Tests
// ============================================================================

#[test]
fn checkpoint_creation() {
    let cp = Checkpoint::new(
        Offset::new(999),
        Hash::from_bytes([1u8; HASH_LENGTH]),
        1000,
        Timestamp::from_nanos(1_000_000),
    );
    assert_eq!(cp.offset, Offset::new(999));
    assert_eq!(cp.record_count, 1000);
}

// ============================================================================
// CheckpointPolicy Tests
// ============================================================================

#[test]
fn checkpoint_policy_default() {
    let policy = CheckpointPolicy::default();
    assert_eq!(policy.every_n_records, 1000);
    assert!(policy.on_shutdown);
    assert!(!policy.explicit_only);
}

#[test]
fn checkpoint_policy_should_checkpoint() {
    let policy = CheckpointPolicy::every(100);

    // Checkpoint at offset 99 (100th record, 0-indexed)
    assert!(policy.should_checkpoint(Offset::new(99)));
    // Checkpoint at offset 199 (200th record)
    assert!(policy.should_checkpoint(Offset::new(199)));
    // No checkpoint at offset 50
    assert!(!policy.should_checkpoint(Offset::new(50)));
}

#[test]
fn checkpoint_policy_explicit_only_never_auto() {
    let policy = CheckpointPolicy::explicit_only();
    assert!(!policy.should_checkpoint(Offset::new(99)));
    assert!(!policy.should_checkpoint(Offset::new(999)));
}

// ============================================================================
// IdempotencyId Tests
// ============================================================================

#[test]
fn idempotency_id_generate_produces_unique() {
    let id1 = IdempotencyId::generate();
    let id2 = IdempotencyId::generate();
    assert_ne!(id1, id2);
}

#[test]
fn idempotency_id_from_bytes_roundtrip() {
    let bytes = [42u8; IDEMPOTENCY_ID_LENGTH];
    let id = IdempotencyId::from_bytes(bytes);
    assert_eq!(id.as_bytes(), &bytes);
}

#[test]
fn idempotency_id_display_shows_hex() {
    let bytes = [0xab; IDEMPOTENCY_ID_LENGTH];
    let id = IdempotencyId::from_bytes(bytes);
    let display = id.to_string();
    assert_eq!(display.len(), 32); // 16 bytes * 2 hex chars
}

// ============================================================================
// Generation Tests
// ============================================================================

#[test]
fn generation_initial_is_zero() {
    assert_eq!(Generation::INITIAL.as_u64(), 0);
}

#[test]
fn generation_next_increments() {
    let g1 = Generation::new(5);
    let g2 = g1.next();
    assert_eq!(g2.as_u64(), 6);
}

#[test]
fn generation_next_saturates_at_max() {
    let g = Generation::new(u64::MAX);
    assert_eq!(g.next().as_u64(), u64::MAX);
}

#[test]
fn generation_ordering() {
    let g1 = Generation::new(1);
    let g2 = Generation::new(2);
    assert!(g1 < g2);
}

// ============================================================================
// RecoveryRecord Tests
// ============================================================================

#[test]
fn recovery_record_no_data_loss() {
    let record = RecoveryRecord::new(
        Generation::new(2),
        Generation::new(1),
        Offset::new(100),
        Offset::new(100),
        None,
        Timestamp::now(),
        RecoveryReason::NodeRestart,
    );
    assert!(!record.had_data_loss());
    assert_eq!(record.discarded_count(), 0);
}

#[test]
fn recovery_record_with_data_loss() {
    let record = RecoveryRecord::new(
        Generation::new(2),
        Generation::new(1),
        Offset::new(100),
        Offset::new(95),
        Some(Offset::new(96)..Offset::new(101)),
        Timestamp::now(),
        RecoveryReason::QuorumLoss,
    );
    assert!(record.had_data_loss());
    assert_eq!(record.discarded_count(), 5);
}

#[test]
fn recovery_reason_display() {
    assert_eq!(RecoveryReason::NodeRestart.to_string(), "node_restart");
    assert_eq!(RecoveryReason::QuorumLoss.to_string(), "quorum_loss");
    assert_eq!(
        RecoveryReason::CorruptionDetected.to_string(),
        "corruption_detected"
    );
    assert_eq!(
        RecoveryReason::ManualIntervention.to_string(),
        "manual_intervention"
    );
}

// ============================================================================
// Property-Based Tests
// ============================================================================

mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn offset_add_is_commutative(a in 0u64..1_000_000, b in 0u64..1_000_000) {
            let oa = Offset::new(a);
            let ob = Offset::new(b);
            prop_assert_eq!(oa + ob, ob + oa);
        }

        #[test]
        fn id_roundtrip_stream_id(id in any::<u64>()) {
            let stream_id = StreamId::new(id);
            let raw: u64 = stream_id.into();
            prop_assert_eq!(raw, id);
        }

        #[test]
        fn id_roundtrip_tenant_id(id in any::<u64>()) {
            let tenant_id = TenantId::new(id);
            let raw: u64 = tenant_id.into();
            prop_assert_eq!(raw, id);
        }

        #[test]
        fn id_roundtrip_group_id(id in any::<u64>()) {
            let group_id = GroupId::new(id);
            let raw: u64 = group_id.into();
            prop_assert_eq!(raw, id);
        }

        // Hash property tests
        #[test]
        fn hash_roundtrip(bytes in any::<[u8; HASH_LENGTH]>()) {
            let hash = Hash::from_bytes(bytes);
            let recovered: [u8; HASH_LENGTH] = hash.into();
            prop_assert_eq!(recovered, bytes);
        }

        #[test]
        fn hash_genesis_only_for_zeros(bytes in any::<[u8; HASH_LENGTH]>()) {
            let hash = Hash::from_bytes(bytes);
            prop_assert_eq!(hash.is_genesis(), bytes == [0u8; HASH_LENGTH]);
        }

        // Timestamp property tests
        #[test]
        fn timestamp_roundtrip(nanos in any::<u64>()) {
            let ts = Timestamp::from_nanos(nanos);
            prop_assert_eq!(ts.as_nanos(), nanos);
        }

        #[test]
        fn timestamp_secs_is_truncation(nanos in any::<u64>()) {
            let ts = Timestamp::from_nanos(nanos);
            prop_assert_eq!(ts.as_secs(), nanos / 1_000_000_000);
        }

        #[test]
        fn timestamp_monotonic_never_decreases(
            last_nanos in any::<u64>()
        ) {
            let last = Timestamp::from_nanos(last_nanos);
            let next = Timestamp::now_monotonic(Some(last));
            prop_assert!(next >= last);
        }

        // RecordKind property tests
        #[test]
        fn record_kind_valid_bytes_roundtrip(byte in 0u8..3) {
            let kind = RecordKind::from_byte(byte).unwrap();
            prop_assert_eq!(kind.as_byte(), byte);
        }

        #[test]
        fn record_kind_invalid_bytes_are_none(byte in 3u8..=255) {
            prop_assert!(RecordKind::from_byte(byte).is_none());
        }

        // Generation property tests
        #[test]
        fn generation_roundtrip(value in any::<u64>()) {
            let g = Generation::new(value);
            prop_assert_eq!(g.as_u64(), value);
        }

        #[test]
        fn generation_next_increases_or_saturates(value in any::<u64>()) {
            let g = Generation::new(value);
            let next = g.next();
            if value == u64::MAX {
                prop_assert_eq!(next.as_u64(), u64::MAX);
            } else {
                prop_assert_eq!(next.as_u64(), value + 1);
            }
        }

        // IdempotencyId property tests
        #[test]
        fn idempotency_id_roundtrip(bytes in any::<[u8; IDEMPOTENCY_ID_LENGTH]>()) {
            let id = IdempotencyId::from_bytes(bytes);
            let recovered: [u8; IDEMPOTENCY_ID_LENGTH] = id.into();
            prop_assert_eq!(recovered, bytes);
        }

        // CheckpointPolicy property tests
        #[test]
        fn checkpoint_policy_every_n_triggers_correctly(
            every_n in 1u64..1000,
            offset in 0u64..10000
        ) {
            let policy = CheckpointPolicy::every(every_n);
            let should = policy.should_checkpoint(Offset::new(offset));
            prop_assert_eq!(should, (offset + 1) % every_n == 0);
        }
    }
}
