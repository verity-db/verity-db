//! Tenant-scoped handle for database operations.
//!
//! A `TenantHandle` provides operations scoped to a specific tenant ID.
//! All data isolation and access control is handled at this layer.

use bytes::Bytes;
use craton_kernel::Command;
use craton_query::{QueryResult, Value};
use craton_types::{DataClass, Offset, Placement, StreamId, StreamName, TenantId};

use crate::error::{Result, CratonError};
use crate::craton::Craton;

/// A tenant-scoped handle for database operations.
///
/// All operations through this handle are scoped to the tenant ID
/// specified when creating the handle.
///
/// # Example
///
/// ```ignore
/// let db = Craton::open("./data")?;
/// let tenant = db.tenant(TenantId::new(1));
///
/// // Create a stream for this tenant
/// tenant.create_stream("orders", DataClass::NonPHI)?;
///
/// // Append events
/// tenant.append("orders", vec![b"order_created".to_vec()])?;
///
/// // Query data
/// let results = tenant.query("SELECT * FROM events LIMIT 10", &[])?;
/// ```
#[derive(Clone)]
pub struct TenantHandle {
    db: Craton,
    tenant_id: TenantId,
}

impl TenantHandle {
    /// Creates a new tenant handle.
    pub(crate) fn new(db: Craton, tenant_id: TenantId) -> Self {
        Self { db, tenant_id }
    }

    /// Returns the tenant ID for this handle.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Creates a new stream for this tenant.
    ///
    /// # Arguments
    ///
    /// * `name` - Stream name (must be unique within the tenant)
    /// * `data_class` - Data classification (`PHI`, `NonPHI`, etc.)
    ///
    /// # Example
    ///
    /// ```ignore
    /// tenant.create_stream("audit_log", DataClass::PHI)?;
    /// ```
    pub fn create_stream(
        &self,
        name: impl Into<String>,
        data_class: DataClass,
    ) -> Result<StreamId> {
        let stream_name = StreamName::new(name);

        // For now, use tenant_id as stream_id base
        // Future: proper stream ID allocation
        let tenant_id_val: u64 = self.tenant_id.into();
        let stream_id = StreamId::new(tenant_id_val * 1_000_000 + 1);

        self.db.submit(Command::create_stream(
            stream_id,
            stream_name,
            data_class,
            Placement::Global,
        ))?;

        Ok(stream_id)
    }

    /// Creates a stream with automatic ID allocation.
    pub fn create_stream_auto(&self, name: impl Into<String>, data_class: DataClass) -> Result<()> {
        let stream_name = StreamName::new(name);

        self.db.submit(Command::create_stream_with_auto_id(
            stream_name,
            data_class,
            Placement::Global,
        ))
    }

    /// Appends events to a stream.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to append to
    /// * `events` - Events to append
    ///
    /// # Returns
    ///
    /// Returns the offset of the first appended event.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let events = vec![
    ///     b"event1".to_vec(),
    ///     b"event2".to_vec(),
    /// ];
    /// let offset = tenant.append(stream_id, events)?;
    /// ```
    pub fn append(&self, stream_id: StreamId, events: Vec<Vec<u8>>) -> Result<Offset> {
        let inner = self
            .db
            .inner()
            .read()
            .map_err(|_| CratonError::internal("lock poisoned"))?;

        // Get current stream offset
        let stream = inner
            .kernel_state
            .get_stream(&stream_id)
            .ok_or(CratonError::StreamNotFound(stream_id))?;

        let expected_offset = stream.current_offset;
        drop(inner);

        let events: Vec<Bytes> = events.into_iter().map(Bytes::from).collect();
        let event_count = events.len();

        self.db
            .submit(Command::append_batch(stream_id, events, expected_offset))?;

        // Return the offset of the first event
        Ok(Offset::new(
            expected_offset.as_u64().saturating_sub(event_count as u64),
        ))
    }

    /// Executes a write operation via SQL.
    ///
    /// Note: In Craton, writes go through the append-only log.
    /// SQL writes are translated to stream appends.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL statement (INSERT, UPDATE, DELETE)
    /// * `params` - Query parameters
    ///
    /// # Example
    ///
    /// ```ignore
    /// tenant.execute(
    ///     "INSERT INTO users (id, name) VALUES ($1, $2)",
    ///     &[Value::BigInt(1), Value::Text("Alice".into())],
    /// )?;
    /// ```
    pub fn execute(&self, _sql: &str, _params: &[Value]) -> Result<()> {
        // Future: Parse SQL INSERT/UPDATE/DELETE and translate to stream appends
        // For now, this is a placeholder
        Err(CratonError::internal(
            "SQL writes not yet implemented - use append() for direct stream writes",
        ))
    }

    /// Executes a SQL query against the current state.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL SELECT statement
    /// * `params` - Query parameters
    ///
    /// # Returns
    ///
    /// Query results as rows.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let results = tenant.query(
    ///     "SELECT * FROM events WHERE offset > $1 LIMIT 10",
    ///     &[Value::BigInt(100)],
    /// )?;
    /// for row in results.rows() {
    ///     println!("{:?}", row);
    /// }
    /// ```
    pub fn query(&self, sql: &str, params: &[Value]) -> Result<QueryResult> {
        let mut inner = self
            .db
            .inner()
            .write()
            .map_err(|_| CratonError::internal("lock poisoned"))?;

        // Clone the query engine to work around borrow checker
        // This is cheap since QueryEngine only holds a Schema reference
        let engine = inner.query_engine.clone();
        let result = engine.query(&mut inner.projection_store, sql, params)?;

        Ok(result)
    }

    /// Executes a SQL query at a specific log position (point-in-time query).
    ///
    /// This is essential for compliance: query the state as it was at a
    /// specific point in the log.
    ///
    /// # Arguments
    ///
    /// * `sql` - SQL SELECT statement
    /// * `params` - Query parameters
    /// * `position` - Log position to query at
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Get state as of log position 1000
    /// let results = tenant.query_at(
    ///     "SELECT * FROM users WHERE id = $1",
    ///     &[Value::BigInt(42)],
    ///     Offset::new(1000),
    /// )?;
    /// ```
    pub fn query_at(&self, sql: &str, params: &[Value], position: Offset) -> Result<QueryResult> {
        let mut inner = self
            .db
            .inner()
            .write()
            .map_err(|_| CratonError::internal("lock poisoned"))?;

        // Validate position is not ahead of current
        let current_pos = craton_store::ProjectionStore::applied_position(&inner.projection_store);
        if position > current_pos {
            return Err(CratonError::PositionAhead {
                requested: position,
                current: current_pos,
            });
        }

        // Clone the query engine to work around borrow checker
        let engine = inner.query_engine.clone();
        let result = engine.query_at(&mut inner.projection_store, sql, params, position)?;

        Ok(result)
    }

    /// Returns the current log position for this tenant.
    pub fn log_position(&self) -> Result<Offset> {
        self.db.log_position()
    }

    /// Reads events from a stream starting at an offset.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to read from
    /// * `from_offset` - Starting offset (inclusive)
    /// * `max_bytes` - Maximum number of bytes to read
    ///
    /// # Example
    ///
    /// ```ignore
    /// let events = tenant.read_events(stream_id, Offset::ZERO, 1024 * 1024_u64)?;
    /// for event in events {
    ///     process(event);
    /// }
    /// ```
    pub fn read_events(
        &self,
        stream_id: StreamId,
        from_offset: Offset,
        max_bytes: u64,
    ) -> Result<Vec<Bytes>> {
        let inner = self
            .db
            .inner()
            .read()
            .map_err(|_| CratonError::internal("lock poisoned"))?;

        let events = inner.storage.read_from(stream_id, from_offset, max_bytes)?;
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_tenant_create_stream() {
        let dir = tempdir().unwrap();
        let db = Craton::open(dir.path()).unwrap();
        let tenant = db.tenant(TenantId::new(1));

        let stream_id = tenant.create_stream("test", DataClass::NonPHI).unwrap();
        let stream_id_val: u64 = stream_id.into();
        assert!(stream_id_val > 0);
    }

    #[test]
    fn test_tenant_append_and_read() {
        let dir = tempdir().unwrap();
        let db = Craton::open(dir.path()).unwrap();
        let tenant = db.tenant(TenantId::new(1));

        let stream_id = tenant.create_stream("events", DataClass::NonPHI).unwrap();

        // Append events
        tenant
            .append(stream_id, vec![b"event1".to_vec(), b"event2".to_vec()])
            .unwrap();

        // Read back
        let events = tenant
            .read_events(stream_id, Offset::ZERO, 1024 * 1024_u64)
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(&events[0][..], b"event1");
        assert_eq!(&events[1][..], b"event2");
    }
}
