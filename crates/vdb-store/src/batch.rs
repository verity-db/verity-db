//! Write batches for atomic updates to the projection store.
//!
//! A [`WriteBatch`] groups multiple write operations that should be applied
//! atomically at a single log position. This ensures the projection state
//! is always consistent with the log.

use bytes::Bytes;
use vdb_types::Offset;

use crate::types::TableId;
use crate::Key;

/// A single write operation within a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOp {
    /// Insert or update a key-value pair.
    Put {
        table: TableId,
        key: Key,
        value: Bytes,
    },
    /// Delete a key.
    Delete { table: TableId, key: Key },
}

impl WriteOp {
    /// Returns the table ID for this operation.
    pub fn table(&self) -> TableId {
        match self {
            WriteOp::Put { table, .. } | WriteOp::Delete { table, .. } => *table,
        }
    }

    /// Returns the key for this operation.
    pub fn key(&self) -> &Key {
        match self {
            WriteOp::Put { key, .. } | WriteOp::Delete { key, .. } => key,
        }
    }
}

/// A batch of write operations to apply atomically.
///
/// Write batches are applied at a specific log position, which is used for
/// MVCC version tracking. The position must be sequential - each batch's
/// position must be exactly one greater than the previous applied position.
///
/// # Example
///
/// ```ignore
/// use vdb_store::{WriteBatch, WriteOp, TableId, Key};
/// use vdb_types::Offset;
/// use bytes::Bytes;
///
/// let batch = WriteBatch::new(Offset::new(42))
///     .put(TableId::new(1), Key::from("user:123"), Bytes::from("Alice"))
///     .put(TableId::new(1), Key::from("user:456"), Bytes::from("Bob"))
///     .delete(TableId::new(2), Key::from("session:old"));
/// ```
#[derive(Debug, Clone)]
pub struct WriteBatch {
    /// The log position this batch corresponds to.
    position: Offset,
    /// The operations in this batch.
    operations: Vec<WriteOp>,
}

impl WriteBatch {
    /// Creates a new empty batch at the given position.
    pub fn new(position: Offset) -> Self {
        Self {
            position,
            operations: Vec::new(),
        }
    }

    /// Creates a batch with pre-allocated capacity.
    pub fn with_capacity(position: Offset, capacity: usize) -> Self {
        Self {
            position,
            operations: Vec::with_capacity(capacity),
        }
    }

    /// Returns the log position for this batch.
    pub fn position(&self) -> Offset {
        self.position
    }

    /// Returns the number of operations in this batch.
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Returns true if the batch has no operations.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Adds a put operation (insert or update).
    #[must_use]
    pub fn put(mut self, table: TableId, key: Key, value: Bytes) -> Self {
        self.operations.push(WriteOp::Put { table, key, value });
        self
    }

    /// Adds a delete operation.
    #[must_use]
    pub fn delete(mut self, table: TableId, key: Key) -> Self {
        self.operations.push(WriteOp::Delete { table, key });
        self
    }

    /// Adds a put operation in-place.
    pub fn push_put(&mut self, table: TableId, key: Key, value: Bytes) {
        self.operations.push(WriteOp::Put { table, key, value });
    }

    /// Adds a delete operation in-place.
    pub fn push_delete(&mut self, table: TableId, key: Key) {
        self.operations.push(WriteOp::Delete { table, key });
    }

    /// Returns an iterator over the operations.
    pub fn iter(&self) -> impl Iterator<Item = &WriteOp> {
        self.operations.iter()
    }

    /// Consumes the batch and returns the operations.
    pub fn into_operations(self) -> Vec<WriteOp> {
        self.operations
    }

    /// Returns a reference to the operations.
    pub fn operations(&self) -> &[WriteOp] {
        &self.operations
    }
}

impl IntoIterator for WriteBatch {
    type Item = WriteOp;
    type IntoIter = std::vec::IntoIter<WriteOp>;

    fn into_iter(self) -> Self::IntoIter {
        self.operations.into_iter()
    }
}

impl<'a> IntoIterator for &'a WriteBatch {
    type Item = &'a WriteOp;
    type IntoIter = std::slice::Iter<'a, WriteOp>;

    fn into_iter(self) -> Self::IntoIter {
        self.operations.iter()
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;

    #[test]
    fn test_batch_builder() {
        let batch = WriteBatch::new(Offset::new(10))
            .put(TableId::new(1), Key::from("key1"), Bytes::from("value1"))
            .delete(TableId::new(1), Key::from("key2"))
            .put(TableId::new(2), Key::from("key3"), Bytes::from("value3"));

        assert_eq!(batch.position(), Offset::new(10));
        assert_eq!(batch.len(), 3);

        let ops: Vec<_> = batch.iter().collect();
        assert!(matches!(ops[0], WriteOp::Put { .. }));
        assert!(matches!(ops[1], WriteOp::Delete { .. }));
        assert!(matches!(ops[2], WriteOp::Put { .. }));
    }

    #[test]
    fn test_empty_batch() {
        let batch = WriteBatch::new(Offset::new(1));
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
    }
}
