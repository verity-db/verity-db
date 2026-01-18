//! # VerityDB
//!
//! Compliance-native, healthcare-focused embedded database.
//!
//! VerityDB is built on a replicated append-only log with deterministic
//! projection to a custom storage engine. This provides:
//!
//! - **Correctness by design** - Ordered log → deterministic apply → snapshot
//! - **Full audit trail** - Every mutation is captured in the immutable log
//! - **Point-in-time recovery** - Replay from any offset
//! - **HIPAA compliance** - Built-in durability and encryption
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
//! This crate is being rebuilt with a custom storage engine.
//! The SQLite-based implementation has been removed.
//!
//! See PLAN.md for the implementation roadmap.

// Re-export commonly used types from vdb-types
pub use vdb_types::{
    DataClass,
    Offset,
    Placement,
    Region,
    StreamId,
    StreamName,
    TenantId,
};

// Re-export runtime for direct access
pub use vdb_runtime::{Runtime, RuntimeHandle};

#[cfg(test)]
mod tests {
    // TODO: Add tests when SDK is implemented
}
