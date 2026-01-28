//! Page cache with LRU eviction and page-aligned I/O.
//!
//! The [`PageCache`] provides:
//! - In-memory caching of frequently accessed pages
//! - LRU eviction when cache is full
//! - Page-aligned reads and writes for optimal disk performance
//! - Dirty page tracking for efficient sync

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::StoreError;
use crate::page::{Page, PageType};
use crate::types::{PageId, PAGE_SIZE};

/// Default cache capacity in pages (16MB with 4KB pages).
const DEFAULT_CACHE_CAPACITY: usize = 4096;

/// A node in the LRU doubly-linked list.
struct LruNode {
    #[allow(dead_code)]
    page_id: PageId,
    prev: Option<PageId>,
    next: Option<PageId>,
}

/// Page cache with LRU eviction policy.
///
/// # Design
///
/// The cache maintains:
/// - A `HashMap` for O(1) page lookups
/// - A doubly-linked list for O(1) LRU tracking
/// - A dirty set for efficient sync
///
/// When the cache is full, the least recently used clean page is evicted.
/// Dirty pages are written to disk before eviction.
pub struct PageCache {
    /// Cached pages indexed by page ID.
    pages: HashMap<PageId, Page>,
    /// LRU tracking: `page_id` -> (prev, next) in LRU order.
    lru: HashMap<PageId, LruNode>,
    /// Head of LRU list (most recently used).
    lru_head: Option<PageId>,
    /// Tail of LRU list (least recently used).
    lru_tail: Option<PageId>,
    /// Maximum number of pages to cache.
    capacity: usize,
    /// The backing file for page storage.
    file: File,
    /// Next page ID to allocate.
    next_page_id: PageId,
}

impl PageCache {
    /// Opens or creates a page cache backed by the given file.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the backing file
    /// * `capacity` - Maximum number of pages to cache (None = default 4096)
    pub fn open(path: &Path, capacity: Option<usize>) -> Result<Self, StoreError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        let file_len = file.metadata()?.len();
        let next_page_id = if file_len == 0 {
            PageId::new(0)
        } else {
            PageId::new(file_len / PAGE_SIZE as u64)
        };

        Ok(Self {
            pages: HashMap::new(),
            lru: HashMap::new(),
            lru_head: None,
            lru_tail: None,
            capacity: capacity.unwrap_or(DEFAULT_CACHE_CAPACITY),
            file,
            next_page_id,
        })
    }

    /// Returns the next page ID that will be allocated.
    pub fn next_page_id(&self) -> PageId {
        self.next_page_id
    }

    /// Allocates a new page with the given type.
    ///
    /// The page is created in memory and marked dirty. It will be written
    /// to disk on the next `sync()` call.
    pub fn allocate(&mut self, page_type: PageType) -> Result<PageId, StoreError> {
        let page_id = self.next_page_id;
        self.next_page_id = self.next_page_id.next();

        let page = Page::new(page_id, page_type);
        self.insert_page(page)?;

        Ok(page_id)
    }

    /// Gets a page by ID, loading from disk if necessary.
    ///
    /// Returns `None` if the page doesn't exist.
    pub fn get(&mut self, page_id: PageId) -> Result<Option<&Page>, StoreError> {
        // Check if in cache
        if self.pages.contains_key(&page_id) {
            self.touch(page_id);
            return Ok(self.pages.get(&page_id));
        }

        // Try to load from disk
        if page_id.as_u64() >= self.next_page_id.as_u64() {
            return Ok(None);
        }

        let page = self.load_page(page_id)?;
        self.insert_page(page)?;

        Ok(self.pages.get(&page_id))
    }

    /// Gets a mutable reference to a page.
    ///
    /// The page is automatically marked as dirty when mutably borrowed.
    pub fn get_mut(&mut self, page_id: PageId) -> Result<Option<&mut Page>, StoreError> {
        // Ensure page is loaded
        if !self.pages.contains_key(&page_id) {
            if page_id.as_u64() >= self.next_page_id.as_u64() {
                return Ok(None);
            }
            let page = self.load_page(page_id)?;
            self.insert_page(page)?;
        }

        self.touch(page_id);
        Ok(self.pages.get_mut(&page_id))
    }

    /// Loads a page from disk.
    fn load_page(&mut self, page_id: PageId) -> Result<Page, StoreError> {
        let mut buf = [0u8; PAGE_SIZE];

        self.file.seek(SeekFrom::Start(page_id.byte_offset()))?;
        self.file.read_exact(&mut buf)?;

        Page::from_bytes(page_id, &buf)
    }

    /// Reads raw bytes from a page without validation.
    ///
    /// Used for special pages like the superblock that have custom formats.
    pub fn read_raw(&mut self, page_id: PageId) -> Result<[u8; PAGE_SIZE], StoreError> {
        let mut buf = [0u8; PAGE_SIZE];

        self.file.seek(SeekFrom::Start(page_id.byte_offset()))?;
        self.file.read_exact(&mut buf)?;

        Ok(buf)
    }

    /// Inserts a page into the cache, evicting if necessary.
    fn insert_page(&mut self, page: Page) -> Result<(), StoreError> {
        let page_id = page.id;

        // Evict if at capacity
        if self.pages.len() >= self.capacity {
            self.evict_one()?;
        }

        self.pages.insert(page_id, page);
        self.add_to_lru(page_id);

        Ok(())
    }

    /// Moves a page to the front of the LRU list.
    fn touch(&mut self, page_id: PageId) {
        if self.lru_head == Some(page_id) {
            return; // Already at front
        }

        self.remove_from_lru(page_id);
        self.add_to_lru(page_id);
    }

    /// Adds a page to the front of the LRU list.
    fn add_to_lru(&mut self, page_id: PageId) {
        let node = LruNode {
            page_id,
            prev: None,
            next: self.lru_head,
        };

        if let Some(old_head) = self.lru_head {
            if let Some(head_node) = self.lru.get_mut(&old_head) {
                head_node.prev = Some(page_id);
            }
        }

        self.lru.insert(page_id, node);
        self.lru_head = Some(page_id);

        if self.lru_tail.is_none() {
            self.lru_tail = Some(page_id);
        }
    }

    /// Removes a page from the LRU list.
    fn remove_from_lru(&mut self, page_id: PageId) {
        let Some(node) = self.lru.remove(&page_id) else {
            return;
        };

        // Update previous node's next pointer
        if let Some(prev_id) = node.prev {
            if let Some(prev_node) = self.lru.get_mut(&prev_id) {
                prev_node.next = node.next;
            }
        } else {
            // This was the head
            self.lru_head = node.next;
        }

        // Update next node's prev pointer
        if let Some(next_id) = node.next {
            if let Some(next_node) = self.lru.get_mut(&next_id) {
                next_node.prev = node.prev;
            }
        } else {
            // This was the tail
            self.lru_tail = node.prev;
        }
    }

    /// Evicts the least recently used page.
    fn evict_one(&mut self) -> Result<(), StoreError> {
        // Find the LRU page (tail of list)
        let Some(page_id) = self.lru_tail else {
            return Ok(()); // Cache is empty
        };

        // Write if dirty before evicting
        if let Some(page) = self.pages.get_mut(&page_id) {
            if page.is_dirty() {
                self.file.seek(SeekFrom::Start(page.id.byte_offset()))?;
                self.file.write_all(page.as_bytes())?;
            }
        }

        // Remove from cache and LRU
        self.pages.remove(&page_id);
        self.remove_from_lru(page_id);

        Ok(())
    }

    /// Syncs all dirty pages to disk.
    pub fn sync(&mut self) -> Result<(), StoreError> {
        // Write each dirty page directly
        for page in self.pages.values_mut() {
            if page.is_dirty() {
                let page_offset = page.id.byte_offset();
                let bytes = page.as_bytes();

                self.file.seek(SeekFrom::Start(page_offset))?;
                self.file.write_all(bytes)?;

                page.mark_clean();
            }
        }

        // Ensure all writes are durable
        self.file.sync_all()?;

        Ok(())
    }

    /// Returns the number of pages currently cached.
    #[allow(dead_code)]
    pub fn cached_count(&self) -> usize {
        self.pages.len()
    }

    /// Returns the number of dirty pages in the cache.
    #[allow(dead_code)]
    pub fn dirty_count(&self) -> usize {
        self.pages.values().filter(|p| p.is_dirty()).count()
    }

    /// Flushes and removes all pages from the cache.
    #[allow(dead_code)]
    pub fn flush(&mut self) -> Result<(), StoreError> {
        self.sync()?;
        self.pages.clear();
        self.lru.clear();
        self.lru_head = None;
        self.lru_tail = None;
        Ok(())
    }

    /// Prefetches a range of pages into the cache.
    ///
    /// This is useful when scanning a range of pages.
    #[allow(dead_code)]
    pub fn prefetch(&mut self, start: PageId, count: usize) -> Result<(), StoreError> {
        for i in 0..count {
            let page_id = PageId::new(start.as_u64() + i as u64);
            if page_id.as_u64() < self.next_page_id.as_u64() && !self.pages.contains_key(&page_id) {
                let page = self.load_page(page_id)?;
                self.insert_page(page)?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for PageCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageCache")
            .field("cached", &self.pages.len())
            .field("capacity", &self.capacity)
            .field("next_page_id", &self.next_page_id)
            .field("dirty", &self.dirty_count())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_allocate_and_get() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");

        let mut cache = PageCache::open(&path, Some(10)).unwrap();

        let page_id = cache.allocate(PageType::Leaf).unwrap();
        assert_eq!(page_id, PageId::new(0));

        let page = cache.get(page_id).unwrap().unwrap();
        assert_eq!(page.page_type(), PageType::Leaf);
        assert_eq!(page.item_count(), 0);
    }

    #[test]
    fn test_sync_and_reload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");

        // Create and populate
        {
            let mut cache = PageCache::open(&path, Some(10)).unwrap();
            let page_id = cache.allocate(PageType::Leaf).unwrap();

            let page = cache.get_mut(page_id).unwrap().unwrap();
            page.insert_item(0, b"test data").unwrap();

            cache.sync().unwrap();
        }

        // Reload and verify
        {
            let mut cache = PageCache::open(&path, Some(10)).unwrap();
            let page = cache.get(PageId::new(0)).unwrap().unwrap();
            assert_eq!(page.get_item(0), b"test data");
        }
    }

    #[test]
    fn test_lru_eviction() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.db");

        let mut cache = PageCache::open(&path, Some(3)).unwrap();

        // Allocate 4 pages (capacity is 3)
        let _p0 = cache.allocate(PageType::Leaf).unwrap();
        let _p1 = cache.allocate(PageType::Leaf).unwrap();
        let _p2 = cache.allocate(PageType::Leaf).unwrap();

        assert_eq!(cache.cached_count(), 3);

        // This should evict the oldest page
        let _p3 = cache.allocate(PageType::Leaf).unwrap();

        assert_eq!(cache.cached_count(), 3);
    }
}
