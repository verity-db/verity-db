//! Effects produced by the kernel.
//!
//! Effects represent side effects that the runtime must execute after
//! a command is applied. The kernel is pure - it produces effects but
//! never executes them directly.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use craton_types::{AuditAction, Offset, StreamId, StreamMetadata};

/// An effect to be executed by the runtime.
///
/// Effects are produced by [`super::kernel::apply_committed`] and describe
/// actions that must be performed outside the pure kernel (storage writes,
/// projection updates, audit logging).
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    /// Write events to the durable storage layer.
    StorageAppend {
        /// The stream to append to.
        stream_id: StreamId,
        /// Starting offset for this batch.
        base_offset: Offset,
        /// The events to persist.
        events: Vec<Bytes>,
    },

    /// Persist stream metadata to the metadata store.
    StreamMetadataWrite(StreamMetadata),

    /// Notify projections that new events are available.
    WakeProjection {
        /// The stream with new events.
        stream_id: StreamId,
        /// First new event offset (inclusive).
        from_offset: Offset,
        /// Last new event offset (exclusive).
        to_offset: Offset,
    },

    /// Append an entry to the immutable audit log.
    AuditLogAppend(AuditAction),
}
