//! vdb-runtime: Orchestrator for VerityDB
//!
//! The runtime is the "imperative shell" that coordinates all VerityDB
//! components. It implements the request lifecycle:
//!
//! 1. Receive request (create_stream, append, etc.)
//! 2. Route to appropriate VSR group via directory
//! 3. Propose command to VSR consensus
//! 4. On commit: apply to kernel, execute effects
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        Runtime                               │
//! │  ┌─────────┐   ┌───────────┐   ┌────────┐   ┌─────────────┐ │
//! │  │Directory│ → │Replicator │ → │ Kernel │ → │   Effect    │ │
//! │  │(routing)│   │(consensus)│   │ (pure) │   │  Executor   │ │
//! │  └─────────┘   └───────────┘   └────────┘   └─────────────┘ │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use vdb_runtime::{Runtime, RuntimeHandle};
//! use vdb_vsr::SingleNodeGroupReplicator;
//! use std::sync::Arc;
//! use tokio::sync::RwLock;
//!
//! let runtime = Arc::new(Runtime::new(
//!     RwLock::new(State::new()),
//!     directory,
//!     SingleNodeGroupReplicator::new(),
//!     storage,
//! ));
//!
//! // For async operations
//! runtime.create_stream(stream_id, name, DataClass::PHI, placement).await?;
//!
//! // For SQLite hooks (sync → async bridge)
//! let handle = RuntimeHandle::new(Arc::clone(&runtime));
//! // handle implements EventPersister
//! ```

mod error;
mod handle;
mod runtime;

pub use error::RuntimeError;
pub use handle::RuntimeHandle;
pub use runtime::Runtime;
