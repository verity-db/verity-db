//! Error types for store operations.

use std::io;

use crate::types::{PageId, TableId};
use crate::Key;

/// Errors that can occur during store operations.
#[derive(thiserror::Error, Debug)]
pub enum StoreError {
    /// Filesystem I/O error.
    #[error("filesystem error: {0}")]
    Io(#[from] io::Error),

    /// Page CRC32 checksum mismatch - data corruption detected.
    #[error("page {page_id} corrupted: CRC mismatch (expected {expected:#010x}, got {actual:#010x})")]
    PageCorrupted {
        page_id: PageId,
        expected: u32,
        actual: u32,
    },

    /// Key exceeds maximum allowed length.
    #[error("key too long: {len} bytes exceeds maximum {max}")]
    KeyTooLong { len: usize, max: usize },

    /// Value exceeds maximum size that fits in a page.
    #[error("value too large: {len} bytes exceeds maximum {max}")]
    ValueTooLarge { len: usize, max: usize },

    /// Page has invalid magic bytes.
    #[error("invalid page magic: expected {expected:#010x}, got {actual:#010x}")]
    InvalidPageMagic { expected: u32, actual: u32 },

    /// Page has unsupported version.
    #[error("unsupported page version: {0}")]
    UnsupportedPageVersion(u8),

    /// Superblock has invalid magic bytes.
    #[error("invalid superblock magic")]
    InvalidSuperblockMagic,

    /// Superblock CRC mismatch.
    #[error("superblock corrupted: CRC mismatch")]
    SuperblockCorrupted,

    /// Batch position is not sequential.
    #[error("non-sequential batch: expected position {expected}, got {actual}")]
    NonSequentialBatch { expected: u64, actual: u64 },

    /// Table not found.
    #[error("table {0:?} not found")]
    TableNotFound(TableId),

    /// Page overflow - not enough space for insert.
    #[error("page overflow: need {needed} bytes, have {available}")]
    PageOverflow { needed: usize, available: usize },

    /// Internal B+tree invariant violation.
    #[error("B+tree invariant violation: {0}")]
    BTreeInvariant(String),

    /// Duplicate key in batch.
    #[error("duplicate key in batch: {0:?}")]
    DuplicateKey(Key),

    /// Page not found in cache or on disk.
    #[error("page {0} not found")]
    PageNotFound(PageId),

    /// Store is read-only.
    #[error("store is read-only")]
    ReadOnly,
}
