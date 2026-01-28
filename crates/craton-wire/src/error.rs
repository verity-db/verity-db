//! Wire protocol error types.

use thiserror::Error;

/// Result type for wire protocol operations.
pub type WireResult<T> = Result<T, WireError>;

/// Errors that can occur during wire protocol operations.
#[derive(Debug, Error)]
pub enum WireError {
    /// Invalid magic bytes in frame header.
    #[error("invalid magic: expected 0x56444220, got 0x{0:08x}")]
    InvalidMagic(u32),

    /// Unsupported protocol version.
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(u16),

    /// Payload exceeds maximum size.
    #[error("payload too large: {size} bytes (max {max})")]
    PayloadTooLarge { size: u32, max: u32 },

    /// Checksum mismatch.
    #[error("checksum mismatch: expected 0x{expected:08x}, got 0x{actual:08x}")]
    ChecksumMismatch { expected: u32, actual: u32 },

    /// Incomplete frame (not enough bytes).
    #[error("incomplete frame: need {needed} bytes, have {available}")]
    IncompleteFrame { needed: usize, available: usize },

    /// Serialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Deserialization error.
    #[error("deserialization error: {0}")]
    Deserialization(String),

    /// I/O error.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<bincode::Error> for WireError {
    fn from(e: bincode::Error) -> Self {
        WireError::Deserialization(e.to_string())
    }
}
