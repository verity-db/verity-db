//! Unit tests for vdb-directory

use vdb_types::{GroupId, Placement, Region};

use crate::{Directory, DirectoryError};

// ============================================================================
// Directory Tests
// ============================================================================

#[test]
fn global_placement_returns_global_group() {
    let directory = Directory::new(GroupId::new(0));
    let group = directory.group_for_placement(&Placement::Global).unwrap();
    assert_eq!(group, GroupId::new(0));
}

#[test]
fn regional_placement_returns_regional_group() {
    let directory = Directory::new(GroupId::new(0))
        .with_region(Region::USEast1, GroupId::new(1))
        .with_region(Region::APSoutheast2, GroupId::new(2));

    let us_group = directory
        .group_for_placement(&Placement::Region(Region::USEast1))
        .unwrap();
    let au_group = directory
        .group_for_placement(&Placement::Region(Region::APSoutheast2))
        .unwrap();

    assert_eq!(us_group, GroupId::new(1));
    assert_eq!(au_group, GroupId::new(2));
}

#[test]
fn unknown_region_returns_error() {
    let directory = Directory::new(GroupId::new(0)).with_region(Region::USEast1, GroupId::new(1));

    let result = directory.group_for_placement(&Placement::Region(Region::APSoutheast2));

    assert!(matches!(result, Err(DirectoryError::RegionNotFound(_))));
}

#[test]
fn custom_region_works() {
    let custom_region = Region::custom("eu-west-1");
    let directory =
        Directory::new(GroupId::new(0)).with_region(custom_region.clone(), GroupId::new(3));

    let group = directory
        .group_for_placement(&Placement::Region(custom_region))
        .unwrap();

    assert_eq!(group, GroupId::new(3));
}

#[test]
fn directory_builder_pattern() {
    // Test that builder pattern works fluently
    let directory = Directory::new(GroupId::new(0))
        .with_region(Region::USEast1, GroupId::new(1))
        .with_region(Region::APSoutheast2, GroupId::new(2))
        .with_region(Region::custom("eu-west-1"), GroupId::new(3));

    // All regions should be accessible
    assert!(
        directory
            .group_for_placement(&Placement::Region(Region::USEast1))
            .is_ok()
    );
    assert!(
        directory
            .group_for_placement(&Placement::Region(Region::APSoutheast2))
            .is_ok()
    );
    assert!(
        directory
            .group_for_placement(&Placement::Region(Region::custom("eu-west-1")))
            .is_ok()
    );
    assert!(directory.group_for_placement(&Placement::Global).is_ok());
}
