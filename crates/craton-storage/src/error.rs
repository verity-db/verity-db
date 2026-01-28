//! Error types for storage operations.

use std::io;

use craton_crypto::ChainHash;
use craton_types::Offset;

/// Errors that can occur during storage operations.
#[derive(thiserror::Error, Debug)]
pub enum StorageError {
    /// Generic write error.
    #[error("error writing batch payload")]
    WriteError,

    /// Filesystem I/O error.
    #[error("filesystem error: {0}")]
    Io(#[from] io::Error),

    /// The data was truncated (not enough bytes).
    #[error("unexpected end of file")]
    UnexpectedEof,

    /// CRC mismatch - the record data is corrupted.
    #[error("corrupted record: CRC mismatch")]
    CorruptedRecord,

    /// Invalid record kind byte.
    #[error("invalid record kind byte {byte:#04x} at offset {offset}")]
    InvalidRecordKind { byte: u8, offset: Offset },

    /// Hash chain verification failed.
    #[error(
        "hash chain verification failed at offset {offset}: expected {expected:?}, found {actual:?}"
    )]
    ChainVerificationFailed {
        offset: Offset,
        expected: Option<ChainHash>,
        actual: Option<ChainHash>,
    },

    /// Checkpoint payload is malformed.
    #[error("invalid checkpoint payload at offset {offset}: {reason}")]
    InvalidCheckpointPayload { offset: Offset, reason: String },

    /// Index file has invalid magic bytes
    #[error("invalid index magic bytes")]
    InvalidIndexMagic,

    /// Index file has unsupported version
    #[error("unsupported index version: {0}")]
    UnsupportedIndexVersion(u8),

    /// Index file checksum mismatch
    #[error("index checksum mismatch: expected {expected:#010x}, got {actual:#010x}")]
    IndexChecksumMismatch { expected: u32, actual: u32 },

    /// Index file is truncated
    #[error("truncated index file: expected {expected} bytes, got {actual}")]
    IndexTruncated { expected: usize, actual: usize },
}
