//! Unit tests for craton-storage
//!
//! Tests for the append-only segment storage layer with hash chain integrity.

use bytes::Bytes;
use tempfile::TempDir;
use craton_crypto::ChainHash;
use craton_types::{Offset, StreamId};

use crate::{OffsetIndex, Record, Storage, StorageError};

// ============================================================================
// Record Serialization Tests
// ============================================================================

#[test]
fn record_to_bytes_produces_correct_format() {
    let record = Record::new(Offset::new(42), None, Bytes::from("hello"));
    let bytes = record.to_bytes();

    // Total size: 8 (offset) + 32 (prev_hash) + 1 (kind) + 4 (len) + 5 (payload) + 4 (crc) = 54 bytes
    assert_eq!(bytes.len(), 54);

    // First 8 bytes: offset (42 in little-endian)
    let offset = i64::from_le_bytes(bytes[0..8].try_into().unwrap());
    assert_eq!(offset, 42);

    // Next 32 bytes: prev_hash (all zeros for genesis)
    assert_eq!(&bytes[8..40], &[0u8; 32]);

    // Next 1 byte: kind (0 = Data)
    assert_eq!(bytes[40], 0);

    // Next 4 bytes: length (5 in little-endian)
    let length = u32::from_le_bytes(bytes[41..45].try_into().unwrap());
    assert_eq!(length, 5);

    // Next 5 bytes: payload
    assert_eq!(&bytes[45..50], b"hello");

    // Last 4 bytes: CRC (verify it matches expected)
    let stored_crc = u32::from_le_bytes(bytes[50..54].try_into().unwrap());
    let computed_crc = crc32fast::hash(&bytes[0..50]);
    assert_eq!(stored_crc, computed_crc);
}

#[test]
fn record_roundtrip_preserves_data() {
    let original = Record::new(Offset::new(123), None, Bytes::from("test payload"));
    let bytes: Bytes = original.to_bytes().into();

    let (parsed, consumed) = Record::from_bytes(&bytes).unwrap();

    assert_eq!(parsed.offset(), Offset::new(123));
    assert_eq!(parsed.prev_hash(), None);
    assert_eq!(parsed.payload().as_ref(), b"test payload");
    assert_eq!(consumed, bytes.len());
}

#[test]
fn record_roundtrip_with_prev_hash() {
    // Create a chain hash from known bytes
    let prev_hash = ChainHash::from_bytes(&[42u8; 32]);
    let original = Record::new(Offset::new(1), Some(prev_hash), Bytes::from("linked"));
    let bytes: Bytes = original.to_bytes().into();

    let (parsed, _) = Record::from_bytes(&bytes).unwrap();

    assert_eq!(parsed.offset(), Offset::new(1));
    assert_eq!(parsed.prev_hash(), Some(prev_hash));
    assert_eq!(parsed.payload().as_ref(), b"linked");
}

#[test]
fn record_from_bytes_detects_corruption() {
    let record = Record::new(Offset::new(0), None, Bytes::from("data"));
    let mut bytes: Vec<u8> = record.to_bytes();

    // Corrupt one byte in the payload (at offset 45, after kind byte)
    bytes[45] ^= 0xFF;

    let result = Record::from_bytes(&Bytes::from(bytes));
    assert!(matches!(result, Err(StorageError::CorruptedRecord)));
}

#[test]
fn record_from_bytes_handles_truncated_header() {
    // Less than 45 bytes (minimum header size: 8 + 32 + 1 + 4)
    let short_data = Bytes::from(vec![0u8; 40]);
    let result = Record::from_bytes(&short_data);
    assert!(matches!(result, Err(StorageError::UnexpectedEof)));
}

#[test]
fn record_from_bytes_handles_truncated_payload() {
    // Create a header claiming 100 bytes of payload
    let mut data = Vec::new();
    data.extend_from_slice(&0i64.to_le_bytes()); // offset
    data.extend_from_slice(&[0u8; 32]); // prev_hash
    data.push(0); // kind: Data
    data.extend_from_slice(&100u32.to_le_bytes()); // length: 100 bytes
    data.extend_from_slice(&[0u8; 50]); // only 50 bytes of payload

    let result = Record::from_bytes(&Bytes::from(data));
    assert!(matches!(result, Err(StorageError::UnexpectedEof)));
}

#[test]
fn record_empty_payload() {
    let record = Record::new(Offset::new(0), None, Bytes::new());
    let bytes: Bytes = record.to_bytes().into();

    let (parsed, _) = Record::from_bytes(&bytes).unwrap();
    assert!(parsed.payload().is_empty());
}

#[test]
fn record_compute_hash_creates_chain() {
    // Genesis record
    let record0 = Record::new(Offset::new(0), None, Bytes::from("hello"));
    let hash0 = record0.compute_hash();

    // Second record links to first
    let record1 = Record::new(Offset::new(1), Some(hash0), Bytes::from("world"));
    let hash1 = record1.compute_hash();

    // Hashes should be different
    assert_ne!(hash0, hash1);

    // Same input should produce same hash
    let record1_copy = Record::new(Offset::new(1), Some(hash0), Bytes::from("world"));
    assert_eq!(record1_copy.compute_hash(), hash1);
}

// ============================================================================
// Index Integration Tests
// ============================================================================

#[test]
fn test_append_and_lookup() {
    let mut index = OffsetIndex::new();
    index.append(0);
    index.append(100);
    index.append(250);

    assert_eq!(index.lookup(Offset::new(0)), Some(0));
    assert_eq!(index.lookup(Offset::new(1)), Some(100));
    assert_eq!(index.lookup(Offset::new(2)), Some(250));
    assert_eq!(index.lookup(Offset::new(3)), None); // out of bounds
    assert_eq!(index.len(), 3);
    assert!(!index.is_empty());
}

#[test]
fn test_save_and_load_roundtrip() {
    let temp_dir = TempDir::new().unwrap();
    let index_path = temp_dir.path().join("test.idx");

    let mut original = OffsetIndex::new();

    original.append(0);
    original.append(100);
    original.append(250);

    original.save(&index_path).expect("save succeeds");

    let loaded = OffsetIndex::load(&index_path).expect("load succeeds");

    // Verify loaded matches original
    assert_eq!(loaded.len(), 3);
    assert_eq!(loaded.lookup(Offset::new(0)), Some(0));
    assert_eq!(loaded.lookup(Offset::new(1)), Some(100));
    assert_eq!(loaded.lookup(Offset::new(2)), Some(250));
}

#[test]
fn test_empty_index_roundtrip() {
    let temp_dir = TempDir::new().unwrap();
    let index_path = temp_dir.path().join("empty.idx");

    let original = OffsetIndex::new();
    assert!(original.is_empty());

    original.save(&index_path).expect("save succeeds");

    let loaded = OffsetIndex::load(&index_path).expect("load succeeds");

    assert!(loaded.is_empty());
    assert_eq!(loaded.len(), 0);
}

#[test]
fn test_large_index_roundtrip() {
    let temp_dir = TempDir::new().unwrap();
    let index_path = temp_dir.path().join("large.idx");

    let mut original = OffsetIndex::new();

    // Add 10,000 positions with increasing byte offsets
    let mut byte_pos = 0u64;
    for _ in 0..10_000 {
        original.append(byte_pos);
        byte_pos += 53; // Simulate ~53 byte records
    }

    original.save(&index_path).expect("save succeeds");

    let loaded = OffsetIndex::load(&index_path).expect("load succeeds");

    assert_eq!(loaded.len(), 10_000);
    assert_eq!(loaded.lookup(Offset::new(0)), Some(0));
    assert_eq!(loaded.lookup(Offset::new(9999)), Some(9999 * 53));
}

#[test]
fn test_load_detects_invalid_magic() {
    let temp_dir = TempDir::new().unwrap();
    let index_path = temp_dir.path().join("bad_magic.idx");

    // Write file with wrong magic bytes
    let mut data = vec![0u8; 20]; // minimum valid size
    data[0..4].copy_from_slice(b"XXXX"); // wrong magic
    std::fs::write(&index_path, &data).unwrap();

    let result = OffsetIndex::load(&index_path);
    assert!(matches!(result, Err(StorageError::InvalidIndexMagic)));
}

#[test]
fn test_load_detects_unsupported_version() {
    let temp_dir = TempDir::new().unwrap();
    let index_path = temp_dir.path().join("bad_version.idx");

    // Write file with correct magic but wrong version
    let mut data = vec![0u8; 24];
    data[0..4].copy_from_slice(b"VDXI");
    data[4] = 0xFF; // unsupported version
    // Add a valid CRC for the data
    let crc = crc32fast::hash(&data[0..20]);
    data[20..24].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(&index_path, &data).unwrap();

    let result = OffsetIndex::load(&index_path);
    assert!(matches!(
        result,
        Err(StorageError::UnsupportedIndexVersion(0xFF))
    ));
}

#[test]
fn test_load_detects_truncated_file() {
    let temp_dir = TempDir::new().unwrap();
    let index_path = temp_dir.path().join("truncated.idx");

    // Write file that's too short (less than header + CRC)
    let data = vec![0u8; 10];
    std::fs::write(&index_path, &data).unwrap();

    let result = OffsetIndex::load(&index_path);
    assert!(matches!(
        result,
        Err(StorageError::IndexTruncated {
            expected: 20,
            actual: 10
        })
    ));
}

#[test]
fn test_load_detects_truncated_positions() {
    let temp_dir = TempDir::new().unwrap();
    let index_path = temp_dir.path().join("truncated_pos.idx");

    // Create valid header claiming 10 positions, but only provide space for 5
    let mut data = vec![0u8; 16 + (5 * 8) + 4]; // header + 5 positions + CRC
    data[0..4].copy_from_slice(b"VDXI");
    data[4] = 0x01; // version
    data[8..16].copy_from_slice(&10u64.to_le_bytes()); // claims 10 positions
    // CRC of the truncated data (will be wrong but we check truncation first)
    let crc = crc32fast::hash(&data[0..data.len() - 4]);
    let crc_start = data.len() - 4;
    data[crc_start..].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(&index_path, &data).unwrap();

    let result = OffsetIndex::load(&index_path);
    // Expected: header(16) + 10 positions(80) + CRC(4) = 100 bytes
    // Actual: header(16) + 5 positions(40) + CRC(4) = 60 bytes
    assert!(matches!(
        result,
        Err(StorageError::IndexTruncated {
            expected: 100,
            actual: 60
        })
    ));
}

#[test]
fn test_load_detects_corrupted_crc() {
    let temp_dir = TempDir::new().unwrap();
    let index_path = temp_dir.path().join("corrupted.idx");

    // Create a valid index, then corrupt it
    let mut index = OffsetIndex::new();
    index.append(0);
    index.append(100);
    index.save(&index_path).expect("save succeeds");

    // Corrupt a byte in the positions area
    let mut data = std::fs::read(&index_path).unwrap();
    data[16] ^= 0xFF; // flip bits in first position
    std::fs::write(&index_path, &data).unwrap();

    let result = OffsetIndex::load(&index_path);
    assert!(matches!(
        result,
        Err(StorageError::IndexChecksumMismatch { .. })
    ));
}

#[test]
fn test_from_positions_creates_valid_index() {
    let positions = vec![0, 100, 250, 400];
    let index = OffsetIndex::from_positions(positions.clone());

    assert_eq!(index.len(), 4);
    assert_eq!(index.positions(), positions.as_slice());
    assert_eq!(index.lookup(Offset::new(2)), Some(250));
}

#[test]
fn test_index_equality() {
    let mut index1 = OffsetIndex::new();
    index1.append(0);
    index1.append(100);

    let index2 = OffsetIndex::from_positions(vec![0, 100]);

    assert_eq!(index1, index2);
}

#[test]
fn test_index_clone() {
    let mut original = OffsetIndex::new();
    original.append(0);
    original.append(100);

    let cloned = original.clone();

    assert_eq!(original, cloned);
    assert_eq!(cloned.lookup(Offset::new(1)), Some(100));
}

// ============================================================================
// Storage Integration Tests
// ============================================================================

mod integration {
    use super::*;
    use tempfile::TempDir;

    fn setup_storage() -> (Storage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = Storage::new(temp_dir.path());
        (storage, temp_dir)
    }

    fn test_events(count: usize) -> Vec<Bytes> {
        (0..count)
            .map(|i| Bytes::from(format!("event-{i}")))
            .collect()
    }

    #[test]
    fn append_and_read_single_event() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        let (new_offset, _hash) = storage
            .append_batch(stream_id, test_events(1), Offset::new(0), None, false)
            .unwrap();

        assert_eq!(new_offset, Offset::new(1));

        let events = storage
            .read_from(stream_id, Offset::new(0), u64::MAX)
            .unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].as_ref(), b"event-0");
    }

    #[test]
    fn append_and_read_multiple_events() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        storage
            .append_batch(stream_id, test_events(5), Offset::new(0), None, false)
            .unwrap();

        let events = storage
            .read_from(stream_id, Offset::new(0), u64::MAX)
            .unwrap();

        assert_eq!(events.len(), 5);
        (0..5).for_each(|i| {
            assert_eq!(events[i].as_ref(), format!("event-{i}").as_bytes());
        });
    }

    #[test]
    fn read_from_middle_offset() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        // Append 10 events
        storage
            .append_batch(stream_id, test_events(10), Offset::new(0), None, false)
            .unwrap();

        // Read from offset 5
        let events = storage
            .read_from(stream_id, Offset::new(5), u64::MAX)
            .unwrap();

        // Should get events 5-9
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].as_ref(), b"event-5");
        assert_eq!(events[4].as_ref(), b"event-9");
    }

    #[test]
    fn read_respects_max_bytes() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        // Create events with known sizes
        let events: Vec<Bytes> = (0..10)
            .map(|i| Bytes::from(format!("event-{i:04}"))) // Each ~10 bytes
            .collect();

        storage
            .append_batch(stream_id, events, Offset::new(0), None, false)
            .unwrap();

        // Read with max_bytes that should limit results
        // Each event is ~10 bytes, so max_bytes=25 should give us 2-3 events
        let events = storage.read_from(stream_id, Offset::new(0), 25).unwrap();

        // Should get fewer than all 10 events
        assert!(events.len() < 10);
        assert!(!events.is_empty());
    }

    #[test]
    fn append_multiple_batches_sequential() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        // Append batch 1 (3 events)
        let (offset_after_batch1, hash_after_batch1) = storage
            .append_batch(stream_id, test_events(3), Offset::new(0), None, false)
            .unwrap();
        assert_eq!(offset_after_batch1, Offset::new(3));

        // Append batch 2 (2 events) starting at offset 3, continuing the chain
        let events2: Vec<Bytes> = vec![Bytes::from("batch2-0"), Bytes::from("batch2-1")];
        let (offset_after_batch2, _) = storage
            .append_batch(
                stream_id,
                events2,
                Offset::new(3),
                Some(hash_after_batch1),
                false,
            )
            .unwrap();
        assert_eq!(offset_after_batch2, Offset::new(5));

        // Read all events
        let events = storage
            .read_from(stream_id, Offset::new(0), u64::MAX)
            .unwrap();

        assert_eq!(events.len(), 5);
        // First 3 from batch 1
        assert_eq!(events[0].as_ref(), b"event-0");
        assert_eq!(events[2].as_ref(), b"event-2");
        // Last 2 from batch 2
        assert_eq!(events[3].as_ref(), b"batch2-0");
        assert_eq!(events[4].as_ref(), b"batch2-1");
    }

    #[test]
    fn append_with_fsync() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        // Append with fsync=true
        let result = storage.append_batch(stream_id, test_events(1), Offset::new(0), None, true);

        // Should succeed (fsync is just durability, shouldn't change behavior)
        assert!(result.is_ok());
    }

    #[test]
    fn multiple_streams_are_isolated() {
        let (mut storage, _dir) = setup_storage();
        let stream1 = StreamId::new(1);
        let stream2 = StreamId::new(2);

        // Append to stream 1
        storage
            .append_batch(
                stream1,
                vec![Bytes::from("stream1-event")],
                Offset::new(0),
                None,
                false,
            )
            .unwrap();

        // Append to stream 2
        storage
            .append_batch(
                stream2,
                vec![Bytes::from("stream2-event")],
                Offset::new(0),
                None,
                false,
            )
            .unwrap();

        // Read from each stream
        let events1 = storage
            .read_from(stream1, Offset::new(0), u64::MAX)
            .unwrap();
        let events2 = storage
            .read_from(stream2, Offset::new(0), u64::MAX)
            .unwrap();

        assert_eq!(events1.len(), 1);
        assert_eq!(events1[0].as_ref(), b"stream1-event");

        assert_eq!(events2.len(), 1);
        assert_eq!(events2[0].as_ref(), b"stream2-event");
    }

    #[test]
    fn hash_chain_is_built_correctly() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        // Append 3 events
        let (_, final_hash) = storage
            .append_batch(stream_id, test_events(3), Offset::new(0), None, false)
            .unwrap();

        // Manually compute what the hash chain should be
        let record0 = Record::new(Offset::new(0), None, Bytes::from("event-0"));
        let hash0 = record0.compute_hash();

        let record1 = Record::new(Offset::new(1), Some(hash0), Bytes::from("event-1"));
        let hash1 = record1.compute_hash();

        let record2 = Record::new(Offset::new(2), Some(hash1), Bytes::from("event-2"));
        let hash2 = record2.compute_hash();

        // The final hash from append_batch should match our manual computation
        assert_eq!(final_hash, hash2);
    }

    #[test]
    fn tampered_record_is_detected() {
        let (mut storage, dir) = setup_storage();
        let stream_id = StreamId::new(1);

        // Write some valid records
        storage
            .append_batch(stream_id, test_events(3), Offset::new(0), None, false)
            .unwrap();

        // Tamper with the file: corrupt the prev_hash of record 1
        let segment_path = dir
            .path()
            .join(stream_id.to_string())
            .join("segment_000000.log");

        let mut data = std::fs::read(&segment_path).unwrap();

        // Record 0 is at offset 0, its size is 8 + 32 + 1 + 4 + 7 + 4 = 56 bytes
        // Record 1 starts at byte 56, its prev_hash is at bytes 56+8 = 64
        // Flip a bit in the prev_hash of record 1
        data[64] ^= 0xFF;

        // Also need to fix the CRC or we'll get CorruptedRecord instead
        // Actually, let's just verify that ANY corruption is caught
        std::fs::write(&segment_path, &data).unwrap();

        // Reading should fail with either ChainVerificationFailed or CorruptedRecord
        let result = storage.read_from(stream_id, Offset::new(0), u64::MAX);
        assert!(result.is_err());
    }

    #[test]
    fn read_records_returns_full_records() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        storage
            .append_batch(stream_id, test_events(3), Offset::new(0), None, false)
            .unwrap();

        let records = storage
            .read_records_from(stream_id, Offset::new(0), u64::MAX)
            .unwrap();

        assert_eq!(records.len(), 3);

        // First record should have no prev_hash
        assert_eq!(records[0].prev_hash(), None);
        assert_eq!(records[0].offset(), Offset::new(0));

        // Second record should link to first
        assert_eq!(records[1].prev_hash(), Some(records[0].compute_hash()));
        assert_eq!(records[1].offset(), Offset::new(1));

        // Third record should link to second
        assert_eq!(records[2].prev_hash(), Some(records[1].compute_hash()));
        assert_eq!(records[2].offset(), Offset::new(2));
    }
}

// ============================================================================
// Checkpoint Tests
// ============================================================================

mod checkpoint_tests {
    use super::*;
    use tempfile::TempDir;
    use craton_types::CheckpointPolicy;

    fn setup_storage() -> (Storage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = Storage::new(temp_dir.path());
        (storage, temp_dir)
    }

    fn test_events(count: usize) -> Vec<Bytes> {
        (0..count)
            .map(|i| Bytes::from(format!("event-{i}")))
            .collect()
    }

    #[test]
    fn create_and_read_checkpoint() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        // Append some data records first
        let (next_offset, last_hash) = storage
            .append_batch(stream_id, test_events(5), Offset::new(0), None, false)
            .unwrap();

        assert_eq!(next_offset, Offset::new(5));

        // Create a checkpoint
        let (cp_next_offset, _cp_hash) = storage
            .create_checkpoint(stream_id, next_offset, Some(last_hash), 5, false)
            .unwrap();

        assert_eq!(cp_next_offset, Offset::new(6));

        // Verify checkpoint was recorded
        let last_cp = storage.last_checkpoint(stream_id).unwrap();
        assert_eq!(last_cp, Some(Offset::new(5)));
    }

    #[test]
    fn read_with_checkpoint_verification() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        // Append 10 events
        let (offset1, hash1) = storage
            .append_batch(stream_id, test_events(5), Offset::new(0), None, false)
            .unwrap();

        // Create checkpoint at offset 5
        let (offset2, hash2) = storage
            .create_checkpoint(stream_id, offset1, Some(hash1), 5, false)
            .unwrap();

        // Append 5 more events
        storage
            .append_batch(stream_id, test_events(5), offset2, Some(hash2), false)
            .unwrap();

        // Read using checkpoint-optimized verification
        let records = storage
            .read_records_verified(stream_id, Offset::new(7), u64::MAX)
            .unwrap();

        // Should get events 7, 8, 9, 10 (checkpoint at 5 is skipped, events 6-10 after checkpoint)
        assert_eq!(records.len(), 4);
        assert_eq!(records[0].offset(), Offset::new(7));
    }

    #[test]
    fn checkpoint_policy_triggers() {
        let temp_dir = TempDir::new().unwrap();
        let policy = CheckpointPolicy::every(3);
        let storage = Storage::with_checkpoint_policy(temp_dir.path(), policy);

        // Verify policy is set
        assert_eq!(storage.checkpoint_policy().every_n_records, 3);
        assert!(
            storage
                .checkpoint_policy()
                .should_checkpoint(Offset::new(2))
        ); // 3rd record
        assert!(
            !storage
                .checkpoint_policy()
                .should_checkpoint(Offset::new(3))
        ); // 4th record
    }

    #[test]
    fn empty_stream_has_no_checkpoints() {
        let (mut storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        let last_cp = storage.last_checkpoint(stream_id).unwrap();
        assert_eq!(last_cp, None);
    }
}

// ============================================================================
// Property-Based Tests
// ============================================================================

mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn record_roundtrip_any_payload(payload in prop::collection::vec(any::<u8>(), 0..1000)) {
            let record = Record::new(Offset::new(42), None, Bytes::from(payload.clone()));
            let bytes: Bytes = record.to_bytes().into();
            let (parsed, consumed) = Record::from_bytes(&bytes).unwrap();

            prop_assert_eq!(parsed.offset(), Offset::new(42));
            prop_assert_eq!(parsed.prev_hash(), None);
            prop_assert_eq!(parsed.payload().as_ref(), payload.as_slice());
            prop_assert_eq!(consumed, bytes.len());
        }

        #[test]
        fn record_roundtrip_any_offset(offset in 0u64..u64::MAX) {
            let record = Record::new(Offset::new(offset), None, Bytes::from("test"));
            let bytes: Bytes = record.to_bytes().into();
            let (parsed, _) = Record::from_bytes(&bytes).unwrap();

            prop_assert_eq!(parsed.offset().as_u64(), offset);
        }

        #[test]
        fn record_roundtrip_with_any_prev_hash(hash_bytes in prop::collection::vec(any::<u8>(), 32..=32)) {
            let hash_arr: [u8; 32] = hash_bytes.try_into().unwrap();
            let prev_hash = ChainHash::from_bytes(&hash_arr);
            let record = Record::new(Offset::new(1), Some(prev_hash), Bytes::from("data"));
            let bytes: Bytes = record.to_bytes().into();
            let (parsed, _) = Record::from_bytes(&bytes).unwrap();

            prop_assert_eq!(parsed.prev_hash(), Some(prev_hash));
        }

        #[test]
        fn corruption_is_detected(
            payload in prop::collection::vec(any::<u8>(), 1..100),
            flip_pos in 0usize..1000
        ) {
            let record = Record::new(Offset::new(0), None, Bytes::from(payload));
            let mut bytes = record.to_bytes();

            // Flip a bit at a valid position (excluding CRC itself)
            let max_pos = bytes.len().saturating_sub(4); // Exclude CRC bytes
            if max_pos > 0 {
                let actual_pos = flip_pos % max_pos;
                bytes[actual_pos] ^= 1;

                let result = Record::from_bytes(&Bytes::from(bytes));
                // Any error is acceptable - could be CorruptedRecord (CRC mismatch)
                // or UnexpectedEof (if length field was corrupted to claim more bytes)
                prop_assert!(result.is_err());
            }
        }
    }
}
