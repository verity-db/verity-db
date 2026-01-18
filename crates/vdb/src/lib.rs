//! # `VerityDB`
//!
//! Compliance-native database for regulated industries.
//!
//! `VerityDB` is built on a replicated append-only log with deterministic
//! projection to a custom storage engine. This provides:
//!
//! - **Correctness by design** - Ordered log → deterministic apply → snapshot
//! - **Full audit trail** - Every mutation is captured in the immutable log
//! - **Point-in-time recovery** - Replay from any offset
//! - **Compliance by construction** - Built-in durability and encryption
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                         VerityDB                            │
//! │  ┌─────────┐   ┌───────────┐   ┌──────────┐   ┌──────────┐ │
//! │  │   Log   │ → │  Kernel   │ → │  Store   │ → │  Query   │ │
//! │  │(append) │   │(pure FSM) │   │(B+tree)  │   │  (SQL)   │ │
//! │  └─────────┘   └───────────┘   └──────────┘   └──────────┘ │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Status
//!
//! This crate is in early development. The foundation layer is being built:
//!
//! - **vdb-types**: Core type definitions ✅
//! - **vdb-crypto**: Cryptographic primitives (hash chains) ✅
//! - **vdb-storage**: Append-only log storage ✅
//! - **vdb-kernel**: Pure functional state machine ✅
//! - **vdb-directory**: Placement routing ✅
//!
//! See [PLAN.md](https://github.com/veritydb/veritydb/blob/main/PLAN.md) for
//! the implementation roadmap.

// Re-export core types from vdb-types
pub use vdb_types::{
    DataClass, GroupId, Offset, Placement, Region, StreamId, StreamMetadata, StreamName, TenantId,
};

// Re-export crypto primitives
pub use vdb_crypto::{chain_hash, ChainHash};

// Re-export storage types
pub use vdb_storage::{Record, Storage, StorageError};

// Re-export kernel types
pub use vdb_kernel::{apply_committed, Command, Effect, KernelError, State};

// Re-export directory
pub use vdb_directory::{Directory, DirectoryError};
