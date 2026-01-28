//! # vdb-query: SQL query layer for `VerityDB` projections
//!
//! This crate provides a minimal SQL query engine for compliance lookups
//! against the projection store.
//!
//! ## SQL Subset
//!
//! Supported SQL features:
//! - `SELECT` with column list or `*`
//! - `FROM` single table
//! - `WHERE` with comparison predicates (`=`, `<`, `>`, `<=`, `>=`, `IN`)
//! - `ORDER BY` (ascending/descending)
//! - `LIMIT`
//! - Parameterized queries (`$1`, `$2`, ...)
//!
//! Intentionally unsupported:
//! - `JOIN` (queries are single-table only)
//! - Subqueries
//! - Aggregations (`COUNT`, `SUM`, etc.)
//! - `GROUP BY`, `HAVING`
//! - `DISTINCT`
//!
//! ## Usage
//!
//! ```ignore
//! use vdb_query::{QueryEngine, Schema, SchemaBuilder, ColumnDef, DataType, Value};
//! use vdb_store::{BTreeStore, TableId};
//!
//! // Define schema
//! let schema = SchemaBuilder::new()
//!     .table(
//!         "users",
//!         TableId::new(1),
//!         vec![
//!             ColumnDef::new("id", DataType::BigInt).not_null(),
//!             ColumnDef::new("name", DataType::Text).not_null(),
//!         ],
//!         vec!["id".into()],
//!     )
//!     .build();
//!
//! // Create engine
//! let engine = QueryEngine::new(schema);
//!
//! // Execute query
//! let mut store = BTreeStore::open("data/projections")?;
//! let result = engine.query(&mut store, "SELECT * FROM users WHERE id = $1", &[Value::BigInt(42)])?;
//! ```
//!
//! ## Point-in-Time Queries
//!
//! For compliance, you can query at a specific log position:
//!
//! ```ignore
//! let result = engine.query_at(
//!     &mut store,
//!     "SELECT * FROM users WHERE id = 1",
//!     &[],
//!     Offset::new(1000),  // Query state as of log position 1000
//! )?;
//! ```

mod error;
mod executor;
mod key_encoder;
mod parser;
mod plan;
mod planner;
mod schema;
mod value;

#[cfg(test)]
mod tests;

// Re-export public types
pub use error::{QueryError, Result};
pub use executor::{QueryResult, Row};
pub use schema::{ColumnDef, ColumnName, DataType, Schema, SchemaBuilder, TableDef, TableName};
pub use value::Value;

use vdb_store::ProjectionStore;
use vdb_types::Offset;

/// Query engine for executing SQL against a projection store.
///
/// The engine is stateless and can be shared across threads.
/// It holds only the schema definition.
#[derive(Debug, Clone)]
pub struct QueryEngine {
    schema: Schema,
}

impl QueryEngine {
    /// Creates a new query engine with the given schema.
    pub fn new(schema: Schema) -> Self {
        Self { schema }
    }

    /// Returns a reference to the schema.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Executes a SQL query against the current store state.
    ///
    /// # Arguments
    ///
    /// * `store` - The projection store to query
    /// * `sql` - SQL query string
    /// * `params` - Query parameters (for `$1`, `$2`, etc.)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let result = engine.query(
    ///     &mut store,
    ///     "SELECT name FROM users WHERE id = $1",
    ///     &[Value::BigInt(42)],
    /// )?;
    /// ```
    pub fn query<S: ProjectionStore>(
        &self,
        store: &mut S,
        sql: &str,
        params: &[Value],
    ) -> Result<QueryResult> {
        // Parse SQL
        let parsed = parser::parse_query(sql)?;

        // Plan query
        let plan = planner::plan_query(&self.schema, &parsed, params)?;

        // Get table definition for executor
        let table_def = self
            .schema
            .get_table(&plan.table_name().into())
            .ok_or_else(|| QueryError::TableNotFound(plan.table_name().to_string()))?;

        // Execute
        executor::execute(store, &plan, table_def)
    }

    /// Executes a SQL query at a specific log position (point-in-time query).
    ///
    /// This enables compliance queries that show the state as it was
    /// at a specific point in the log.
    ///
    /// # Arguments
    ///
    /// * `store` - The projection store to query
    /// * `sql` - SQL query string
    /// * `params` - Query parameters
    /// * `position` - Log position to query at
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Get user state as of log position 1000
    /// let result = engine.query_at(
    ///     &mut store,
    ///     "SELECT * FROM users WHERE id = 1",
    ///     &[],
    ///     Offset::new(1000),
    /// )?;
    /// ```
    pub fn query_at<S: ProjectionStore>(
        &self,
        store: &mut S,
        sql: &str,
        params: &[Value],
        position: Offset,
    ) -> Result<QueryResult> {
        // Parse SQL
        let parsed = parser::parse_query(sql)?;

        // Plan query
        let plan = planner::plan_query(&self.schema, &parsed, params)?;

        // Get table definition
        let table_def = self
            .schema
            .get_table(&plan.table_name().into())
            .ok_or_else(|| QueryError::TableNotFound(plan.table_name().to_string()))?;

        // Execute at position
        executor::execute_at(store, &plan, table_def, position)
    }

    /// Parses a SQL query without executing it.
    ///
    /// Useful for validation or query plan inspection.
    pub fn prepare(&self, sql: &str, params: &[Value]) -> Result<PreparedQuery> {
        let parsed = parser::parse_query(sql)?;
        let plan = planner::plan_query(&self.schema, &parsed, params)?;

        Ok(PreparedQuery {
            plan,
            schema: self.schema.clone(),
        })
    }
}

/// A prepared (planned) query ready for execution.
#[derive(Debug, Clone)]
pub struct PreparedQuery {
    plan: plan::QueryPlan,
    schema: Schema,
}

impl PreparedQuery {
    /// Executes this prepared query against the current store state.
    pub fn execute<S: ProjectionStore>(&self, store: &mut S) -> Result<QueryResult> {
        let table_def = self
            .schema
            .get_table(&self.plan.table_name().into())
            .ok_or_else(|| QueryError::TableNotFound(self.plan.table_name().to_string()))?;

        executor::execute(store, &self.plan, table_def)
    }

    /// Executes this prepared query at a specific log position.
    pub fn execute_at<S: ProjectionStore>(
        &self,
        store: &mut S,
        position: Offset,
    ) -> Result<QueryResult> {
        let table_def = self
            .schema
            .get_table(&self.plan.table_name().into())
            .ok_or_else(|| QueryError::TableNotFound(self.plan.table_name().to_string()))?;

        executor::execute_at(store, &self.plan, table_def, position)
    }

    /// Returns the column names this query will return.
    pub fn columns(&self) -> &[ColumnName] {
        self.plan.column_names()
    }

    /// Returns the table name being queried.
    pub fn table_name(&self) -> &str {
        self.plan.table_name()
    }
}
