//! Unit tests for vdb-types

use crate::{GroupId, Offset, Region, StreamId, StreamName, TenantId};

// ============================================================================
// ID Type Tests
// ============================================================================

#[test]
fn stream_id_from_u64_roundtrip() {
    let id = StreamId::new(42);
    let raw: u64 = id.into();
    assert_eq!(raw, 42);
}

#[test]
fn offset_addition() {
    let a = Offset::new(10);
    let b = Offset::new(5);
    assert_eq!((a + b).as_i64(), 15);
}

#[test]
fn offset_add_assign() {
    let mut a = Offset::new(10);
    a += Offset::new(5);
    assert_eq!(a.as_i64(), 15);
}

#[test]
fn offset_subtraction() {
    let a = Offset::new(10);
    let b = Offset::new(3);
    assert_eq!((a - b).as_i64(), 7);
}

// ============================================================================
// StreamName Tests
// ============================================================================

#[test]
fn stream_name_from_str() {
    let name = StreamName::new("test-stream");
    assert_eq!(name.as_str(), "test-stream");
}

#[test]
fn stream_name_from_string() {
    let name = StreamName::new(String::from("test-stream"));
    assert_eq!(name.as_str(), "test-stream");
}

// ============================================================================
// Region Tests
// ============================================================================

#[test]
fn region_display_known_regions() {
    assert_eq!(Region::USEast1.to_string(), "us-east-1");
    assert_eq!(Region::APSoutheast2.to_string(), "ap-southeast-2");
}

#[test]
fn region_display_custom() {
    let region = Region::custom("eu-west-1");
    assert_eq!(region.to_string(), "eu-west-1");
}

// ============================================================================
// Property-Based Tests
// ============================================================================

mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn offset_add_is_commutative(a in 0i64..1_000_000, b in 0i64..1_000_000) {
            let oa = Offset::new(a);
            let ob = Offset::new(b);
            prop_assert_eq!(oa + ob, ob + oa);
        }

        #[test]
        fn id_roundtrip_stream_id(id in any::<u64>()) {
            let stream_id = StreamId::new(id);
            let raw: u64 = stream_id.into();
            prop_assert_eq!(raw, id);
        }

        #[test]
        fn id_roundtrip_tenant_id(id in any::<u64>()) {
            let tenant_id = TenantId::new(id);
            let raw: u64 = tenant_id.into();
            prop_assert_eq!(raw, id);
        }

        #[test]
        fn id_roundtrip_group_id(id in any::<u64>()) {
            let group_id = GroupId::new(id);
            let raw: u64 = group_id.into();
            prop_assert_eq!(raw, id);
        }
    }
}
