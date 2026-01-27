//! Append-only event log storage.
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

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use bytes::Bytes;
use vdb_crypto::ChainHash;
use vdb_types::{Offset, StreamId};

use crate::{OffsetIndex, Record, StorageError};

/// Current segment filename. Future: segment rotation will make this dynamic.
const SEGMENT_FILENAME: &str = "segment_000000.log";

/// Append-only event log storage.
///
/// Manages segment files on disk, providing append and read operations for
/// event streams. Each stream gets its own directory with numbered segment files.
///
/// # Invariants
///
/// - Records are append-only; existing data is never modified
/// - Each record links to the previous via `prev_hash` (hash chain)
/// - The offset index stays in sync with the log (updated atomically with appends)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Storage {
    /// Root directory for all stream data.
    data_dir: PathBuf,

    /// In-memory cache of offset indexes, keyed by stream.
    /// Loaded lazily on first access, kept in sync during appends.
    index_cache: HashMap<StreamId, OffsetIndex>,
}

impl Storage {
    /// Creates a new storage instance with the given data directory.
    ///
    /// The directory will be created if it doesn't exist when the first
    /// write occurs.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            index_cache: HashMap::new(),
        }
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
}
