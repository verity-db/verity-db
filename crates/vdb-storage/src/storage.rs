//! Append-only event log storage.
//!
//! The Storage struct manages segment files on disk, providing append and read
//! operations for event streams. Each stream gets its own directory with
//! numbered segment files.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use bytes::Bytes;
use vdb_crypto::ChainHash;
use vdb_types::{Offset, StreamId};

use crate::{Record, StorageError};

/// Append-only event log storage.
///
/// Storage manages segment files on disk, providing append and read
/// operations for event streams. Each stream gets its own directory
/// with numbered segment files.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Storage {
    /// Root directory for all stream data.
    data_dir: PathBuf,
}

impl Storage {
    /// Creates a new storage instance with the given data directory.
    ///
    /// The directory will be created if it doesn't exist when the first
    /// write occurs.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// Returns the data directory path.
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    /// Appends a batch of events to a stream, building the hash chain.
    ///
    /// Each event is written as a [`Record`] with a cryptographic link to the
    /// previous record, forming a tamper-evident chain.
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
        &self,
        stream_id: StreamId,
        events: Vec<Bytes>,
        expected_offset: Offset,
        prev_hash: Option<ChainHash>,
        fsync: bool,
    ) -> Result<(Offset, ChainHash), StorageError> {
        // Precondition: batch must not be empty
        assert!(!events.is_empty(), "cannot append empty batch");

        let event_count = events.len();
        let stream_dir = self.data_dir.join(stream_id.to_string());
        fs::create_dir_all(&stream_dir)?;

        let segment_path = stream_dir.join("segment_000000.log");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&segment_path)?;

        let mut current_offset = expected_offset;
        let mut current_hash = prev_hash;

        for event in events {
            let record = Record::new(current_offset, current_hash, event);
            file.write_all(&record.to_bytes())?;

            current_hash = Some(record.compute_hash());
            current_offset += Offset::from(1u64);
        }

        if fsync {
            file.sync_all()?;
        }

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
        let segment_path = self
            .data_dir
            .join(stream_id.to_string())
            .join("segment_000000.log");

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
