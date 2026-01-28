//! Record type for the append-only log.
//!
//! Each record contains an offset, record kind, optional link to previous
//! record's hash, and a payload. Records are serialized with CRC32 checksums
//! for integrity.
//!
//! # Record Format
//!
//! ```text
//! [offset:u64][prev_hash:32B][kind:u8][length:u32][payload:bytes][crc32:u32]
//!     8B           32B         1B        4B         variable        4B
//! ```

use bytes::Bytes;
use craton_crypto::{ChainHash, chain_hash};
use craton_types::{Offset, RecordKind};

use crate::StorageError;

/// A single record in the event log.
///
/// Records are the on-disk representation of events. Each record contains
/// an offset (logical position), a record kind, the event payload, and is
/// serialized with a CRC32 checksum for integrity.
///
/// # Record Kinds
///
/// - [`RecordKind::Data`]: Normal application data
/// - [`RecordKind::Checkpoint`]: Periodic verification anchor
/// - [`RecordKind::Tombstone`]: Logical deletion marker
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Record {
    offset: Offset,
    prev_hash: Option<ChainHash>,
    kind: RecordKind,
    payload: Bytes,
}

impl Record {
    /// Creates a new data record with the given offset and payload.
    pub fn new(offset: Offset, prev_hash: Option<ChainHash>, payload: Bytes) -> Self {
        Self {
            offset,
            prev_hash,
            kind: RecordKind::Data,
            payload,
        }
    }

    /// Creates a new record with a specific kind.
    pub fn with_kind(
        offset: Offset,
        prev_hash: Option<ChainHash>,
        kind: RecordKind,
        payload: Bytes,
    ) -> Self {
        Self {
            offset,
            prev_hash,
            kind,
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

    /// Returns the kind of this record.
    pub fn kind(&self) -> RecordKind {
        self.kind
    }

    /// Returns the payload of this record.
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    /// Returns true if this is a checkpoint record.
    pub fn is_checkpoint(&self) -> bool {
        self.kind == RecordKind::Checkpoint
    }

    /// Computes the hash of this record for chain linking.
    ///
    /// The hash covers the kind byte and payload to ensure the record kind
    /// is part of the tamper-evident chain.
    pub fn compute_hash(&self) -> ChainHash {
        // Include kind in hash computation for tamper-evidence
        let mut data = vec![self.kind.as_byte()];
        data.extend_from_slice(&self.payload);
        chain_hash(self.prev_hash.as_ref(), &data)
    }

    /// Serializes the record to bytes.
    ///
    /// Format: `[offset:u64][prev_hash:32B][kind:u8][length:u32][payload][crc32:u32]`
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

        // kind (1 byte)
        buf.push(self.kind.as_byte());

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
    /// - [`StorageError::InvalidRecordKind`] if the kind byte is invalid
    pub fn from_bytes(data: &Bytes) -> Result<(Self, usize), StorageError> {
        // Header size: offset(8) + prev_hash(32) + kind(1) + len(4) = 45 bytes
        const HEADER_SIZE: usize = 45;

        if data.len() < HEADER_SIZE {
            return Err(StorageError::UnexpectedEof);
        }

        // Read offset (bytes 0-7)
        let offset = Offset::new(u64::from_le_bytes(
            data[0..8]
                .try_into()
                .expect("slice is exactly 8 bytes after bounds check"),
        ));

        // Read prev_hash (bytes 8-39)
        let prev_hash_bytes: [u8; 32] = data[8..40]
            .try_into()
            .expect("slice is exactly 32 bytes after bounds check");
        let prev_hash = if prev_hash_bytes == [0u8; 32] {
            None
        } else {
            Some(ChainHash::from_bytes(&prev_hash_bytes))
        };

        // Read kind (byte 40)
        let kind = RecordKind::from_byte(data[40]).ok_or(StorageError::InvalidRecordKind {
            byte: data[40],
            offset,
        })?;

        // Read length (bytes 41-44)
        let length = u32::from_le_bytes(
            data[41..45]
                .try_into()
                .expect("slice is exactly 4 bytes after bounds check"),
        ) as usize;

        // Check we have enough for payload + crc(4)
        let total_size = HEADER_SIZE + length + 4;
        if data.len() < total_size {
            return Err(StorageError::UnexpectedEof);
        }

        // Read payload (bytes 45..45+length) - zero-copy!
        let payload = data.slice(45..45 + length);

        // Read and verify CRC (last 4 bytes)
        let stored_crc = u32::from_le_bytes(
            data[45 + length..total_size]
                .try_into()
                .expect("slice is exactly 4 bytes after bounds check"),
        );
        let computed_crc = crc32fast::hash(&data[0..45 + length]);

        if stored_crc != computed_crc {
            return Err(StorageError::CorruptedRecord);
        }

        Ok((
            Record {
                offset,
                prev_hash,
                kind,
                payload,
            },
            total_size,
        ))
    }
}
