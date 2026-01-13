//! ChangeEvent - the core event type captured from SQLite mutations.
//!
//! These events form the write-ahead log for VerityDB projections. Every
//! INSERT, UPDATE, DELETE, and schema change is captured and persisted
//! before the SQLite write completes, enabling point-in-time recovery
//! and full audit trails.

use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::{ProjectionError, SqlValue};

/// A database mutation event captured via SQLite's preupdate_hook.
///
/// Each variant contains all information needed to replay the mutation,
/// enabling recovery from the event log. Events are serialized and appended
/// to the durable log before the corresponding SQLite write completes.
///
/// # Variants
///
/// - [`Insert`](ChangeEvent::Insert) - New row added to a table
/// - [`Update`](ChangeEvent::Update) - Existing row modified (captures before/after)
/// - [`Delete`](ChangeEvent::Delete) - Row removed (captures deleted data)
/// - [`SchemaChange`](ChangeEvent::SchemaChange) - DDL statement (CREATE, ALTER, DROP)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ChangeEvent {
    /// A new row was inserted into a table.
    Insert {
        table_name: TableName,
        row_id: RowId,
        /// Column names in insertion order.
        column_names: Vec<ColumnName>,
        /// Values in the same order as column_names.
        values: Vec<SqlValue>,
    },
    /// An existing row was updated.
    /// Captures both old and new values for audit compliance.
    Update {
        table_name: TableName,
        row_id: RowId,
        /// Column values before the update.
        old_values: Vec<(ColumnName, SqlValue)>,
        /// Column values after the update.
        new_values: Vec<(ColumnName, SqlValue)>,
    },
    /// A row was deleted from a table.
    /// Captures the deleted data for audit compliance.
    Delete {
        table_name: TableName,
        row_id: RowId,
        /// The values that were in the deleted row.
        deleted_values: Vec<SqlValue>,
    },
    /// A schema change (DDL) was executed.
    /// Captured during migrations to enable replay from scratch.
    SchemaChange { sql_statement: SqlStatement },
}

/// A SQLite table name.
///
/// Lightweight newtype for type safety. Values come from SQLite's preupdate_hook
/// and are trusted (SQLite has already validated them as real table names).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Eq, Hash)]
pub struct TableName(String);

impl TableName {
    /// Creates a TableName from a value provided by SQLite.
    /// Use this for values from preupdate_hook (trusted source).
    pub fn from_sqlite(name: &str) -> Self {
        Self(name.to_owned())
    }

    /// Returns true if this is an internal table that should be skipped.
    /// Internal tables include SQLite system tables (`sqlite_*`) and
    /// VerityDB metadata tables (`_vdb_*`).
    pub fn is_internal(&self) -> bool {
        self.0.starts_with("sqlite_") || self.0.starts_with("_vdb_")
    }

    /// Returns the table name for use in SQL identifiers.
    pub fn as_identifier(&self) -> &str {
        &self.0
    }
}

impl Display for TableName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for TableName {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<TableName> for String {
    fn from(table_name: TableName) -> Self {
        table_name.0
    }
}

/// SQLite's internal row identifier.
///
/// Every SQLite table has a 64-bit signed integer rowid (unless it's a WITHOUT ROWID table).
/// This uniquely identifies a row within a table and is stable across the row's lifetime.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RowId(i64);

impl Display for RowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<i64> for RowId {
    fn from(value: i64) -> Self {
        debug_assert!(value >= 0, "RowId cannot be negative");
        Self(value)
    }
}

impl From<RowId> for i64 {
    fn from(row_id: RowId) -> Self {
        row_id.0
    }
}

/// A SQLite column name.
///
/// Lightweight newtype for type safety. Values come from SQLite's preupdate_hook
/// and are trusted (SQLite has already validated them as real column names).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnName(String);

impl Display for ColumnName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for ColumnName {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<ColumnName> for String {
    fn from(column_name: ColumnName) -> Self {
        column_name.0
    }
}

/// A validated DDL (Data Definition Language) SQL statement.
///
/// Only schema-modifying statements (CREATE, ALTER, DROP) are allowed.
/// This ensures that [`ChangeEvent::SchemaChange`] only contains DDL,
/// while DML (INSERT, UPDATE, DELETE) is captured via the other variants.
///
/// # Example
///
/// ```
/// use vdb_projections::SqlStatement;
///
/// let stmt = SqlStatement::from_ddl("CREATE TABLE users (id INTEGER PRIMARY KEY)").unwrap();
/// assert_eq!(stmt.as_str(), "CREATE TABLE users (id INTEGER PRIMARY KEY)");
///
/// // DML is rejected
/// assert!(SqlStatement::from_ddl("INSERT INTO users VALUES (1)").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SqlStatement(String);

impl SqlStatement {
    /// Creates a SqlStatement from trusted migration code.
    ///
    /// Validates that the statement is DDL (CREATE, ALTER, DROP).
    /// Returns an error if the statement appears to be DML.
    pub fn from_ddl(sql: impl Into<String>) -> Result<Self, ProjectionError> {
        let sql = sql.into();
        let normalized = sql.trim().to_uppercase();

        // Only allow DDL statements
        const DDL_PREFIXES: &[&str] = &[
            "CREATE ",
            "ALTER ",
            "DROP ",
            "CREATE INDEX",
            "DROP INDEX",
            "CREATE TRIGGER",
            "DROP TRIGGER",
        ];

        if !DDL_PREFIXES.iter().any(|p| normalized.starts_with(p)) {
            return Err(ProjectionError::InvalidDdlStatement { statement: sql });
        }

        Ok(Self(sql))
    }

    /// Returns the SQL statement as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
