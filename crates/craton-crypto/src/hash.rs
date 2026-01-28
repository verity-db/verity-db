//! Dual-hash abstraction for compliance vs performance paths.
//!
//! `Craton` uses FIPS-approved algorithms for compliance-critical operations.
//! Additional cryptographic hashes may be used internally for performance.
//!
//! # Algorithm Selection
//!
//! | Purpose | Algorithm | FIPS | Use Case |
//! |---------|-----------|------|----------|
//! | Compliance | SHA-256 | Yes (180-4) | Audit trails, exports, proofs |
//! | Internal | BLAKE3 | No | Dedup, Merkle trees, fingerprints |
//!
//! # Example
//!
//! ```
//! use craton_crypto::hash::{HashPurpose, InternalHash, internal_hash, hash_with_purpose};
//!
//! // Fast internal hash (BLAKE3) for deduplication
//! let hash = internal_hash(b"content to fingerprint");
//!
//! // Purpose-driven selection
//! let (algo, digest) = hash_with_purpose(HashPurpose::Internal, b"data");
//! ```
//!
//! # Compliance Note
//!
//! All externally-verifiable proofs use FIPS-approved SHA-256 via [`crate::chain_hash`].
//! BLAKE3 is used only for internal performance optimization and never appears
//! in audit trails, checkpoints, or exported data.

use std::fmt::Debug;

use sha2::{Digest, Sha256};

// ============================================================================
// Constants
// ============================================================================

/// Length of both SHA-256 and BLAKE3 hashes in bytes (256 bits).
pub const HASH_LENGTH: usize = 32;

/// Maximum data size for hashing (64 MiB).
///
/// This is a sanity limit to catch accidental misuse.
/// Only used in debug assertions.
#[allow(dead_code)]
const MAX_DATA_LENGTH: usize = 64 * 1024 * 1024;

// ============================================================================
// HashPurpose
// ============================================================================

/// Distinguishes compliance-critical vs internal hashing.
///
/// This enum enforces the boundary between FIPS-compliant operations
/// and internal performance-optimized operations at the type level.
///
/// # Algorithm Selection
///
/// - [`HashPurpose::Compliance`] → SHA-256 (FIPS 180-4)
/// - [`HashPurpose::Internal`] → BLAKE3
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HashPurpose {
    /// FIPS-compliant hashing for audit trails, exports, proofs.
    ///
    /// Uses SHA-256 (FIPS 180-4). Required for:
    /// - Log record hash chains
    /// - Checkpoint sealing signatures
    /// - Audit exports and third-party proofs
    /// - Any data examined by regulators or auditors
    Compliance,

    /// High-performance hashing for internal operations.
    ///
    /// Uses BLAKE3. Appropriate for:
    /// - Content addressing and deduplication
    /// - Merkle tree construction for snapshots
    /// - Internal consistency verification
    /// - Streaming message fingerprinting
    Internal,
}

// ============================================================================
// HashAlgorithm
// ============================================================================

/// Hash algorithm identifier for versioned/tagged hashes.
///
/// Used when storing hashes to record which algorithm was used,
/// enabling future algorithm migration if needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum HashAlgorithm {
    /// SHA-256 (FIPS 180-4) - compliance path.
    Sha256 = 1,
    /// BLAKE3 - internal performance path.
    Blake3 = 2,
}

impl HashAlgorithm {
    /// Returns the algorithm used for the given purpose.
    #[must_use]
    pub const fn for_purpose(purpose: HashPurpose) -> Self {
        match purpose {
            HashPurpose::Compliance => Self::Sha256,
            HashPurpose::Internal => Self::Blake3,
        }
    }
}

// ============================================================================
// InternalHash
// ============================================================================

/// A 32-byte BLAKE3 hash for internal operations.
///
/// BLAKE3 is used for internal operations where performance matters
/// and FIPS compliance is not required. It is ~3-5x faster than SHA-256
/// and supports parallel hashing for large inputs.
///
/// # When to Use
///
/// - Content addressing and deduplication
/// - Merkle tree construction for snapshots
/// - Internal consistency verification
/// - Streaming message fingerprinting
///
/// # When NOT to Use
///
/// For compliance-critical paths (audit trails, exports, proofs),
/// use [`crate::ChainHash`] and [`crate::chain_hash`] instead.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InternalHash([u8; HASH_LENGTH]);

impl InternalHash {
    /// Length of the hash in bytes.
    pub const LENGTH: usize = HASH_LENGTH;

    /// Creates an `InternalHash` from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; HASH_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the hash as a byte slice.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; HASH_LENGTH] {
        &self.0
    }
}

impl From<[u8; HASH_LENGTH]> for InternalHash {
    fn from(value: [u8; HASH_LENGTH]) -> Self {
        Self(value)
    }
}

impl From<InternalHash> for [u8; HASH_LENGTH] {
    fn from(value: InternalHash) -> Self {
        value.0
    }
}

impl Debug for InternalHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "InternalHash({:016x}...)",
            u64::from_le_bytes(self.0[..8].try_into().unwrap())
        )
    }
}

// ============================================================================
// Hash Functions
// ============================================================================

/// Computes a BLAKE3 hash for internal operations.
///
/// This is the fast path for internal operations where FIPS compliance
/// is not required. BLAKE3 is ~3-5x faster than SHA-256 and supports
/// parallel hashing.
///
/// # Arguments
///
/// * `data` - The data to hash
///
/// # Returns
///
/// A 32-byte [`InternalHash`] (BLAKE3).
///
/// # Panics
///
/// Debug builds will panic if `data` exceeds 64 MiB.
///
/// # Example
///
/// ```
/// use craton_crypto::hash::internal_hash;
///
/// let hash = internal_hash(b"content for deduplication");
/// ```
#[must_use]
pub fn internal_hash(data: &[u8]) -> InternalHash {
    // Precondition: data length is reasonable
    debug_assert!(
        data.len() <= MAX_DATA_LENGTH,
        "data exceeds {MAX_DATA_LENGTH} byte sanity limit"
    );

    let hash = blake3::hash(data);
    let hash_bytes: [u8; HASH_LENGTH] = *hash.as_bytes();

    // Postcondition: hash isn't degenerate
    debug_assert!(
        hash_bytes.iter().any(|&b| b != 0),
        "BLAKE3 produced all-zero hash, indicating a bug"
    );

    InternalHash(hash_bytes)
}

/// Computes a hash using the algorithm for the given purpose.
///
/// This function selects the appropriate algorithm based on the purpose:
/// - [`HashPurpose::Compliance`] → SHA-256 (FIPS 180-4)
/// - [`HashPurpose::Internal`] → BLAKE3
///
/// # Arguments
///
/// * `purpose` - The intended use of this hash
/// * `data` - The data to hash
///
/// # Returns
///
/// A tuple of (`HashAlgorithm`, 32-byte digest).
///
/// # Panics
///
/// Debug builds will panic if `data` exceeds 64 MiB.
///
/// # Example
///
/// ```
/// use craton_crypto::hash::{HashPurpose, HashAlgorithm, hash_with_purpose};
///
/// // Compliance path uses SHA-256
/// let (algo, digest) = hash_with_purpose(HashPurpose::Compliance, b"audit data");
/// assert_eq!(algo, HashAlgorithm::Sha256);
///
/// // Internal path uses BLAKE3
/// let (algo, digest) = hash_with_purpose(HashPurpose::Internal, b"internal data");
/// assert_eq!(algo, HashAlgorithm::Blake3);
/// ```
#[must_use]
pub fn hash_with_purpose(purpose: HashPurpose, data: &[u8]) -> (HashAlgorithm, [u8; HASH_LENGTH]) {
    // Precondition: data length is reasonable
    debug_assert!(
        data.len() <= MAX_DATA_LENGTH,
        "data exceeds {MAX_DATA_LENGTH} byte sanity limit"
    );

    let (algorithm, hash_bytes) = match purpose {
        HashPurpose::Compliance => {
            let digest = Sha256::digest(data);
            (HashAlgorithm::Sha256, digest.into())
        }
        HashPurpose::Internal => {
            let hash = blake3::hash(data);
            (HashAlgorithm::Blake3, *hash.as_bytes())
        }
    };

    // Postcondition: hash isn't degenerate
    debug_assert!(
        hash_bytes.iter().any(|&b| b != 0),
        "Hash produced all-zero output, indicating a bug"
    );

    (algorithm, hash_bytes)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_hash_deterministic() {
        let data = b"test data for hashing";

        let hash1 = internal_hash(data);
        let hash2 = internal_hash(data);

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_internal_hash_different_inputs() {
        let hash1 = internal_hash(b"input one");
        let hash2 = internal_hash(b"input two");

        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_with_purpose_compliance_uses_sha256() {
        let (algo, _) = hash_with_purpose(HashPurpose::Compliance, b"data");
        assert_eq!(algo, HashAlgorithm::Sha256);
    }

    #[test]
    fn test_hash_with_purpose_internal_uses_blake3() {
        let (algo, _) = hash_with_purpose(HashPurpose::Internal, b"data");
        assert_eq!(algo, HashAlgorithm::Blake3);
    }

    #[test]
    fn test_hash_with_purpose_deterministic() {
        let data = b"same data";

        let (algo1, digest1) = hash_with_purpose(HashPurpose::Internal, data);
        let (algo2, digest2) = hash_with_purpose(HashPurpose::Internal, data);

        assert_eq!(algo1, algo2);
        assert_eq!(digest1, digest2);
    }

    #[test]
    fn test_compliance_and_internal_differ() {
        let data = b"same data";

        let (_, compliance_digest) = hash_with_purpose(HashPurpose::Compliance, data);
        let (_, internal_digest) = hash_with_purpose(HashPurpose::Internal, data);

        // Different algorithms produce different hashes
        assert_ne!(compliance_digest, internal_digest);
    }

    #[test]
    fn test_internal_hash_matches_blake3_crate() {
        let data = b"verify against blake3 crate directly";

        let internal = internal_hash(data);
        let direct = blake3::hash(data);

        assert_eq!(internal.as_bytes(), direct.as_bytes());
    }

    #[test]
    fn test_compliance_matches_sha256_crate() {
        use sha2::{Digest, Sha256};

        let data = b"verify against sha2 crate directly";

        let (_, digest) = hash_with_purpose(HashPurpose::Compliance, data);
        let direct: [u8; 32] = Sha256::digest(data).into();

        assert_eq!(digest, direct);
    }

    #[test]
    fn test_algorithm_for_purpose() {
        assert_eq!(
            HashAlgorithm::for_purpose(HashPurpose::Compliance),
            HashAlgorithm::Sha256
        );
        assert_eq!(
            HashAlgorithm::for_purpose(HashPurpose::Internal),
            HashAlgorithm::Blake3
        );
    }

    #[test]
    fn test_internal_hash_conversions() {
        let bytes = [42u8; HASH_LENGTH];

        let hash = InternalHash::from(bytes);
        let back: [u8; HASH_LENGTH] = hash.into();

        assert_eq!(bytes, back);
    }
}
