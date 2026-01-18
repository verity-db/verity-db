//! # vdb-kernel: Functional core of `VerityDB`
//!
//! The kernel is the pure, deterministic heart of the system. It receives
//! committed commands and produces state changes plus effects to execute.
//!
//! ## Key Principles
//!
//! - **No IO**: The kernel never touches disk, network, or any external resource
//! - **No clocks**: Timestamps are added by the runtime, not the kernel
//! - **No randomness**: Same input always produces same output
//! - **Pure functions**: `apply_committed(state, command) -> (state, effects)`
//!
//! ## Architecture
//!
//! - [`command`]: Commands that can be submitted (`CreateStream`, `AppendBatch`)
//! - [`effects`]: Effects for the runtime to execute (`StorageAppend`, `WakeProjection`)
//! - [`state`]: In-memory kernel state
//! - [`kernel`]: The `apply_committed` function that ties it all together
//!
//! ## Example
//!
//! ```ignore
//! use vdb_kernel::{command::Command, kernel::apply_committed, state::State};
//!
//! let state = State::new();
//! let cmd = Command::create_stream(...);
//!
//! match apply_committed(state, cmd) {
//!     Ok((new_state, effects)) => {
//!         // Execute effects via runtime...
//!     }
//!     Err(e) => {
//!         // Handle error...
//!     }
//! }
//! ```

pub mod command;
pub mod effects;
pub mod kernel;
pub mod state;
// pub mod slices; // TODO: Add vertical slices when needed

#[cfg(test)]
mod tests;

// Re-export commonly used items
pub use command::Command;
pub use effects::Effect;
pub use kernel::{KernelError, apply_committed};
pub use state::State;
