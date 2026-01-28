//! Core types for the projection store.

use std::fmt::{self, Debug, Display};

use bytes::Bytes;

// ============================================================================
// Constants
// ============================================================================

/// Page size in bytes (4KB).
///
/// This is the fundamental unit of I/O and storage. All pages are exactly
/// this size, enabling:
/// - Page-aligned I/O for optimal disk performance
/// - Predictable memory allocation
/// - Simple free space management
pub const PAGE_SIZE: usize = 4096;

/// Maximum key length in bytes.
///
/// Keys must fit in a single page along with their value and overhead.
/// This limit ensures reasonable B+tree fanout.
pub const MAX_KEY_LENGTH: usize = 1024;

/// Maximum value length in bytes.
///
/// Values must fit in a page with key and overhead. For larger values,
/// applications should store references to external storage.
#[allow(dead_code)]
pub const MAX_VALUE_LENGTH: usize = PAGE_SIZE - 128; // Leave room for overhead

/// Page header size in bytes.
pub const PAGE_HEADER_SIZE: usize = 32;

/// CRC32 checksum size in bytes.
pub const CRC_SIZE: usize = 4;

/// Minimum B+tree order (minimum keys per node, except root).
pub const BTREE_MIN_KEYS: usize = 4;

// ============================================================================
// Page ID
// ============================================================================

/// Unique identifier for a page within the store.
///
/// Page 0 is always the superblock. Page IDs are assigned sequentially
/// as new pages are allocated.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PageId(u64);

impl PageId {
    /// The superblock page (always page 0).
    pub const SUPERBLOCK: PageId = PageId(0);

    /// Creates a new page ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the page ID as a u64.
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Returns the byte offset of this page in the file.
    pub fn byte_offset(self) -> u64 {
        self.0 * PAGE_SIZE as u64
    }

    /// Returns the next page ID.
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl Debug for PageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PageId({})", self.0)
    }
}

impl Display for PageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for PageId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

impl From<PageId> for u64 {
    fn from(id: PageId) -> Self {
        id.0
    }
}

// ============================================================================
// Table ID
// ============================================================================

/// Unique identifier for a table within the store.
///
/// Tables provide namespace isolation for keys. Different projections
/// can use different tables to avoid key conflicts.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TableId(u64);

impl TableId {
    /// Creates a new table ID.
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the table ID as a u64.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl Debug for TableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TableId({})", self.0)
    }
}

impl Display for TableId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for TableId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

impl From<TableId> for u64 {
    fn from(id: TableId) -> Self {
        id.0
    }
}

// ============================================================================
// Key
// ============================================================================

/// A key in the projection store.
///
/// Keys are arbitrary byte sequences up to [`MAX_KEY_LENGTH`] bytes.
/// They are compared lexicographically for B+tree ordering.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Key(Bytes);

impl Key {
    /// Creates a new key from bytes.
    ///
    /// # Panics
    ///
    /// Debug builds panic if the key exceeds [`MAX_KEY_LENGTH`].
    pub fn new(data: impl Into<Bytes>) -> Self {
        let bytes = data.into();
        debug_assert!(
            bytes.len() <= MAX_KEY_LENGTH,
            "key length {} exceeds maximum {}",
            bytes.len(),
            MAX_KEY_LENGTH
        );
        Self(bytes)
    }

    /// Creates a key from a static byte slice.
    pub fn from_static(data: &'static [u8]) -> Self {
        Self::new(Bytes::from_static(data))
    }

    /// Returns the key as a byte slice.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the length of the key in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the key is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the underlying Bytes.
    pub fn into_bytes(self) -> Bytes {
        self.0
    }

    /// The minimum possible key (empty).
    pub fn min() -> Self {
        Self(Bytes::new())
    }

    /// The maximum possible key (all 0xFF bytes at max length).
    pub fn max() -> Self {
        Self(Bytes::from(vec![0xFF; MAX_KEY_LENGTH]))
    }
}

impl Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Show first 16 bytes in hex for debugging
        write!(f, "Key(")?;
        for (i, byte) in self.0.iter().take(16).enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
            write!(f, "{byte:02x}")?;
        }
        if self.0.len() > 16 {
            write!(f, "...+{} more", self.0.len() - 16)?;
        }
        write!(f, ")")
    }
}

impl Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Try to display as UTF-8 string if valid, otherwise hex
        if let Ok(s) = std::str::from_utf8(&self.0) {
            if s.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                return write!(f, "{s}");
            }
        }
        // Fall back to hex
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl From<&[u8]> for Key {
    fn from(data: &[u8]) -> Self {
        Self::new(Bytes::copy_from_slice(data))
    }
}

impl From<Vec<u8>> for Key {
    fn from(data: Vec<u8>) -> Self {
        Self::new(Bytes::from(data))
    }
}

impl From<Bytes> for Key {
    fn from(data: Bytes) -> Self {
        Self::new(data)
    }
}

impl From<&str> for Key {
    fn from(s: &str) -> Self {
        Self::new(Bytes::copy_from_slice(s.as_bytes()))
    }
}

impl From<String> for Key {
    fn from(s: String) -> Self {
        Self::new(Bytes::from(s))
    }
}

impl AsRef<[u8]> for Key {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
