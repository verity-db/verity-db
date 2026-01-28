//! Ed25519 digital signatures for record non-repudiation.
//!
//! Provides thin, type-safe wrappers around `ed25519-dalek` for signing
//! and verifying records. Each record in the audit log can be signed to
//! prove authorship and detect tampering.
//!
//! # Example
//!
//! ```
//! use craton_crypto::signature::{SigningKey, Signature};
//!
//! // Generate a new signing key
//! let signing_key = SigningKey::generate();
//! let verifying_key = signing_key.verifying_key();
//!
//! // Sign a message
//! let message = b"audit record data";
//! let signature = signing_key.sign(message);
//!
//! // Verify the signature
//! assert!(verifying_key.verify(message, &signature).is_ok());
//!
//! // Tampered messages fail verification
//! let tampered = b"tampered data";
//! assert!(verifying_key.verify(tampered, &signature).is_err());
//! ```

use ed25519_dalek::{PUBLIC_KEY_LENGTH, SECRET_KEY_LENGTH, SIGNATURE_LENGTH, Signer};
use rand::rngs::OsRng;
use zeroize::ZeroizeOnDrop;

use crate::error::CryptoError;

// ============================================================================
// Constants
// ============================================================================

/// Length of a signing key in bytes (32 bytes).
pub const SIGNING_KEY_LENGTH: usize = SECRET_KEY_LENGTH;

/// Length of a verifying key in bytes (32 bytes).
pub const VERIFYING_KEY_LENGTH: usize = PUBLIC_KEY_LENGTH;

// ============================================================================
// SigningKey
// ============================================================================

/// An Ed25519 signing key for creating digital signatures.
///
/// This is the secret key that must be kept confidential. Use [`SigningKey::generate`]
/// to create a new random key, or [`SigningKey::from_bytes`] to restore from storage.
///
/// # Security
///
/// - Never log or expose the key bytes
/// - Store encrypted at rest
/// - Use one key per identity/purpose
#[derive(Clone, ZeroizeOnDrop)]
pub struct SigningKey(ed25519_dalek::SigningKey);

impl SigningKey {
    // ========================================================================
    // Functional Core (pure, testable)
    // ========================================================================

    /// Creates a signing key from random bytes (pure, no IO).
    ///
    /// This is the functional core - it performs no IO and is fully testable.
    /// Use [`Self::generate`] for the public API that handles randomness.
    ///
    /// # Security
    ///
    /// Only use bytes from a CSPRNG. This is `pub(crate)` to prevent misuse.
    pub(crate) fn from_random_bytes(bytes: [u8; SIGNING_KEY_LENGTH]) -> Self {
        // Precondition: caller provided non-degenerate random bytes
        debug_assert!(bytes.iter().any(|&b| b != 0), "random bytes are all zeros");

        Self(ed25519_dalek::SigningKey::from_bytes(&bytes))
    }

    /// Restores a signing key from its 32-byte representation.
    ///
    /// # Note
    ///
    /// Any 32 bytes form a valid Ed25519 secret key, so this cannot fail.
    /// However, you should only use bytes from a previously generated key.
    pub fn from_bytes(bytes: &[u8; SIGNING_KEY_LENGTH]) -> Self {
        Self(ed25519_dalek::SigningKey::from_bytes(bytes))
    }

    // ========================================================================
    // Imperative Shell (IO boundary)
    // ========================================================================

    /// Generates a new random signing key using the OS CSPRNG.
    ///
    /// This is the imperative shell - it handles IO (randomness) and delegates
    /// to the pure [`Self::from_random_bytes`] for the actual construction.
    ///
    /// # Panics
    ///
    /// Panics if the OS CSPRNG fails (should never happen on supported platforms).
    pub fn generate() -> Self {
        let mut csprng = OsRng;
        let key = ed25519_dalek::SigningKey::generate(&mut csprng);
        Self::from_random_bytes(key.to_bytes())
    }

    /// Returns the corresponding public key for signature verification.
    ///
    /// The verifying key can be shared publicly and used by others to verify
    /// signatures created with this signing key.
    pub fn verifying_key(&self) -> VerifyingKey {
        VerifyingKey(self.0.verifying_key())
    }

    /// Signs a message, producing a 64-byte signature.
    ///
    /// The signature proves that the holder of this signing key authored
    /// the message. Anyone with the corresponding [`VerifyingKey`] can
    /// verify the signature.
    ///
    /// # Arguments
    ///
    /// * `message` - The data to sign (typically a serialized record)
    ///
    /// # Returns
    ///
    /// A [`Signature`] that can be verified with [`VerifyingKey::verify`].
    pub fn sign(&self, message: &[u8]) -> Signature {
        // Precondition: message length is reasonable for signing
        // (Ed25519 can sign any length, but we assert it's not absurdly large)
        debug_assert!(
            message.len() <= 64 * 1024 * 1024,
            "message exceeds 64MB sanity limit"
        );

        let signature = self.0.sign(message);

        // Postcondition: signature has correct length and isn't degenerate
        let sig_bytes = signature.to_bytes();
        debug_assert_eq!(sig_bytes.len(), SIGNATURE_LENGTH);
        debug_assert!(
            sig_bytes.iter().any(|&b| b != 0),
            "signature is all zeros, indicating a bug"
        );

        Signature(signature)
    }

    /// Returns the raw 32-byte key material.
    ///
    /// # Security
    ///
    /// Handle with care — this is secret key material.
    pub fn to_bytes(&self) -> [u8; SIGNING_KEY_LENGTH] {
        self.0.to_bytes()
    }
}

// ============================================================================
// VerifyingKey
// ============================================================================

/// An Ed25519 public key for verifying digital signatures.
///
/// This is the public key that can be freely shared. It can verify signatures
/// created by the corresponding [`SigningKey`] but cannot create new signatures.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct VerifyingKey(ed25519_dalek::VerifyingKey);

impl VerifyingKey {
    /// Parses a verifying key from its 32-byte representation.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::SignatureError`] if the bytes don't represent
    /// a valid Ed25519 public key point.
    pub fn from_bytes(bytes: &[u8; VERIFYING_KEY_LENGTH]) -> Result<Self, CryptoError> {
        // Precondition: bytes aren't all zeros (common mistake)
        debug_assert!(
            bytes.iter().any(|&b| b != 0),
            "verifying key bytes are all zeros"
        );

        let key = ed25519_dalek::VerifyingKey::from_bytes(bytes)?;
        Ok(Self(key))
    }

    /// Verifies a signature against a message.
    ///
    /// Uses strict verification which rejects non-canonical signatures,
    /// providing stronger security guarantees.
    ///
    /// # Arguments
    ///
    /// * `message` - The original message that was signed
    /// * `signature` - The signature to verify
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError::SignatureError`] if the signature is invalid.
    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<(), CryptoError> {
        // Precondition: message length matches signing expectations
        debug_assert!(
            message.len() <= 64 * 1024 * 1024,
            "message exceeds 64MB sanity limit"
        );

        self.0.verify_strict(message, &signature.0)?;
        Ok(())
    }

    /// Returns the raw 32-byte public key.
    pub fn to_bytes(&self) -> [u8; VERIFYING_KEY_LENGTH] {
        self.0.to_bytes()
    }
}

impl std::fmt::Debug for VerifyingKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bytes = self.to_bytes();
        write!(
            f,
            "VerifyingKey({:02x}{:02x}{:02x}{:02x}...)",
            bytes[0], bytes[1], bytes[2], bytes[3]
        )
    }
}

// ============================================================================
// Signature
// ============================================================================

/// An Ed25519 signature (64 bytes).
///
/// A signature proves that a message was signed by the holder of a particular
/// signing key. Use [`VerifyingKey::verify`] to validate signatures.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature(ed25519_dalek::Signature);

impl Signature {
    /// Parses a signature from its 64-byte representation.
    ///
    /// # Note
    ///
    /// This only checks structural validity (correct length). The signature
    /// may still fail cryptographic verification — use [`VerifyingKey::verify`]
    /// to validate against a message.
    pub fn from_bytes(bytes: &[u8; SIGNATURE_LENGTH]) -> Self {
        // Precondition: signature bytes aren't all zeros (likely bug)
        debug_assert!(
            bytes.iter().any(|&b| b != 0),
            "signature bytes are all zeros"
        );

        Self(ed25519_dalek::Signature::from_bytes(bytes))
    }

    /// Returns the raw 64-byte signature.
    pub fn to_bytes(&self) -> [u8; SIGNATURE_LENGTH] {
        self.0.to_bytes()
    }
}

impl std::fmt::Debug for Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bytes = self.to_bytes();
        write!(
            f,
            "Signature({:02x}{:02x}{:02x}{:02x}...)",
            bytes[0], bytes[1], bytes[2], bytes[3]
        )
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let message = b"test message for signing";
        let signature = signing_key.sign(message);

        // Verification should succeed
        assert!(verifying_key.verify(message, &signature).is_ok());
    }

    #[test]
    fn tampered_message_fails_verification() {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let message = b"original message";
        let signature = signing_key.sign(message);

        // Tampered message should fail
        let tampered = b"tampered message";
        assert!(verifying_key.verify(tampered, &signature).is_err());
    }

    #[test]
    fn tampered_signature_fails_verification() {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let message = b"test message";
        let signature = signing_key.sign(message);

        // Flip a bit in the signature
        let mut bad_sig_bytes = signature.to_bytes();
        bad_sig_bytes[0] ^= 0x01;
        let bad_signature = Signature::from_bytes(&bad_sig_bytes);

        assert!(verifying_key.verify(message, &bad_signature).is_err());
    }

    #[test]
    fn wrong_key_fails_verification() {
        let signing_key = SigningKey::generate();
        let wrong_key = SigningKey::generate();

        let message = b"test message";
        let signature = signing_key.sign(message);

        // Different key should fail
        assert!(
            wrong_key
                .verifying_key()
                .verify(message, &signature)
                .is_err()
        );
    }

    #[test]
    fn signing_key_roundtrip() {
        let original = SigningKey::generate();
        let bytes = original.to_bytes();
        let restored = SigningKey::from_bytes(&bytes);

        // Should produce same verifying key
        assert_eq!(
            original.verifying_key().to_bytes(),
            restored.verifying_key().to_bytes()
        );

        // Should produce same signatures
        let message = b"test";
        let sig1 = original.sign(message);
        let sig2 = restored.sign(message);
        assert_eq!(sig1.to_bytes(), sig2.to_bytes());
    }

    #[test]
    fn verifying_key_roundtrip() {
        let signing_key = SigningKey::generate();
        let original = signing_key.verifying_key();

        let bytes = original.to_bytes();
        let restored = VerifyingKey::from_bytes(&bytes).unwrap();

        assert_eq!(original, restored);
    }

    #[test]
    fn signature_roundtrip() {
        let signing_key = SigningKey::generate();
        let signature = signing_key.sign(b"test");

        let bytes = signature.to_bytes();
        let restored = Signature::from_bytes(&bytes);

        assert_eq!(signature, restored);
    }

    #[test]
    fn empty_message_can_be_signed() {
        let signing_key = SigningKey::generate();
        let verifying_key = signing_key.verifying_key();

        let signature = signing_key.sign(b"");
        assert!(verifying_key.verify(b"", &signature).is_ok());
    }

    #[test]
    fn deterministic_signatures() {
        // Ed25519 signatures are deterministic (same key + message = same signature)
        let signing_key = SigningKey::generate();
        let message = b"determinism test";

        let sig1 = signing_key.sign(message);
        let sig2 = signing_key.sign(message);

        assert_eq!(sig1.to_bytes(), sig2.to_bytes());
    }
}
