//! vdb-directory: Placement routing for `VerityDB`
//!
//! The directory determines which VSR replication group handles a given
//! stream based on its placement policy. This is a critical component for
//! HIPAA compliance - PHI data must stay within designated regions.
//!
//! # Placement Policies
//!
//! - **Regional**: Data pinned to a specific geographic region (for PHI)
//! - **Global**: Data can be replicated across all regions (for non-PHI)
//!
//! # Example
//!
//! ```
//! use vdb_directory::Directory;
//! use vdb_types::{GroupId, Placement, Region};
//!
//! let directory = Directory::new(GroupId::new(0))  // Global group
//!     .with_region(Region::APSoutheast2, GroupId::new(1))
//!     .with_region(Region::USEast1, GroupId::new(2));
//!
//! // PHI data routes to regional group
//! let group = directory.group_for_placement(&Placement::Region(Region::APSoutheast2));
//! assert_eq!(group.unwrap(), GroupId::new(1));
//!
//! // Non-PHI data routes to global group
//! let group = directory.group_for_placement(&Placement::Global);
//! assert_eq!(group.unwrap(), GroupId::new(0));
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use vdb_types::{GroupId, Placement, Region};

/// Routes stream placements to VSR replication groups.
///
/// The directory maintains a mapping from regions to their dedicated
/// replication groups, plus a global group for non-regional data.
///
/// # Thread Safety
///
/// Directory is `Clone` and can be shared across threads. It's typically
/// created once at startup and passed to the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directory {
    /// The replication group for global (non-regional) placements.
    global_group: GroupId,
    /// Mapping from regions to their replication groups.
    regional_groups: HashMap<Region, GroupId>,
}

impl Directory {
    /// Creates a new directory with the specified global group.
    ///
    /// The global group handles all `Placement::Global` streams.
    /// Regional groups must be added with [`with_region`](Self::with_region).
    pub fn new(global_group: GroupId) -> Self {
        Self {
            global_group,
            regional_groups: HashMap::new(),
        }
    }

    /// Adds a regional group mapping.
    ///
    /// This is a builder method that takes ownership and returns `self`
    /// for chaining.
    ///
    /// # Example
    ///
    /// ```
    /// use vdb_directory::Directory;
    /// use vdb_types::{GroupId, Region};
    ///
    /// let directory = Directory::new(GroupId::new(0))
    ///     .with_region(Region::APSoutheast2, GroupId::new(1))
    ///     .with_region(Region::USEast1, GroupId::new(2));
    /// ```
    pub fn with_region(mut self, region: Region, group: GroupId) -> Self {
        self.regional_groups.insert(region, group);
        self
    }

    /// Returns the replication group for the given placement.
    ///
    /// - `Placement::Global` → returns the global group
    /// - `Placement::Region(r)` → returns the group for region `r`
    ///
    /// # Errors
    ///
    /// Returns [`DirectoryError::RegionNotFound`] if a regional placement
    /// specifies a region that hasn't been configured.
    pub fn group_for_placement(&self, placement: &Placement) -> Result<GroupId, DirectoryError> {
        match placement {
            Placement::Region(region) => self
                .regional_groups
                .get(region)
                .copied()
                .ok_or_else(|| DirectoryError::RegionNotFound(region.clone())),
            Placement::Global => Ok(self.global_group),
        }
    }
}

/// Errors that can occur during directory lookups.
#[derive(thiserror::Error, Debug)]
pub enum DirectoryError {
    /// The specified region is not configured in the directory.
    #[error("region not found: {0}")]
    RegionNotFound(Region),
}

#[cfg(test)]
mod tests;
