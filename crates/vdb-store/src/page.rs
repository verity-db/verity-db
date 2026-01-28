//! Page-based storage with 4KB pages and CRC32 integrity checks.
//!
//! # Page Layout
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │ Header (32 bytes)                                               │
//! │ ┌─────────────┬─────────┬──────────┬────────────┬─────────────┐ │
//! │ │ Magic (4B)  │ Ver (1B)│ Type (1B)│ Items (2B) │ Free off(2B)│ │
//! │ │             │         │          │            │             │ │
//! │ │ Reserved (22B)                                              │ │
//! │ └─────────────┴─────────┴──────────┴────────────┴─────────────┘ │
//! ├─────────────────────────────────────────────────────────────────┤
//! │ Slot Directory (grows downward from header)                     │
//! │ Each slot: offset (2B) + length (2B) = 4 bytes                  │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                        Free Space                               │
//! ├─────────────────────────────────────────────────────────────────┤
//! │ Data Area (grows upward from bottom)                            │
//! │ Items are stored here, referenced by slot directory             │
//! ├─────────────────────────────────────────────────────────────────┤
//! │ CRC32 (4 bytes at offset PAGE_SIZE - 4)                         │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The slot directory and data area grow toward each other. When they meet,
//! the page is full and must be split.

use crate::error::StoreError;
use crate::types::{PageId, CRC_SIZE, PAGE_HEADER_SIZE, PAGE_SIZE};

// ============================================================================
// Constants
// ============================================================================

/// Magic bytes identifying a valid page: "VDPG"
const PAGE_MAGIC: [u8; 4] = *b"VDPG";

/// Current page format version.
const PAGE_VERSION: u8 = 1;

/// Size of each slot directory entry (offset + length).
const SLOT_SIZE: usize = 4;

/// Offset where the CRC32 is stored (end of page).
const CRC_OFFSET: usize = PAGE_SIZE - CRC_SIZE;

// ============================================================================
// Page Type
// ============================================================================

/// Type of page in the B+tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PageType {
    /// Leaf node containing key-value pairs.
    Leaf = 0,
    /// Internal node containing keys and child pointers.
    Internal = 1,
    /// Free page (available for allocation).
    Free = 2,
}

impl PageType {
    /// Creates a `PageType` from its byte representation.
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Leaf),
            1 => Some(Self::Internal),
            2 => Some(Self::Free),
            _ => None,
        }
    }

    /// Returns the byte representation.
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

// ============================================================================
// Page Header
// ============================================================================

/// Header at the start of each page.
#[derive(Debug, Clone, Copy)]
pub struct PageHeader {
    /// Page type (leaf, internal, free).
    pub page_type: PageType,
    /// Number of items in this page.
    pub item_count: u16,
    /// Offset where free space begins (end of slot directory).
    pub free_offset: u16,
}

impl PageHeader {
    /// Creates a new header for an empty page.
    pub fn new(page_type: PageType) -> Self {
        Self {
            page_type,
            item_count: 0,
            // Free space starts right after the header
            free_offset: PAGE_HEADER_SIZE as u16,
        }
    }

    /// Serializes the header to bytes.
    pub fn serialize(self, buf: &mut [u8; PAGE_HEADER_SIZE]) {
        buf[0..4].copy_from_slice(&PAGE_MAGIC);
        buf[4] = PAGE_VERSION;
        buf[5] = self.page_type.as_byte();
        buf[6..8].copy_from_slice(&self.item_count.to_le_bytes());
        buf[8..10].copy_from_slice(&self.free_offset.to_le_bytes());
        // Reserved bytes (10..32) are already zeroed
        buf[10..32].fill(0);
    }

    /// Deserializes the header from bytes.
    pub fn deserialize(buf: &[u8; PAGE_HEADER_SIZE]) -> Result<Self, StoreError> {
        // Validate magic
        let magic: [u8; 4] = buf[0..4]
            .try_into()
            .expect("slice length equals 4 after bounds check");
        if magic != PAGE_MAGIC {
            return Err(StoreError::InvalidPageMagic {
                expected: u32::from_le_bytes(PAGE_MAGIC),
                actual: u32::from_le_bytes(magic),
            });
        }

        // Validate version
        let version = buf[4];
        if version != PAGE_VERSION {
            return Err(StoreError::UnsupportedPageVersion(version));
        }

        // Parse page type
        let page_type = PageType::from_byte(buf[5]).ok_or(StoreError::UnsupportedPageVersion(buf[5]))?;

        let item_count = u16::from_le_bytes(buf[6..8].try_into().unwrap());
        let free_offset = u16::from_le_bytes(buf[8..10].try_into().unwrap());

        Ok(Self {
            page_type,
            item_count,
            free_offset,
        })
    }
}

// ============================================================================
// Slot Directory Entry
// ============================================================================

/// Entry in the slot directory pointing to an item.
#[derive(Debug, Clone, Copy, Default)]
pub struct Slot {
    /// Offset from start of page where item data begins.
    pub offset: u16,
    /// Length of item data in bytes.
    pub length: u16,
}

impl Slot {
    /// Creates a new slot.
    pub fn new(offset: u16, length: u16) -> Self {
        Self { offset, length }
    }

    /// Serializes the slot to bytes.
    pub fn serialize(self) -> [u8; SLOT_SIZE] {
        let mut buf = [0u8; SLOT_SIZE];
        buf[0..2].copy_from_slice(&self.offset.to_le_bytes());
        buf[2..4].copy_from_slice(&self.length.to_le_bytes());
        buf
    }

    /// Deserializes the slot from bytes.
    pub fn deserialize(buf: [u8; SLOT_SIZE]) -> Self {
        Self {
            offset: u16::from_le_bytes(buf[0..2].try_into().unwrap()),
            length: u16::from_le_bytes(buf[2..4].try_into().unwrap()),
        }
    }
}

// ============================================================================
// Page
// ============================================================================

/// A 4KB page with header, slot directory, and data area.
///
/// # Invariants
///
/// - `data.len() == PAGE_SIZE`
/// - CRC32 at `data[CRC_OFFSET..CRC_OFFSET+4]` matches computed CRC
/// - `free_offset >= PAGE_HEADER_SIZE`
/// - `free_offset + slot_directory_size <= data_start`
/// - All slot offsets point to valid data within the page
#[derive(Clone)]
pub struct Page {
    /// The page's unique identifier.
    pub id: PageId,
    /// The raw page data (exactly 4KB).
    data: [u8; PAGE_SIZE],
    /// Cached header for fast access.
    header: PageHeader,
    /// True if the page has been modified since last sync.
    dirty: bool,
    /// True if this page contains raw data (e.g., superblock) that shouldn't
    /// have its CRC updated by `as_bytes()`.
    is_raw: bool,
}

impl Page {
    /// Creates a new empty page with the given type.
    pub fn new(id: PageId, page_type: PageType) -> Self {
        let mut data = [0u8; PAGE_SIZE];
        let header = PageHeader::new(page_type);

        // Serialize header
        let mut header_buf = [0u8; PAGE_HEADER_SIZE];
        header.serialize(&mut header_buf);
        data[..PAGE_HEADER_SIZE].copy_from_slice(&header_buf);

        let mut page = Self {
            id,
            data,
            header,
            dirty: true,
            is_raw: false,
        };

        // Compute and store CRC
        page.update_crc();
        page
    }

    /// Loads a page from raw bytes, validating CRC.
    pub fn from_bytes(id: PageId, data: &[u8; PAGE_SIZE]) -> Result<Self, StoreError> {
        // Verify CRC first
        let stored_crc = u32::from_le_bytes(
            data[CRC_OFFSET..CRC_OFFSET + CRC_SIZE]
                .try_into()
                .unwrap(),
        );
        let computed_crc = crc32fast::hash(&data[..CRC_OFFSET]);

        if stored_crc != computed_crc {
            return Err(StoreError::PageCorrupted {
                page_id: id,
                expected: stored_crc,
                actual: computed_crc,
            });
        }

        // Parse header
        let header_bytes: [u8; PAGE_HEADER_SIZE] = data[..PAGE_HEADER_SIZE]
            .try_into()
            .expect("slice length equals PAGE_HEADER_SIZE");
        let header = PageHeader::deserialize(&header_bytes)?;

        Ok(Self {
            id,
            data: *data,
            header,
            dirty: false,
            is_raw: false,
        })
    }

    /// Returns the page type.
    pub fn page_type(&self) -> PageType {
        self.header.page_type
    }

    /// Returns the number of items in this page.
    pub fn item_count(&self) -> usize {
        self.header.item_count as usize
    }

    /// Returns true if the page has been modified.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Marks the page as clean (after syncing to disk).
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Returns the amount of free space available for new items.
    pub fn free_space(&self) -> usize {
        let slot_dir_end = self.slot_directory_end();
        let data_start = self.data_area_start();
        data_start.saturating_sub(slot_dir_end)
    }

    /// Returns where the slot directory ends.
    fn slot_directory_end(&self) -> usize {
        PAGE_HEADER_SIZE + (self.header.item_count as usize * SLOT_SIZE)
    }

    /// Returns where the data area starts (grows from bottom up).
    fn data_area_start(&self) -> usize {
        if self.header.item_count == 0 {
            CRC_OFFSET
        } else {
            // Find the lowest offset in the slot directory
            let mut min_offset = CRC_OFFSET;
            for i in 0..self.header.item_count as usize {
                let slot = self.get_slot(i);
                min_offset = min_offset.min(slot.offset as usize);
            }
            min_offset
        }
    }

    /// Gets the slot at the given index.
    pub fn get_slot(&self, index: usize) -> Slot {
        debug_assert!(index < self.header.item_count as usize, "slot index out of bounds");
        let offset = PAGE_HEADER_SIZE + (index * SLOT_SIZE);
        let slot_bytes: [u8; SLOT_SIZE] = self.data[offset..offset + SLOT_SIZE]
            .try_into()
            .unwrap();
        Slot::deserialize(slot_bytes)
    }

    /// Gets the data for the item at the given index.
    pub fn get_item(&self, index: usize) -> &[u8] {
        let slot = self.get_slot(index);
        &self.data[slot.offset as usize..(slot.offset + slot.length) as usize]
    }

    /// Inserts an item at the given slot index.
    ///
    /// Shifts existing slots to make room if needed.
    ///
    /// # Errors
    ///
    /// Returns `PageOverflow` if there isn't enough space.
    pub fn insert_item(&mut self, index: usize, data: &[u8]) -> Result<(), StoreError> {
        let needed = SLOT_SIZE + data.len();
        let available = self.free_space();

        if needed > available {
            return Err(StoreError::PageOverflow { needed, available });
        }

        // Allocate space for data (grow from bottom up)
        let data_offset = self.data_area_start() - data.len();

        // Shift existing slots if inserting in the middle
        let item_count = self.header.item_count as usize;
        if index < item_count {
            // Move slots [index..] one position right
            let src_start = PAGE_HEADER_SIZE + (index * SLOT_SIZE);
            let src_end = PAGE_HEADER_SIZE + (item_count * SLOT_SIZE);
            let dst_start = src_start + SLOT_SIZE;
            self.data.copy_within(src_start..src_end, dst_start);
        }

        // Write the new slot
        let slot = Slot::new(data_offset as u16, data.len() as u16);
        let slot_offset = PAGE_HEADER_SIZE + (index * SLOT_SIZE);
        self.data[slot_offset..slot_offset + SLOT_SIZE].copy_from_slice(&slot.serialize());

        // Write the data
        self.data[data_offset..data_offset + data.len()].copy_from_slice(data);

        // Update header
        self.header.item_count += 1;
        self.header.free_offset = (PAGE_HEADER_SIZE + (self.header.item_count as usize * SLOT_SIZE)) as u16;
        self.sync_header();
        self.dirty = true;

        // Postcondition: item count increased
        debug_assert_eq!(self.header.item_count as usize, item_count + 1);

        Ok(())
    }

    /// Removes the item at the given slot index.
    ///
    /// Note: This doesn't reclaim the data space. Use `compact()` to defragment.
    pub fn remove_item(&mut self, index: usize) {
        debug_assert!(index < self.header.item_count as usize, "slot index out of bounds");

        let item_count = self.header.item_count as usize;

        // Shift slots [index+1..] one position left
        if index < item_count - 1 {
            let src_start = PAGE_HEADER_SIZE + ((index + 1) * SLOT_SIZE);
            let src_end = PAGE_HEADER_SIZE + (item_count * SLOT_SIZE);
            let dst_start = PAGE_HEADER_SIZE + (index * SLOT_SIZE);
            self.data.copy_within(src_start..src_end, dst_start);
        }

        // Update header
        self.header.item_count -= 1;
        self.header.free_offset = (PAGE_HEADER_SIZE + (self.header.item_count as usize * SLOT_SIZE)) as u16;
        self.sync_header();
        self.dirty = true;
    }

    /// Updates the item at the given index with new data.
    ///
    /// If the new data fits in the old slot, it's updated in place.
    /// Otherwise, returns `PageOverflow`.
    #[allow(dead_code)]
    pub fn update_item(&mut self, index: usize, data: &[u8]) -> Result<(), StoreError> {
        let slot = self.get_slot(index);
        let old_len = slot.length as usize;

        if data.len() <= old_len {
            // Update in place (may waste some space)
            self.data[slot.offset as usize..slot.offset as usize + data.len()]
                .copy_from_slice(data);

            // Update slot length if data is smaller
            if data.len() < old_len {
                let new_slot = Slot::new(slot.offset, data.len() as u16);
                let slot_offset = PAGE_HEADER_SIZE + (index * SLOT_SIZE);
                self.data[slot_offset..slot_offset + SLOT_SIZE].copy_from_slice(&new_slot.serialize());
            }

            self.dirty = true;
            Ok(())
        } else {
            // Need more space - check if we can allocate it
            let additional = data.len() - old_len;
            if additional > self.free_space() {
                return Err(StoreError::PageOverflow {
                    needed: additional,
                    available: self.free_space(),
                });
            }

            // Allocate new space and update slot
            let new_offset = self.data_area_start() - data.len();
            self.data[new_offset..new_offset + data.len()].copy_from_slice(data);

            let new_slot = Slot::new(new_offset as u16, data.len() as u16);
            let slot_offset = PAGE_HEADER_SIZE + (index * SLOT_SIZE);
            self.data[slot_offset..slot_offset + SLOT_SIZE].copy_from_slice(&new_slot.serialize());

            self.dirty = true;
            Ok(())
        }
    }

    /// Syncs the header to the data array.
    fn sync_header(&mut self) {
        let mut header_buf = [0u8; PAGE_HEADER_SIZE];
        self.header.serialize(&mut header_buf);
        self.data[..PAGE_HEADER_SIZE].copy_from_slice(&header_buf);
    }

    /// Updates the CRC32 checksum.
    pub fn update_crc(&mut self) {
        self.sync_header();
        let crc = crc32fast::hash(&self.data[..CRC_OFFSET]);
        self.data[CRC_OFFSET..CRC_OFFSET + CRC_SIZE].copy_from_slice(&crc.to_le_bytes());
    }

    /// Returns the raw page data for writing to disk.
    ///
    /// Automatically updates CRC before returning (unless this is a raw data page).
    pub fn as_bytes(&mut self) -> &[u8; PAGE_SIZE] {
        if !self.is_raw {
            self.update_crc();
        }
        &self.data
    }

    /// Returns the raw page data without updating CRC (for reading).
    #[allow(dead_code)]
    pub fn data(&self) -> &[u8; PAGE_SIZE] {
        &self.data
    }

    /// Overwrites the page with raw bytes (for special pages like superblock).
    ///
    /// This bypasses the normal page format and directly sets the raw bytes.
    /// Use only for pages with custom formats. The data will be written as-is
    /// without CRC updates.
    pub fn set_raw_data(&mut self, data: &[u8; PAGE_SIZE]) {
        self.data = *data;
        self.dirty = true;
        self.is_raw = true;
    }

    /// Compacts the page by defragmenting the data area.
    ///
    /// This reclaims space from removed items and gaps from updates.
    #[allow(dead_code)]
    pub fn compact(&mut self) {
        if self.header.item_count == 0 {
            return;
        }

        // Collect all items with their current data
        let items: Vec<Vec<u8>> = (0..self.header.item_count as usize)
            .map(|i| self.get_item(i).to_vec())
            .collect();

        // Clear data area
        self.data[PAGE_HEADER_SIZE + (self.header.item_count as usize * SLOT_SIZE)..CRC_OFFSET]
            .fill(0);

        // Rewrite items from the bottom up
        let mut data_offset = CRC_OFFSET;
        for (i, item) in items.iter().enumerate() {
            data_offset -= item.len();
            self.data[data_offset..data_offset + item.len()].copy_from_slice(item);

            // Update slot
            let slot = Slot::new(data_offset as u16, item.len() as u16);
            let slot_offset = PAGE_HEADER_SIZE + (i * SLOT_SIZE);
            self.data[slot_offset..slot_offset + SLOT_SIZE].copy_from_slice(&slot.serialize());
        }

        self.dirty = true;
    }
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("id", &self.id)
            .field("type", &self.header.page_type)
            .field("items", &self.header.item_count)
            .field("free_space", &self.free_space())
            .field("dirty", &self.dirty)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod page_tests {
    use super::*;

    #[test]
    fn test_new_page() {
        let page = Page::new(PageId::new(1), PageType::Leaf);
        assert_eq!(page.page_type(), PageType::Leaf);
        assert_eq!(page.item_count(), 0);
        assert!(page.is_dirty());
        assert!(page.free_space() > 0);
    }

    #[test]
    fn test_insert_and_get() {
        let mut page = Page::new(PageId::new(1), PageType::Leaf);

        page.insert_item(0, b"hello").unwrap();
        assert_eq!(page.item_count(), 1);
        assert_eq!(page.get_item(0), b"hello");

        page.insert_item(1, b"world").unwrap();
        assert_eq!(page.item_count(), 2);
        assert_eq!(page.get_item(0), b"hello");
        assert_eq!(page.get_item(1), b"world");
    }

    #[test]
    fn test_insert_at_beginning() {
        let mut page = Page::new(PageId::new(1), PageType::Leaf);

        page.insert_item(0, b"second").unwrap();
        page.insert_item(0, b"first").unwrap();

        assert_eq!(page.item_count(), 2);
        assert_eq!(page.get_item(0), b"first");
        assert_eq!(page.get_item(1), b"second");
    }

    #[test]
    fn test_remove_item() {
        let mut page = Page::new(PageId::new(1), PageType::Leaf);

        page.insert_item(0, b"a").unwrap();
        page.insert_item(1, b"b").unwrap();
        page.insert_item(2, b"c").unwrap();

        page.remove_item(1); // Remove "b"

        assert_eq!(page.item_count(), 2);
        assert_eq!(page.get_item(0), b"a");
        assert_eq!(page.get_item(1), b"c");
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut page = Page::new(PageId::new(42), PageType::Internal);
        page.insert_item(0, b"test data").unwrap();

        let bytes = *page.as_bytes();
        let loaded = Page::from_bytes(PageId::new(42), &bytes).unwrap();

        assert_eq!(loaded.page_type(), PageType::Internal);
        assert_eq!(loaded.item_count(), 1);
        assert_eq!(loaded.get_item(0), b"test data");
    }

    #[test]
    fn test_crc_corruption_detection() {
        let mut page = Page::new(PageId::new(1), PageType::Leaf);
        page.insert_item(0, b"data").unwrap();

        let mut bytes = *page.as_bytes();
        // Corrupt one byte
        bytes[100] ^= 0xFF;

        let result = Page::from_bytes(PageId::new(1), &bytes);
        assert!(matches!(result, Err(StoreError::PageCorrupted { .. })));
    }
}
