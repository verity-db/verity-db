//! Checkpoint support for verified reads.
//!
//! Checkpoints are periodic verification anchors stored IN the log as
//! [`RecordKind::Checkpoint`] records. They enable efficient verified reads
//! by providing trusted anchor points, reducing verification from O(n) to O(k)
//! where k is the distance to the nearest checkpoint.
//!
//! # Checkpoint Payload Format
//!
//! ```text
//! [chain_hash:32B][record_count:u64]
//!       32B             8B
//! ```

use bytes::Bytes;
use craton_crypto::ChainHash;
use craton_types::{Checkpoint, CheckpointPolicy, Offset, Timestamp};

use crate::StorageError;

/// Size of the checkpoint payload in bytes.
pub const CHECKPOINT_PAYLOAD_SIZE: usize = 40; // 32 (hash) + 8 (count)

/// Serializes checkpoint data to bytes for storage as a record payload.
///
/// Format: `[chain_hash:32B][record_count:u64]`
pub fn serialize_checkpoint_payload(chain_hash: &ChainHash, record_count: u64) -> Bytes {
    let mut buf = Vec::with_capacity(CHECKPOINT_PAYLOAD_SIZE);
    buf.extend_from_slice(chain_hash.as_bytes());
    buf.extend_from_slice(&record_count.to_le_bytes());

    debug_assert_eq!(buf.len(), CHECKPOINT_PAYLOAD_SIZE);
    Bytes::from(buf)
}

/// Deserializes checkpoint data from a record payload.
///
/// # Errors
///
/// Returns an error if the payload is too small.
pub fn deserialize_checkpoint_payload(
    payload: &Bytes,
    offset: Offset,
) -> Result<(ChainHash, u64), StorageError> {
    if payload.len() < CHECKPOINT_PAYLOAD_SIZE {
        return Err(StorageError::InvalidCheckpointPayload {
            offset,
            reason: format!(
                "payload too small: {} < {}",
                payload.len(),
                CHECKPOINT_PAYLOAD_SIZE
            ),
        });
    }

    let chain_hash_bytes: [u8; 32] = payload[0..32]
        .try_into()
        .expect("slice is exactly 32 bytes after bounds check");
    let chain_hash = ChainHash::from_bytes(&chain_hash_bytes);

    let record_count = u64::from_le_bytes(
        payload[32..40]
            .try_into()
            .expect("slice is exactly 8 bytes after bounds check"),
    );

    Ok((chain_hash, record_count))
}

/// Creates a [`Checkpoint`] from deserialized checkpoint data.
pub fn create_checkpoint(
    offset: Offset,
    chain_hash: ChainHash,
    record_count: u64,
    timestamp: Timestamp,
) -> Checkpoint {
    // Convert ChainHash to craton_types::Hash
    let hash = craton_types::Hash::from_bytes(*chain_hash.as_bytes());
    Checkpoint::new(offset, hash, record_count, timestamp)
}

/// In-memory sparse index of checkpoint offsets.
///
/// This index is rebuilt on startup by scanning for [`RecordKind::Checkpoint`]
/// records. It provides O(log n) lookup for the nearest checkpoint.
///
/// # Invariants
///
/// - Offsets are stored in ascending order
/// - Each offset corresponds to a valid checkpoint record in the log
#[derive(Debug, Clone, Default)]
pub struct CheckpointIndex {
    /// Checkpoint offsets in ascending order.
    offsets: Vec<Offset>,
}

impl CheckpointIndex {
    /// Creates an empty checkpoint index.
    pub fn new() -> Self {
        Self {
            offsets: Vec::new(),
        }
    }

    /// Adds a checkpoint offset to the index.
    ///
    /// # Panics
    ///
    /// Panics if the offset is not greater than the last offset (checkpoints
    /// must be added in order).
    pub fn add(&mut self, offset: Offset) {
        if let Some(&last) = self.offsets.last() {
            assert!(
                offset > last,
                "checkpoint offset must be greater than previous: {offset} <= {last}"
            );
        }
        self.offsets.push(offset);
    }

    /// Finds the nearest checkpoint at or before the given offset.
    ///
    /// Returns `None` if there are no checkpoints before the offset.
    pub fn find_nearest(&self, offset: Offset) -> Option<Offset> {
        // Binary search for the largest offset <= target
        match self.offsets.binary_search(&offset) {
            Ok(idx) => Some(self.offsets[idx]),
            Err(idx) => {
                if idx == 0 {
                    None
                } else {
                    Some(self.offsets[idx - 1])
                }
            }
        }
    }

    /// Returns the number of checkpoints in the index.
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    /// Returns true if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Returns the last checkpoint offset, if any.
    pub fn last(&self) -> Option<Offset> {
        self.offsets.last().copied()
    }

    /// Returns an iterator over checkpoint offsets.
    pub fn iter(&self) -> impl Iterator<Item = &Offset> {
        self.offsets.iter()
    }
}

/// Determines if a checkpoint should be created at the given offset.
///
/// This is a helper that applies the checkpoint policy.
pub fn should_create_checkpoint(policy: &CheckpointPolicy, offset: Offset) -> bool {
    policy.should_checkpoint(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_payload_roundtrip() {
        let hash = ChainHash::from_bytes(&[0xab; 32]);
        let count = 12345u64;

        let payload = serialize_checkpoint_payload(&hash, count);
        let (recovered_hash, recovered_count) =
            deserialize_checkpoint_payload(&payload, Offset::new(0)).unwrap();

        assert_eq!(recovered_hash, hash);
        assert_eq!(recovered_count, count);
    }

    #[test]
    fn checkpoint_index_find_nearest() {
        let mut index = CheckpointIndex::new();
        index.add(Offset::new(99));
        index.add(Offset::new(199));
        index.add(Offset::new(299));

        // Exact match
        assert_eq!(index.find_nearest(Offset::new(199)), Some(Offset::new(199)));

        // Between checkpoints
        assert_eq!(index.find_nearest(Offset::new(150)), Some(Offset::new(99)));
        assert_eq!(index.find_nearest(Offset::new(250)), Some(Offset::new(199)));

        // Before first checkpoint
        assert_eq!(index.find_nearest(Offset::new(50)), None);

        // After last checkpoint
        assert_eq!(index.find_nearest(Offset::new(500)), Some(Offset::new(299)));
    }

    #[test]
    fn checkpoint_index_empty() {
        let index = CheckpointIndex::new();
        assert!(index.is_empty());
        assert_eq!(index.find_nearest(Offset::new(100)), None);
    }
}
