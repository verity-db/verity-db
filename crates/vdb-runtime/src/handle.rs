//! Handle for bridging sync SQLite hooks to async Runtime.

use bytes::Bytes;
use std::sync::Arc;
use vdb_types::{EventPersister, Offset, PersistError, StreamId};
use vdb_vsr::GroupReplicator;

use crate::{Runtime, RuntimeError};

/// A cheaply-cloneable handle to the Runtime.
///
/// Implements `EventPersister` to bridge sync SQLite hooks to async Runtime.
/// This is what you pass to `TransactionHooks::new()`.
#[derive(Debug, Clone)]
pub struct RuntimeHandle<R: GroupReplicator> {
    inner: Arc<Runtime<R>>,
}

impl<R: GroupReplicator> RuntimeHandle<R> {
    /// Creates a new handle wrapping the runtime.
    pub fn new(runtime: Arc<Runtime<R>>) -> Self {
        Self { inner: runtime }
    }

    /// Access the underlying runtime for direct async operations.
    pub fn runtime(&self) -> &Runtime<R> {
        &self.inner
    }
}

impl<R: GroupReplicator + 'static> EventPersister for RuntimeHandle<R> {
    fn persist_blocking(
        &self,
        stream_id: StreamId,
        events: Vec<Bytes>,
    ) -> Result<Offset, PersistError> {
        // block_in_place: tells tokio "I'm about to block, move other tasks"
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.inner.append_raw(stream_id, events))
        })
        .map_err(|e| {
            tracing::error!(error = %e, "Event persistence failed");
            match e {
                RuntimeError::VsrError(_) => PersistError::ConsensusFailed,
                RuntimeError::StorageError(_) => PersistError::StorageError,
                _ => PersistError::StorageError,
            }
        })
    }
}
