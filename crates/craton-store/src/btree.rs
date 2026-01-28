//! B+tree implementation with MVCC support.
//!
//! This module implements a disk-based B+tree with:
//! - Point lookups via `get()` and `get_at()`
//! - Range scans via `scan()` and `scan_at()`
//! - MVCC for point-in-time queries
//! - Automatic node splitting on overflow
//!
//! # Architecture
//!
//! The B+tree stores data in pages managed by a `PageCache`. Each page is either:
//! - A leaf node containing key-value pairs with version chains
//! - An internal node containing keys and child pointers
//!
//! All values are stored in leaf nodes, which are linked for efficient range scans.

use std::ops::Range;

use bytes::Bytes;
use craton_types::Offset;

use crate::Key;
use crate::cache::PageCache;
use crate::error::StoreError;
use crate::node::{InternalNode, LeafNode};
use crate::page::PageType;
use crate::types::{BTREE_MIN_KEYS, PageId};
use crate::version::RowVersion;

/// Maximum depth of the B+tree (prevents stack overflow in recursive operations).
const MAX_TREE_DEPTH: usize = 32;

/// Metadata for a B+tree index.
///
/// This struct stores just the tree metadata. Operations require passing
/// a mutable reference to the page cache.
#[derive(Debug, Clone, Default)]
pub struct BTreeMeta {
    /// Root page ID (None if tree is empty).
    pub root: Option<PageId>,
    /// Current height of the tree (1 = just root leaf).
    pub height: usize,
}

impl BTreeMeta {
    /// Creates metadata for a new empty tree.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates metadata with an existing root.
    #[allow(dead_code)]
    pub fn with_root(root: PageId, height: usize) -> Self {
        Self {
            root: Some(root),
            height,
        }
    }
}

/// B+tree operations that work with a page cache.
///
/// This struct provides the actual B+tree operations. It's designed to be
/// used transiently - create it, perform operations, then discard.
pub struct BTree<'a> {
    meta: &'a mut BTreeMeta,
    cache: &'a mut PageCache,
}

impl<'a> BTree<'a> {
    /// Creates a new `BTree` handle for operations.
    pub fn new(meta: &'a mut BTreeMeta, cache: &'a mut PageCache) -> Self {
        Self { meta, cache }
    }

    /// Returns the root page ID.
    #[allow(dead_code)]
    pub fn root(&self) -> Option<PageId> {
        self.meta.root
    }

    /// Returns the height of the tree.
    #[allow(dead_code)]
    pub fn height(&self) -> usize {
        self.meta.height
    }

    /// Gets the current value for a key.
    pub fn get(&mut self, key: &Key) -> Result<Option<Bytes>, StoreError> {
        let Some(root) = self.meta.root else {
            return Ok(None);
        };

        let leaf_id = self.find_leaf(root, key, 0)?;
        let page = self
            .cache
            .get(leaf_id)?
            .ok_or(StoreError::PageNotFound(leaf_id))?;
        let leaf = LeafNode::from_page(page)?;

        if let Some(entry) = leaf.get(key) {
            if let Some(version) = entry.versions.current() {
                return Ok(Some(version.data.clone()));
            }
        }

        Ok(None)
    }

    /// Gets the value visible at a specific position.
    pub fn get_at(&mut self, key: &Key, pos: Offset) -> Result<Option<Bytes>, StoreError> {
        let Some(root) = self.meta.root else {
            return Ok(None);
        };

        let leaf_id = self.find_leaf(root, key, 0)?;
        let page = self
            .cache
            .get(leaf_id)?
            .ok_or(StoreError::PageNotFound(leaf_id))?;
        let leaf = LeafNode::from_page(page)?;

        if let Some(entry) = leaf.get(key) {
            if let Some(version) = entry.versions.at(pos) {
                return Ok(Some(version.data.clone()));
            }
        }

        Ok(None)
    }

    /// Scans a range of keys, returning current values.
    pub fn scan(
        &mut self,
        range: Range<Key>,
        limit: usize,
    ) -> Result<Vec<(Key, Bytes)>, StoreError> {
        let Some(root) = self.meta.root else {
            return Ok(Vec::new());
        };

        let mut results = Vec::new();
        let start_leaf_id = self.find_leaf(root, &range.start, 0)?;

        let mut current_leaf_id = Some(start_leaf_id);

        while let Some(leaf_id) = current_leaf_id {
            if results.len() >= limit {
                break;
            }

            let page = self
                .cache
                .get(leaf_id)?
                .ok_or(StoreError::PageNotFound(leaf_id))?;
            let leaf = LeafNode::from_page(page)?;

            for entry in leaf.range(&range.start, &range.end) {
                if entry.key >= range.end {
                    current_leaf_id = None;
                    break;
                }

                if let Some(version) = entry.versions.current() {
                    results.push((entry.key.clone(), version.data.clone()));
                    if results.len() >= limit {
                        break;
                    }
                }
            }

            if current_leaf_id.is_some() {
                current_leaf_id = leaf.next_leaf;
            }
        }

        Ok(results)
    }

    /// Scans a range of keys at a specific position.
    pub fn scan_at(
        &mut self,
        range: Range<Key>,
        limit: usize,
        pos: Offset,
    ) -> Result<Vec<(Key, Bytes)>, StoreError> {
        let Some(root) = self.meta.root else {
            return Ok(Vec::new());
        };

        let mut results = Vec::new();
        let start_leaf_id = self.find_leaf(root, &range.start, 0)?;

        let mut current_leaf_id = Some(start_leaf_id);

        while let Some(leaf_id) = current_leaf_id {
            if results.len() >= limit {
                break;
            }

            let page = self
                .cache
                .get(leaf_id)?
                .ok_or(StoreError::PageNotFound(leaf_id))?;
            let leaf = LeafNode::from_page(page)?;

            for entry in leaf.range(&range.start, &range.end) {
                if entry.key >= range.end {
                    current_leaf_id = None;
                    break;
                }

                if let Some(version) = entry.versions.at(pos) {
                    results.push((entry.key.clone(), version.data.clone()));
                    if results.len() >= limit {
                        break;
                    }
                }
            }

            if current_leaf_id.is_some() {
                current_leaf_id = leaf.next_leaf;
            }
        }

        Ok(results)
    }

    /// Inserts or updates a key-value pair.
    pub fn put(&mut self, key: Key, value: Bytes, pos: Offset) -> Result<(), StoreError> {
        let version = RowVersion::new(pos, value);

        match self.meta.root {
            None => {
                // Create new root leaf
                let page_id = self.cache.allocate(PageType::Leaf)?;
                let page = self.cache.get_mut(page_id)?.unwrap();
                let mut leaf = LeafNode::new();
                leaf.insert(key, version);
                leaf.to_page(page)?;

                self.meta.root = Some(page_id);
                self.meta.height = 1;
            }
            Some(root) => {
                // Insert into existing tree
                if let Some((split_key, new_child)) =
                    self.insert_recursive(root, key, version, 0)?
                {
                    // Root split - create new root
                    let new_root_id = self.cache.allocate(PageType::Internal)?;
                    let page = self.cache.get_mut(new_root_id)?.unwrap();
                    let internal = InternalNode::from_split(root, split_key, new_child);
                    internal.to_page(page)?;

                    self.meta.root = Some(new_root_id);
                    self.meta.height += 1;
                }
            }
        }

        Ok(())
    }

    /// Deletes a key by marking its current version as deleted.
    pub fn delete(&mut self, key: &Key, pos: Offset) -> Result<bool, StoreError> {
        let Some(root) = self.meta.root else {
            return Ok(false);
        };

        let leaf_id = self.find_leaf(root, key, 0)?;
        let page = self
            .cache
            .get_mut(leaf_id)?
            .ok_or(StoreError::PageNotFound(leaf_id))?;
        let mut leaf = LeafNode::from_page(page)?;

        let deleted = leaf.delete(key, pos);
        if deleted {
            leaf.to_page(page)?;
        }

        Ok(deleted)
    }

    /// Finds the leaf page containing the key.
    fn find_leaf(
        &mut self,
        page_id: PageId,
        key: &Key,
        depth: usize,
    ) -> Result<PageId, StoreError> {
        if depth >= MAX_TREE_DEPTH {
            return Err(StoreError::BTreeInvariant("tree too deep".into()));
        }

        let page = self
            .cache
            .get(page_id)?
            .ok_or(StoreError::PageNotFound(page_id))?;

        match page.page_type() {
            PageType::Leaf => Ok(page_id),
            PageType::Internal => {
                let internal = InternalNode::from_page(page)?;
                let child_id = internal.find_child(key);
                self.find_leaf(child_id, key, depth + 1)
            }
            PageType::Free => Err(StoreError::BTreeInvariant(
                "hit free page during search".into(),
            )),
        }
    }

    /// Recursively inserts into the tree, returning split info if the node split.
    fn insert_recursive(
        &mut self,
        page_id: PageId,
        key: Key,
        version: RowVersion,
        depth: usize,
    ) -> Result<Option<(Key, PageId)>, StoreError> {
        if depth >= MAX_TREE_DEPTH {
            return Err(StoreError::BTreeInvariant("tree too deep".into()));
        }

        let page = self
            .cache
            .get(page_id)?
            .ok_or(StoreError::PageNotFound(page_id))?;
        let page_type = page.page_type();

        match page_type {
            PageType::Leaf => self.insert_into_leaf(page_id, key, version),
            PageType::Internal => {
                let page = self.cache.get(page_id)?.unwrap();
                let internal = InternalNode::from_page(page)?;
                let child_id = internal.find_child(&key);
                drop(internal);

                // Recursively insert into child
                if let Some((child_split_key, new_child_id)) =
                    self.insert_recursive(child_id, key, version, depth + 1)?
                {
                    // Child split, insert the new key into this internal node
                    self.insert_into_internal(page_id, child_split_key, new_child_id)
                } else {
                    Ok(None)
                }
            }
            PageType::Free => Err(StoreError::BTreeInvariant(
                "hit free page during insert".into(),
            )),
        }
    }

    /// Inserts into a leaf node, splitting if necessary.
    fn insert_into_leaf(
        &mut self,
        page_id: PageId,
        key: Key,
        version: RowVersion,
    ) -> Result<Option<(Key, PageId)>, StoreError> {
        let page = self
            .cache
            .get_mut(page_id)?
            .ok_or(StoreError::PageNotFound(page_id))?;
        let mut leaf = LeafNode::from_page(page)?;

        leaf.insert(key, version);

        // Check if we need to split
        if leaf.len() > BTREE_MIN_KEYS * 2 {
            // Split the leaf
            let (split_key, mut right_leaf) = leaf.split();

            // Allocate new page for right half
            let right_page_id = self.cache.allocate(PageType::Leaf)?;

            // Link leaves
            right_leaf.next_leaf = leaf.next_leaf;
            leaf.next_leaf = Some(right_page_id);

            // Write both leaves
            let left_page = self.cache.get_mut(page_id)?.unwrap();
            leaf.to_page(left_page)?;

            let right_page = self.cache.get_mut(right_page_id)?.unwrap();
            right_leaf.to_page(right_page)?;

            Ok(Some((split_key, right_page_id)))
        } else {
            // No split needed
            leaf.to_page(page)?;
            Ok(None)
        }
    }

    /// Inserts into an internal node, splitting if necessary.
    fn insert_into_internal(
        &mut self,
        page_id: PageId,
        key: Key,
        child_id: PageId,
    ) -> Result<Option<(Key, PageId)>, StoreError> {
        let page = self
            .cache
            .get_mut(page_id)?
            .ok_or(StoreError::PageNotFound(page_id))?;
        let mut internal = InternalNode::from_page(page)?;

        internal.insert(key, child_id);

        // Check if we need to split
        if internal.key_count() > BTREE_MIN_KEYS * 2 {
            // Split the internal node
            let (split_key, right_internal) = internal.split();

            // Allocate new page for right half
            let right_page_id = self.cache.allocate(PageType::Internal)?;

            // Write both nodes
            let left_page = self.cache.get_mut(page_id)?.unwrap();
            internal.to_page(left_page)?;

            let right_page = self.cache.get_mut(right_page_id)?.unwrap();
            right_internal.to_page(right_page)?;

            Ok(Some((split_key, right_page_id)))
        } else {
            // No split needed
            internal.to_page(page)?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod btree_tests {
    use super::*;
    use tempfile::tempdir;

    fn create_cache() -> (tempfile::TempDir, PageCache) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("btree_test.db");
        let cache = PageCache::open(&path, Some(100)).unwrap();
        (dir, cache)
    }

    #[test]
    fn test_empty_tree() {
        let (_dir, mut cache) = create_cache();
        let mut meta = BTreeMeta::new();
        let mut tree = BTree::new(&mut meta, &mut cache);

        assert!(tree.root().is_none());
        assert_eq!(tree.get(&Key::from("key")).unwrap(), None);
    }

    #[test]
    fn test_single_insert_and_get() {
        let (_dir, mut cache) = create_cache();
        let mut meta = BTreeMeta::new();

        {
            let mut tree = BTree::new(&mut meta, &mut cache);
            tree.put(Key::from("hello"), Bytes::from("world"), Offset::new(1))
                .unwrap();
        }

        {
            let mut tree = BTree::new(&mut meta, &mut cache);
            assert!(tree.root().is_some());
            assert_eq!(
                tree.get(&Key::from("hello")).unwrap(),
                Some(Bytes::from("world"))
            );
            assert_eq!(tree.get(&Key::from("missing")).unwrap(), None);
        }
    }

    #[test]
    fn test_multiple_inserts() {
        let (_dir, mut cache) = create_cache();
        let mut meta = BTreeMeta::new();

        {
            let mut tree = BTree::new(&mut meta, &mut cache);
            for i in 0..10 {
                let key = Key::from(format!("key{i:02}"));
                let value = Bytes::from(format!("value{i}"));
                tree.put(key, value, Offset::new(i as u64)).unwrap();
            }
        }

        {
            let mut tree = BTree::new(&mut meta, &mut cache);
            for i in 0..10 {
                let key = Key::from(format!("key{i:02}"));
                let expected = Bytes::from(format!("value{i}"));
                assert_eq!(tree.get(&key).unwrap(), Some(expected));
            }
        }
    }

    #[test]
    fn test_mvcc_get_at() {
        let (_dir, mut cache) = create_cache();
        let mut meta = BTreeMeta::new();

        let key = Key::from("mvcc-key");

        {
            let mut tree = BTree::new(&mut meta, &mut cache);

            // Insert v1 at position 1
            tree.put(key.clone(), Bytes::from("v1"), Offset::new(1))
                .unwrap();

            // Insert v2 at position 5
            tree.put(key.clone(), Bytes::from("v2"), Offset::new(5))
                .unwrap();

            // Insert v3 at position 10
            tree.put(key.clone(), Bytes::from("v3"), Offset::new(10))
                .unwrap();

            // Current should be v3
            assert_eq!(tree.get(&key).unwrap(), Some(Bytes::from("v3")));

            // Point-in-time queries
            assert_eq!(tree.get_at(&key, Offset::new(0)).unwrap(), None);
            assert_eq!(
                tree.get_at(&key, Offset::new(1)).unwrap(),
                Some(Bytes::from("v1"))
            );
            assert_eq!(
                tree.get_at(&key, Offset::new(3)).unwrap(),
                Some(Bytes::from("v1"))
            );
            assert_eq!(
                tree.get_at(&key, Offset::new(5)).unwrap(),
                Some(Bytes::from("v2"))
            );
            assert_eq!(
                tree.get_at(&key, Offset::new(8)).unwrap(),
                Some(Bytes::from("v2"))
            );
            assert_eq!(
                tree.get_at(&key, Offset::new(10)).unwrap(),
                Some(Bytes::from("v3"))
            );
            assert_eq!(
                tree.get_at(&key, Offset::new(100)).unwrap(),
                Some(Bytes::from("v3"))
            );
        }
    }

    #[test]
    fn test_delete() {
        let (_dir, mut cache) = create_cache();
        let mut meta = BTreeMeta::new();

        {
            let mut tree = BTree::new(&mut meta, &mut cache);

            tree.put(Key::from("key"), Bytes::from("value"), Offset::new(1))
                .unwrap();
            assert_eq!(
                tree.get(&Key::from("key")).unwrap(),
                Some(Bytes::from("value"))
            );

            tree.delete(&Key::from("key"), Offset::new(5)).unwrap();
            assert_eq!(tree.get(&Key::from("key")).unwrap(), None);

            // But visible at old positions
            assert_eq!(
                tree.get_at(&Key::from("key"), Offset::new(3)).unwrap(),
                Some(Bytes::from("value"))
            );
        }
    }

    #[test]
    fn test_scan_range() {
        let (_dir, mut cache) = create_cache();
        let mut meta = BTreeMeta::new();

        {
            let mut tree = BTree::new(&mut meta, &mut cache);
            for i in 0..20 {
                let key = Key::from(format!("key{i:02}"));
                let value = Bytes::from(format!("value{i}"));
                tree.put(key, value, Offset::new(i as u64)).unwrap();
            }

            let results = tree
                .scan(Key::from("key05")..Key::from("key10"), 100)
                .unwrap();

            assert_eq!(results.len(), 5);
            assert_eq!(results[0].0, Key::from("key05"));
            assert_eq!(results[4].0, Key::from("key09"));
        }
    }

    #[test]
    fn test_node_splitting() {
        let (_dir, mut cache) = create_cache();
        let mut meta = BTreeMeta::new();

        {
            let mut tree = BTree::new(&mut meta, &mut cache);

            // Insert enough keys to trigger splits
            for i in 0..50 {
                let key = Key::from(format!("key{i:03}"));
                let value = Bytes::from(format!("value{i}"));
                tree.put(key, value, Offset::new(i as u64)).unwrap();
            }

            // Tree should have grown
            assert!(tree.height() >= 1);

            // All keys should still be findable
            for i in 0..50 {
                let key = Key::from(format!("key{i:03}"));
                let expected = Bytes::from(format!("value{i}"));
                assert_eq!(
                    tree.get(&key).unwrap(),
                    Some(expected),
                    "failed for key{i:03}"
                );
            }
        }
    }
}
