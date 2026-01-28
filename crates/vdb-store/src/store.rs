//! `ProjectionStore` implementation using B+trees.
//!
//! This module provides `BTreeStore`, a complete implementation of the
//! `ProjectionStore` trait using page-based B+trees with MVCC.

use std::ops::Range;
use std::path::Path;

use bytes::Bytes;
use vdb_types::{Hash, Offset};

use crate::batch::{WriteBatch, WriteOp};
use crate::btree::{BTree, BTreeMeta};
use crate::cache::PageCache;
use crate::error::StoreError;
use crate::superblock::Superblock;
use crate::types::{PageId, TableId};
use crate::{Key, ProjectionStore};

/// Default page cache capacity (4096 pages = 16MB).
const DEFAULT_CACHE_CAPACITY: usize = 4096;

/// B+tree-based projection store implementation.
///
/// Provides a persistent key-value store with:
/// - MVCC for point-in-time queries
/// - B+tree indexing for efficient lookups and scans
/// - Page-based storage with CRC32 integrity checks
/// - Crash recovery via superblock
pub struct BTreeStore {
    /// Page cache for I/O.
    cache: PageCache,
    /// Store metadata.
    superblock: Superblock,
}

impl BTreeStore {
    /// Opens or creates a store at the given path.
    ///
    /// If the file exists, loads the superblock and verifies integrity.
    /// If the file doesn't exist, creates a new empty store.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with_capacity(path, DEFAULT_CACHE_CAPACITY)
    }

    /// Opens or creates a store with a custom cache capacity.
    pub fn open_with_capacity(
        path: impl AsRef<Path>,
        cache_capacity: usize,
    ) -> Result<Self, StoreError> {
        let mut cache = PageCache::open(path.as_ref(), Some(cache_capacity))?;

        let superblock = if cache.next_page_id() == PageId::new(0) {
            // New file - create superblock
            let sb = Superblock::new();

            // Allocate page 0 for superblock
            let page_id = cache.allocate(crate::page::PageType::Free)?;
            debug_assert_eq!(page_id, PageId::SUPERBLOCK);

            // Write initial superblock
            Self::write_superblock(&mut cache, &sb)?;

            sb
        } else {
            // Existing file - load superblock (read raw since it has custom format)
            let raw_data = cache.read_raw(PageId::SUPERBLOCK)?;
            Superblock::deserialize(&raw_data)?
        };

        Ok(Self { cache, superblock })
    }

    /// Writes the superblock to page 0.
    fn write_superblock(cache: &mut PageCache, sb: &Superblock) -> Result<(), StoreError> {
        // The superblock has its own format, stored in page 0
        let page = cache.get_mut(PageId::SUPERBLOCK)?;
        if let Some(page) = page {
            let sb_bytes = sb.serialize();
            page.set_raw_data(&sb_bytes);
        }
        Ok(())
    }

    /// Returns the applied position.
    pub fn applied_position(&self) -> Offset {
        self.superblock.applied_position
    }

    /// Returns the applied hash.
    pub fn applied_hash(&self) -> Hash {
        self.superblock.applied_hash
    }

    /// Ensures a table exists, returning its metadata.
    fn ensure_table(&mut self, table: TableId) -> BTreeMeta {
        self.superblock
            .tables
            .get(&table)
            .cloned()
            .unwrap_or_default()
    }

    /// Updates the metadata for a table.
    fn update_table(&mut self, table: TableId, meta: BTreeMeta) {
        self.superblock.tables.insert(table, meta);
    }
}

impl ProjectionStore for BTreeStore {
    fn apply(&mut self, batch: WriteBatch) -> Result<(), StoreError> {
        let pos = batch.position();

        // Verify sequential position
        let expected = if self.superblock.applied_position == Offset::ZERO {
            Offset::new(1)
        } else {
            Offset::new(self.superblock.applied_position.as_u64() + 1)
        };

        // Allow first batch to be position 1 when starting from 0
        if pos != expected && !(self.superblock.applied_position == Offset::ZERO && pos == Offset::new(1)) {
            return Err(StoreError::NonSequentialBatch {
                expected: expected.as_u64(),
                actual: pos.as_u64(),
            });
        }

        // Process each operation
        for op in batch {
            match op {
                WriteOp::Put { table, key, value } => {
                    let mut meta = self.ensure_table(table);
                    {
                        let mut tree = BTree::new(&mut meta, &mut self.cache);
                        tree.put(key, value, pos)?;
                    }
                    self.update_table(table, meta);
                }
                WriteOp::Delete { table, key } => {
                    let mut meta = self.ensure_table(table);
                    {
                        let mut tree = BTree::new(&mut meta, &mut self.cache);
                        tree.delete(&key, pos)?;
                    }
                    self.update_table(table, meta);
                }
            }
        }

        // Update applied position
        self.superblock.applied_position = pos;

        Ok(())
    }

    fn applied_position(&self) -> Offset {
        self.superblock.applied_position
    }

    fn get(&mut self, table: TableId, key: &Key) -> Result<Option<Bytes>, StoreError> {
        let mut meta = match self.superblock.tables.get(&table) {
            Some(m) => m.clone(),
            None => return Ok(None),
        };

        let mut tree = BTree::new(&mut meta, &mut self.cache);
        tree.get(key)
    }

    fn get_at(
        &mut self,
        table: TableId,
        key: &Key,
        pos: Offset,
    ) -> Result<Option<Bytes>, StoreError> {
        let mut meta = match self.superblock.tables.get(&table) {
            Some(m) => m.clone(),
            None => return Ok(None),
        };

        let mut tree = BTree::new(&mut meta, &mut self.cache);
        tree.get_at(key, pos)
    }

    fn scan(
        &mut self,
        table: TableId,
        range: Range<Key>,
        limit: usize,
    ) -> Result<Vec<(Key, Bytes)>, StoreError> {
        let mut meta = match self.superblock.tables.get(&table) {
            Some(m) => m.clone(),
            None => return Ok(Vec::new()),
        };

        let mut tree = BTree::new(&mut meta, &mut self.cache);
        tree.scan(range, limit)
    }

    fn scan_at(
        &mut self,
        table: TableId,
        range: Range<Key>,
        limit: usize,
        pos: Offset,
    ) -> Result<Vec<(Key, Bytes)>, StoreError> {
        let mut meta = match self.superblock.tables.get(&table) {
            Some(m) => m.clone(),
            None => return Ok(Vec::new()),
        };

        let mut tree = BTree::new(&mut meta, &mut self.cache);
        tree.scan_at(range, limit, pos)
    }

    fn sync(&mut self) -> Result<(), StoreError> {
        // Update superblock's next_page_id from cache
        self.superblock.next_page_id = self.cache.next_page_id();

        // Write superblock to page 0
        Self::write_superblock(&mut self.cache, &self.superblock)?;

        // Sync all dirty pages (including superblock)
        self.cache.sync()?;

        Ok(())
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_new_store() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.db");

        let store = BTreeStore::open(&path).unwrap();
        assert_eq!(store.applied_position(), Offset::ZERO);
    }

    #[test]
    fn test_apply_batch() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.db");

        let mut store = BTreeStore::open(&path).unwrap();

        let batch = WriteBatch::new(Offset::new(1))
            .put(TableId::new(1), Key::from("key1"), Bytes::from("value1"))
            .put(TableId::new(1), Key::from("key2"), Bytes::from("value2"));

        store.apply(batch).unwrap();

        assert_eq!(ProjectionStore::applied_position(&store), Offset::new(1));
        assert_eq!(
            store.get(TableId::new(1), &Key::from("key1")).unwrap(),
            Some(Bytes::from("value1"))
        );
        assert_eq!(
            store.get(TableId::new(1), &Key::from("key2")).unwrap(),
            Some(Bytes::from("value2"))
        );
    }

    #[test]
    fn test_sequential_batches() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.db");

        let mut store = BTreeStore::open(&path).unwrap();

        // First batch
        store.apply(
            WriteBatch::new(Offset::new(1))
                .put(TableId::new(1), Key::from("a"), Bytes::from("1"))
        ).unwrap();

        // Second batch
        store.apply(
            WriteBatch::new(Offset::new(2))
                .put(TableId::new(1), Key::from("b"), Bytes::from("2"))
        ).unwrap();

        // Third batch
        store.apply(
            WriteBatch::new(Offset::new(3))
                .put(TableId::new(1), Key::from("c"), Bytes::from("3"))
        ).unwrap();

        assert_eq!(ProjectionStore::applied_position(&store), Offset::new(3));
    }

    #[test]
    fn test_mvcc_queries() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.db");

        let mut store = BTreeStore::open(&path).unwrap();

        // Version 1
        store.apply(
            WriteBatch::new(Offset::new(1))
                .put(TableId::new(1), Key::from("key"), Bytes::from("v1"))
        ).unwrap();

        // Version 2
        store.apply(
            WriteBatch::new(Offset::new(2))
                .put(TableId::new(1), Key::from("key"), Bytes::from("v2"))
        ).unwrap();

        // Current is v2
        assert_eq!(
            store.get(TableId::new(1), &Key::from("key")).unwrap(),
            Some(Bytes::from("v2"))
        );

        // Point-in-time queries
        assert_eq!(
            store.get_at(TableId::new(1), &Key::from("key"), Offset::new(1)).unwrap(),
            Some(Bytes::from("v1"))
        );
        assert_eq!(
            store.get_at(TableId::new(1), &Key::from("key"), Offset::new(2)).unwrap(),
            Some(Bytes::from("v2"))
        );
    }

    #[test]
    fn test_delete() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.db");

        let mut store = BTreeStore::open(&path).unwrap();

        store.apply(
            WriteBatch::new(Offset::new(1))
                .put(TableId::new(1), Key::from("key"), Bytes::from("value"))
        ).unwrap();

        assert!(store.get(TableId::new(1), &Key::from("key")).unwrap().is_some());

        store.apply(
            WriteBatch::new(Offset::new(2))
                .delete(TableId::new(1), Key::from("key"))
        ).unwrap();

        // Current is deleted
        assert!(store.get(TableId::new(1), &Key::from("key")).unwrap().is_none());

        // But visible at old position
        assert_eq!(
            store.get_at(TableId::new(1), &Key::from("key"), Offset::new(1)).unwrap(),
            Some(Bytes::from("value"))
        );
    }

    #[test]
    fn test_multiple_tables() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.db");

        let mut store = BTreeStore::open(&path).unwrap();

        store.apply(
            WriteBatch::new(Offset::new(1))
                .put(TableId::new(1), Key::from("key"), Bytes::from("table1"))
                .put(TableId::new(2), Key::from("key"), Bytes::from("table2"))
        ).unwrap();

        assert_eq!(
            store.get(TableId::new(1), &Key::from("key")).unwrap(),
            Some(Bytes::from("table1"))
        );
        assert_eq!(
            store.get(TableId::new(2), &Key::from("key")).unwrap(),
            Some(Bytes::from("table2"))
        );

        // Non-existent table returns None
        assert!(store.get(TableId::new(999), &Key::from("key")).unwrap().is_none());
    }

    #[test]
    fn test_scan() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("store.db");

        let mut store = BTreeStore::open(&path).unwrap();

        let mut batch = WriteBatch::new(Offset::new(1));
        for i in 0..10 {
            batch.push_put(
                TableId::new(1),
                Key::from(format!("key{i:02}")),
                Bytes::from(format!("value{i}")),
            );
        }
        store.apply(batch).unwrap();

        let results = store.scan(
            TableId::new(1),
            Key::from("key03")..Key::from("key07"),
            100,
        ).unwrap();

        assert_eq!(results.len(), 4);
        assert_eq!(results[0].0, Key::from("key03"));
        assert_eq!(results[3].0, Key::from("key06"));
    }
}
