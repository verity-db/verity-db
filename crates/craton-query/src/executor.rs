//! Query executor: executes query plans against a projection store.

use std::cmp::Ordering;
use std::ops::Bound;

use bytes::Bytes;
use craton_store::{Key, ProjectionStore};
use craton_types::Offset;

use crate::error::{QueryError, Result};
use crate::key_encoder::successor_key;
use crate::plan::{QueryPlan, ScanOrder, SortSpec};
use crate::schema::{ColumnName, TableDef};
use crate::value::Value;

/// Result of executing a query.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Column names in result order.
    pub columns: Vec<ColumnName>,
    /// Result rows.
    pub rows: Vec<Row>,
}

impl QueryResult {
    /// Creates an empty result with the given columns.
    pub fn empty(columns: Vec<ColumnName>) -> Self {
        Self {
            columns,
            rows: vec![],
        }
    }

    /// Returns the number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Returns true if there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// A single result row.
pub type Row = Vec<Value>;

/// Executes a query plan against the current store state.
pub fn execute<S: ProjectionStore>(
    store: &mut S,
    plan: &QueryPlan,
    table_def: &TableDef,
) -> Result<QueryResult> {
    match plan {
        QueryPlan::PointLookup {
            table_id,
            key,
            columns,
            column_names,
            ..
        } => {
            let result = store.get(*table_id, key)?;
            match result {
                Some(bytes) => {
                    let row = decode_and_project(&bytes, columns, table_def)?;
                    Ok(QueryResult {
                        columns: column_names.clone(),
                        rows: vec![row],
                    })
                }
                None => Ok(QueryResult::empty(column_names.clone())),
            }
        }

        QueryPlan::RangeScan {
            table_id,
            start,
            end,
            filter,
            limit,
            order,
            columns,
            column_names,
            ..
        } => {
            let (start_key, end_key) = bounds_to_range(start, end);
            let scan_limit = limit.map(|l| l * 2).unwrap_or(10000); // Over-fetch for filtering

            let pairs = store.scan(*table_id, start_key..end_key, scan_limit)?;

            let mut rows = Vec::new();
            let row_iter: Box<dyn Iterator<Item = &(Key, Bytes)>> = match order {
                ScanOrder::Ascending => Box::new(pairs.iter()),
                ScanOrder::Descending => Box::new(pairs.iter().rev()),
            };

            for (_, bytes) in row_iter {
                let full_row = decode_row(bytes, table_def)?;

                // Apply filter
                if let Some(f) = filter {
                    if !f.matches(&full_row) {
                        continue;
                    }
                }

                // Project columns
                let projected = project_row(&full_row, columns);
                rows.push(projected);

                // Check limit
                if let Some(lim) = limit {
                    if rows.len() >= *lim {
                        break;
                    }
                }
            }

            Ok(QueryResult {
                columns: column_names.clone(),
                rows,
            })
        }

        QueryPlan::TableScan {
            table_id,
            filter,
            limit,
            order,
            columns,
            column_names,
            ..
        } => {
            // Scan entire table
            let scan_limit = limit.map(|l| l * 10).unwrap_or(100_000);
            let pairs = store.scan(*table_id, Key::min()..Key::max(), scan_limit)?;

            let mut rows = Vec::new();

            for (_, bytes) in &pairs {
                let full_row = decode_row(bytes, table_def)?;

                // Apply filter
                if let Some(f) = filter {
                    if !f.matches(&full_row) {
                        continue;
                    }
                }

                // Project columns
                let projected = project_row(&full_row, columns);
                rows.push(projected);
            }

            // Apply sort
            if let Some(sort_spec) = order {
                sort_rows(&mut rows, sort_spec);
            }

            // Apply limit
            if let Some(lim) = limit {
                rows.truncate(*lim);
            }

            Ok(QueryResult {
                columns: column_names.clone(),
                rows,
            })
        }
    }
}

/// Executes a query plan at a specific log position.
pub fn execute_at<S: ProjectionStore>(
    store: &mut S,
    plan: &QueryPlan,
    table_def: &TableDef,
    position: Offset,
) -> Result<QueryResult> {
    match plan {
        QueryPlan::PointLookup {
            table_id,
            key,
            columns,
            column_names,
            ..
        } => {
            let result = store.get_at(*table_id, key, position)?;
            match result {
                Some(bytes) => {
                    let row = decode_and_project(&bytes, columns, table_def)?;
                    Ok(QueryResult {
                        columns: column_names.clone(),
                        rows: vec![row],
                    })
                }
                None => Ok(QueryResult::empty(column_names.clone())),
            }
        }

        QueryPlan::RangeScan {
            table_id,
            start,
            end,
            filter,
            limit,
            order,
            columns,
            column_names,
            ..
        } => {
            let (start_key, end_key) = bounds_to_range(start, end);
            let scan_limit = limit.map(|l| l * 2).unwrap_or(10000);

            let pairs = store.scan_at(*table_id, start_key..end_key, scan_limit, position)?;

            let mut rows = Vec::new();
            let row_iter: Box<dyn Iterator<Item = &(Key, Bytes)>> = match order {
                ScanOrder::Ascending => Box::new(pairs.iter()),
                ScanOrder::Descending => Box::new(pairs.iter().rev()),
            };

            for (_, bytes) in row_iter {
                let full_row = decode_row(bytes, table_def)?;

                if let Some(f) = filter {
                    if !f.matches(&full_row) {
                        continue;
                    }
                }

                let projected = project_row(&full_row, columns);
                rows.push(projected);

                if let Some(lim) = limit {
                    if rows.len() >= *lim {
                        break;
                    }
                }
            }

            Ok(QueryResult {
                columns: column_names.clone(),
                rows,
            })
        }

        QueryPlan::TableScan {
            table_id,
            filter,
            limit,
            order,
            columns,
            column_names,
            ..
        } => {
            let scan_limit = limit.map(|l| l * 10).unwrap_or(100_000);
            let pairs = store.scan_at(*table_id, Key::min()..Key::max(), scan_limit, position)?;

            let mut rows = Vec::new();

            for (_, bytes) in &pairs {
                let full_row = decode_row(bytes, table_def)?;

                if let Some(f) = filter {
                    if !f.matches(&full_row) {
                        continue;
                    }
                }

                let projected = project_row(&full_row, columns);
                rows.push(projected);
            }

            if let Some(sort_spec) = order {
                sort_rows(&mut rows, sort_spec);
            }

            if let Some(lim) = limit {
                rows.truncate(*lim);
            }

            Ok(QueryResult {
                columns: column_names.clone(),
                rows,
            })
        }
    }
}

/// Converts bounds to a range.
///
/// The store scan uses a half-open range [start, end), so we need to:
/// - For Included start: use the key as-is
/// - For Excluded start: use the successor key (to skip the excluded value)
/// - For Included end: use successor key (to include the value)
/// - For Excluded end: use the key as-is
fn bounds_to_range(start: &Bound<Key>, end: &Bound<Key>) -> (Key, Key) {
    let start_key = match start {
        Bound::Included(k) => k.clone(),
        Bound::Excluded(k) => successor_key(k),
        Bound::Unbounded => Key::min(),
    };

    let end_key = match end {
        Bound::Included(k) => successor_key(k),
        Bound::Excluded(k) => k.clone(),
        Bound::Unbounded => Key::max(),
    };

    (start_key, end_key)
}

/// Decodes a JSON row and projects columns.
fn decode_and_project(bytes: &Bytes, columns: &[usize], table_def: &TableDef) -> Result<Row> {
    let full_row = decode_row(bytes, table_def)?;
    Ok(project_row(&full_row, columns))
}

/// Decodes a JSON row to values.
fn decode_row(bytes: &Bytes, table_def: &TableDef) -> Result<Row> {
    let json: serde_json::Value = serde_json::from_slice(bytes)?;

    let obj = json.as_object().ok_or_else(|| QueryError::TypeMismatch {
        expected: "object".to_string(),
        actual: format!("{json:?}"),
    })?;

    let mut row = Vec::with_capacity(table_def.columns.len());

    for col_def in &table_def.columns {
        let col_name = col_def.name.as_str();
        let json_val = obj.get(col_name).unwrap_or(&serde_json::Value::Null);
        let value = Value::from_json(json_val, col_def.data_type)?;
        row.push(value);
    }

    Ok(row)
}

/// Projects a row to selected columns.
fn project_row(full_row: &[Value], columns: &[usize]) -> Row {
    if columns.is_empty() {
        // Empty columns means all columns
        return full_row.to_vec();
    }

    columns
        .iter()
        .map(|&idx| full_row.get(idx).cloned().unwrap_or(Value::Null))
        .collect()
}

/// Sorts rows according to the sort specification.
fn sort_rows(rows: &mut [Row], spec: &SortSpec) {
    rows.sort_by(|a, b| {
        for (col_idx, order) in &spec.columns {
            let a_val = a.get(*col_idx);
            let b_val = b.get(*col_idx);

            let cmp = match (a_val, b_val) {
                (Some(av), Some(bv)) => av.compare(bv).unwrap_or(Ordering::Equal),
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Less,
                (Some(_), None) => Ordering::Greater,
            };

            if cmp != Ordering::Equal {
                return match order {
                    ScanOrder::Ascending => cmp,
                    ScanOrder::Descending => cmp.reverse(),
                };
            }
        }
        Ordering::Equal
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Filter;
    use crate::plan::FilterCondition;
    use crate::plan::FilterOp;

    #[test]
    fn test_project_row() {
        let row = vec![
            Value::BigInt(1),
            Value::Text("alice".to_string()),
            Value::BigInt(30),
        ];

        let projected = project_row(&row, &[0, 2]);
        assert_eq!(projected, vec![Value::BigInt(1), Value::BigInt(30)]);
    }

    #[test]
    fn test_project_row_all() {
        let row = vec![Value::BigInt(1), Value::Text("bob".to_string())];
        let projected = project_row(&row, &[]);
        assert_eq!(projected, row);
    }

    #[test]
    fn test_filter_matches() {
        let row = vec![Value::BigInt(42), Value::Text("alice".to_string())];

        let filter = Filter::new(vec![FilterCondition {
            column_idx: 0,
            op: FilterOp::Eq,
            value: Value::BigInt(42),
        }]);

        assert!(filter.matches(&row));

        let filter_miss = Filter::new(vec![FilterCondition {
            column_idx: 0,
            op: FilterOp::Eq,
            value: Value::BigInt(99),
        }]);

        assert!(!filter_miss.matches(&row));
    }

    #[test]
    fn test_sort_rows() {
        let mut rows = vec![
            vec![Value::BigInt(3), Value::Text("c".to_string())],
            vec![Value::BigInt(1), Value::Text("a".to_string())],
            vec![Value::BigInt(2), Value::Text("b".to_string())],
        ];

        let spec = SortSpec {
            columns: vec![(0, ScanOrder::Ascending)],
        };

        sort_rows(&mut rows, &spec);

        assert_eq!(rows[0][0], Value::BigInt(1));
        assert_eq!(rows[1][0], Value::BigInt(2));
        assert_eq!(rows[2][0], Value::BigInt(3));
    }

    #[test]
    fn test_sort_rows_descending() {
        let mut rows = vec![
            vec![Value::BigInt(1)],
            vec![Value::BigInt(3)],
            vec![Value::BigInt(2)],
        ];

        let spec = SortSpec {
            columns: vec![(0, ScanOrder::Descending)],
        };

        sort_rows(&mut rows, &spec);

        assert_eq!(rows[0][0], Value::BigInt(3));
        assert_eq!(rows[1][0], Value::BigInt(2));
        assert_eq!(rows[2][0], Value::BigInt(1));
    }
}
