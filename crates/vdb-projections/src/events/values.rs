//! SQLite value types for event serialization.
//!
//! This module provides owned representations of SQLite values that can be
//! serialized to the event log. Unlike sqlx's borrowed value types, these
//! can outlive the database connection and be sent across threads.

use serde::{Deserialize, Serialize};
use sqlx::{Decode, Sqlite, TypeInfo, ValueRef, sqlite::SqliteValueRef};

use crate::ProjectionError;

/// An owned SQLite value extracted from a preupdate_hook callback.
///
/// Maps directly to SQLite's five storage classes. Used in [`ChangeEvent`](crate::ChangeEvent)
/// to capture row data for the event log.
///
/// # Example
///
/// ```ignore
/// use vdb_projections::SqlValue;
///
/// let values = vec![
///     SqlValue::Integer(42),
///     SqlValue::Text("hello".to_string()),
///     SqlValue::Null,
/// ];
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum SqlValue {
    /// SQL NULL value.
    Null,
    /// 64-bit signed integer (SQLite INTEGER).
    Integer(i64),
    /// 64-bit IEEE floating point (SQLite REAL).
    Real(f64),
    /// UTF-8 string (SQLite TEXT).
    Text(String),
    /// Raw bytes (SQLite BLOB).
    Blob(Vec<u8>),
}

/// Converts a borrowed SQLite value reference into an owned [`SqlValue`].
///
/// This conversion copies the underlying data, allowing the value to outlive
/// the database connection. Used internally by the preupdate_hook to capture
/// row data before it's modified.
impl<'r> TryFrom<SqliteValueRef<'r>> for SqlValue {
    type Error = ProjectionError;

    fn try_from(value: SqliteValueRef<'r>) -> Result<Self, Self::Error> {
        match value.type_info().name() {
            "NULL" => Ok(SqlValue::Null),
            "INTEGER" => Ok(SqlValue::Integer(Decode::<Sqlite>::decode(value).map_err(
                |e| ProjectionError::DecodeError {
                    type_name: "INTEGER",
                    source: e,
                },
            )?)),
            "REAL" => Ok(SqlValue::Real(Decode::<Sqlite>::decode(value).map_err(
                |e| ProjectionError::DecodeError {
                    type_name: "REAL",
                    source: e,
                },
            )?)),
            "TEXT" => Ok(SqlValue::Text(Decode::<Sqlite>::decode(value).map_err(
                |e| ProjectionError::DecodeError {
                    type_name: "TEXT",
                    source: e,
                },
            )?)),
            "BLOB" => Ok(SqlValue::Blob(Decode::<Sqlite>::decode(value).map_err(
                |e| ProjectionError::DecodeError {
                    type_name: "BLOB",
                    source: e,
                },
            )?)),
            other => Err(ProjectionError::UnsupportedSqliteType {
                type_name: other.to_string(),
            }),
        }
    }
}
