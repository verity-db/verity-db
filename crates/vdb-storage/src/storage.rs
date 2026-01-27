//! Append-only event log storage with checkpoint support.
//!
//! The [`Storage`] struct manages segment files on disk, providing append and read
//! operations for event streams. Each stream gets its own directory with
//! numbered segment files.
//!
//! # File Layout
//!
//! ```text
//! {data_dir}/
//! └── {stream_id}/
//!     ├── segment_000000.log      <- append-only event log
//!     └── segment_000000.log.idx  <- offset index for O(1) lookups
//! ```
//!
//! # Hash Chain Integrity
//!
//! Every record contains a cryptographic link (`prev_hash`) to the previous record,
//! forming a tamper-evident chain. Reads verify this chain from genesis (or a
//! checkpoint) to detect any corruption or tampering.
//!
//! # Checkpoints
//!
//! Checkpoints are periodic verification anchors stored as special records in the
//! log. They enable efficient verified reads by reducing verification from O(n)
//! to O(k) where k is the distance to the nearest checkpoint.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use bytes::Bytes;
use vdb_crypto::ChainHash;
use vdb_types::{CheckpointPolicy, Offset, RecordKind, StreamId};

use crate::checkpoint::{
    deserialize_checkpoint_payload, serialize_checkpoint_payload, CheckpointIndex,
};
use crate::{OffsetIndex, Record, StorageError};

/// Current segment filename. Future: segment rotation will make this dynamic.
const SEGMENT_FILENAME: &str = "segment_000000.log";

/// Append-only event log storage with checkpoint support.
///
/// Manages segment files on disk, providing append and read operations for
/// event streams. Each stream gets its own directory with numbered segment files.
///
/// # Invariants
///
/// - Records are append-only; existing data is never modified
/// - Each record links to the previous via `prev_hash` (hash chain)
/// - The offset index stays in sync with the log (updated atomically with appends)
/// - Checkpoints are created according to the configured policy
#[derive(Debug, Clone)]
pub struct Storage {
    /// Root directory for all stream data.
    data_dir: PathBuf,

    /// In-memory cache of offset indexes, keyed by stream.
    /// Loaded lazily on first access, kept in sync during appends.
    index_cache: HashMap<StreamId, OffsetIndex>,

    /// In-memory cache of checkpoint indexes, keyed by stream.
    /// Rebuilt on first access by scanning for checkpoint records.
    checkpoint_cache: HashMap<StreamId, CheckpointIndex>,

    /// Policy for automatic checkpoint creation.
    checkpoint_policy: CheckpointPolicy,
}

impl Storage {
    /// Creates a new storage instance with the given data directory.
    ///
    /// The directory will be created if it doesn't exist when the first
    /// write occurs. Uses the default checkpoint policy.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self::with_checkpoint_policy(data_dir, CheckpointPolicy::default())
    }

    /// Creates a new storage instance with a custom checkpoint policy.
    pub fn with_checkpoint_policy(
        data_dir: impl Into<PathBuf>,
        checkpoint_policy: CheckpointPolicy,
    ) -> Self {
        Self {
            data_dir: data_dir.into(),
            index_cache: HashMap::new(),
            checkpoint_cache: HashMap::new(),
            checkpoint_policy,
        }
    }

    /// Returns the current checkpoint policy.
    pub fn checkpoint_policy(&self) -> &CheckpointPolicy {
        &self.checkpoint_policy
    }

    /// Returns the data directory path.
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    fn segment_path(&self, stream_id: StreamId) -> PathBuf {
        self.data_dir
            .join(stream_id.to_string())
            .join(SEGMENT_FILENAME)
    }

    /// Returns the path to the index file for a stream.
    pub fn index_path(&self, stream_id: StreamId) -> PathBuf {
        let mut path = self.segment_path(stream_id);
        path.set_extension("log.idx");
        path
    }

    /// Rebuilds the offset index by scanning the log file.
    ///
    /// This is the recovery path when the index file is missing or corrupted.
    /// Scans every record in the log to reconstruct byte positions.
    ///
    /// # Performance
    ///
    /// O(n) where n is the number of records. For large logs, this can be slow.
    /// The rebuilt index is saved to disk for future use.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::CorruptedRecord`] if any record in the log is invalid.
    pub fn rebuild_index(&self, stream_id: StreamId) -> Result<OffsetIndex, StorageError> {
        let segment_path = self.segment_path(stream_id);

        // Empty stream has empty index
        if !segment_path.exists() {
            return Ok(OffsetIndex::new());
        }

        let data: Bytes = fs::read(&segment_path)?.into();
        let mut index = OffsetIndex::new();
        let mut pos = 0;

        while pos < data.len() {
            // Record byte position BEFORE parsing (this is where the record starts)
            index.append(pos as u64);
            let (_, consumed) = Record::from_bytes(&data.slice(pos..))?;
            pos += consumed;
        }

        // Postcondition: index has one entry per record
        debug_assert_eq!(
            index.len(),
            {
                // Count records by re-scanning (debug only)
                let mut count = 0;
                let mut p = 0;
                while p < data.len() {
                    let (_, c) = Record::from_bytes(&data.slice(p..)).unwrap();
                    p += c;
                    count += 1;
                }
                count
            },
            "index entry count mismatch"
        );

        index.save(&self.index_path(stream_id))?;

        Ok(index)
    }

    /// Loads the offset index from disk, or rebuilds it if missing/corrupted.
    ///
    /// This is the primary way to obtain an index. It attempts to load the
    /// persisted index first (fast path), falling back to a full log scan
    /// if the index is unavailable or invalid.
    ///
    /// # Errors
    ///
    /// Returns an error only if rebuilding fails (e.g., corrupted log).
    /// Index loading failures trigger a rebuild rather than returning an error.
    pub fn load_or_rebuild_index(&self, stream_id: StreamId) -> Result<OffsetIndex, StorageError> {
        let index_path = self.index_path(stream_id);

        if let Ok(index) = OffsetIndex::load(&index_path) {
            return Ok(index);
        }

        // Index missing or corrupted; rebuild from log
        tracing::warn!(
            stream_id = %stream_id,
            "index missing or corrupted, rebuilding from log"
        );
        self.rebuild_index(stream_id)
    }

    /// Appends a batch of events to a stream, building the hash chain.
    ///
    /// Each event is written as a [`Record`] with a cryptographic link to the
    /// previous record, forming a tamper-evident chain. The offset index is
    /// updated atomically with the append to maintain O(1) lookup capability.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to append to
    /// * `events` - The event payloads to append (must not be empty)
    /// * `expected_offset` - The offset to start writing at
    /// * `prev_hash` - Hash of the previous record (`None` for genesis)
    /// * `fsync` - Whether to fsync after writing (recommended for durability)
    ///
    /// # Returns
    ///
    /// A tuple of:
    /// - The next offset (for subsequent appends)
    /// - The hash of the last record written (for chain continuity)
    ///
    /// # Panics
    ///
    /// Panics if `events` is empty. Empty batches are a caller bug.
    ///
    /// # Note
    ///
    /// This method does not validate that `expected_offset` matches the
    /// current end of the log. That validation is done at the kernel level.
    pub fn append_batch(
        &mut self,
        stream_id: StreamId,
        events: Vec<Bytes>,
        expected_offset: Offset,
        prev_hash: Option<ChainHash>,
        fsync: bool,
    ) -> Result<(Offset, ChainHash), StorageError> {
        // Precondition: batch must not be empty
        assert!(!events.is_empty(), "cannot append empty batch");

        let event_count = events.len();

        // Ensure stream directory exists
        let stream_dir = self.data_dir.join(stream_id.to_string());
        fs::create_dir_all(&stream_dir)?;

        // Open segment file for appending
        let segment_path = self.segment_path(stream_id);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&segment_path)?;

        // Get current file size as starting byte position for new records
        let mut byte_position: u64 = file.metadata()?.len();

        // Pre-compute paths before taking mutable borrow of index_cache
        let index_path = self.index_path(stream_id);

        // Load index into cache if not present
        if !self.index_cache.contains_key(&stream_id) {
            let loaded = self.load_or_rebuild_index(stream_id)?;
            self.index_cache.insert(stream_id, loaded);
        }

        let index = self
            .index_cache
            .get_mut(&stream_id)
            .expect("index exists: just inserted or already present");

        let mut current_offset = expected_offset;
        let mut current_hash = prev_hash;

        for event in events {
            // Record byte position BEFORE writing (where this record starts)
            index.append(byte_position);

            let record = Record::new(current_offset, current_hash, event);
            let record_bytes = record.to_bytes();

            // Update position AFTER computing record size
            byte_position += record_bytes.len() as u64;

            file.write_all(&record_bytes)?;

            current_hash = Some(record.compute_hash());
            current_offset += Offset::from(1u64);
        }

        // Ensure durability if requested
        if fsync {
            file.sync_all()?;
        }

        // Persist index to disk (keeps index in sync with log)
        index.save(&index_path)?;

        // Postcondition: we wrote exactly event_count records
        debug_assert_eq!(
            current_offset.as_u64() - expected_offset.as_u64(),
            event_count as u64,
            "offset mismatch after batch write"
        );

        // Safe: we asserted events is non-empty, so at least one iteration ran
        Ok((current_offset, current_hash.expect("batch was non-empty")))
    }

    /// Reads events from a stream with full hash chain verification.
    ///
    /// This is a convenience wrapper around [`Self::read_records_from`] that
    /// returns just the payloads. The hash chain is verified from genesis
    /// before returning any data.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to read from
    /// * `from_offset` - The first offset to include (inclusive)
    /// * `max_bytes` - Maximum total payload bytes to return
    ///
    /// # Returns
    ///
    /// A vector of event payloads.
    ///
    /// # Errors
    ///
    /// - [`StorageError::ChainVerificationFailed`] if the hash chain is broken
    /// - [`StorageError::CorruptedRecord`] if any record fails CRC check
    pub fn read_from(
        &self,
        stream_id: StreamId,
        from_offset: Offset,
        max_bytes: u64,
    ) -> Result<Vec<Bytes>, StorageError> {
        let records = self.read_records_from(stream_id, from_offset, max_bytes)?;
        Ok(records.into_iter().map(|r| r.payload().clone()).collect())
    }

    /// Reads records from a stream with full hash chain verification.
    ///
    /// Verifies the hash chain from genesis up to and including the requested
    /// records. This ensures tamper detection: if any record has been modified,
    /// the chain will be broken and an error is returned.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to read from
    /// * `from_offset` - The first offset to include in results (inclusive)
    /// * `max_bytes` - Maximum total payload bytes to return
    ///
    /// # Returns
    ///
    /// A vector of [`Record`]s with verified hash chain integrity.
    ///
    /// # Errors
    ///
    /// - [`StorageError::ChainVerificationFailed`] if the hash chain is broken
    /// - [`StorageError::CorruptedRecord`] if any record fails CRC check
    ///
    /// # Note
    ///
    /// Even when reading from a non-zero offset, verification starts from
    /// genesis (offset 0) to ensure full chain integrity. Future checkpoint
    /// support will allow verification from a known-good snapshot instead.
    pub fn read_records_from(
        &self,
        stream_id: StreamId,
        from_offset: Offset,
        max_bytes: u64,
    ) -> Result<Vec<Record>, StorageError> {
        let segment_path = self.segment_path(stream_id);

        let data: Bytes = fs::read(&segment_path)?.into();

        let mut results = Vec::new();
        let mut bytes_read: u64 = 0;
        let mut pos = 0;
        let mut expected_prev_hash: Option<ChainHash> = None;
        let mut records_verified: u64 = 0;

        while pos < data.len() && bytes_read < max_bytes {
            let (record, consumed) = Record::from_bytes(&data.slice(pos..))?;

            // Verify hash chain integrity
            if record.prev_hash() != expected_prev_hash {
                return Err(StorageError::ChainVerificationFailed {
                    offset: record.offset(),
                    expected: expected_prev_hash,
                    actual: record.prev_hash(),
                });
            }

            // Update expected hash for next record
            expected_prev_hash = Some(record.compute_hash());
            records_verified += 1;
            pos += consumed;

            // Only collect records at or after the requested offset
            if record.offset() < from_offset {
                continue;
            }

            bytes_read += record.payload().len() as u64;
            results.push(record);
        }

        // Postcondition: we verified all records we read
        debug_assert!(
            records_verified == 0 || expected_prev_hash.is_some(),
            "verified records but no final hash"
        );

        Ok(results)
    }

    // ========================================================================
    // Checkpoint Support
    // ========================================================================

    /// Rebuilds the checkpoint index by scanning the log for checkpoint records.
    ///
    /// This is called on startup or when the checkpoint index is not cached.
    /// Unlike the offset index, checkpoint indexes are not persisted (they're
    /// small and fast to rebuild).
    ///
    /// # Performance
    ///
    /// O(n) where n is the number of records. However, this only happens once
    /// per stream per Storage instance.
    pub fn rebuild_checkpoint_index(
        &self,
        stream_id: StreamId,
    ) -> Result<CheckpointIndex, StorageError> {
        let segment_path = self.segment_path(stream_id);

        if !segment_path.exists() {
            return Ok(CheckpointIndex::new());
        }

        let data: Bytes = fs::read(&segment_path)?.into();
        let mut checkpoint_index = CheckpointIndex::new();
        let mut pos = 0;

        while pos < data.len() {
            let (record, consumed) = Record::from_bytes(&data.slice(pos..))?;

            if record.is_checkpoint() {
                checkpoint_index.add(record.offset());
            }

            pos += consumed;
        }

        tracing::debug!(
            stream_id = %stream_id,
            checkpoint_count = checkpoint_index.len(),
            "rebuilt checkpoint index"
        );

        Ok(checkpoint_index)
    }

    /// Gets the checkpoint index for a stream, rebuilding if necessary.
    fn get_or_rebuild_checkpoint_index(
        &mut self,
        stream_id: StreamId,
    ) -> Result<&CheckpointIndex, StorageError> {
        if !self.checkpoint_cache.contains_key(&stream_id) {
            let index = self.rebuild_checkpoint_index(stream_id)?;
            self.checkpoint_cache.insert(stream_id, index);
        }
        Ok(self.checkpoint_cache.get(&stream_id).expect("just inserted"))
    }

    /// Creates a checkpoint at the current position.
    ///
    /// A checkpoint is a special record that contains the cumulative chain hash
    /// and record count. It serves as a verified anchor point for future reads.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to checkpoint
    /// * `current_offset` - The offset for this checkpoint record
    /// * `prev_hash` - The hash of the previous record
    /// * `record_count` - Total records including this checkpoint
    /// * `fsync` - Whether to fsync after writing
    ///
    /// # Returns
    ///
    /// A tuple of the next offset and the checkpoint record's hash.
    pub fn create_checkpoint(
        &mut self,
        stream_id: StreamId,
        current_offset: Offset,
        prev_hash: Option<ChainHash>,
        record_count: u64,
        fsync: bool,
    ) -> Result<(Offset, ChainHash), StorageError> {
        // Get the chain hash at this point (which is prev_hash of the checkpoint)
        let chain_hash = prev_hash.unwrap_or_else(|| ChainHash::from_bytes(&[0u8; 32]));

        // Serialize checkpoint payload
        let payload = serialize_checkpoint_payload(&chain_hash, record_count);

        // Create checkpoint record
        let record = Record::with_kind(current_offset, prev_hash, RecordKind::Checkpoint, payload);
        let record_bytes = record.to_bytes();
        let record_hash = record.compute_hash();

        // Ensure stream directory exists
        let stream_dir = self.data_dir.join(stream_id.to_string());
        fs::create_dir_all(&stream_dir)?;

        // Open segment file for appending
        let segment_path = self.segment_path(stream_id);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&segment_path)?;

        // Get current file size as byte position
        let byte_position = file.metadata()?.len();

        // Write checkpoint record
        file.write_all(&record_bytes)?;

        if fsync {
            file.sync_all()?;
        }

        // Update offset index
        let index_path = self.index_path(stream_id);
        if !self.index_cache.contains_key(&stream_id) {
            let loaded = self.load_or_rebuild_index(stream_id)?;
            self.index_cache.insert(stream_id, loaded);
        }
        let index = self.index_cache.get_mut(&stream_id).expect("just loaded");
        index.append(byte_position);
        index.save(&index_path)?;

        // Update checkpoint index
        if let Some(cp_index) = self.checkpoint_cache.get_mut(&stream_id) {
            cp_index.add(current_offset);
        }

        tracing::info!(
            stream_id = %stream_id,
            offset = %current_offset,
            record_count = record_count,
            "created checkpoint"
        );

        let next_offset = current_offset + Offset::from(1u64);
        Ok((next_offset, record_hash))
    }

    /// Reads records with checkpoint-optimized verification.
    ///
    /// Instead of verifying from genesis, this method verifies from the nearest
    /// checkpoint before `from_offset`. This reduces verification cost from O(n)
    /// to O(k) where k is the distance to the nearest checkpoint.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to read from
    /// * `from_offset` - The first offset to include (inclusive)
    /// * `max_bytes` - Maximum payload bytes to return
    ///
    /// # Returns
    ///
    /// A vector of verified records.
    pub fn read_records_verified(
        &mut self,
        stream_id: StreamId,
        from_offset: Offset,
        max_bytes: u64,
    ) -> Result<Vec<Record>, StorageError> {
        let segment_path = self.segment_path(stream_id);
        if !segment_path.exists() {
            return Ok(Vec::new());
        }

        // Find nearest checkpoint
        let checkpoint_index = self.get_or_rebuild_checkpoint_index(stream_id)?;
        let verification_start = checkpoint_index.find_nearest(from_offset);

        let data: Bytes = fs::read(&segment_path)?.into();

        // Load offset index for seeking
        let offset_index = if let Some(idx) = self.index_cache.get(&stream_id) {
            idx.clone()
        } else {
            self.load_or_rebuild_index(stream_id)?
        };

        // Determine starting position and expected hash
        let (start_pos, mut expected_prev_hash) = match verification_start {
            Some(cp_offset) => {
                // Start from checkpoint
                let byte_pos = offset_index
                    .lookup(cp_offset)
                    .ok_or(StorageError::UnexpectedEof)?;

                // Read checkpoint record to get its chain_hash
                let (cp_record, _) = Record::from_bytes(&data.slice(byte_pos as usize..))?;
                debug_assert!(cp_record.is_checkpoint());

                // Get the chain hash stored in the checkpoint
                let (chain_hash, _) =
                    deserialize_checkpoint_payload(cp_record.payload(), cp_offset)?;

                // The checkpoint's prev_hash is the chain_hash it recorded
                // After the checkpoint, we verify against the checkpoint's own hash
                (byte_pos as usize, Some(chain_hash))
            }
            None => {
                // No checkpoint found, start from genesis
                (0, None)
            }
        };

        let mut results = Vec::new();
        let mut bytes_read: u64 = 0;
        let mut pos = start_pos;

        while pos < data.len() && bytes_read < max_bytes {
            let (record, consumed) = Record::from_bytes(&data.slice(pos..))?;

            // Verify hash chain integrity
            if record.prev_hash() != expected_prev_hash {
                return Err(StorageError::ChainVerificationFailed {
                    offset: record.offset(),
                    expected: expected_prev_hash,
                    actual: record.prev_hash(),
                });
            }

            expected_prev_hash = Some(record.compute_hash());
            pos += consumed;

            // Only collect records at or after the requested offset (skip checkpoint records)
            if record.offset() >= from_offset && !record.is_checkpoint() {
                bytes_read += record.payload().len() as u64;
                results.push(record);
            }
        }

        Ok(results)
    }

    /// Returns the last checkpoint for a stream, if any.
    pub fn last_checkpoint(&mut self, stream_id: StreamId) -> Result<Option<Offset>, StorageError> {
        let index = self.get_or_rebuild_checkpoint_index(stream_id)?;
        Ok(index.last())
    }
}
