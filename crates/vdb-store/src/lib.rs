//! # vdb-store: Page-based B+tree projection store with MVCC
//!
//! This crate provides a persistent key-value store optimized for projection state
//! in `VerityDB`. Key features:
//!
//! - **B+tree indexing**: Efficient point lookups and range scans
//! - **MVCC**: Multi-Version Concurrency Control for point-in-time queries
//! - **Page-based storage**: 4KB pages with CRC32 integrity checks
//! - **Crash recovery**: Superblock with applied position tracking
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │  ProjectionStore (public API)                       │
//! ├─────────────────────────────────────────────────────┤
//! │  BTree (search, insert, delete, scan with MVCC)     │
//! ├─────────────────────────────────────────────────────┤
//! │  PageCache (LRU cache with page-aligned I/O)        │
//! ├─────────────────────────────────────────────────────┤
//! │  Page Layer (4KB pages, CRC32, slot directory)      │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use vdb_store::{ProjectionStore, BTreeStore, WriteBatch, WriteOp};
//! use vdb_types::Offset;
//!
//! // Open or create a store
//! let mut store = BTreeStore::open("data/projections")?;
//!
//! // Apply a batch of changes
//! let batch = WriteBatch::new(Offset::new(1))
//!     .put(table_id, key, value)
//!     .delete(table_id, other_key);
//! store.apply(batch)?;
//!
//! // Point-in-time query
//! let value = store.get_at(table_id, &key, Offset::new(1))?;
//! ```

mod batch;
mod btree;
mod cache;
mod error;
mod node;
mod page;
mod store;
mod superblock;
mod types;
mod version;

#[cfg(test)]
mod tests;

// Public API
pub use batch::{WriteBatch, WriteOp};
pub use error::StoreError;
pub use store::BTreeStore;
pub use types::{Key, MAX_KEY_LENGTH, PAGE_SIZE, TableId};
pub use version::RowVersion;

use std::ops::Range;

use bytes::Bytes;
use vdb_types::Offset;

/// Trait for projection stores that maintain derived state from the log.
///
/// Implementations must support:
/// - Sequential batch application (log replay)
/// - Point-in-time queries via MVCC
/// - Durable persistence with crash recovery
///
/// Note: Read methods take `&mut self` because they may load pages into the cache.
pub trait ProjectionStore: Send + Sync {
    /// Applies a batch of writes at a specific log position.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] if:
    /// - Batch position is not sequential (must be `applied_position + 1`)
    /// - I/O error during page writes
    /// - Page corruption detected
    fn apply(&mut self, batch: WriteBatch) -> Result<(), StoreError>;

    /// Returns the last applied log position.
    ///
    /// Used for crash recovery to determine where to resume replay.
    fn applied_position(&self) -> Offset;

    /// Gets the current value for a key.
    ///
    /// Returns the most recent visible version (where `deleted_at` is MAX).
    /// Takes `&mut self` because it may load pages into the cache.
    fn get(&mut self, table: TableId, key: &Key) -> Result<Option<Bytes>, StoreError>;

    /// Gets the value visible at a specific log position.
    ///
    /// Returns the version where `created_at <= pos < deleted_at`.
    /// Takes `&mut self` because it may load pages into the cache.
    fn get_at(
        &mut self,
        table: TableId,
        key: &Key,
        pos: Offset,
    ) -> Result<Option<Bytes>, StoreError>;

    /// Scans a range of keys, returning current values.
    ///
    /// Returns up to `limit` key-value pairs in sorted order.
    /// Takes `&mut self` because it may load pages into the cache.
    fn scan(
        &mut self,
        table: TableId,
        range: Range<Key>,
        limit: usize,
    ) -> Result<Vec<(Key, Bytes)>, StoreError>;

    /// Scans a range of keys at a specific log position.
    /// Takes `&mut self` because it may load pages into the cache.
    fn scan_at(
        &mut self,
        table: TableId,
        range: Range<Key>,
        limit: usize,
        pos: Offset,
    ) -> Result<Vec<(Key, Bytes)>, StoreError>;

    /// Flushes all dirty pages and the superblock to disk.
    ///
    /// Called after applying batches to ensure durability.
    fn sync(&mut self) -> Result<(), StoreError>;
}
