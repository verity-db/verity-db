//! The kernel - pure functional core of `VerityDB`.
//!
//! The kernel applies committed commands to produce new state and effects.
//! It is completely pure: no IO, no clocks, no randomness. This makes it
//! deterministic and easy to test.
//!
//! # Example
//!
//! ```ignore
//! let state = State::new();
//! let cmd = Command::create_stream(...);
//!
//! let (new_state, effects) = apply_committed(state, cmd)?;
//! // Runtime executes effects...
//! ```

use vdb_types::{AuditAction, Offset, StreamId, StreamMetadata};

use crate::command::Command;
use crate::effects::Effect;
use crate::state::State;

/// Applies a committed command to the state, producing new state and effects.
///
/// Takes ownership of state, returns new state. No cloning of the `BTreeMap`.
pub fn apply_committed(state: State, cmd: Command) -> Result<(State, Vec<Effect>), KernelError> {
    let mut effects = Vec::new();

    match cmd {
        Command::CreateStream {
            stream_id,
            stream_name,
            data_class,
            placement,
        } => {
            if state.stream_exists(&stream_id) {
                return Err(KernelError::StreamIdUniqueConstraint(stream_id));
            }

            // Create metadata
            let meta = StreamMetadata::new(
                stream_id,
                stream_name.clone(),
                data_class,
                placement.clone(),
            );

            // Effects need their own copies of metadata
            effects.push(Effect::StreamMetadataWrite(meta.clone()));
            effects.push(Effect::AuditLogAppend(AuditAction::StreamCreated {
                stream_id,
                stream_name,
                data_class,
                placement,
            }));

            // State takes ownership of meta
            Ok((state.with_stream(meta), effects))
        }

        Command::CreateStreamWithAutoId {
            stream_name,
            data_class,
            placement,
        } => {
            let (state, meta) =
                state.with_new_stream(stream_name.clone(), data_class, placement.clone());

            effects.push(Effect::StreamMetadataWrite(meta.clone()));
            effects.push(Effect::AuditLogAppend(AuditAction::StreamCreated {
                stream_id: meta.stream_id,
                stream_name,
                data_class,
                placement,
            }));

            Ok((state, effects))
        }

        Command::AppendBatch {
            stream_id,
            events,
            expected_offset,
        } => {
            let stream = state
                .get_stream(&stream_id)
                .ok_or(KernelError::StreamNotFound(stream_id))?;

            if stream.current_offset != expected_offset {
                return Err(KernelError::UnexpectedStreamOffset {
                    stream_id,
                    expected: expected_offset,
                    actual: stream.current_offset,
                });
            }

            let event_count = events.len();
            let base_offset = stream.current_offset;
            let new_offset = base_offset + Offset::from(event_count as i64);

            // StorageAppend takes ownership of events (moved, not cloned)
            effects.push(Effect::StorageAppend {
                stream_id,
                base_offset,
                events,
            });

            effects.push(Effect::WakeProjection {
                stream_id,
                from_offset: base_offset,
                to_offset: new_offset,
            });

            effects.push(Effect::AuditLogAppend(AuditAction::EventsAppended {
                stream_id,
                count: event_count as u32,
                from_offset: base_offset,
            }));

            Ok((state.with_updated_offset(stream_id, new_offset), effects))
        }
    }
}

/// Errors that can occur when applying commands to the kernel.
#[derive(thiserror::Error, Debug)]
pub enum KernelError {
    #[error("stream with id {0} already exists")]
    StreamIdUniqueConstraint(StreamId),

    #[error("stream with id {0} not found")]
    StreamNotFound(StreamId),

    #[error("offset mismatch for stream {stream_id}: expected {expected}, actual {actual}")]
    UnexpectedStreamOffset {
        stream_id: StreamId,
        expected: Offset,
        actual: Offset,
    },
}
