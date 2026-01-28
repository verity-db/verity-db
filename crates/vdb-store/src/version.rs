//! MVCC version tracking for point-in-time queries.
//!
//! Each row can have multiple versions, each tagged with the log position
//! where it was created and (optionally) deleted. This enables:
//!
//! - Point-in-time queries: find the version visible at any past position
//! - Current queries: find the version that hasn't been deleted yet
//! - Audit trails: know exactly which log entry created/updated a row

use bytes::Bytes;
use vdb_types::Offset;

/// A versioned row value with MVCC visibility tracking.
///
/// # Visibility Rules
///
/// A version is visible at position `pos` if:
/// - `created_at <= pos` (version exists at this position)
/// - `deleted_at > pos` (version hasn't been deleted yet)
///
/// For current queries, we use `deleted_at == Offset::MAX` to indicate
/// the version is still active.
///
/// # Invariants
///
/// - `created_at < deleted_at` (a version cannot be deleted before creation)
/// - At most one version is visible at any given position
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowVersion {
    /// Log position when this version was created.
    pub created_at: Offset,
    /// Log position when this version was deleted (MAX if current).
    pub deleted_at: Offset,
    /// The row data.
    pub data: Bytes,
}

/// Maximum offset value, used to mark versions that haven't been deleted.
/// We use `u64::MAX` since no valid log position can reach this value.
const NOT_DELETED_RAW: u64 = u64::MAX;

impl RowVersion {
    /// Returns the sentinel value for "not deleted" versions.
    #[inline]
    #[must_use]
    pub fn not_deleted() -> Offset {
        Offset::new(NOT_DELETED_RAW)
    }

    /// Creates a new version that is currently active.
    ///
    /// # Arguments
    ///
    /// * `created_at` - Log position when this version was created
    /// * `data` - The row data
    pub fn new(created_at: Offset, data: Bytes) -> Self {
        Self {
            created_at,
            deleted_at: Self::not_deleted(),
            data,
        }
    }

    /// Creates a new version with explicit deletion position.
    ///
    /// # Preconditions
    ///
    /// `created_at < deleted_at` (enforced by debug assertion)
    pub fn with_deletion(created_at: Offset, deleted_at: Offset, data: Bytes) -> Self {
        debug_assert!(
            created_at < deleted_at,
            "created_at {created_at} must be less than deleted_at {deleted_at}"
        );
        Self {
            created_at,
            deleted_at,
            data,
        }
    }

    /// Returns true if this version is visible at the given position.
    ///
    /// A version is visible if `created_at <= pos < deleted_at`.
    #[must_use]
    pub fn is_visible_at(&self, pos: Offset) -> bool {
        self.created_at <= pos && pos < self.deleted_at
    }

    /// Returns true if this version is currently active (not deleted).
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.deleted_at == Self::not_deleted()
    }

    /// Marks this version as deleted at the given position.
    ///
    /// # Preconditions
    ///
    /// - `pos > self.created_at` (cannot delete before creation)
    /// - `self.is_current()` (cannot delete an already deleted version)
    pub fn mark_deleted(&mut self, pos: Offset) {
        debug_assert!(
            pos > self.created_at,
            "deletion position {pos} must be after creation {}",
            self.created_at
        );
        debug_assert!(self.is_current(), "cannot delete an already deleted version");
        self.deleted_at = pos;
    }

    /// Returns the size of this version in bytes (for space calculations).
    #[must_use]
    pub fn size(&self) -> usize {
        // created_at (8) + deleted_at (8) + data length
        16 + self.data.len()
    }
}

/// A list of versions for a single key, ordered by creation time.
///
/// Versions are stored newest-first for efficient current lookups.
/// For point-in-time queries, we scan to find the visible version.
#[derive(Debug, Clone, Default)]
pub struct VersionChain {
    /// Versions ordered by `created_at` descending (newest first).
    versions: Vec<RowVersion>,
}

impl VersionChain {
    /// Creates an empty version chain.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a version chain with a single version.
    pub fn single(version: RowVersion) -> Self {
        Self {
            versions: vec![version],
        }
    }

    /// Creates a version chain from a pre-ordered vector (newest-first).
    ///
    /// Used during deserialization where versions are already in the correct order.
    pub fn from_vec(versions: Vec<RowVersion>) -> Self {
        Self { versions }
    }

    /// Returns true if the chain has no versions.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    /// Returns the number of versions in the chain.
    pub fn len(&self) -> usize {
        self.versions.len()
    }

    /// Returns the current (most recent, non-deleted) version.
    #[must_use]
    pub fn current(&self) -> Option<&RowVersion> {
        self.versions.first().filter(|v| v.is_current())
    }

    /// Returns the version visible at the given position.
    #[must_use]
    pub fn at(&self, pos: Offset) -> Option<&RowVersion> {
        self.versions.iter().find(|v| v.is_visible_at(pos))
    }

    /// Adds a new version to the chain.
    ///
    /// The new version becomes the current version. Any existing current
    /// version is marked as deleted at the new version's creation position.
    ///
    /// # Preconditions
    ///
    /// - New version's `created_at` must be > any existing version's `created_at`
    pub fn add(&mut self, version: RowVersion) {
        // Mark the previous current version as deleted
        if let Some(prev) = self.versions.first_mut() {
            if prev.is_current() {
                debug_assert!(
                    version.created_at > prev.created_at,
                    "new version created_at {} must be after previous {}",
                    version.created_at, prev.created_at
                );
                prev.deleted_at = version.created_at;
            }
        }

        // Insert at the front (newest first)
        self.versions.insert(0, version);

        // Postcondition: first version is the newest
        debug_assert!(
            self.versions
                .windows(2)
                .all(|w| w[0].created_at > w[1].created_at),
            "versions must be ordered by created_at descending"
        );
    }

    /// Marks the current version as deleted at the given position.
    ///
    /// Returns true if a version was deleted, false if no current version exists.
    pub fn delete_at(&mut self, pos: Offset) -> bool {
        if let Some(current) = self.versions.first_mut() {
            if current.is_current() {
                current.mark_deleted(pos);
                return true;
            }
        }
        false
    }

    /// Returns an iterator over all versions.
    pub fn iter(&self) -> impl Iterator<Item = &RowVersion> {
        self.versions.iter()
    }

    /// Returns the total size of all versions in bytes.
    pub fn total_size(&self) -> usize {
        self.versions.iter().map(RowVersion::size).sum()
    }

    /// Compacts the chain by removing versions older than the given position.
    ///
    /// Versions that were deleted before `min_visible` can be safely removed
    /// since no query can see them anymore.
    #[allow(dead_code)]
    pub fn compact(&mut self, min_visible: Offset) {
        self.versions
            .retain(|v| v.deleted_at > min_visible || v.is_current());
    }
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn test_version_visibility() {
        let v = RowVersion::with_deletion(
            Offset::new(5),
            Offset::new(10),
            Bytes::from("test"),
        );

        assert!(!v.is_visible_at(Offset::new(4))); // Before creation
        assert!(v.is_visible_at(Offset::new(5)));  // At creation
        assert!(v.is_visible_at(Offset::new(7)));  // Between
        assert!(v.is_visible_at(Offset::new(9)));  // Just before deletion
        assert!(!v.is_visible_at(Offset::new(10))); // At deletion
        assert!(!v.is_visible_at(Offset::new(15))); // After deletion
    }

    #[test]
    fn test_current_version() {
        let v = RowVersion::new(Offset::new(5), Bytes::from("test"));
        assert!(v.is_current());
        assert!(v.is_visible_at(Offset::new(5)));
        assert!(v.is_visible_at(Offset::new(1000)));
    }

    #[test]
    fn test_version_chain() {
        let mut chain = VersionChain::new();
        assert!(chain.is_empty());
        assert!(chain.current().is_none());

        // Add first version
        chain.add(RowVersion::new(Offset::new(1), Bytes::from("v1")));
        assert_eq!(chain.len(), 1);
        assert!(chain.current().is_some());
        assert_eq!(chain.current().unwrap().data, Bytes::from("v1"));

        // Add second version - first should be marked deleted
        chain.add(RowVersion::new(Offset::new(5), Bytes::from("v2")));
        assert_eq!(chain.len(), 2);
        assert_eq!(chain.current().unwrap().data, Bytes::from("v2"));

        // Point-in-time queries
        assert!(chain.at(Offset::new(0)).is_none());
        assert_eq!(chain.at(Offset::new(1)).unwrap().data, Bytes::from("v1"));
        assert_eq!(chain.at(Offset::new(3)).unwrap().data, Bytes::from("v1"));
        assert_eq!(chain.at(Offset::new(5)).unwrap().data, Bytes::from("v2"));
        assert_eq!(chain.at(Offset::new(100)).unwrap().data, Bytes::from("v2"));
    }
}
