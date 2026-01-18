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
//! | [`signature`] | Ed25519 signatures for non-repudiation | ✅ Ready |
//! | `encryption` | Envelope encryption for tenant isolation | 🔲 Planned |
//!
//! ## Quick Start
//!
//! ```
//! use vdb_crypto::{chain_hash, ChainHash, SigningKey};
//!
//! // Build a tamper-evident chain of records
//! let hash0 = chain_hash(None, b"genesis record");
//! let hash1 = chain_hash(Some(&hash0), b"second record");
//!
//! // Sign records for non-repudiation
//! let signing_key = SigningKey::generate();
//! let signature = signing_key.sign(hash1.as_bytes());
//!
//! // Verify the signature
//! let verifying_key = signing_key.verifying_key();
//! assert!(verifying_key.verify(hash1.as_bytes(), &signature).is_ok());
//! ```
//!
//! ## Planned Features
//!
//! - **Envelope Encryption**: Per-tenant data encryption with key rotation

pub mod chain;
pub mod error;
pub mod signature;

// Re-export primary types at crate root for convenience
pub use chain::{ChainHash, chain_hash};
pub use error::CryptoError;
pub use signature::{Signature, SigningKey, VerifyingKey};
