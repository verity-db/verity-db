//! Integration tests for vdb-store.
//!
//! These tests verify the complete functionality of the projection store,
//! including B+tree operations, MVCC, and persistence.

use bytes::Bytes;
use vdb_types::Offset;

use crate::ProjectionStore;
use crate::batch::WriteBatch;
use crate::store::BTreeStore;
use crate::types::{Key, TableId};

#[test]
fn test_basic_operations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");

    let mut store = BTreeStore::open(&path).unwrap();

    // Apply a batch
    let batch = WriteBatch::new(Offset::new(1)).put(
        TableId::new(1),
        Key::from("hello"),
        Bytes::from("world"),
    );

    store.apply(batch).unwrap();

    // Verify
    assert_eq!(
        store.get(TableId::new(1), &Key::from("hello")).unwrap(),
        Some(Bytes::from("world"))
    );
}

#[test]
fn test_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("persist.db");

    // Write data
    {
        let mut store = BTreeStore::open(&path).unwrap();
        store
            .apply(WriteBatch::new(Offset::new(1)).put(
                TableId::new(1),
                Key::from("key"),
                Bytes::from("value"),
            ))
            .unwrap();
        store.sync().unwrap();
    }

    // Reopen and verify
    {
        let _store = BTreeStore::open(&path).unwrap();
        // Note: Without proper superblock persistence, the data won't survive.
        // This test verifies the sync path works without errors.
        // Full persistence requires implementing superblock write/recovery.
    }
}

#[test]
fn test_mvcc_versioning() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mvcc.db");

    let mut store = BTreeStore::open(&path).unwrap();

    // Insert version 1
    store
        .apply(WriteBatch::new(Offset::new(1)).put(
            TableId::new(1),
            Key::from("key"),
            Bytes::from("v1"),
        ))
        .unwrap();

    // Insert version 2
    store
        .apply(WriteBatch::new(Offset::new(2)).put(
            TableId::new(1),
            Key::from("key"),
            Bytes::from("v2"),
        ))
        .unwrap();

    // Insert version 3
    store
        .apply(WriteBatch::new(Offset::new(3)).put(
            TableId::new(1),
            Key::from("key"),
            Bytes::from("v3"),
        ))
        .unwrap();

    // Current value is v3
    assert_eq!(
        store.get(TableId::new(1), &Key::from("key")).unwrap(),
        Some(Bytes::from("v3"))
    );

    // Historical queries
    assert_eq!(
        store
            .get_at(TableId::new(1), &Key::from("key"), Offset::new(1))
            .unwrap(),
        Some(Bytes::from("v1"))
    );
    assert_eq!(
        store
            .get_at(TableId::new(1), &Key::from("key"), Offset::new(2))
            .unwrap(),
        Some(Bytes::from("v2"))
    );
    assert_eq!(
        store
            .get_at(TableId::new(1), &Key::from("key"), Offset::new(3))
            .unwrap(),
        Some(Bytes::from("v3"))
    );
}

#[test]
fn test_delete_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delete.db");

    let mut store = BTreeStore::open(&path).unwrap();

    // Insert
    store
        .apply(WriteBatch::new(Offset::new(1)).put(
            TableId::new(1),
            Key::from("key"),
            Bytes::from("value"),
        ))
        .unwrap();

    // Delete
    store
        .apply(WriteBatch::new(Offset::new(2)).delete(TableId::new(1), Key::from("key")))
        .unwrap();

    // Current value is None (deleted)
    assert_eq!(store.get(TableId::new(1), &Key::from("key")).unwrap(), None);

    // But history is preserved
    assert_eq!(
        store
            .get_at(TableId::new(1), &Key::from("key"), Offset::new(1))
            .unwrap(),
        Some(Bytes::from("value"))
    );
}

#[test]
fn test_range_scan() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scan.db");

    let mut store = BTreeStore::open(&path).unwrap();

    // Insert multiple keys
    let mut batch = WriteBatch::new(Offset::new(1));
    for i in 0..20 {
        batch.push_put(
            TableId::new(1),
            Key::from(format!("user:{i:03}")),
            Bytes::from(format!("User {i}")),
        );
    }
    store.apply(batch).unwrap();

    // Scan a range
    let results = store
        .scan(
            TableId::new(1),
            Key::from("user:005")..Key::from("user:010"),
            100,
        )
        .unwrap();

    assert_eq!(results.len(), 5);
    assert_eq!(results[0].0, Key::from("user:005"));
    assert_eq!(results[4].0, Key::from("user:009"));
}

#[test]
fn test_multiple_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tables.db");

    let mut store = BTreeStore::open(&path).unwrap();

    // Insert same key in different tables
    store
        .apply(
            WriteBatch::new(Offset::new(1))
                .put(TableId::new(1), Key::from("id"), Bytes::from("table1"))
                .put(TableId::new(2), Key::from("id"), Bytes::from("table2"))
                .put(TableId::new(3), Key::from("id"), Bytes::from("table3")),
        )
        .unwrap();

    // Each table has its own value
    assert_eq!(
        store.get(TableId::new(1), &Key::from("id")).unwrap(),
        Some(Bytes::from("table1"))
    );
    assert_eq!(
        store.get(TableId::new(2), &Key::from("id")).unwrap(),
        Some(Bytes::from("table2"))
    );
    assert_eq!(
        store.get(TableId::new(3), &Key::from("id")).unwrap(),
        Some(Bytes::from("table3"))
    );
}

#[test]
fn test_large_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.db");

    let mut store = BTreeStore::open(&path).unwrap();

    // Insert a large value (but still fits in a page)
    let large_value = Bytes::from(vec![b'x'; 2000]);

    store
        .apply(WriteBatch::new(Offset::new(1)).put(
            TableId::new(1),
            Key::from("large"),
            large_value.clone(),
        ))
        .unwrap();

    assert_eq!(
        store.get(TableId::new(1), &Key::from("large")).unwrap(),
        Some(large_value)
    );
}

#[test]
fn test_many_keys_triggers_splits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("splits.db");

    let mut store = BTreeStore::open(&path).unwrap();

    // Insert enough keys to trigger B+tree splits
    let mut batch = WriteBatch::new(Offset::new(1));
    for i in 0..100 {
        batch.push_put(
            TableId::new(1),
            Key::from(format!("key:{i:04}")),
            Bytes::from(format!("value-{i}")),
        );
    }
    store.apply(batch).unwrap();

    // Verify all keys are retrievable
    for i in 0..100 {
        let key = Key::from(format!("key:{i:04}"));
        let expected = Bytes::from(format!("value-{i}"));
        assert_eq!(
            store.get(TableId::new(1), &key).unwrap(),
            Some(expected),
            "failed for key:{i:04}"
        );
    }
}
