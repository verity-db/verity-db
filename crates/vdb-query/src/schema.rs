//! Schema definitions for query planning.
//!
//! Provides explicit mapping of SQL table/column names to store types.

use std::collections::BTreeMap;
use std::fmt::{self, Debug, Display};

use vdb_store::TableId;

// ============================================================================
// Names
// ============================================================================

/// SQL table name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableName(String);

impl TableName {
    /// Creates a new table name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the table name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Debug for TableName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TableName({:?})", self.0)
    }
}

impl Display for TableName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for TableName {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for TableName {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// SQL column name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ColumnName(String);

impl ColumnName {
    /// Creates a new column name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the column name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Debug for ColumnName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ColumnName({:?})", self.0)
    }
}

impl Display for ColumnName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ColumnName {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for ColumnName {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

// ============================================================================
// Data Types
// ============================================================================

/// SQL data types supported by the query engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataType {
    /// 64-bit signed integer.
    BigInt,
    /// Variable-length UTF-8 text.
    Text,
    /// Boolean value.
    Boolean,
    /// Timestamp (nanoseconds since epoch).
    Timestamp,
    /// Variable-length binary data.
    Bytes,
}

impl Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataType::BigInt => write!(f, "BIGINT"),
            DataType::Text => write!(f, "TEXT"),
            DataType::Boolean => write!(f, "BOOLEAN"),
            DataType::Timestamp => write!(f, "TIMESTAMP"),
            DataType::Bytes => write!(f, "BYTES"),
        }
    }
}

// ============================================================================
// Column Definition
// ============================================================================

/// Definition of a table column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDef {
    /// Column name.
    pub name: ColumnName,
    /// Column data type.
    pub data_type: DataType,
    /// Whether the column can contain NULL values.
    pub nullable: bool,
}

impl ColumnDef {
    /// Creates a new column definition.
    pub fn new(name: impl Into<ColumnName>, data_type: DataType) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable: true,
        }
    }

    /// Makes this column non-nullable.
    pub fn not_null(mut self) -> Self {
        self.nullable = false;
        self
    }
}

// ============================================================================
// Table Definition
// ============================================================================

/// Definition of a table in the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableDef {
    /// Underlying store table ID.
    pub table_id: TableId,
    /// Column definitions in order.
    pub columns: Vec<ColumnDef>,
    /// Primary key column names (in order).
    pub primary_key: Vec<ColumnName>,
}

impl TableDef {
    /// Creates a new table definition.
    pub fn new(table_id: TableId, columns: Vec<ColumnDef>, primary_key: Vec<ColumnName>) -> Self {
        // Validate primary key columns exist
        for pk_col in &primary_key {
            debug_assert!(
                columns.iter().any(|c| &c.name == pk_col),
                "primary key column '{pk_col}' not found in columns"
            );
        }

        Self {
            table_id,
            columns,
            primary_key,
        }
    }

    /// Finds a column by name.
    pub fn find_column(&self, name: &ColumnName) -> Option<(usize, &ColumnDef)> {
        self.columns
            .iter()
            .enumerate()
            .find(|(_, c)| &c.name == name)
    }

    /// Returns true if the given column is part of the primary key.
    pub fn is_primary_key(&self, name: &ColumnName) -> bool {
        self.primary_key.contains(name)
    }

    /// Returns the index of a column in the primary key.
    pub fn primary_key_position(&self, name: &ColumnName) -> Option<usize> {
        self.primary_key.iter().position(|pk| pk == name)
    }

    /// Returns the column indices that form the primary key.
    pub fn primary_key_indices(&self) -> Vec<usize> {
        self.primary_key
            .iter()
            .filter_map(|pk| self.find_column(pk).map(|(idx, _)| idx))
            .collect()
    }
}

// ============================================================================
// Schema
// ============================================================================

/// Schema registry mapping SQL names to store types.
#[derive(Debug, Clone, Default)]
pub struct Schema {
    tables: BTreeMap<TableName, TableDef>,
}

impl Schema {
    /// Creates an empty schema.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a table to the schema.
    pub fn add_table(&mut self, name: impl Into<TableName>, def: TableDef) {
        self.tables.insert(name.into(), def);
    }

    /// Looks up a table by name.
    pub fn get_table(&self, name: &TableName) -> Option<&TableDef> {
        self.tables.get(name)
    }

    /// Returns all table names.
    pub fn table_names(&self) -> impl Iterator<Item = &TableName> {
        self.tables.keys()
    }

    /// Returns the number of tables.
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// Returns true if the schema has no tables.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

// ============================================================================
// Builder
// ============================================================================

/// Builder for constructing schemas fluently.
#[derive(Debug, Default)]
pub struct SchemaBuilder {
    schema: Schema,
}

impl SchemaBuilder {
    /// Creates a new schema builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a table to the schema.
    pub fn table(
        mut self,
        name: impl Into<TableName>,
        table_id: TableId,
        columns: Vec<ColumnDef>,
        primary_key: Vec<ColumnName>,
    ) -> Self {
        let def = TableDef::new(table_id, columns, primary_key);
        self.schema.add_table(name, def);
        self
    }

    /// Builds the schema.
    pub fn build(self) -> Schema {
        self.schema
    }
}
