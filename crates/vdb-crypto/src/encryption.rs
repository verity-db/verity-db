//! AES-256-GCM authenticated encryption for tenant data isolation.
//!
//! Provides envelope encryption where each tenant's data is encrypted with
//! a unique key. AES-256-GCM is a FIPS 197 approved AEAD cipher that provides
//! both confidentiality and integrity.
//!
//! # Example
//!
//! ```
//! use vdb_crypto::encryption::{EncryptionKey, Nonce, encrypt, decrypt};
//!
//! let key = EncryptionKey::generate();
//! let position: u64 = 42;  // Log position of the record
//! let nonce = Nonce::from_position(position);
//!
//! let plaintext = b"sensitive tenant data";
//! let ciphertext = encrypt(&key, &nonce, plaintext);
//!
//! let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();
//! assert_eq!(decrypted, plaintext.as_slice());
//! ```
//!
//! # Security
//!
//! - **Never reuse a nonce** with the same key. For `VerityDB`, nonces are
//!   derived from log position to guarantee uniqueness.
//! - Store keys encrypted at rest (key hierarchy not yet implemented).
//! - The authentication tag prevents tampering — decryption fails if the
//!   ciphertext or tag is modified.

use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::CryptoError;

// ============================================================================
// Constants
// ============================================================================

/// Length of an AES-256-GCM encryption key in bytes (256 bits).
///
/// AES-256-GCM is chosen as a FIPS 197 approved algorithm, providing
/// compliance for regulated industries (healthcare, finance, government).
pub const KEY_LENGTH: usize = 32;

/// Length of an AES-256-GCM nonce in bytes (96 bits).
///
/// In `VerityDB`, nonces are derived from the log position to guarantee
/// uniqueness without requiring random generation.
pub const NONCE_LENGTH: usize = 12;

/// Length of the AES-GCM authentication tag in bytes (128 bits).
///
/// The authentication tag ensures integrity — decryption fails if the
/// ciphertext or tag has been tampered with.
pub const TAG_LENGTH: usize = 16;

// ============================================================================
// EncryptionKey
// ============================================================================

/// An AES-256-GCM encryption key (256 bits).
///
/// This is secret key material that must be protected. Use [`EncryptionKey::generate`]
/// to create a new random key, or [`EncryptionKey::from_bytes`] to restore from
/// secure storage.
///
/// Key material is securely zeroed from memory when dropped via [`ZeroizeOnDrop`].
///
/// # Security
///
/// - Never log or expose the key bytes
/// - Store encrypted at rest (wrap with a KEK from the key hierarchy)
/// - Use one key per tenant/segment for isolation
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey {
    key: [u8; KEY_LENGTH],
}

impl EncryptionKey {
    /// Generates a new random encryption key using the OS CSPRNG.
    ///
    /// # Panics
    ///
    /// Panics if the OS CSPRNG fails (catastrophic system error).
    pub fn generate() -> Self {
        let key: [u8; KEY_LENGTH] = generate_random();

        // Postcondition: CSPRNG produced non-degenerate output
        debug_assert!(key.iter().any(|&b| b != 0), "CSPRNG produced all-zero key");

        Self { key }
    }

    /// Restores an encryption key from its 32-byte representation.
    ///
    /// # Security
    ///
    /// Only use bytes from a previously generated key or a secure KDF.
    pub fn from_bytes(bytes: &[u8; KEY_LENGTH]) -> Self {
        // Precondition: caller didn't pass degenerate key material
        debug_assert!(bytes.iter().any(|&b| b != 0), "key bytes are all zeros");

        Self { key: *bytes }
    }

    /// Returns the raw 32-byte key material.
    ///
    /// # Security
    ///
    /// Handle with care — this is secret key material.
    pub fn to_bytes(&self) -> [u8; KEY_LENGTH] {
        self.key
    }
}

// ============================================================================
// Nonce
// ============================================================================

/// An AES-256-GCM nonce (96 bits) derived from log position.
///
/// Nonces in `VerityDB` are deterministically derived from the record's log
/// position, guaranteeing uniqueness without requiring random generation.
/// The 8-byte position is placed in the first 8 bytes; the remaining 4 bytes
/// are reserved (currently zero).
///
/// ```text
/// Nonce layout (12 bytes):
/// ┌────────────────────────────┬──────────────┐
/// │  position (8 bytes, LE)    │  reserved    │
/// │  [0..8]                    │  [8..12]     │
/// └────────────────────────────┴──────────────┘
/// ```
///
/// # Security
///
/// **Never reuse a nonce with the same key.** Nonce reuse completely breaks
/// the confidentiality of AES-GCM. Position-derived nonces guarantee uniqueness
/// as long as each position is encrypted at most once per key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Nonce {
    bytes: [u8; NONCE_LENGTH],
}

impl Nonce {
    /// Creates a nonce from a log position.
    ///
    /// The position's little-endian bytes occupy the first 8 bytes of the
    /// nonce. The remaining 4 bytes are reserved for future use (e.g., key
    /// rotation counter) and are currently set to zero.
    ///
    /// # Arguments
    ///
    /// * `position` - The log position of the record being encrypted
    pub fn from_position(position: u64) -> Self {
        let mut nonce = [0u8; NONCE_LENGTH];
        nonce[..8].copy_from_slice(&position.to_le_bytes());

        Self { bytes: nonce }
    }

    /// Returns the raw 12-byte nonce.
    ///
    /// Use this for serialization or when interfacing with external systems.
    pub fn to_bytes(&self) -> [u8; NONCE_LENGTH] {
        self.bytes
    }
}

// ============================================================================
// Ciphertext
// ============================================================================

/// Encrypted data with its authentication tag.
///
/// Contains the ciphertext followed by a 16-byte AES-GCM authentication tag.
/// The total length is `plaintext.len() + TAG_LENGTH`. Decryption will fail
/// if the ciphertext or tag has been modified.
///
/// ```text
/// Ciphertext layout:
/// ┌────────────────────────────┬──────────────────┐
/// │  encrypted data            │  auth tag        │
/// │  [0..plaintext.len()]      │  [last 16 bytes] │
/// └────────────────────────────┴──────────────────┘
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Ciphertext {
    data: Vec<u8>,
}

impl Ciphertext {
    /// Creates a ciphertext from raw bytes.
    ///
    /// # Arguments
    ///
    /// * `bytes` - The encrypted data with authentication tag appended
    ///
    /// # Panics
    ///
    /// Debug builds panic if `bytes.len() < TAG_LENGTH` (no room for auth tag).
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        debug_assert!(
            bytes.len() >= TAG_LENGTH,
            "ciphertext too short: must be at least {TAG_LENGTH} bytes for auth tag"
        );

        Self { data: bytes }
    }

    /// Returns the raw ciphertext bytes (including authentication tag).
    ///
    /// Use this for serialization or storage.
    pub fn to_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Returns the length of the ciphertext (including authentication tag).
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true if the ciphertext is empty (which would be invalid).
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

// ============================================================================
// Encrypt / Decrypt
// ============================================================================

/// Maximum plaintext size for encryption (64 MiB).
///
/// This is a sanity limit to catch accidental misuse. AES-GCM can encrypt
/// larger messages, but individual records should never approach this size.
#[allow(dead_code)]
const MAX_PLAINTEXT_LENGTH: usize = 64 * 1024 * 1024;

/// Encrypts plaintext using AES-256-GCM.
///
/// Returns a [`Ciphertext`] containing the encrypted data with a 16-byte
/// authentication tag appended. The ciphertext length is `plaintext.len() + 16`.
///
/// # Arguments
///
/// * `key` - The encryption key
/// * `nonce` - A unique nonce derived from log position (never reuse with the same key!)
/// * `plaintext` - The data to encrypt
///
/// # Returns
///
/// A [`Ciphertext`] containing the encrypted data and authentication tag.
///
/// # Panics
///
/// Debug builds panic if `plaintext` exceeds 64 MiB (sanity limit).
pub fn encrypt(key: &EncryptionKey, nonce: &Nonce, plaintext: &[u8]) -> Ciphertext {
    // Precondition: plaintext length is reasonable
    debug_assert!(
        plaintext.len() <= MAX_PLAINTEXT_LENGTH,
        "plaintext exceeds {MAX_PLAINTEXT_LENGTH} byte sanity limit"
    );

    let cipher = Aes256Gcm::new_from_slice(&key.key).expect("KEY_LENGTH is always valid");
    let nonce_array = nonce.bytes.into();

    let data = cipher
        .encrypt(&nonce_array, plaintext)
        .expect("AES-GCM encryption cannot fail with valid inputs");

    // Postcondition: ciphertext is plaintext + tag
    debug_assert_eq!(
        data.len(),
        plaintext.len() + TAG_LENGTH,
        "ciphertext length mismatch"
    );

    Ciphertext { data }
}

/// Decrypts ciphertext using AES-256-GCM.
///
/// Verifies the authentication tag and returns the original plaintext if valid.
///
/// # Arguments
///
/// * `key` - The encryption key (must match the key used for encryption)
/// * `nonce` - The nonce used during encryption (same log position)
/// * `ciphertext` - The encrypted data with authentication tag
///
/// # Errors
///
/// Returns [`CryptoError::DecryptionError`] if:
/// - The key is incorrect
/// - The nonce is incorrect
/// - The ciphertext has been tampered with
/// - The authentication tag is invalid
///
/// # Panics
///
/// Debug builds panic if `ciphertext` is shorter than [`TAG_LENGTH`] bytes.
pub fn decrypt(
    key: &EncryptionKey,
    nonce: &Nonce,
    ciphertext: &Ciphertext,
) -> Result<Vec<u8>, CryptoError> {
    // Precondition: ciphertext has at least the auth tag
    let ciphertext_len = ciphertext.data.len();
    debug_assert!(
        ciphertext_len >= TAG_LENGTH,
        "ciphertext too short: {ciphertext_len} bytes, need at least {TAG_LENGTH}"
    );

    let cipher = Aes256Gcm::new_from_slice(&key.key).expect("KEY_LENGTH is always valid");
    let nonce_array = nonce.bytes.into();

    let plaintext = cipher
        .decrypt(&nonce_array, ciphertext.data.as_slice())
        .map_err(|_| CryptoError::DecryptionError)?;

    // Postcondition: plaintext is ciphertext minus tag
    debug_assert_eq!(
        plaintext.len(),
        ciphertext.data.len() - TAG_LENGTH,
        "plaintext length mismatch"
    );

    Ok(plaintext)
}

// ============================================================================
// Internal Helpers
// ============================================================================

/// Fills a buffer with cryptographically secure random bytes.
///
/// # Panics
///
/// Panics if the OS CSPRNG fails. This indicates a catastrophic system error
/// (e.g., /dev/urandom unavailable) and cannot be meaningfully recovered from.
fn generate_random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("CSPRNG failure");
    bytes
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(42);
        let plaintext = b"sensitive tenant data";

        let ciphertext = encrypt(&key, &nonce, plaintext);
        let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encrypt_decrypt_empty_plaintext() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(0);
        let plaintext = b"";

        let ciphertext = encrypt(&key, &nonce, plaintext);
        let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
        assert_eq!(ciphertext.len(), TAG_LENGTH); // Just the tag
    }

    #[test]
    fn ciphertext_length_is_plaintext_plus_tag() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(100);
        let plaintext = b"hello world";

        let ciphertext = encrypt(&key, &nonce, plaintext);

        assert_eq!(ciphertext.len(), plaintext.len() + TAG_LENGTH);
    }

    #[test]
    fn wrong_key_fails_decryption() {
        let key1 = EncryptionKey::generate();
        let key2 = EncryptionKey::generate();
        let nonce = Nonce::from_position(42);
        let plaintext = b"secret message";

        let ciphertext = encrypt(&key1, &nonce, plaintext);
        let result = decrypt(&key2, &nonce, &ciphertext);

        assert!(result.is_err());
    }

    #[test]
    fn wrong_nonce_fails_decryption() {
        let key = EncryptionKey::generate();
        let nonce1 = Nonce::from_position(42);
        let nonce2 = Nonce::from_position(43);
        let plaintext = b"secret message";

        let ciphertext = encrypt(&key, &nonce1, plaintext);
        let result = decrypt(&key, &nonce2, &ciphertext);

        assert!(result.is_err());
    }

    #[test]
    fn tampered_ciphertext_fails_decryption() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(42);
        let plaintext = b"secret message";

        let ciphertext = encrypt(&key, &nonce, plaintext);

        // Tamper with the ciphertext
        let mut tampered_bytes = ciphertext.to_bytes().to_vec();
        tampered_bytes[0] ^= 0x01; // Flip a bit
        let tampered = Ciphertext::from_bytes(tampered_bytes);

        let result = decrypt(&key, &nonce, &tampered);

        assert!(result.is_err());
    }

    #[test]
    fn tampered_tag_fails_decryption() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(42);
        let plaintext = b"secret message";

        let ciphertext = encrypt(&key, &nonce, plaintext);

        // Tamper with the auth tag (last 16 bytes)
        let mut tampered_bytes = ciphertext.to_bytes().to_vec();
        let len = tampered_bytes.len();
        tampered_bytes[len - 1] ^= 0x01; // Flip a bit in the tag
        let tampered = Ciphertext::from_bytes(tampered_bytes);

        let result = decrypt(&key, &nonce, &tampered);

        assert!(result.is_err());
    }

    #[test]
    fn nonce_from_position_layout() {
        let nonce = Nonce::from_position(0x0102_0304_0506_0708);
        let bytes = nonce.to_bytes();

        // Little-endian: least significant byte first
        assert_eq!(bytes[0], 0x08);
        assert_eq!(bytes[1], 0x07);
        assert_eq!(bytes[2], 0x06);
        assert_eq!(bytes[3], 0x05);
        assert_eq!(bytes[4], 0x04);
        assert_eq!(bytes[5], 0x03);
        assert_eq!(bytes[6], 0x02);
        assert_eq!(bytes[7], 0x01);

        // Reserved bytes are zero
        assert_eq!(bytes[8], 0x00);
        assert_eq!(bytes[9], 0x00);
        assert_eq!(bytes[10], 0x00);
        assert_eq!(bytes[11], 0x00);
    }

    #[test]
    fn nonce_position_zero_is_valid() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(0);
        let plaintext = b"first record";

        let ciphertext = encrypt(&key, &nonce, plaintext);
        let decrypted = decrypt(&key, &nonce, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn encryption_key_roundtrip() {
        let original = EncryptionKey::generate();
        let bytes = original.to_bytes();
        let restored = EncryptionKey::from_bytes(&bytes);

        // Same key should produce same ciphertext
        let nonce = Nonce::from_position(1);
        let plaintext = b"test";

        let ct1 = encrypt(&original, &nonce, plaintext);
        let ct2 = encrypt(&restored, &nonce, plaintext);

        assert_eq!(ct1.to_bytes(), ct2.to_bytes());
    }

    #[test]
    fn ciphertext_roundtrip() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(999);
        let plaintext = b"data to serialize";

        let ciphertext = encrypt(&key, &nonce, plaintext);

        // Simulate serialization/deserialization
        let bytes = ciphertext.to_bytes().to_vec();
        let restored = Ciphertext::from_bytes(bytes);

        let decrypted = decrypt(&key, &nonce, &restored).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn different_positions_produce_different_ciphertexts() {
        let key = EncryptionKey::generate();
        let plaintext = b"same plaintext";

        let ct1 = encrypt(&key, &Nonce::from_position(1), plaintext);
        let ct2 = encrypt(&key, &Nonce::from_position(2), plaintext);

        // Same plaintext, different nonces = different ciphertexts
        assert_ne!(ct1.to_bytes(), ct2.to_bytes());
    }

    #[test]
    fn encryption_is_deterministic() {
        let key = EncryptionKey::generate();
        let nonce = Nonce::from_position(42);
        let plaintext = b"deterministic test";

        let ct1 = encrypt(&key, &nonce, plaintext);
        let ct2 = encrypt(&key, &nonce, plaintext);

        // Same key + nonce + plaintext = same ciphertext
        assert_eq!(ct1.to_bytes(), ct2.to_bytes());
    }
}
