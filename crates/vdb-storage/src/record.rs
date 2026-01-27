//! Record type for the append-only log.
//!
//! Each record contains an offset, optional link to previous record's hash,
//! and a payload. Records are serialized with CRC32 checksums for integrity.

use bytes::Bytes;
use vdb_crypto::{ChainHash, chain_hash};
use vdb_types::Offset;

use crate::StorageError;

/// A single record in the event log.
///
/// Records are the on-disk representation of events. Each record contains
/// an offset (logical position), the event payload, and is serialized with
/// a CRC32 checksum for integrity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    /// Returns the hash of the previous record, if any.
    pub fn prev_hash(&self) -> Option<ChainHash> {
        self.prev_hash
    }

    /// Returns the payload of this record.
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    /// Computes the hash of this record for chain linking.
    pub fn compute_hash(&self) -> ChainHash {
        chain_hash(self.prev_hash.as_ref(), &self.payload)
    }

    /// Serializes the record to bytes.
    ///
    /// Format: `[offset:i64][prev_hash:32B][length:u32][payload][crc32:u32]`
    ///
    /// All integers are little-endian.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // offset (8 bytes)
        buf.extend_from_slice(&self.offset.as_u64().to_le_bytes());

        // prev_hash (32 bytes) - zeros if genesis
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
        // Header size: offset(8) + prev_hash(32) + len(4) = 44 bytes
        const HEADER_SIZE: usize = 44;

        if data.len() < HEADER_SIZE {
            return Err(StorageError::UnexpectedEof);
        }

        // Read offset (bytes 0-7)
        let offset = Offset::new(u64::from_le_bytes(data[0..8].try_into().unwrap()));

        // Read prev_hash (bytes 8-39)
        let prev_hash_bytes: [u8; 32] = data[8..40].try_into().unwrap();
        let prev_hash = if prev_hash_bytes == [0u8; 32] {
            None
        } else {
            Some(ChainHash::from_bytes(&prev_hash_bytes))
        };

        // Read length (bytes 40-43)
        let length = u32::from_le_bytes(data[40..44].try_into().unwrap()) as usize;

        // Check we have enough for payload + crc(4)
        let total_size = HEADER_SIZE + length + 4;
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
