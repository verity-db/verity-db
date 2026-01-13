//! Event abstractions for projection handlers.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use vdb_types::{Offset, StreamId};

/// Trait for domain events that can be applied to projections.
pub trait Event: Serialize + for<'de> Deserialize<'de> + Send + Sync {
    /// Returns the event type name (used for routing/deserialization).
    fn event_type(&self) -> &'static str;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope<E> {
    pub stream_id: StreamId,
    pub offset: Offset,
    pub timestamp: OffsetDateTime,
    pub payload: E,
}

impl<E: Event> EventEnvelope<E> {
    pub fn new(stream_id: StreamId, offset: Offset, payload: E) -> Self {
        Self {
            stream_id,
            offset,
            timestamp: OffsetDateTime::now_utc(),
            payload,
        }
    }

    pub fn event_type(&self) -> &'static str {
        self.payload.event_type()
    }
}
