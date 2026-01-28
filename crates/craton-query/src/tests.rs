//! Integration tests for craton-query.

use std::collections::HashMap;
use std::ops::Range;

use bytes::Bytes;
use craton_store::{Key, ProjectionStore, StoreError, TableId, WriteBatch, WriteOp};
use craton_types::Offset;

use crate::QueryEngine;
use crate::schema::{ColumnDef, DataType, SchemaBuilder};
use crate::value::Value;

// ============================================================================
// Mock Store
// ============================================================================

/// A simple in-memory mock store for testing.
#[derive(Debug, Default)]
struct MockStore {
    tables: HashMap<TableId, Vec<(Key, Bytes)>>,
    position: Offset,
}

impl MockStore {
    fn new() -> Self {
        Self::default()
    }

    fn insert(&mut self, table_id: TableId, key: Key, value: Bytes) {
        let table = self.tables.entry(table_id).or_default();
        table.push((key, value));
        table.sort_by(|a, b| a.0.cmp(&b.0));
    }

    fn insert_json(&mut self, table_id: TableId, key: Key, json: &serde_json::Value) {
        let bytes = Bytes::from(serde_json::to_vec(json).expect("json serialization failed"));
        self.insert(table_id, key, bytes);
    }
}

impl ProjectionStore for MockStore {
    fn apply(&mut self, batch: WriteBatch) -> Result<(), StoreError> {
        for op in batch.operations() {
            match op {
                WriteOp::Put { table, key, value } => {
                    self.insert(*table, key.clone(), value.clone());
                }
                WriteOp::Delete { table, key } => {
                    if let Some(t) = self.tables.get_mut(table) {
                        t.retain(|(k, _)| k != key);
                    }
                }
            }
        }
        self.position = batch.position();
        Ok(())
    }

    fn applied_position(&self) -> Offset {
        self.position
    }

    fn get(&mut self, table: TableId, key: &Key) -> Result<Option<Bytes>, StoreError> {
        Ok(self
            .tables
            .get(&table)
            .and_then(|t| t.iter().find(|(k, _)| k == key))
            .map(|(_, v)| v.clone()))
    }

    fn get_at(
        &mut self,
        table: TableId,
        key: &Key,
        _pos: Offset,
    ) -> Result<Option<Bytes>, StoreError> {
        // Mock doesn't support MVCC, just use current state
        self.get(table, key)
    }

    fn scan(
        &mut self,
        table: TableId,
        range: Range<Key>,
        limit: usize,
    ) -> Result<Vec<(Key, Bytes)>, StoreError> {
        let Some(entries) = self.tables.get(&table) else {
            return Ok(vec![]);
        };

        let result: Vec<_> = entries
            .iter()
            .filter(|(k, _)| k >= &range.start && k < &range.end)
            .take(limit)
            .cloned()
            .collect();

        Ok(result)
    }

    fn scan_at(
        &mut self,
        table: TableId,
        range: Range<Key>,
        limit: usize,
        _pos: Offset,
    ) -> Result<Vec<(Key, Bytes)>, StoreError> {
        self.scan(table, range, limit)
    }

    fn sync(&mut self) -> Result<(), StoreError> {
        Ok(())
    }
}

// ============================================================================
// Test Schema
// ============================================================================

fn test_schema() -> crate::Schema {
    SchemaBuilder::new()
        .table(
            "users",
            TableId::new(1),
            vec![
                ColumnDef::new("id", DataType::BigInt).not_null(),
                ColumnDef::new("name", DataType::Text).not_null(),
                ColumnDef::new("age", DataType::BigInt),
            ],
            vec!["id".into()],
        )
        .table(
            "orders",
            TableId::new(2),
            vec![
                ColumnDef::new("order_id", DataType::BigInt).not_null(),
                ColumnDef::new("user_id", DataType::BigInt).not_null(),
                ColumnDef::new("total", DataType::BigInt),
            ],
            vec!["order_id".into()],
        )
        .build()
}

fn test_store() -> MockStore {
    use crate::key_encoder::encode_key;

    let mut store = MockStore::new();

    // Insert test users
    store.insert_json(
        TableId::new(1),
        encode_key(&[Value::BigInt(1)]),
        &serde_json::json!({"id": 1, "name": "Alice", "age": 30}),
    );
    store.insert_json(
        TableId::new(1),
        encode_key(&[Value::BigInt(2)]),
        &serde_json::json!({"id": 2, "name": "Bob", "age": 25}),
    );
    store.insert_json(
        TableId::new(1),
        encode_key(&[Value::BigInt(3)]),
        &serde_json::json!({"id": 3, "name": "Charlie", "age": 35}),
    );

    // Insert test orders
    store.insert_json(
        TableId::new(2),
        encode_key(&[Value::BigInt(100)]),
        &serde_json::json!({"order_id": 100, "user_id": 1, "total": 500}),
    );
    store.insert_json(
        TableId::new(2),
        encode_key(&[Value::BigInt(101)]),
        &serde_json::json!({"order_id": 101, "user_id": 2, "total": 300}),
    );

    store
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn test_select_all() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT * FROM users", &[])
        .unwrap();

    assert_eq!(result.columns.len(), 3);
    assert_eq!(result.rows.len(), 3);
}

#[test]
fn test_select_columns() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT name, age FROM users", &[])
        .unwrap();

    assert_eq!(result.columns.len(), 2);
    assert_eq!(result.columns[0].as_str(), "name");
    assert_eq!(result.columns[1].as_str(), "age");
}

#[test]
fn test_point_lookup() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT * FROM users WHERE id = 1", &[])
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::BigInt(1));
    assert_eq!(result.rows[0][1], Value::Text("Alice".to_string()));
}

#[test]
fn test_point_lookup_not_found() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT * FROM users WHERE id = 999", &[])
        .unwrap();

    assert!(result.rows.is_empty());
}

#[test]
fn test_range_scan_gt() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT * FROM users WHERE id > 1", &[])
        .unwrap();

    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_range_scan_lt() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT * FROM users WHERE id < 3", &[])
        .unwrap();

    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_table_scan_with_filter() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT * FROM users WHERE name = 'Bob'", &[])
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][1], Value::Text("Bob".to_string()));
}

#[test]
fn test_limit() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT * FROM users LIMIT 2", &[])
        .unwrap();

    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_order_by() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT * FROM users ORDER BY age ASC", &[])
        .unwrap();

    assert_eq!(result.rows.len(), 3);
    assert_eq!(result.rows[0][2], Value::BigInt(25)); // Bob
    assert_eq!(result.rows[1][2], Value::BigInt(30)); // Alice
    assert_eq!(result.rows[2][2], Value::BigInt(35)); // Charlie
}

#[test]
fn test_order_by_desc() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT * FROM users ORDER BY age DESC", &[])
        .unwrap();

    assert_eq!(result.rows[0][2], Value::BigInt(35)); // Charlie
    assert_eq!(result.rows[2][2], Value::BigInt(25)); // Bob
}

#[test]
fn test_parameterized_query() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(
            &mut store,
            "SELECT * FROM users WHERE id = $1",
            &[Value::BigInt(2)],
        )
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][1], Value::Text("Bob".to_string()));
}

#[test]
fn test_multiple_params() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(
            &mut store,
            "SELECT * FROM users WHERE id >= $1 AND id <= $2",
            &[Value::BigInt(1), Value::BigInt(2)],
        )
        .unwrap();

    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_in_predicate() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine
        .query(&mut store, "SELECT * FROM users WHERE id IN (1, 3)", &[])
        .unwrap();

    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_prepared_query() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let prepared = engine
        .prepare("SELECT name FROM users WHERE id = $1", &[Value::BigInt(1)])
        .unwrap();

    assert_eq!(prepared.columns().len(), 1);
    assert_eq!(prepared.table_name(), "users");

    let result = prepared.execute(&mut store).unwrap();
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn test_unknown_table() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine.query(&mut store, "SELECT * FROM nonexistent", &[]);
    assert!(matches!(result, Err(crate::QueryError::TableNotFound(_))));
}

#[test]
fn test_unknown_column() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine.query(&mut store, "SELECT nonexistent FROM users", &[]);
    assert!(matches!(
        result,
        Err(crate::QueryError::ColumnNotFound { .. })
    ));
}

#[test]
fn test_missing_param() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    let result = engine.query(&mut store, "SELECT * FROM users WHERE id = $1", &[]);
    assert!(matches!(
        result,
        Err(crate::QueryError::ParameterNotFound(1))
    ));
}

#[test]
fn test_query_at_position() {
    let schema = test_schema();
    let mut store = test_store();
    let engine = QueryEngine::new(schema);

    // Note: Our mock store doesn't truly support MVCC, but this tests the API
    let result = engine
        .query_at(
            &mut store,
            "SELECT * FROM users WHERE id = 1",
            &[],
            Offset::new(100),
        )
        .unwrap();

    assert_eq!(result.rows.len(), 1);
}

// ============================================================================
// Key Encoding Property Tests
// ============================================================================

#[cfg(test)]
mod key_encoding_tests {
    use super::*;
    use crate::key_encoder::{decode_key, encode_key};
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn bigint_encoding_round_trip(v: i64) {
            let key = encode_key(&[Value::BigInt(v)]);
            let decoded = decode_key(&key);
            prop_assert_eq!(decoded, vec![Value::BigInt(v)]);
        }

        #[test]
        fn bigint_ordering_preserved(a: i64, b: i64) {
            let key_a = encode_key(&[Value::BigInt(a)]);
            let key_b = encode_key(&[Value::BigInt(b)]);

            prop_assert_eq!(a.cmp(&b), key_a.cmp(&key_b));
        }

        #[test]
        fn text_round_trip(s in "\\PC*") {
            let key = encode_key(&[Value::Text(s.clone())]);
            let decoded = decode_key(&key);
            prop_assert_eq!(decoded, vec![Value::Text(s)]);
        }

        #[test]
        fn composite_key_round_trip(a: i64, s in "[a-z]{0,10}") {
            let values = vec![Value::BigInt(a), Value::Text(s.clone())];
            let key = encode_key(&values);
            let decoded = decode_key(&key);
            prop_assert_eq!(decoded, values);
        }
    }
}
