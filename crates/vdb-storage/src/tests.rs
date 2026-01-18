//! Unit tests for vdb-storage
//!
//! Tests for the append-only segment storage layer.

use bytes::Bytes;
use vdb_types::{Offset, StreamId};

use crate::{Record, Storage, StorageError};

// ============================================================================
// Record Serialization Tests
// ============================================================================

#[test]
fn record_to_bytes_produces_correct_format() {
    let record = Record::new(Offset::new(42), Bytes::from("hello"));
    let bytes = record.to_bytes();

    // Total size: 8 (offset) + 4 (len) + 5 (payload) + 4 (crc) = 21 bytes
    assert_eq!(bytes.len(), 21);

    // First 8 bytes: offset (42 in little-endian)
    let offset = i64::from_le_bytes(bytes[0..8].try_into().unwrap());
    assert_eq!(offset, 42);

    // Next 4 bytes: length (5 in little-endian)
    let length = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    assert_eq!(length, 5);

    // Next 5 bytes: payload
    assert_eq!(&bytes[12..17], b"hello");

    // Last 4 bytes: CRC (verify it's non-zero and matches expected)
    let stored_crc = u32::from_le_bytes(bytes[17..21].try_into().unwrap());
    let computed_crc = crc32fast::hash(&bytes[0..17]);
    assert_eq!(stored_crc, computed_crc);
}

#[test]
fn record_roundtrip_preserves_data() {
    let original = Record::new(Offset::new(123), Bytes::from("test payload"));
    let bytes: Bytes = original.to_bytes().into();

    let (parsed, consumed) = Record::from_bytes(&bytes).unwrap();

    assert_eq!(parsed.offset(), Offset::new(123));
    assert_eq!(parsed.payload().as_ref(), b"test payload");
    assert_eq!(consumed, bytes.len());
}

#[test]
fn record_from_bytes_detects_corruption() {
    let record = Record::new(Offset::new(0), Bytes::from("data"));
    let mut bytes: Vec<u8> = record.to_bytes();

    // Corrupt one byte in the payload
    bytes[12] ^= 0xFF;

    let result = Record::from_bytes(&Bytes::from(bytes));
    assert!(matches!(result, Err(StorageError::CorruptedRecord)));
}

#[test]
fn record_from_bytes_handles_truncated_header() {
    // Less than 12 bytes (minimum header size)
    let short_data = Bytes::from(vec![0u8; 10]);
    let result = Record::from_bytes(&short_data);
    assert!(matches!(result, Err(StorageError::UnexpectedEof)));
}

#[test]
fn record_from_bytes_handles_truncated_payload() {
    // Create a header claiming 100 bytes of payload
    let mut data = Vec::new();
    data.extend_from_slice(&0i64.to_le_bytes()); // offset
    data.extend_from_slice(&100u32.to_le_bytes()); // length: 100 bytes
    data.extend_from_slice(&[0u8; 50]); // only 50 bytes of payload

    let result = Record::from_bytes(&Bytes::from(data));
    assert!(matches!(result, Err(StorageError::UnexpectedEof)));
}

#[test]
fn record_empty_payload() {
    let record = Record::new(Offset::new(0), Bytes::new());
    let bytes: Bytes = record.to_bytes().into();

    let (parsed, _) = Record::from_bytes(&bytes).unwrap();
    assert!(parsed.payload().is_empty());
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

        let new_offset = storage
            .append_batch(stream_id, test_events(1), Offset::new(0), false)
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
            .append_batch(stream_id, test_events(5), Offset::new(0), false)
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
            .append_batch(stream_id, test_events(10), Offset::new(0), false)
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
            .append_batch(stream_id, events, Offset::new(0), false)
            .unwrap();

        // Read with max_bytes that should limit results
        // Each event is ~10 bytes, so max_bytes=25 should give us 2-3 events
        let events = storage
            .read_from(stream_id, Offset::new(0), 25)
            .unwrap();

        // Should get fewer than all 10 events
        assert!(events.len() < 10);
        assert!(!events.is_empty());
    }

    #[test]
    fn append_multiple_batches_sequential() {
        let (storage, _dir) = setup_storage();
        let stream_id = StreamId::new(1);

        // Append batch 1 (3 events)
        let offset_after_batch1 = storage
            .append_batch(stream_id, test_events(3), Offset::new(0), false)
            .unwrap();
        assert_eq!(offset_after_batch1, Offset::new(3));

        // Append batch 2 (2 events) starting at offset 3
        let events2: Vec<Bytes> = vec![Bytes::from("batch2-0"), Bytes::from("batch2-1")];
        let offset_after_batch2 = storage
            .append_batch(stream_id, events2, Offset::new(3), false)
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
        let result = storage.append_batch(stream_id, test_events(1), Offset::new(0), true);

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
            .append_batch(stream1, vec![Bytes::from("stream1-event")], Offset::new(0), false)
            .unwrap();

        // Append to stream 2
        storage
            .append_batch(stream2, vec![Bytes::from("stream2-event")], Offset::new(0), false)
            .unwrap();

        // Read from each stream
        let events1 = storage.read_from(stream1, Offset::new(0), u64::MAX).unwrap();
        let events2 = storage.read_from(stream2, Offset::new(0), u64::MAX).unwrap();

        assert_eq!(events1.len(), 1);
        assert_eq!(events1[0].as_ref(), b"stream1-event");

        assert_eq!(events2.len(), 1);
        assert_eq!(events2[0].as_ref(), b"stream2-event");
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
            let record = Record::new(Offset::new(42), Bytes::from(payload.clone()));
            let bytes: Bytes = record.to_bytes().into();
            let (parsed, consumed) = Record::from_bytes(&bytes).unwrap();

            prop_assert_eq!(parsed.offset(), Offset::new(42));
            prop_assert_eq!(parsed.payload().as_ref(), payload.as_slice());
            prop_assert_eq!(consumed, bytes.len());
        }

        #[test]
        fn record_roundtrip_any_offset(offset in 0i64..i64::MAX) {
            let record = Record::new(Offset::new(offset), Bytes::from("test"));
            let bytes: Bytes = record.to_bytes().into();
            let (parsed, _) = Record::from_bytes(&bytes).unwrap();

            prop_assert_eq!(parsed.offset().as_i64(), offset);
        }

        #[test]
        fn corruption_is_detected(
            payload in prop::collection::vec(any::<u8>(), 1..100),
            flip_pos in 0usize..1000
        ) {
            let record = Record::new(Offset::new(0), Bytes::from(payload));
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
