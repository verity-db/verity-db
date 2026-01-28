//! Commands that can be submitted to the kernel.
//!
//! Commands represent requests to modify system state. They are validated
//! and committed through VSR consensus before being applied to the kernel.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use craton_types::{DataClass, Offset, Placement, StreamId, StreamName};

/// A command to be applied to the kernel.
///
/// Commands are the inputs to the kernel's state machine. Each command
/// is validated, proposed to VSR, and once committed, applied to produce
/// a new state and effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Create a new event stream.
    CreateStream {
        stream_id: StreamId,
        stream_name: StreamName,
        data_class: DataClass,
        placement: Placement,
    },

    /// Create a stream with an auto-allocated ID
    CreateStreamWithAutoId {
        stream_name: StreamName,
        data_class: DataClass,
        placement: Placement,
    },

    /// Append a batch of events to an existing stream.
    AppendBatch {
        stream_id: StreamId,
        events: Vec<Bytes>,
        expected_offset: Offset,
    },
}

impl Command {
    /// Creates a new `CreateStream` command.
    ///
    /// Takes ownership of `stream_name` and placement (heap data).
    /// `StreamId` and `DataClass` are Copy.
    pub fn create_stream(
        stream_id: StreamId,
        stream_name: StreamName,
        data_class: DataClass,
        placement: Placement,
    ) -> Self {
        Self::CreateStream {
            stream_id,
            stream_name,
            data_class,
            placement,
        }
    }

    pub fn create_stream_with_auto_id(
        stream_name: StreamName,
        data_class: DataClass,
        placement: Placement,
    ) -> Self {
        Self::CreateStreamWithAutoId {
            stream_name,
            data_class,
            placement,
        }
    }

    /// Creates a new `AppendBatch` command.
    pub fn append_batch(stream_id: StreamId, events: Vec<Bytes>, expected_offset: Offset) -> Self {
        Self::AppendBatch {
            stream_id,
            events,
            expected_offset,
        }
    }
}
