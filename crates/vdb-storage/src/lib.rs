//! vdb-storage: Append-only segment storage for `VerityDB`
//!
//! This crate implements the durable event log storage layer. Events are
//! stored in segment files with a simple binary format that includes
//! checksums for integrity verification.
//!
//! # Record Format
//!
//! Each record is stored as:
//! ```text
//! [offset:i64][length:u32][payload:bytes][crc32:u32]
//!     8B          4B         variable        4B
//! ```
//!
//! - **offset**: The logical position of this event in the stream
//! - **length**: Size of the payload in bytes
//! - **payload**: The event data
//! - **crc32**: Checksum of offset + length + payload for corruption detection
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
use vdb_types::{Offset, StreamId};

/// A single record in the event log.
///
/// Records are the on-disk representation of events. Each record contains
/// an offset (logical position), the event payload, and is serialized with
/// a CRC32 checksum for integrity.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Record {
    offset: Offset,
    payload: Bytes,
}

impl Record {
    /// Creates a new record with the given offset and payload.
    pub fn new(offset: Offset, payload: Bytes) -> Self {
        Self { offset, payload }
    }

    /// Returns the offset of this record.
    pub fn offset(&self) -> Offset {
        self.offset
    }

    /// Returns the payload of this record.
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    /// Serializes the record to bytes.
    ///
    /// Format: `[offset:i64][length:u32][payload][crc32:u32]`
    ///
    /// All integers are little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // offset (8 bytes)
        buf.extend_from_slice(&self.offset.as_i64().to_le_bytes());

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
        // Need at least header: offset(8) + len(4) = 12 bytes
        if data.len() < 12 {
            return Err(StorageError::UnexpectedEof);
        }

        // Read offset (bytes 0-7)
        let offset = Offset::new(i64::from_le_bytes(data[0..8].try_into().unwrap()));

        // Read length (bytes 8-11)
        let length = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;

        // Check we have enough for payload + crc(4)
        let total_size = 12 + length + 4;
        if data.len() < total_size {
            return Err(StorageError::UnexpectedEof);
        }

        // Read payload (bytes 12..12+length) - zero-copy!
        let payload = data.slice(12..12 + length);

        // Read and verify CRC (last 4 bytes)
        let stored_crc = u32::from_le_bytes(data[12 + length..total_size].try_into().unwrap());
        let computed_crc = crc32fast::hash(&data[0..12 + length]);

        if stored_crc != computed_crc {
            return Err(StorageError::CorruptedRecord);
        }

        Ok((Record { offset, payload }, total_size))
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

    /// Appends a batch of events to a stream.
    ///
    /// Events are written sequentially starting at `expected_offset`.
    /// Each event becomes a separate record on disk.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to append to
    /// * `events` - The event payloads to append
    /// * `expected_offset` - The offset to start writing at
    /// * `fsync` - Whether to fsync after writing (recommended for durability)
    ///
    /// # Returns
    ///
    /// The new offset after all events are written (i.e., the offset of
    /// the next event that would be written).
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
        fsync: bool,
    ) -> Result<Offset, StorageError> {
        let stream_dir = self.data_dir.join(stream_id.to_string());
        fs::create_dir_all(&stream_dir)?;

        let segment_path = stream_dir.join("segment_000000.log");

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&segment_path)?;

        let mut current_offset = expected_offset;

        for event in events {
            let record = Record {
                offset: current_offset,
                payload: event,
            };
            file.write_all(&record.to_bytes())?;
            current_offset += Offset::from(1);
        }

        if fsync {
            file.sync_all()?;
        }

        Ok(current_offset)
    }

    /// Reads events from a stream starting at the given offset.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to read from
    /// * `from_offset` - The first offset to include (inclusive)
    /// * `max_bytes` - Maximum total payload bytes to return
    ///
    /// # Returns
    ///
    /// A vector of event payloads. Uses zero-copy [`Bytes`] slicing.
    pub fn read_from(
        &self,
        stream_id: StreamId,
        from_offset: Offset,
        max_bytes: u64,
    ) -> Result<Vec<Bytes>, StorageError> {
        let segment_path = self
            .data_dir
            .join(stream_id.to_string())
            .join("segment_000000.log");

        let data: Bytes = fs::read(&segment_path)?.into();

        let mut results = Vec::new();
        let mut bytes_read: u64 = 0;
        let mut pos = 0;

        while pos < data.len() && bytes_read < max_bytes {
            let (record, consumed) = Record::from_bytes(&data.slice(pos..))?;
            pos += consumed;

            // Skip records before our target offset
            if record.offset < from_offset {
                continue;
            }

            bytes_read += record.payload.len() as u64;
            results.push(record.payload);
        }

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
}

#[cfg(test)]
mod tests;
