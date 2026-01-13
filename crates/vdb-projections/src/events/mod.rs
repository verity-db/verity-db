//! CRUD event types captured via SQLite's preupdate_hook.
//!
//! This module contains the core event types that represent database mutations.
//! Events are captured synchronously before each INSERT, UPDATE, or DELETE
//! completes, providing a durable audit trail and enabling point-in-time recovery.
//!
//! # Types
//!
//! - [`ChangeEvent`] - The main event enum (Insert, Update, Delete, SchemaChange)
//! - [`SqlValue`] - Owned SQLite value for serialization
//! - [`TableName`], [`ColumnName`], [`RowId`] - Type-safe identifiers
//! - [`SqlStatement`] - Validated DDL statement for schema changes

mod change;
mod values;

pub use change::{ChangeEvent, ColumnName, RowId, SqlStatement, TableName};
pub use values::SqlValue;
