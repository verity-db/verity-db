//! craton-storage: Append-only segment storage for `Craton`
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
//! use craton_storage::Storage;
//! use craton_types::{Offset, StreamId};
//! use bytes::Bytes;
//!
//! let storage = Storage::new("/data/craton");
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

// Modules
mod checkpoint;
mod error;
mod index;
mod record;
mod storage;

// Re-exports
pub use checkpoint::{
    CheckpointIndex, create_checkpoint, deserialize_checkpoint_payload,
    serialize_checkpoint_payload, should_create_checkpoint,
};
pub use error::StorageError;
pub use index::OffsetIndex;
pub use record::Record;
pub use storage::Storage;

#[cfg(test)]
mod tests;
