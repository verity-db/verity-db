//! Query plan intermediate representation.
//!
//! Defines the execution plan produced by the query planner.

use std::ops::Bound;

use craton_store::{Key, TableId};

use crate::schema::ColumnName;
use crate::value::Value;

/// A query execution plan.
#[derive(Debug, Clone)]
pub enum QueryPlan {
    /// Point lookup: WHERE pk = value
    PointLookup {
        /// Table to query.
        table_id: TableId,
        /// Table name (for error messages).
        table_name: String,
        /// Encoded primary key.
        key: Key,
        /// Column indices to project (empty = all columns).
        columns: Vec<usize>,
        /// Column names to return.
        column_names: Vec<ColumnName>,
    },

    /// Range scan on primary key.
    RangeScan {
        /// Table to query.
        table_id: TableId,
        /// Table name (for error messages).
        table_name: String,
        /// Start bound (inclusive/exclusive/unbounded).
        start: Bound<Key>,
        /// End bound (inclusive/exclusive/unbounded).
        end: Bound<Key>,
        /// Additional filter to apply after fetching.
        filter: Option<Filter>,
        /// Maximum rows to return.
        limit: Option<usize>,
        /// Sort order.
        order: ScanOrder,
        /// Column indices to project (empty = all columns).
        columns: Vec<usize>,
        /// Column names to return.
        column_names: Vec<ColumnName>,
    },

    /// Full table scan with optional filter.
    TableScan {
        /// Table to query.
        table_id: TableId,
        /// Table name (for error messages).
        table_name: String,
        /// Filter to apply.
        filter: Option<Filter>,
        /// Maximum rows to return (after filtering).
        limit: Option<usize>,
        /// Sort order (client-side).
        order: Option<SortSpec>,
        /// Column indices to project (empty = all columns).
        columns: Vec<usize>,
        /// Column names to return.
        column_names: Vec<ColumnName>,
    },
}

impl QueryPlan {
    /// Returns the column names this plan will return.
    pub fn column_names(&self) -> &[ColumnName] {
        match self {
            QueryPlan::PointLookup { column_names, .. }
            | QueryPlan::RangeScan { column_names, .. }
            | QueryPlan::TableScan { column_names, .. } => column_names,
        }
    }

    /// Returns the column indices to project.
    #[allow(dead_code)]
    pub fn column_indices(&self) -> &[usize] {
        match self {
            QueryPlan::PointLookup { columns, .. }
            | QueryPlan::RangeScan { columns, .. }
            | QueryPlan::TableScan { columns, .. } => columns,
        }
    }

    /// Returns the table name.
    pub fn table_name(&self) -> &str {
        match self {
            QueryPlan::PointLookup { table_name, .. }
            | QueryPlan::RangeScan { table_name, .. }
            | QueryPlan::TableScan { table_name, .. } => table_name,
        }
    }
}

/// Scan order for range scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanOrder {
    /// Ascending order (natural B+tree order).
    #[default]
    Ascending,
    /// Descending order (reverse iteration).
    Descending,
}

/// Sort specification for table scans.
#[derive(Debug, Clone)]
pub struct SortSpec {
    /// Columns to sort by.
    pub columns: Vec<(usize, ScanOrder)>,
}

/// Filter to apply to scanned rows.
#[derive(Debug, Clone)]
pub struct Filter {
    /// Conditions (all must match).
    pub conditions: Vec<FilterCondition>,
}

impl Filter {
    /// Creates a filter with the given conditions.
    pub fn new(conditions: Vec<FilterCondition>) -> Self {
        Self { conditions }
    }

    /// Returns true if the filter has no conditions.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.conditions.is_empty()
    }

    /// Evaluates the filter against a row.
    pub fn matches(&self, row: &[Value]) -> bool {
        self.conditions.iter().all(|c| c.matches(row))
    }
}

/// A single filter condition.
#[derive(Debug, Clone)]
pub struct FilterCondition {
    /// Column index to compare.
    pub column_idx: usize,
    /// Comparison operator.
    pub op: FilterOp,
    /// Value to compare against.
    pub value: Value,
}

impl FilterCondition {
    /// Evaluates this condition against a row.
    pub fn matches(&self, row: &[Value]) -> bool {
        let Some(cell) = row.get(self.column_idx) else {
            return false;
        };

        match &self.op {
            FilterOp::Eq => cell == &self.value,
            FilterOp::Lt => cell.compare(&self.value) == Some(std::cmp::Ordering::Less),
            FilterOp::Le => matches!(
                cell.compare(&self.value),
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            ),
            FilterOp::Gt => cell.compare(&self.value) == Some(std::cmp::Ordering::Greater),
            FilterOp::Ge => matches!(
                cell.compare(&self.value),
                Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
            ),
            FilterOp::In(values) => values.contains(cell),
        }
    }
}

/// Filter comparison operator.
#[derive(Debug, Clone)]
pub enum FilterOp {
    /// Equal.
    Eq,
    /// Less than.
    Lt,
    /// Less than or equal.
    Le,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Ge,
    /// In list.
    In(Vec<Value>),
}
