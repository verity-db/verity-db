//! preupdate_hook implementation for capturing SQLite mutations.
//!
//! This module provides the core event capture mechanism for VerityDB.
//! SQLite's preupdate_hook fires synchronously before each INSERT, UPDATE,
//! or DELETE operation, allowing us to capture the change and send it to
//! the event log before the write completes.

use sqlx::{
    SqlitePool,
    sqlite::{PreupdateHookResult, SqliteOperation},
};
use std::sync::{
    Arc,
    mpsc::{self, Receiver},
};

use crate::{ChangeEvent, ProjectionError, RowId, SqlValue, TableName, schema::SchemaCache};

/// Manages the SQLite preupdate_hook for capturing database mutations.
///
/// The handler installs a hook that fires before each INSERT, UPDATE, or DELETE.
/// Captured events are sent through a channel for asynchronous processing and
/// persistence to the event log.
///
/// # Example
///
/// ```ignore
/// use vdb_projections::realtime::PreUpdateHandler;
/// use std::sync::Arc;
///
/// let handler = PreUpdateHandler::new(pool, Arc::clone(&schema_cache));
/// let rx = handler.install().await?;
///
/// // Process events in a separate task
/// tokio::spawn(async move {
///     while let Ok(event) = rx.recv() {
///         event_log.append(event).await;
///     }
/// });
/// ```
pub struct PreUpdateHandler {
    pool: SqlitePool,
    schema_cache: Arc<SchemaCache>,
}

impl PreUpdateHandler {
    /// Creates a new PreUpdateHandler.
    ///
    /// # Arguments
    /// * `pool` - SQLite connection pool
    /// * `schema_cache` - Shared schema cache for column name lookups
    pub fn new(pool: SqlitePool, schema_cache: Arc<SchemaCache>) -> Self {
        Self { pool, schema_cache }
    }

    /// Installs the preupdate hook and returns a receiver for captured events.
    ///
    /// The hook fires synchronously before each database mutation. Events are
    /// sent through the returned channel for asynchronous processing.
    ///
    /// # Returns
    /// A receiver that yields [`ChangeEvent`]s for each captured mutation.
    ///
    /// # Panics
    /// Panics if a mutation occurs on a table not in the schema cache.
    /// This indicates the table was created outside VerityDB migrations.
    pub async fn install(&self) -> Result<Receiver<ChangeEvent>, ProjectionError> {
        let (tx, rx) = mpsc::channel();

        let mut conn = self.pool.acquire().await?;
        let mut handle = conn.lock_handle().await?;

        let schema_cache = Arc::clone(&self.schema_cache);

        // Spawn the hook
        handle.set_preupdate_hook(move |result: PreupdateHookResult<'_>| {
            let table = result.table;
            let column_count = result.get_column_count();

            let table_name = TableName::from_sqlite(table);
            if table_name.is_internal() {
                return; // skip, don't panic
            }

            let columns = schema_cache
                .get_columns(&table_name).unwrap_or_else(|| {
                    panic!(
                    "table '{}' not in schema cache - was it created outside of VerityDB migrations?",
                        table_name
                )
                });

            let change_event = match result.operation {
                SqliteOperation::Insert => {
                    let row_id = RowId::from(result.get_new_row_id().unwrap());
                    let mut values = Vec::with_capacity(column_count as usize);
                    for i in 0..column_count {
                        let value_ref = result.get_new_column_value(i).unwrap();
                        values.push(SqlValue::try_from(value_ref).unwrap());
                    }
                    ChangeEvent::Insert {
                        table_name,
                        row_id,
                        column_names: columns,
                        values,
                    }
                }
                SqliteOperation::Update => {
                    let row_id = RowId::from(result.get_old_row_id().unwrap());
                    let mut old_values = Vec::with_capacity(column_count as usize);
                    let mut new_values = Vec::with_capacity(column_count as usize);
                    for i in 0..column_count {
                        let old_value_ref = result.get_old_column_value(i).unwrap();
                        let new_value_ref = result.get_new_column_value(i).unwrap();
                        old_values.push((
                            columns[i as usize].clone(),
                            SqlValue::try_from(old_value_ref).unwrap(),
                        ));
                        new_values.push((
                            columns[i as usize].clone(),
                            SqlValue::try_from(new_value_ref).unwrap(),
                        ));
                    }
                    ChangeEvent::Update {
                        table_name,
                        row_id,
                        old_values,
                        new_values,
                    }
                }
                SqliteOperation::Delete => {
                    let row_id = RowId::from(result.get_old_row_id().unwrap());
                    let mut deleted_values = Vec::with_capacity(column_count as usize);
                    for i in 0..column_count {
                        let deleted_value_ref = result.get_old_column_value(i).unwrap();
                        deleted_values.push(SqlValue::try_from(deleted_value_ref).unwrap());
                    }

                    ChangeEvent::Delete {
                        table_name,
                        row_id,
                        deleted_values,
                    }
                }
                _ => {
                    return;
                }
            };

            let _ = tx.send(change_event); // Ignore error if reciever dropped
        });

        Ok(rx)
    }
}
