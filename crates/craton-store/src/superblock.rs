//! Superblock for store metadata persistence.
//!
//! The superblock is stored in page 0 and contains:
//! - Magic bytes for identification
//! - Applied position (last log offset processed)
//! - Applied hash (log hash at that position)
//! - Table roots (mapping from `TableId` to B+tree root pages)
//! - Next available page ID
//!
//! # Format
//!
//! ```text
//! ┌───────────────────────────────────────────────────────────────┐
//! │ Magic (8 bytes): "VDBSTORE"                                   │
//! │ Version (4 bytes): u32 LE                                     │
//! │ Applied Position (8 bytes): u64 LE                            │
//! │ Applied Hash (32 bytes): SHA-256                              │
//! │ Next Page ID (8 bytes): u64 LE                                │
//! │ Table Count (4 bytes): u32 LE                                 │
//! │ Tables: [TableId (8B), Root PageId (8B), Height (4B)] * count │
//! │ Padding to page size - 4                                      │
//! │ CRC32 (4 bytes): u32 LE                                       │
//! └───────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;

use craton_types::{HASH_LENGTH, Hash, Offset};

use crate::btree::BTreeMeta;
use crate::error::StoreError;
use crate::types::{CRC_SIZE, PAGE_SIZE, PageId, TableId};

// ============================================================================
// Constants
// ============================================================================

/// Magic bytes identifying a valid superblock.
const SUPERBLOCK_MAGIC: &[u8; 8] = b"VDBSTORE";

/// Current superblock format version.
const SUPERBLOCK_VERSION: u32 = 1;

/// Header size before table entries.
#[allow(dead_code)]
const HEADER_SIZE: usize = 8 + 4 + 8 + HASH_LENGTH + 8 + 4; // 64 bytes

/// Size of each table entry.
#[allow(dead_code)]
const TABLE_ENTRY_SIZE: usize = 8 + 8 + 4; // TableId + PageId + Height = 20 bytes

// ============================================================================
// Superblock
// ============================================================================

/// Store metadata persisted in page 0.
#[derive(Debug, Clone)]
pub struct Superblock {
    /// Last applied log position.
    pub applied_position: Offset,
    /// Hash at the applied position for verification.
    pub applied_hash: Hash,
    /// Next page ID to allocate.
    pub next_page_id: PageId,
    /// Table ID to B+tree metadata mapping.
    pub tables: HashMap<TableId, BTreeMeta>,
}

impl Superblock {
    /// Creates a new superblock for an empty store.
    pub fn new() -> Self {
        Self {
            applied_position: Offset::ZERO,
            applied_hash: Hash::GENESIS,
            next_page_id: PageId::new(1), // Page 0 is the superblock
            tables: HashMap::new(),
        }
    }

    /// Serializes the superblock to a page-sized buffer.
    pub fn serialize(&self) -> [u8; PAGE_SIZE] {
        let mut buf = [0u8; PAGE_SIZE];
        let mut offset = 0;

        // Magic
        buf[offset..offset + 8].copy_from_slice(SUPERBLOCK_MAGIC);
        offset += 8;

        // Version
        buf[offset..offset + 4].copy_from_slice(&SUPERBLOCK_VERSION.to_le_bytes());
        offset += 4;

        // Applied position
        buf[offset..offset + 8].copy_from_slice(&self.applied_position.as_u64().to_le_bytes());
        offset += 8;

        // Applied hash
        buf[offset..offset + HASH_LENGTH].copy_from_slice(self.applied_hash.as_bytes());
        offset += HASH_LENGTH;

        // Next page ID
        buf[offset..offset + 8].copy_from_slice(&self.next_page_id.as_u64().to_le_bytes());
        offset += 8;

        // Table count
        buf[offset..offset + 4].copy_from_slice(&(self.tables.len() as u32).to_le_bytes());
        offset += 4;

        // Table entries
        for (table_id, meta) in &self.tables {
            buf[offset..offset + 8].copy_from_slice(&table_id.as_u64().to_le_bytes());
            offset += 8;

            let root_id = meta.root.map_or(u64::MAX, PageId::as_u64);
            buf[offset..offset + 8].copy_from_slice(&root_id.to_le_bytes());
            offset += 8;

            buf[offset..offset + 4].copy_from_slice(&(meta.height as u32).to_le_bytes());
            offset += 4;
        }

        // CRC32 at the end
        let crc = crc32fast::hash(&buf[..PAGE_SIZE - CRC_SIZE]);
        buf[PAGE_SIZE - CRC_SIZE..].copy_from_slice(&crc.to_le_bytes());

        buf
    }

    /// Deserializes a superblock from a page-sized buffer.
    pub fn deserialize(buf: &[u8; PAGE_SIZE]) -> Result<Self, StoreError> {
        // Verify CRC first
        let stored_crc = u32::from_le_bytes(buf[PAGE_SIZE - CRC_SIZE..].try_into().unwrap());
        let computed_crc = crc32fast::hash(&buf[..PAGE_SIZE - CRC_SIZE]);

        if stored_crc != computed_crc {
            return Err(StoreError::SuperblockCorrupted);
        }

        let mut offset = 0;

        // Magic
        let magic: [u8; 8] = buf[offset..offset + 8].try_into().unwrap();
        if &magic != SUPERBLOCK_MAGIC {
            return Err(StoreError::InvalidSuperblockMagic);
        }
        offset += 8;

        // Version
        let version = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap());
        if version != SUPERBLOCK_VERSION {
            return Err(StoreError::UnsupportedPageVersion(version as u8));
        }
        offset += 4;

        // Applied position
        let applied_position = Offset::new(u64::from_le_bytes(
            buf[offset..offset + 8].try_into().unwrap(),
        ));
        offset += 8;

        // Applied hash
        let applied_hash = Hash::from_bytes(buf[offset..offset + HASH_LENGTH].try_into().unwrap());
        offset += HASH_LENGTH;

        // Next page ID
        let next_page_id = PageId::new(u64::from_le_bytes(
            buf[offset..offset + 8].try_into().unwrap(),
        ));
        offset += 8;

        // Table count
        let table_count = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;

        // Table entries
        let mut tables = HashMap::with_capacity(table_count);
        for _ in 0..table_count {
            let table_id = TableId::new(u64::from_le_bytes(
                buf[offset..offset + 8].try_into().unwrap(),
            ));
            offset += 8;

            let root_id = u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap());
            let root = if root_id == u64::MAX {
                None
            } else {
                Some(PageId::new(root_id))
            };
            offset += 8;

            let height = u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
            offset += 4;

            let meta = BTreeMeta { root, height };
            tables.insert(table_id, meta);
        }

        Ok(Self {
            applied_position,
            applied_hash,
            next_page_id,
            tables,
        })
    }

    /// Returns the maximum number of tables that can fit in a superblock.
    #[allow(dead_code)]
    pub fn max_tables() -> usize {
        (PAGE_SIZE - HEADER_SIZE - CRC_SIZE) / TABLE_ENTRY_SIZE
    }
}

impl Default for Superblock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod superblock_tests {
    use super::*;

    #[test]
    fn test_empty_superblock_roundtrip() {
        let sb = Superblock::new();
        let bytes = sb.serialize();
        let loaded = Superblock::deserialize(&bytes).unwrap();

        assert_eq!(loaded.applied_position, Offset::ZERO);
        assert_eq!(loaded.applied_hash, Hash::GENESIS);
        assert_eq!(loaded.next_page_id, PageId::new(1));
        assert!(loaded.tables.is_empty());
    }

    #[test]
    fn test_superblock_with_tables() {
        let mut sb = Superblock::new();
        sb.applied_position = Offset::new(100);
        sb.next_page_id = PageId::new(50);

        sb.tables
            .insert(TableId::new(1), BTreeMeta::with_root(PageId::new(10), 2));
        sb.tables
            .insert(TableId::new(2), BTreeMeta::with_root(PageId::new(20), 3));
        sb.tables.insert(TableId::new(3), BTreeMeta::new()); // Empty tree

        let bytes = sb.serialize();
        let loaded = Superblock::deserialize(&bytes).unwrap();

        assert_eq!(loaded.applied_position, Offset::new(100));
        assert_eq!(loaded.next_page_id, PageId::new(50));
        assert_eq!(loaded.tables.len(), 3);

        let t1 = loaded.tables.get(&TableId::new(1)).unwrap();
        assert_eq!(t1.root, Some(PageId::new(10)));
        assert_eq!(t1.height, 2);

        let t3 = loaded.tables.get(&TableId::new(3)).unwrap();
        assert_eq!(t3.root, None);
    }

    #[test]
    fn test_superblock_corruption_detection() {
        let sb = Superblock::new();
        let mut bytes = sb.serialize();

        // Corrupt one byte
        bytes[50] ^= 0xFF;

        let result = Superblock::deserialize(&bytes);
        assert!(matches!(result, Err(StoreError::SuperblockCorrupted)));
    }

    #[test]
    fn test_max_tables() {
        // Should be able to fit many tables
        assert!(Superblock::max_tables() > 100);
    }
}
