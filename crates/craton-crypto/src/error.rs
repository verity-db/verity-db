//! Error types for cryptographic operations.

use ed25519_dalek::SignatureError;

/// Errors that can occur during cryptographic operations.
#[derive(thiserror::Error, Debug)]
pub enum CryptoError {
    /// Ed25519 signature verification failed.
    #[error(transparent)]
    SignatureError(#[from] SignatureError),

    /// AES-256-GCM decryption failed.
    ///
    /// This occurs when:
    /// - The key is incorrect
    /// - The nonce is incorrect
    /// - The ciphertext has been tampered with
    /// - The authentication tag is invalid
    #[error("decryption failed: authentication tag mismatch or corrupted ciphertext")]
    DecryptionError,
}
