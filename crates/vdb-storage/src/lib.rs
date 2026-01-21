//! vdb-storage: Append-only segment storage for `VerityDB`
//!
//! This crate implements the durable event log storage layer. Events are
//! stored in segment files with a simple binary format that includes
//! cryptographic hash chains for tamper detection and CRC32 checksums
//! for corruption detection.
//!
//! # Record Format
//!
//! Each record is stored as:
//! ```text
//! [offset:i64][prev_hash:32B][length:u32][payload:bytes][crc32:u32]
//!     8B           32B           4B         variable        4B
//! ```
//!
//! - **offset**: The logical position of this event in the stream
//! - **`prev_hash`**: SHA-256 hash of the previous record (all zeros for genesis)
//! - **length**: Size of the payload in bytes
//! - **payload**: The event data
//! - **crc32**: Checksum of all preceding fields for corruption detection
//!
//! # Hash Chain
//!
//! Records form a tamper-evident chain where each record includes the hash
//! of the previous record. This allows verification that the log has not
//! been modified:
//!
//! ```text
//! Record 0: prev_hash = [0; 32]  →  hash_0 = SHA-256(payload_0)
//! Record 1: prev_hash = hash_0   →  hash_1 = SHA-256(hash_0 || payload_1)
//! Record 2: prev_hash = hash_1   →  hash_2 = SHA-256(hash_1 || payload_2)
//! ```
//!
//! # File Layout
//!
//! ```text
//! data_dir/
//!   {stream_id}/
//!     segment_000000.log   # First segment (future: rotation)
//!     segment_000001.log   # Second segment, etc.
//! ```
//!
//! # Example
//!
//! ```ignore
//! use vdb_storage::Storage;
//! use vdb_types::{Offset, StreamId};
//! use bytes::Bytes;
//!
//! let storage = Storage::new("/data/veritydb");
//!
//! // Append events
//! let events = vec![Bytes::from("event1"), Bytes::from("event2")];
//! let new_offset = storage.append_batch(
//!     StreamId::new(1),
//!     events,
//!     Offset::new(0),
//!     true,  // fsync for durability
//! )?;
//!
//! // Read events back
//! let events = storage.read_from(StreamId::new(1), Offset::new(0), 1024)?;
//! ```

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use bytes::Bytes;
use vdb_crypto::{ChainHash, chain_hash};
use vdb_types::{Offset, StreamId};

/// A single record in the event log.
///
/// Records are the on-disk representation of events. Each record contains
/// an offset (logical position), the event payload, and is serialized with
/// a CRC32 checksum for integrity.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Record {
    offset: Offset,
    prev_hash: Option<ChainHash>,
    payload: Bytes,
}

impl Record {
    /// Creates a new record with the given offset and payload.
    pub fn new(offset: Offset, prev_hash: Option<ChainHash>, payload: Bytes) -> Self {
        Self {
            offset,
            prev_hash,
            payload,
        }
    }

    /// Returns the offset of this record.
    pub fn offset(&self) -> Offset {
        self.offset
    }

    pub fn prev_hash(&self) -> Option<ChainHash> {
        self.prev_hash
    }

    /// Returns the payload of this record.
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    pub fn compute_hash(&self) -> ChainHash {
        chain_hash(self.prev_hash.as_ref(), &self.payload)
    }

    /// Serializes the record to bytes.
    ///
    /// Format: `[offset:i64][prev_hash: u32 if has_prev=1][length:u32][payload][crc32:u32]`
    ///
    /// All integers are little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // offset (8 bytes)
        buf.extend_from_slice(&self.offset.as_i64().to_le_bytes());

        match &self.prev_hash {
            Some(hash) => buf.extend_from_slice(hash.as_bytes()),
            None => buf.extend_from_slice(&[0u8; 32]),
        }

        // length (4 bytes)
        buf.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());

        // payload (variable)
        buf.extend_from_slice(&self.payload);

        // crc (4 bytes) - checksum of everything above
        let crc = crc32fast::hash(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());

        buf
    }

    /// Deserializes a record from bytes.
    ///
    /// Returns the parsed record and the number of bytes consumed.
    /// Uses zero-copy slicing for the payload via [`Bytes::slice`].
    ///
    /// # Errors
    ///
    /// - [`StorageError::UnexpectedEof`] if the data is truncated
    /// - [`StorageError::CorruptedRecord`] if the CRC doesn't match
    pub fn from_bytes(data: &Bytes) -> Result<(Self, usize), StorageError> {
        // Need at least header: offset(8) + prev_hash(32) + len(4) = 44 bytes
        if data.len() < 44 {
            return Err(StorageError::UnexpectedEof);
        }

        // Read offset (bytes 0-7)
        let offset = Offset::new(i64::from_le_bytes(data[0..8].try_into().unwrap()));

        let prev_hash_bytes: [u8; 32] = data[8..40].try_into().unwrap();
        let prev_hash = if prev_hash_bytes == [0u8; 32] {
            None
        } else {
            Some(ChainHash::from_bytes(&prev_hash_bytes))
        };

        // Read length (bytes 40-44)
        let length = u32::from_le_bytes(data[40..44].try_into().unwrap()) as usize;

        // Check we have enough for payload + crc(4)
        let total_size = 44 + length + 4;
        if data.len() < total_size {
            return Err(StorageError::UnexpectedEof);
        }

        // Read payload (bytes 44..44+length) - zero-copy!
        let payload = data.slice(44..44 + length);

        // Read and verify CRC (last 4 bytes)
        let stored_crc = u32::from_le_bytes(data[44 + length..total_size].try_into().unwrap());
        let computed_crc = crc32fast::hash(&data[0..44 + length]);

        if stored_crc != computed_crc {
            return Err(StorageError::CorruptedRecord);
        }

        Ok((
            Record {
                offset,
                prev_hash,
                payload,
            },
            total_size,
        ))
    }
}

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
            current_offset += Offset::from(1);
        }

        if fsync {
            file.sync_all()?;
        }

        // Postcondition: we wrote exactly event_count records
        debug_assert_eq!(
            current_offset.as_i64() - expected_offset.as_i64(),
            event_count as i64,
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
        Ok(records.into_iter().map(|r| r.payload).collect())
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
                    offset: record.offset,
                    expected: expected_prev_hash,
                    actual: record.prev_hash(),
                });
            }

            // Update expected hash for next record
            expected_prev_hash = Some(record.compute_hash());
            records_verified += 1;
            pos += consumed;

            // Only collect records at or after the requested offset
            if record.offset < from_offset {
                continue;
            }

            bytes_read += record.payload.len() as u64;
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

/// Errors that can occur during storage operations.
#[derive(thiserror::Error, Debug)]
pub enum StorageError {
    /// Generic write error.
    #[error("error writing batch payload")]
    WriteError,

    /// Filesystem I/O error.
    #[error("filesystem error: {0}")]
    FsError(#[from] io::Error),

    /// The data was truncated (not enough bytes).
    #[error("unexpected end of file")]
    UnexpectedEof,

    /// CRC mismatch - the record data is corrupted.
    #[error("corrupted record: CRC mismatch")]
    CorruptedRecord,

    #[error(
        "hash chain verification failed at offset {offset}: expected {expected:?}, found {actual:?}"
    )]
    ChainVerificationFailed {
        offset: Offset,
        expected: Option<ChainHash>,
        actual: Option<ChainHash>, // Option because first record might have None
    },
}

#[cfg(test)]
mod tests;
