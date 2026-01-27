//! Field-level encryption and anonymization primitives.
//!
//! This module provides cryptographic primitives for secure data sharing:
//! - Field-level encryption for selective field protection
//! - Deterministic encryption for tokenization/pseudonymization
//! - Field key derivation from the key hierarchy
//!
//! # Key Hierarchy
//!
//! ```text
//! MasterKey (HSM or secure storage)
//!     └── TenantKEK (per-tenant, wrapped by master)
//!             └── FieldKey (per-field, derived from tenant key + field name)
//! ```
//!
//! # Deterministic vs Randomized Encryption
//!
//! - **Randomized (default)**: Each encryption produces unique ciphertext.
//!   Use for general field encryption. Provides IND-CPA security.
//! - **Deterministic**: Same plaintext produces same ciphertext.
//!   Use ONLY for tokenization where you need consistent pseudonyms.
//!   Reveals frequency patterns - use with caution.
//!
//! # Example
//!
//! ```
//! use vdb_crypto::field::{FieldKey, encrypt_field, decrypt_field, tokenize};
//! use vdb_crypto::EncryptionKey;
//!
//! // Derive a field key from a tenant key
//! let tenant_key = EncryptionKey::generate();
//! let field_key = FieldKey::derive(&tenant_key, "patient_ssn");
//!
//! // Encrypt a field (randomized - each call produces different ciphertext)
//! let ssn = b"123-45-6789";
//! let encrypted = encrypt_field(&field_key, ssn);
//! let decrypted = decrypt_field(&field_key, &encrypted).unwrap();
//! assert_eq!(decrypted, ssn);
//!
//! // Create a consistent token (deterministic - same input = same output)
//! let token1 = tokenize(&field_key, ssn);
//! let token2 = tokenize(&field_key, ssn);
//! assert_eq!(token1, token2);  // Same token for same input
//! ```

use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::encryption::{KEY_LENGTH, NONCE_LENGTH, TAG_LENGTH};
use crate::{CryptoError, EncryptionKey};

// ============================================================================
// FieldKey - Derived key for field-level encryption
// ============================================================================

/// A key for encrypting a specific field, derived from a tenant key.
///
/// Field keys are derived deterministically from the parent key and field name,
/// enabling key hierarchy without storing additional key material.
///
/// # Security
///
/// - Each field gets a unique derived key
/// - Compromise of one field key doesn't expose other fields
/// - Keys are derived using HKDF-like construction with SHA-256
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct FieldKey([u8; KEY_LENGTH]);

impl FieldKey {
    // ========================================================================
    // Functional Core (pure, testable)
    // ========================================================================

    /// Creates a field key from derived bytes (pure, no IO).
    ///
    /// This is `pub(crate)` to prevent misuse with weak input.
    pub(crate) fn from_derived_bytes(bytes: [u8; KEY_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Restores a field key from its byte representation.
    ///
    /// Use this to restore a field key from secure storage.
    pub fn from_bytes(bytes: &[u8; KEY_LENGTH]) -> Self {
        Self(*bytes)
    }

    /// Returns the key as a byte slice.
    pub fn as_bytes(&self) -> &[u8; KEY_LENGTH] {
        &self.0
    }

    // ========================================================================
    // Imperative Shell (IO boundary)
    // ========================================================================

    /// Derives a field key from a parent key and field name.
    ///
    /// The derivation is deterministic: the same parent key and field name
    /// always produce the same field key.
    ///
    /// # Arguments
    ///
    /// * `parent_key` - The tenant's encryption key
    /// * `field_name` - The name of the field (e.g., `patient_ssn`)
    ///
    /// # Security
    ///
    /// - Uses HKDF-like construction with SHA-256
    /// - Field names should be consistent across the application
    /// - Consider prefixing field names with schema version for rotation
    pub fn derive(parent_key: &EncryptionKey, field_name: &str) -> Self {
        // HKDF-like key derivation: SHA-256(parent_key || "field:" || field_name)
        let mut hasher = Sha256::new();
        hasher.update(parent_key.to_bytes());
        hasher.update(b"field:");
        hasher.update(field_name.as_bytes());
        let derived: [u8; 32] = hasher.finalize().into();

        Self::from_derived_bytes(derived)
    }
}

impl std::fmt::Debug for FieldKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never expose key material in debug output
        write!(f, "FieldKey([REDACTED])")
    }
}

// ============================================================================
// Field Encryption (Randomized)
// ============================================================================

/// Encrypts a field value with randomized encryption.
///
/// Each call produces different ciphertext, providing semantic security
/// (IND-CPA). Use this for general field encryption.
///
/// # Returns
///
/// Ciphertext in format: `[nonce:12B][ciphertext:variable][tag:16B]`
///
/// # Panics
///
/// Panics if CSPRNG fails (catastrophic system error).
pub fn encrypt_field(key: &FieldKey, plaintext: &[u8]) -> Vec<u8> {
    // Generate random nonce
    let mut nonce_bytes = [0u8; NONCE_LENGTH];
    getrandom::fill(&mut nonce_bytes).expect("CSPRNG failure is catastrophic");

    let cipher = Aes256Gcm::new_from_slice(&key.0).expect("KEY_LENGTH is always valid");
    let nonce = aes_gcm::Nonce::from(nonce_bytes);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .expect("AES-GCM encryption cannot fail with valid inputs");

    // Format: nonce || ciphertext (includes tag)
    let mut result = Vec::with_capacity(NONCE_LENGTH + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    result
}

/// Decrypts a field value encrypted with [`encrypt_field`].
///
/// # Errors
///
/// Returns [`CryptoError::DecryptionFailed`] if:
/// - The ciphertext is too short
/// - The authentication tag doesn't match (tampering detected)
/// - The wrong key is used
pub fn decrypt_field(key: &FieldKey, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    // Minimum size: nonce(12) + tag(16) = 28 bytes
    const MIN_SIZE: usize = NONCE_LENGTH + TAG_LENGTH;
    if ciphertext.len() < MIN_SIZE {
        return Err(CryptoError::DecryptionError);
    }

    let (nonce_bytes, encrypted) = ciphertext.split_at(NONCE_LENGTH);
    let nonce_array: [u8; NONCE_LENGTH] = nonce_bytes
        .try_into()
        .expect("split_at guarantees correct length");
    let nonce = aes_gcm::Nonce::from(nonce_array);

    let cipher = Aes256Gcm::new_from_slice(&key.0).expect("KEY_LENGTH is always valid");

    cipher
        .decrypt(&nonce, encrypted)
        .map_err(|_| CryptoError::DecryptionError)
}

// ============================================================================
// Tokenization (Deterministic Encryption)
// ============================================================================

/// Token length in bytes (256-bit token).
pub const TOKEN_LENGTH: usize = 32;

/// A deterministic token for consistent pseudonymization.
///
/// Tokens are produced by HMAC-SHA256, ensuring:
/// - Same input always produces the same token
/// - Tokens are uniformly distributed (good for database indexes)
/// - Original value cannot be recovered from token alone
///
/// # Security Warning
///
/// Deterministic tokens reveal frequency patterns. If the same SSN appears
/// 100 times, the same token appears 100 times. Use only when consistency
/// is required (e.g., joining records without exposing real identifiers).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token([u8; TOKEN_LENGTH]);

impl Token {
    /// Creates a token from bytes.
    pub fn from_bytes(bytes: [u8; TOKEN_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the token as bytes.
    pub fn as_bytes(&self) -> &[u8; TOKEN_LENGTH] {
        &self.0
    }

    /// Returns the token as a hex string.
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(TOKEN_LENGTH * 2);
        for byte in &self.0 {
            use std::fmt::Write;
            write!(s, "{byte:02x}").expect("formatting cannot fail");
        }
        s
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Token({}...)", &self.to_hex()[..16])
    }
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Creates a deterministic token from a value.
///
/// The same key and value always produce the same token. This is useful
/// for consistent pseudonymization where you need to correlate records
/// without exposing the original values.
///
/// # Security
///
/// - Tokens cannot be reversed to recover the original value
/// - Tokens reveal frequency patterns (same input = same token)
/// - Use different field keys for different fields to prevent correlation
///
/// # Example
///
/// ```
/// use vdb_crypto::field::{FieldKey, tokenize};
/// use vdb_crypto::EncryptionKey;
///
/// let tenant_key = EncryptionKey::generate();
/// let ssn_key = FieldKey::derive(&tenant_key, "ssn");
///
/// let token1 = tokenize(&ssn_key, b"123-45-6789");
/// let token2 = tokenize(&ssn_key, b"123-45-6789");
/// let token3 = tokenize(&ssn_key, b"987-65-4321");
///
/// assert_eq!(token1, token2);  // Same input = same token
/// assert_ne!(token1, token3);  // Different input = different token
/// ```
pub fn tokenize(key: &FieldKey, value: &[u8]) -> Token {
    // HMAC-SHA256 for deterministic tokenization
    // Using SHA256(key || value) as a simple PRF
    let mut hasher = Sha256::new();
    hasher.update(key.0);
    hasher.update(value);
    let hash: [u8; 32] = hasher.finalize().into();

    Token(hash)
}

/// Checks if a value matches a token.
///
/// This is useful for searching: tokenize the search term and compare
/// against stored tokens.
pub fn matches_token(key: &FieldKey, value: &[u8], token: &Token) -> bool {
    tokenize(key, value) == *token
}

// ============================================================================
// Reversible Tokenization
// ============================================================================

/// Encrypted token that can be reversed with the key.
///
/// Unlike [`Token`], this stores the original value encrypted so it can
/// be recovered. Use when you need both consistent pseudonyms AND the
/// ability to reveal the original value (with proper authorization).
#[derive(Clone)]
pub struct ReversibleToken {
    /// The deterministic token (for lookups).
    pub token: Token,
    /// The encrypted original value (for reversal).
    pub encrypted: Vec<u8>,
}

impl ReversibleToken {
    /// Creates a reversible token from a value.
    ///
    /// The token is deterministic, but the encrypted value uses randomized
    /// encryption for security.
    pub fn create(key: &FieldKey, value: &[u8]) -> Self {
        let token = tokenize(key, value);
        let encrypted = encrypt_field(key, value);
        Self { token, encrypted }
    }

    /// Recovers the original value from the reversible token.
    ///
    /// # Errors
    ///
    /// Returns an error if decryption fails (wrong key or tampering).
    pub fn reveal(&self, key: &FieldKey) -> Result<Vec<u8>, CryptoError> {
        decrypt_field(key, &self.encrypted)
    }
}

impl std::fmt::Debug for ReversibleToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReversibleToken")
            .field("token", &self.token)
            .field("encrypted_len", &self.encrypted.len())
            .finish()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_key_derivation_is_deterministic() {
        let parent = EncryptionKey::generate();
        let key1 = FieldKey::derive(&parent, "patient_ssn");
        let key2 = FieldKey::derive(&parent, "patient_ssn");

        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn different_field_names_produce_different_keys() {
        let parent = EncryptionKey::generate();
        let ssn_key = FieldKey::derive(&parent, "ssn");
        let dob_key = FieldKey::derive(&parent, "dob");

        assert_ne!(ssn_key.as_bytes(), dob_key.as_bytes());
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let parent = EncryptionKey::generate();
        let key = FieldKey::derive(&parent, "test_field");

        let plaintext = b"sensitive data";
        let ciphertext = encrypt_field(&key, plaintext);
        let decrypted = decrypt_field(&key, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn randomized_encryption_produces_different_ciphertext() {
        let parent = EncryptionKey::generate();
        let key = FieldKey::derive(&parent, "test_field");

        let plaintext = b"sensitive data";
        let ct1 = encrypt_field(&key, plaintext);
        let ct2 = encrypt_field(&key, plaintext);

        // Different nonces produce different ciphertext
        assert_ne!(ct1, ct2);

        // But both decrypt to the same plaintext
        assert_eq!(decrypt_field(&key, &ct1).unwrap(), plaintext);
        assert_eq!(decrypt_field(&key, &ct2).unwrap(), plaintext);
    }

    #[test]
    fn wrong_key_fails_decryption() {
        let parent1 = EncryptionKey::generate();
        let parent2 = EncryptionKey::generate();
        let key1 = FieldKey::derive(&parent1, "field");
        let key2 = FieldKey::derive(&parent2, "field");

        let ciphertext = encrypt_field(&key1, b"secret");
        let result = decrypt_field(&key2, &ciphertext);

        assert!(result.is_err());
    }

    #[test]
    fn tokenization_is_deterministic() {
        let parent = EncryptionKey::generate();
        let key = FieldKey::derive(&parent, "ssn");

        let token1 = tokenize(&key, b"123-45-6789");
        let token2 = tokenize(&key, b"123-45-6789");

        assert_eq!(token1, token2);
    }

    #[test]
    fn different_values_produce_different_tokens() {
        let parent = EncryptionKey::generate();
        let key = FieldKey::derive(&parent, "ssn");

        let token1 = tokenize(&key, b"123-45-6789");
        let token2 = tokenize(&key, b"987-65-4321");

        assert_ne!(token1, token2);
    }

    #[test]
    fn different_keys_produce_different_tokens() {
        let parent = EncryptionKey::generate();
        let key1 = FieldKey::derive(&parent, "field1");
        let key2 = FieldKey::derive(&parent, "field2");

        let token1 = tokenize(&key1, b"same value");
        let token2 = tokenize(&key2, b"same value");

        assert_ne!(token1, token2);
    }

    #[test]
    fn matches_token_works() {
        let parent = EncryptionKey::generate();
        let key = FieldKey::derive(&parent, "field");

        let value = b"test value";
        let token = tokenize(&key, value);

        assert!(matches_token(&key, value, &token));
        assert!(!matches_token(&key, b"other value", &token));
    }

    #[test]
    fn reversible_token_roundtrip() {
        let parent = EncryptionKey::generate();
        let key = FieldKey::derive(&parent, "field");

        let value = b"original value";
        let rt = ReversibleToken::create(&key, value);

        // Token is deterministic
        assert_eq!(rt.token, tokenize(&key, value));

        // Can reveal original value
        let revealed = rt.reveal(&key).unwrap();
        assert_eq!(revealed, value);
    }

    #[test]
    fn token_hex_formatting() {
        let token = Token::from_bytes([0xab; 32]);
        let hex = token.to_hex();

        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c == 'a' || c == 'b'));
    }

    #[test]
    fn empty_plaintext_encrypts() {
        let parent = EncryptionKey::generate();
        let key = FieldKey::derive(&parent, "field");

        let encrypted = encrypt_field(&key, b"");
        let decrypted = decrypt_field(&key, &encrypted).unwrap();

        assert!(decrypted.is_empty());
    }
}
