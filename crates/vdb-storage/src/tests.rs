//! Unit tests for vdb-storage
//!
//! Tests for the append-only segment storage layer with hash chain integrity.

use bytes::Bytes;
use vdb_crypto::ChainHash;
use vdb_types::{Offset, StreamId};

use crate::{Record, Storage, StorageError};

// ============================================================================
// Record Serialization Tests
// ============================================================================

#[test]
fn record_to_bytes_produces_correct_format() {
    let record = Record::new(Offset::new(42), None, Bytes::from("hello"));
    let bytes = record.to_bytes();

    // Total size: 8 (offset) + 32 (prev_hash) + 4 (len) + 5 (payload) + 4 (crc) = 53 bytes
    assert_eq!(bytes.len(), 53);

    // First 8 bytes: offset (42 in little-endian)
    let offset = i64::from_le_bytes(bytes[0..8].try_into().unwrap());
    assert_eq!(offset, 42);

    // Next 32 bytes: prev_hash (all zeros for genesis)
    assert_eq!(&bytes[8..40], &[0u8; 32]);

    // Next 4 bytes: length (5 in little-endian)
    let length = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
    assert_eq!(length, 5);

    // Next 5 bytes: payload
    assert_eq!(&bytes[44..49], b"hello");

    // Last 4 bytes: CRC (verify it matches expected)
    let stored_crc = u32::from_le_bytes(bytes[49..53].try_into().unwrap());
    let computed_crc = crc32fast::hash(&bytes[0..49]);
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

    // Corrupt one byte in the payload (at offset 44)
    bytes[44] ^= 0xFF;

    let result = Record::from_bytes(&Bytes::from(bytes));
    assert!(matches!(result, Err(StorageError::CorruptedRecord)));
}

#[test]
fn record_from_bytes_handles_truncated_header() {
    // Less than 44 bytes (minimum header size: 8 + 32 + 4)
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
        let (storage, _dir) = setup_storage();
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
        let (storage, _dir) = setup_storage();
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
        let (storage, _dir) = setup_storage();
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
        let (storage, _dir) = setup_storage();
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
        let (storage, _dir) = setup_storage();
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
        let (storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        // Append with fsync=true
        let result = storage.append_batch(stream_id, test_events(1), Offset::new(0), None, true);

        // Should succeed (fsync is just durability, shouldn't change behavior)
        assert!(result.is_ok());
    }

    #[test]
    fn multiple_streams_are_isolated() {
        let (storage, _dir) = setup_storage();
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
        let (storage, _dir) = setup_storage();
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
        let (storage, dir) = setup_storage();
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

        // Record 0 is at offset 0, its size is 8 + 32 + 4 + 7 + 4 = 55 bytes
        // Record 1 starts at byte 55, its prev_hash is at bytes 55+8 = 63
        // Flip a bit in the prev_hash of record 1
        data[63] ^= 0xFF;

        // Also need to fix the CRC or we'll get CorruptedRecord instead
        // Actually, let's just verify that ANY corruption is caught
        std::fs::write(&segment_path, &data).unwrap();

        // Reading should fail with either ChainVerificationFailed or CorruptedRecord
        let result = storage.read_from(stream_id, Offset::new(0), u64::MAX);
        assert!(result.is_err());
    }

    #[test]
    fn read_records_returns_full_records() {
        let (storage, _dir) = setup_storage();
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
        fn record_roundtrip_any_offset(offset in 0i64..i64::MAX) {
            let record = Record::new(Offset::new(offset), None, Bytes::from("test"));
            let bytes: Bytes = record.to_bytes().into();
            let (parsed, _) = Record::from_bytes(&bytes).unwrap();

            prop_assert_eq!(parsed.offset().as_i64(), offset);
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
