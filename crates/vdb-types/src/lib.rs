//! # vdb-types: Core types for `VerityDB`
//!
//! This crate contains shared types used across the `VerityDB` system:
//! - Entity IDs ([`TenantId`], [`StreamId`], [`Offset`], [`GroupId`])
//! - Data classification ([`DataClass`])
//! - Placement rules ([`Placement`], [`Region`])
//! - Stream metadata ([`StreamMetadata`])
//! - Audit actions ([`AuditAction`])
//! - Event persistence ([`EventPersister`], [`PersistError`])

use std::{
    fmt::{Debug, Display},
    ops::{Add, AddAssign, Sub},
};

use bytes::Bytes;
use serde::{Deserialize, Serialize};

// ============================================================================
// Entity IDs - All Copy (cheap 8-byte values)
// ============================================================================

/// Unique identifier for a tenant (organization/customer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TenantId(u64);

impl TenantId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl From<u64> for TenantId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<TenantId> for u64 {
    fn from(id: TenantId) -> Self {
        id.0
    }
}

/// Unique identifier for a stream within the system.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct StreamId(u64);

impl StreamId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl Add for StreamId {
    type Output = StreamId;

    fn add(self, rhs: Self) -> Self::Output {
        let v = self.0 + rhs.0;
        StreamId::new(v)
    }
}

impl Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for StreamId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<StreamId> for u64 {
    fn from(id: StreamId) -> Self {
        id.0
    }
}

/// Position of an event within a stream.
///
/// Offsets are zero-indexed and sequential. The first event in a stream
/// has offset 0, the second has offset 1, and so on.
///
/// Uses i64 internally (signed 64-bit for compatibility with various systems).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct Offset(i64);

impl Offset {
    pub const ZERO: Offset = Offset(0);

    pub fn new(offset: i64) -> Self {
        debug_assert!(offset >= 0, "Offset cannot be negative");
        Self(offset)
    }

    pub fn as_i64(&self) -> i64 {
        self.0
    }
}

impl Display for Offset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Add for Offset {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Offset {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for Offset {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl From<i64> for Offset {
    fn from(value: i64) -> Self {
        debug_assert!(value >= 0, "Offset cannot be negative");
        Self(value)
    }
}

impl From<Offset> for i64 {
    fn from(offset: Offset) -> Self {
        offset.0
    }
}

/// Unique identifier for a replication group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GroupId(u64);

impl Display for GroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl GroupId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl From<u64> for GroupId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl From<GroupId> for u64 {
    fn from(id: GroupId) -> Self {
        id.0
    }
}

// ============================================================================
// Stream Name - Clone (contains String, but rarely cloned)
// ============================================================================

/// Human-readable name for a stream.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StreamName(String);

impl StreamName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for StreamName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for StreamName {
    fn from(name: String) -> Self {
        Self(name)
    }
}

impl From<&str> for StreamName {
    fn from(name: &str) -> Self {
        Self(name.to_string())
    }
}

impl From<StreamName> for String {
    fn from(value: StreamName) -> Self {
        value.0
    }
}

// ============================================================================
// Data Classification - Copy (simple enum, no heap data)
// ============================================================================

/// Classification of data for compliance purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DataClass {
    /// Protected Health Information - subject to HIPAA restrictions.
    PHI,
    /// Non-PHI data that doesn't contain health information.
    NonPHI,
    /// Data that has been de-identified per HIPAA Safe Harbor.
    Deidentified,
}

// ============================================================================
// Placement - Clone (Region::Custom contains String)
// ============================================================================

/// Placement policy for a stream.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Placement {
    /// Data must remain within the specified region.
    Region(Region),
    /// Data can be replicated globally across all regions.
    Global,
}

/// Geographic region for data placement.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Region {
    /// US East (N. Virginia) - us-east-1
    USEast1,
    /// Asia Pacific (Sydney) - ap-southeast-2
    APSoutheast2,
    /// Custom region identifier
    Custom(String),
}

impl Region {
    pub fn custom(name: impl Into<String>) -> Self {
        Self::Custom(name.into())
    }
}

impl Display for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Region::USEast1 => write!(f, "us-east-1"),
            Region::APSoutheast2 => write!(f, "ap-southeast-2"),
            Region::Custom(custom) => write!(f, "{custom}"),
        }
    }
}

// ============================================================================
// Stream Metadata - Clone (created once per stream, cloned rarely)
// ============================================================================

/// Metadata describing a stream's configuration and current state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StreamMetadata {
    pub stream_id: StreamId,
    pub stream_name: StreamName,
    pub data_class: DataClass,
    pub placement: Placement,
    pub current_offset: Offset,
}

impl StreamMetadata {
    /// Creates new stream metadata with offset initialized to 0.
    pub fn new(
        stream_id: StreamId,
        stream_name: StreamName,
        data_class: DataClass,
        placement: Placement,
    ) -> Self {
        Self {
            stream_id,
            stream_name,
            data_class,
            placement,
            current_offset: Offset::default(),
        }
    }
}

// ============================================================================
// Batch Payload - NOT Clone (contains Vec<Bytes>, move only)
// ============================================================================

/// A batch of events to append to a stream.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchPayload {
    pub stream_id: StreamId,
    /// The events to append (zero-copy Bytes).
    pub events: Vec<Bytes>,
    /// Expected current offset for optimistic concurrency.
    pub expected_offset: Offset,
}

impl BatchPayload {
    pub fn new(stream_id: StreamId, events: Vec<Bytes>, expected_offset: Offset) -> Self {
        Self {
            stream_id,
            events,
            expected_offset,
        }
    }
}

// ============================================================================
// Audit Actions - Clone (for flexibility in logging)
// ============================================================================

/// Actions recorded in the audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditAction {
    /// A new stream was created.
    StreamCreated {
        stream_id: StreamId,
        stream_name: StreamName,
        data_class: DataClass,
        placement: Placement,
    },
    /// Events were appended to a stream.
    EventsAppended {
        stream_id: StreamId,
        count: u32,
        from_offset: Offset,
    },
}

// ============================================================================
// Event Persistence - Trait for durable event log writes
// ============================================================================

/// Abstraction for persisting events to the durable event log.
///
/// This trait is the bridge between the projection layer and the
/// `VerityDB` replication system. Implementations must block until
/// persistence is confirmed.
///
/// # Healthcare Compliance
///
/// This is the critical path for HIPAA compliance. The implementation must:
/// - **Block until VSR consensus** completes (quorum durability)
/// - **Return `Err`** if consensus fails (triggers rollback)
/// - **Never return `Ok`** unless events are durably stored
///
/// # Implementation Notes
///
/// The implementor (typically `Runtime`) must handle the sync→async bridge:
///
/// ```ignore
/// impl EventPersister for RuntimeHandle {
///     fn persist_blocking(&self, stream_id: StreamId, events: Vec<Bytes>) -> Result<Offset, PersistError> {
///         // Bridge sync callback to async runtime
///         tokio::task::block_in_place(|| {
///             tokio::runtime::Handle::current().block_on(async {
///                 self.inner.append(stream_id, events).await
///             })
///         })
///         .map_err(|e| {
///             tracing::error!(error = %e, "VSR persistence failed");
///             PersistError::ConsensusFailed
///         })
///     }
/// }
/// ```
///
/// # Why `Vec<Bytes>` instead of typed events?
///
/// Events are serialized before reaching this trait. This keeps `vdb-types`
/// decoupled from domain-specific event schemas.
pub trait EventPersister: Send + Sync + Debug {
    /// Persist a batch of serialized events to the durable event log.
    ///
    /// This method **blocks** until VSR consensus confirms the events are
    /// durably stored on a quorum of nodes.
    ///
    /// # Arguments
    ///
    /// * `stream_id` - The stream to append events to
    /// * `events` - Serialized events
    ///
    /// # Returns
    ///
    /// * `Ok(offset)` - Events persisted, returns the new stream offset
    /// * `Err(PersistError)` - Persistence failed, caller should rollback
    ///
    /// # Errors
    ///
    /// * [`PersistError::ConsensusFailed`] - VSR quorum unavailable after retries
    /// * [`PersistError::StorageError`] - Disk I/O or serialization failure
    /// * [`PersistError::ShuttingDown`] - System is terminating
    fn persist_blocking(
        &self,
        stream_id: StreamId,
        events: Vec<Bytes>,
    ) -> Result<Offset, PersistError>;
}

/// Error returned when event persistence fails.
///
/// The hook uses this to decide whether to rollback the transaction.
/// Specific underlying errors are logged by the implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistError {
    /// VSR consensus failed after retries (quorum unavailable)
    ConsensusFailed,
    /// Storage I/O error
    StorageError,
    /// System is shutting down
    ShuttingDown,
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConsensusFailed => write!(f, "consensus failed after retries"),
            Self::StorageError => write!(f, "storage I/O error"),
            Self::ShuttingDown => write!(f, "system is shutting down"),
        }
    }
}

impl std::error::Error for PersistError {}

#[cfg(test)]
mod tests;
