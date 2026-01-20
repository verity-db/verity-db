//! Hash chain for tamper-evident record linking.
//!
//! Each record's hash incorporates the previous record's hash, creating
//! a chain where modifying any record invalidates all subsequent hashes.
//!
//! ```text
//! Record 0: hash_0 = H(data_0)
//! Record 1: hash_1 = H(hash_0 || data_1)
//! Record 2: hash_2 = H(hash_1 || data_2)
//! ```
//!
//! # Example
//!
//! ```
//! use vdb_crypto::{chain_hash, ChainHash};
//!
//! let hash0 = chain_hash(None, b"genesis");
//! let hash1 = chain_hash(Some(&hash0), b"second");
//! let hash2 = chain_hash(Some(&hash1), b"third");
//! ```

use std::fmt::Debug;

use sha2::{Digest, Sha256};

// ============================================================================
// Constants
// ============================================================================

/// Length of a SHA-256 hash in bytes (256 bits).
///
/// SHA-256 is chosen as a FIPS 180-4 approved algorithm, providing
/// compliance for regulated industries (healthcare, finance, government).
pub const HASH_LENGTH: usize = 32;

/// Maximum data size for hashing (64 MiB).
///
/// This is a sanity limit to catch accidental misuse (e.g., passing
/// an entire file instead of a record). SHA-256 can hash arbitrary
/// lengths, but records should never approach this size.
///
/// Only used in debug assertions, hence allowed dead code in release.
#[allow(dead_code)]
const MAX_DATA_LENGTH: usize = 64 * 1024 * 1024;

// ============================================================================
// ChainHash
// ============================================================================

/// A 32-byte SHA-256 hash used for chaining records.
///
/// SHA-256 is chosen as a FIPS 180-4 approved algorithm, providing
/// compliance for regulated industries. While algorithms like Blake3
/// offer better performance, SHA-256 is required for FIPS compliance.
///
/// Each record's hash incorporates the previous record's hash,
/// creating a tamper-evident chain. If any record is modified,
/// all subsequent hashes become invalid.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
pub struct ChainHash([u8; HASH_LENGTH]);

impl ChainHash {
    /// Returns the hash as a byte slice.
    pub fn as_bytes(&self) -> &[u8; HASH_LENGTH] {
        &self.0
    }
}

impl From<[u8; HASH_LENGTH]> for ChainHash {
    fn from(value: [u8; HASH_LENGTH]) -> Self {
        Self(value)
    }
}

impl From<ChainHash> for [u8; HASH_LENGTH] {
    fn from(value: ChainHash) -> Self {
        value.0
    }
}

impl Debug for ChainHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ChainHash({:016x}...)",
            u64::from_le_bytes(self.0[..8].try_into().unwrap())
        )
    }
}

// ============================================================================
// Hash Chain Function
// ============================================================================

/// Computes the next hash in the chain.
///
/// Links `data` to the previous hash (if any), creating a tamper-evident
/// chain. Uses SHA-256 for FIPS 180-4 compliant, secure hashing.
///
/// # Arguments
///
/// * `prev` - The previous record's hash, or `None` for the first record
/// * `data` - The data to hash (typically a serialized record)
///
/// # Returns
///
/// A new [`ChainHash`] that incorporates both `prev` and `data`.
///
/// # Panics
///
/// Debug builds will panic if `data` exceeds [`MAX_DATA_LENGTH`] (64 MiB).
pub fn chain_hash(prev: Option<&ChainHash>, data: &[u8]) -> ChainHash {
    // Precondition: data length is reasonable (catches misuse)
    debug_assert!(
        data.len() <= MAX_DATA_LENGTH,
        "data exceeds {MAX_DATA_LENGTH} byte sanity limit"
    );

    let mut hasher = Sha256::new();

    if let Some(prev) = prev {
        hasher.update(prev.0);
    }
    hasher.update(data);

    let hash_bytes: [u8; HASH_LENGTH] = hasher.finalize().into();

    // Postcondition: hash isn't degenerate (all zeros would indicate a bug)
    debug_assert!(
        hash_bytes.iter().any(|&b| b != 0),
        "SHA-256 produced all-zero hash, indicating a bug"
    );

    ChainHash(hash_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chain_incorporates_prev_hash() {
        let data = b"same data";

        let hash1 = chain_hash(None, data);
        let hash2 = chain_hash(Some(&hash1), data);

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_chain_is_deterministic() {
        let prev = ChainHash::from([42u8; 32]);
        let data = b"test data";

        let hash1 = chain_hash(Some(&prev), data);
        let hash2 = chain_hash(Some(&prev), data);

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_chain_replay() {
        let r0 = chain_hash(None, b"genesis");
        let r1 = chain_hash(Some(&r0), b"second");
        let r2 = chain_hash(Some(&r1), b"third");

        let replay0 = chain_hash(None, b"genesis");
        let replay1 = chain_hash(Some(&replay0), b"second");
        let replay2 = chain_hash(Some(&replay1), b"third");

        assert_eq!(r0, replay0);
        assert_eq!(r1, replay1);
        assert_eq!(r2, replay2);
    }
}
