//! # vdb-crypto: Cryptographic primitives for `VerityDB`
//!
//! This crate provides the cryptographic foundation for `VerityDB`'s
//! tamper-evident append-only log.
//!
//! ## Modules
//!
//! | Module | Purpose | Status |
//! |--------|---------|--------|
//! | [`chain`] | Hash chains for tamper evidence | ✅ Ready |
//! | `signature` | Ed25519 signatures for non-repudiation | 🔲 Planned |
//! | `encryption` | Envelope encryption for tenant isolation | 🔲 Planned |
//!
//! ## Quick Start
//!
//! ```
//! use vdb_crypto::{chain_hash, ChainHash};
//!
//! // Build a tamper-evident chain of records
//! let hash0 = chain_hash(None, b"genesis record");
//! let hash1 = chain_hash(Some(&hash0), b"second record");
//! let hash2 = chain_hash(Some(&hash1), b"third record");
//!
//! // If any record is modified, subsequent hashes become invalid
//! ```
//!
//! ## Planned Features
//!
//! - **Ed25519 Signatures**: Non-repudiation for record authorship
//! - **Envelope Encryption**: Per-tenant data encryption with key rotation

pub mod chain;

// Re-export primary types at crate root for convenience
pub use chain::{ChainHash, chain_hash};
