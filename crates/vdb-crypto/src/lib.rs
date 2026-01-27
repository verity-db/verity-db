//! # vdb-crypto: Cryptographic primitives for `VerityDB`
//!
//! This crate provides the cryptographic foundation for `VerityDB`'s
//! tamper-evident append-only log.
//!
//! ## Modules
//!
//! | Module | Purpose | Status |
//! |--------|---------|--------|
//! | [`chain`] | Hash chains for tamper evidence (SHA-256) | ✅ Ready |
//! | [`hash`] | Dual-hash abstraction (SHA-256/BLAKE3) | ✅ Ready |
//! | [`signature`] | Ed25519 signatures for non-repudiation | ✅ Ready |
//! | [`encryption`] | AES-256-GCM encryption and key wrapping | ✅ Ready |
//!
//! ## Quick Start
//!
//! ```
//! use vdb_crypto::{chain_hash, ChainHash, SigningKey, internal_hash, HashPurpose};
//! use vdb_crypto::{EncryptionKey, WrappedKey};
//!
//! // Build a tamper-evident chain of records (SHA-256 for compliance)
//! let hash0 = chain_hash(None, b"genesis record");
//! let hash1 = chain_hash(Some(&hash0), b"second record");
//!
//! // Fast internal hash (BLAKE3) for deduplication
//! let fingerprint = internal_hash(b"content to deduplicate");
//!
//! // Sign records for non-repudiation
//! let signing_key = SigningKey::generate();
//! let signature = signing_key.sign(hash1.as_bytes());
//!
//! // Verify the signature
//! let verifying_key = signing_key.verifying_key();
//! assert!(verifying_key.verify(hash1.as_bytes(), &signature).is_ok());
//!
//! // Wrap a key for secure storage (key hierarchy)
//! let kek = EncryptionKey::generate();
//! let dek = EncryptionKey::generate();
//! let wrapped = WrappedKey::new(&kek, &dek.to_bytes());
//! let unwrapped = wrapped.unwrap_key(&kek).unwrap();
//! assert_eq!(dek.to_bytes(), unwrapped);
//! ```

pub mod chain;
pub mod encryption;
pub mod error;
pub mod hash;
pub mod signature;

// Re-export primary types at crate root for convenience
pub use chain::{ChainHash, HASH_LENGTH, chain_hash};
pub use encryption::{
    DataEncryptionKey, EncryptionKey, InMemoryMasterKey, KeyEncryptionKey, MasterKeyProvider,
    WrappedKey,
};
pub use error::CryptoError;
pub use hash::{HashAlgorithm, HashPurpose, InternalHash, hash_with_purpose, internal_hash};
pub use signature::{Signature, SigningKey, VerifyingKey};
